//! Camera capture modes. A mode is one capability: pixel format, frame size
//! and frame interval travel together and are never matched independently.

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModeError {
    #[error("pixel format must be exactly 4 ASCII characters, got {0:?}")]
    InvalidPixelFormat(String),
    #[error("frame size must be non-zero, got {width}x{height}")]
    InvalidSize { width: u32, height: u32 },
    #[error("frame interval must be non-zero, got {numerator}/{denominator}")]
    InvalidInterval { numerator: u32, denominator: u32 },
    #[error("mode {requested} is not supported by this camera")]
    Unsupported {
        requested: CaptureMode,
        /// Supported modes with the same pixel format and size, for the message only.
        alternatives: Vec<CaptureMode>,
    },
}

/// V4L2-style FourCC pixel format such as `MJPG` or `YUYV`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PixelFormat([u8; 4]);

impl PixelFormat {
    pub fn parse(text: &str) -> Result<Self, ModeError> {
        let bytes: [u8; 4] = text
            .as_bytes()
            .try_into()
            .map_err(|_| ModeError::InvalidPixelFormat(text.to_owned()))?;
        if bytes.iter().all(u8::is_ascii_graphic) {
            Ok(Self(bytes))
        } else {
            Err(ModeError::InvalidPixelFormat(text.to_owned()))
        }
    }
}

impl PixelFormat {
    /// The raw FourCC bytes, for handing to the driver.
    pub fn bytes(&self) -> [u8; 4] {
        self.0
    }
}

impl TryFrom<String> for PixelFormat {
    type Error = ModeError;
    fn try_from(value: String) -> Result<Self, ModeError> {
        Self::parse(&value)
    }
}

impl From<PixelFormat> for String {
    fn from(value: PixelFormat) -> String {
        value.to_string()
    }
}

impl fmt::Display for PixelFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0
            .iter()
            .try_for_each(|byte| write!(f, "{}", char::from(*byte)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "RawFrameSize")]
pub struct FrameSize {
    pub width: u32,
    pub height: u32,
}

/// Unvalidated wire shape; deserialization goes through `FrameSize::new`.
#[derive(Deserialize)]
struct RawFrameSize {
    width: u32,
    height: u32,
}

impl TryFrom<RawFrameSize> for FrameSize {
    type Error = ModeError;
    fn try_from(raw: RawFrameSize) -> Result<Self, ModeError> {
        Self::new(raw.width, raw.height)
    }
}

impl FrameSize {
    pub fn new(width: u32, height: u32) -> Result<Self, ModeError> {
        if width == 0 || height == 0 {
            return Err(ModeError::InvalidSize { width, height });
        }
        Ok(Self { width, height })
    }
}

/// Seconds per frame as a reduced fraction, so `2/60` and `1/30` are equal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "RawFrameInterval")]
pub struct FrameInterval {
    numerator: u32,
    denominator: u32,
}

/// Unvalidated wire shape; deserialization goes through `FrameInterval::new`.
#[derive(Deserialize)]
struct RawFrameInterval {
    numerator: u32,
    denominator: u32,
}

impl TryFrom<RawFrameInterval> for FrameInterval {
    type Error = ModeError;
    fn try_from(raw: RawFrameInterval) -> Result<Self, ModeError> {
        Self::new(raw.numerator, raw.denominator)
    }
}

impl FrameInterval {
    pub fn new(numerator: u32, denominator: u32) -> Result<Self, ModeError> {
        if numerator == 0 || denominator == 0 {
            return Err(ModeError::InvalidInterval {
                numerator,
                denominator,
            });
        }
        let divisor = gcd(numerator, denominator);
        Ok(Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        })
    }

    pub fn numerator(&self) -> u32 {
        self.numerator
    }

    pub fn denominator(&self) -> u32 {
        self.denominator
    }

    pub fn fps(&self) -> f64 {
        f64::from(self.denominator) / f64::from(self.numerator)
    }
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CaptureMode {
    pub format: PixelFormat,
    pub size: FrameSize,
    pub interval: FrameInterval,
}

impl fmt::Display for CaptureMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {}x{} @ {:.2} fps",
            self.format,
            self.size.width,
            self.size.height,
            self.interval.fps()
        )
    }
}

/// What the camera actually advertises. Built from enumerated modes only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraCapabilities {
    modes: Vec<CaptureMode>,
}

impl CameraCapabilities {
    pub fn new(modes: impl IntoIterator<Item = CaptureMode>) -> Self {
        let mut modes: Vec<CaptureMode> = modes.into_iter().collect();
        modes.sort();
        modes.dedup();
        Self { modes }
    }

    pub fn modes(&self) -> &[CaptureMode] {
        &self.modes
    }

    /// Accepts only an exact advertised mode. No fallback, no rounding.
    pub fn validate(&self, requested: &CaptureMode) -> Result<CaptureMode, ModeError> {
        if self.modes.contains(requested) {
            return Ok(*requested);
        }
        let alternatives = self
            .modes
            .iter()
            .filter(|mode| mode.format == requested.format && mode.size == requested.size)
            .copied()
            .collect();
        Err(ModeError::Unsupported {
            requested: *requested,
            alternatives,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ModeField {
    Format,
    Size,
    Interval,
}

/// Requested versus negotiated mode, reported separately.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NegotiationReport {
    pub requested: CaptureMode,
    pub negotiated: CaptureMode,
    pub differences: Vec<ModeField>,
}

impl NegotiationReport {
    pub fn new(requested: CaptureMode, negotiated: CaptureMode) -> Self {
        let differences = [
            (ModeField::Format, requested.format != negotiated.format),
            (ModeField::Size, requested.size != negotiated.size),
            (
                ModeField::Interval,
                requested.interval != negotiated.interval,
            ),
        ]
        .into_iter()
        .filter_map(|(field, differs)| differs.then_some(field))
        .collect();
        Self {
            requested,
            negotiated,
            differences,
        }
    }

    pub fn is_exact(&self) -> bool {
        self.differences.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(fourcc: &str, w: u32, h: u32, num: u32, den: u32) -> CaptureMode {
        CaptureMode {
            format: PixelFormat::parse(fourcc).unwrap(),
            size: FrameSize::new(w, h).unwrap(),
            interval: FrameInterval::new(num, den).unwrap(),
        }
    }

    fn samsung_like() -> CameraCapabilities {
        CameraCapabilities::new([
            mode("YUYV", 640, 480, 1, 30),
            mode("MJPG", 1280, 720, 1, 30),
            mode("MJPG", 1280, 720, 1, 15),
        ])
    }

    #[test]
    fn pixel_format_roundtrips_four_ascii_characters() {
        let format = PixelFormat::parse("MJPG").unwrap();
        assert_eq!(format.to_string(), "MJPG");
        assert!(PixelFormat::parse("MJP").is_err());
        assert!(PixelFormat::parse("MJPEG").is_err());
        assert!(PixelFormat::parse("MJ\u{e9}G").is_err());
    }

    #[test]
    fn deserialization_enforces_the_same_invariants_as_the_constructors() {
        assert!(serde_json::from_str::<FrameSize>(r#"{"width":0,"height":720}"#).is_err());
        assert!(
            serde_json::from_str::<FrameInterval>(r#"{"numerator":1,"denominator":0}"#).is_err()
        );
        assert!(
            serde_json::from_str::<FrameInterval>(r#"{"numerator":0,"denominator":30}"#).is_err()
        );
        assert!(serde_json::from_str::<PixelFormat>(r#""MJPEG""#).is_err());
        let interval: FrameInterval =
            serde_json::from_str(r#"{"numerator":2,"denominator":60}"#).unwrap();
        assert_eq!(interval, FrameInterval::new(1, 30).unwrap());
    }

    #[test]
    fn interval_exposes_its_reduced_fraction() {
        let interval = FrameInterval::new(2, 60).unwrap();
        assert_eq!((interval.numerator(), interval.denominator()), (1, 30));
    }

    #[test]
    fn pixel_format_exposes_its_fourcc_bytes() {
        assert_eq!(PixelFormat::parse("MJPG").unwrap().bytes(), *b"MJPG");
    }

    #[test]
    fn zero_size_and_zero_interval_are_rejected() {
        assert_eq!(
            FrameSize::new(0, 720),
            Err(ModeError::InvalidSize {
                width: 0,
                height: 720
            })
        );
        assert!(FrameInterval::new(0, 30).is_err());
        assert!(FrameInterval::new(1, 0).is_err());
    }

    #[test]
    fn equivalent_intervals_compare_equal_and_report_fps() {
        let a = FrameInterval::new(2, 60).unwrap();
        let b = FrameInterval::new(1, 30).unwrap();
        assert_eq!(a, b);
        assert!((a.fps() - 30.0).abs() < f64::EPSILON);
    }

    #[test]
    fn format_size_and_interval_match_only_as_one_capability() {
        let caps = samsung_like();
        assert!(caps.validate(&mode("MJPG", 1280, 720, 1, 30)).is_ok());
        // Each field exists somewhere, but never together.
        let mixed = mode("YUYV", 1280, 720, 1, 30);
        assert!(matches!(
            caps.validate(&mixed),
            Err(ModeError::Unsupported { .. })
        ));
    }

    #[test]
    fn unsupported_fps_is_rejected_without_fallback() {
        let caps = samsung_like();
        let sixty = mode("MJPG", 1280, 720, 1, 60);
        match caps.validate(&sixty) {
            Err(ModeError::Unsupported {
                requested,
                alternatives,
            }) => {
                assert_eq!(requested, sixty);
                assert_eq!(
                    alternatives,
                    vec![
                        mode("MJPG", 1280, 720, 1, 15),
                        mode("MJPG", 1280, 720, 1, 30)
                    ]
                );
            }
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn capabilities_are_deduplicated_and_ordered() {
        let caps = CameraCapabilities::new([
            mode("MJPG", 1280, 720, 1, 30),
            mode("MJPG", 1280, 720, 2, 60),
            mode("MJPG", 1280, 720, 1, 15),
        ]);
        assert_eq!(caps.modes().len(), 2);
        assert!(caps.modes().windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn negotiation_report_lists_every_field_that_differs() {
        let requested = mode("MJPG", 1920, 1080, 1, 60);
        let negotiated = mode("MJPG", 1280, 720, 1, 30);
        let report = NegotiationReport::new(requested, negotiated);
        assert!(!report.is_exact());
        assert_eq!(
            report.differences,
            vec![ModeField::Size, ModeField::Interval]
        );
        assert_eq!(report.requested, requested);
        assert_eq!(report.negotiated, negotiated);

        let exact = NegotiationReport::new(negotiated, negotiated);
        assert!(exact.is_exact());
        assert!(exact.differences.is_empty());
    }

    #[test]
    fn mode_serializes_with_camel_case_report_and_textual_format() {
        let report = NegotiationReport::new(
            mode("MJPG", 1280, 720, 1, 30),
            mode("MJPG", 1280, 720, 1, 30),
        );
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["requested"]["format"], "MJPG");
        assert_eq!(json["differences"], serde_json::json!([]));
    }
}
