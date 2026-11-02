//! Maps what a V4L2 device enumerates into core capabilities. Anything that is
//! not a plain discrete mode is reported as skipped, never invented.

use lumencast_core::camera::{CameraProbe, SkippedMode};
use lumencast_core::mode::{
    CameraCapabilities, CaptureMode, FrameInterval, FrameSize, PixelFormat,
};

/// Frame intervals as the driver reported them for one format and size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntervalProbe {
    /// Seconds per frame as `(numerator, denominator)` pairs.
    Discrete(Vec<(u32, u32)>),
    Stepwise,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SizeProbe {
    pub width: u32,
    pub height: u32,
    pub intervals: IntervalProbe,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SizesProbe {
    Discrete(Vec<SizeProbe>),
    Stepwise,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatProbe {
    pub fourcc: String,
    pub sizes: SizesProbe,
}

pub fn map_probe(formats: &[FormatProbe]) -> CameraProbe {
    let mut modes = Vec::new();
    let mut skipped = Vec::new();
    for format in formats {
        let skip = |size: Option<(u32, u32)>, reason: String| SkippedMode {
            format: format.fourcc.clone(),
            size,
            reason,
        };
        let pixel_format = match PixelFormat::parse(&format.fourcc) {
            Ok(pixel_format) => pixel_format,
            Err(error) => {
                skipped.push(skip(None, error.to_string()));
                continue;
            }
        };
        match &format.sizes {
            SizesProbe::Stepwise => {
                skipped.push(skip(None, "stepwise frame sizes are not enumerated".into()));
            }
            SizesProbe::Failed(error) => {
                skipped.push(skip(
                    None,
                    format!("frame size enumeration failed: {error}"),
                ));
            }
            SizesProbe::Discrete(sizes) => {
                for probe in sizes {
                    map_size(pixel_format, probe, &mut modes, |reason| {
                        skipped.push(skip(Some((probe.width, probe.height)), reason));
                    });
                }
            }
        }
    }
    CameraProbe {
        capabilities: CameraCapabilities::new(modes),
        skipped,
    }
}

fn map_size(
    format: PixelFormat,
    probe: &SizeProbe,
    modes: &mut Vec<CaptureMode>,
    mut skip: impl FnMut(String),
) {
    let size = match FrameSize::new(probe.width, probe.height) {
        Ok(size) => size,
        Err(error) => return skip(error.to_string()),
    };
    match &probe.intervals {
        IntervalProbe::Stepwise => skip("stepwise frame intervals are not enumerated".into()),
        IntervalProbe::Failed(error) => skip(format!("frame interval enumeration failed: {error}")),
        IntervalProbe::Discrete(intervals) => {
            for (numerator, denominator) in intervals {
                match FrameInterval::new(*numerator, *denominator) {
                    Ok(interval) => modes.push(CaptureMode {
                        format,
                        size,
                        interval,
                    }),
                    Err(error) => skip(error.to_string()),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumencast_core::mode::{CaptureMode, FrameInterval, FrameSize, PixelFormat};

    fn mode(fourcc: &str, w: u32, h: u32, num: u32, den: u32) -> CaptureMode {
        CaptureMode {
            format: PixelFormat::parse(fourcc).unwrap(),
            size: FrameSize::new(w, h).unwrap(),
            interval: FrameInterval::new(num, den).unwrap(),
        }
    }

    fn discrete(fourcc: &str, sizes: Vec<SizeProbe>) -> FormatProbe {
        FormatProbe {
            fourcc: fourcc.into(),
            sizes: SizesProbe::Discrete(sizes),
        }
    }

    fn size(width: u32, height: u32, intervals: IntervalProbe) -> SizeProbe {
        SizeProbe {
            width,
            height,
            intervals,
        }
    }

    #[test]
    fn discrete_probes_become_one_capability_per_format_size_interval() {
        let result = map_probe(&[
            discrete(
                "MJPG",
                vec![size(
                    1280,
                    720,
                    IntervalProbe::Discrete(vec![(1, 30), (1, 15)]),
                )],
            ),
            discrete(
                "YUYV",
                vec![size(640, 480, IntervalProbe::Discrete(vec![(1, 30)]))],
            ),
        ]);
        assert!(result.skipped.is_empty());
        assert_eq!(
            result.capabilities.modes(),
            CameraCapabilities::new([
                mode("MJPG", 1280, 720, 1, 15),
                mode("MJPG", 1280, 720, 1, 30),
                mode("YUYV", 640, 480, 1, 30),
            ])
            .modes()
        );
    }

    #[test]
    fn no_sixty_fps_capability_appears_unless_the_driver_reported_it() {
        let result = map_probe(&[discrete(
            "MJPG",
            vec![size(1280, 720, IntervalProbe::Discrete(vec![(1, 30)]))],
        )]);
        assert!(result
            .capabilities
            .modes()
            .iter()
            .all(|mode| mode.interval.fps() < 31.0));
    }

    #[test]
    fn stepwise_intervals_are_skipped_with_a_reason() {
        let result = map_probe(&[discrete(
            "MJPG",
            vec![size(1280, 720, IntervalProbe::Stepwise)],
        )]);
        assert!(result.capabilities.modes().is_empty());
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(result.skipped[0].format, "MJPG");
        assert_eq!(result.skipped[0].size, Some((1280, 720)));
        assert!(result.skipped[0].reason.contains("stepwise"));
    }

    #[test]
    fn stepwise_sizes_and_enumeration_failures_are_skipped() {
        let result = map_probe(&[
            FormatProbe {
                fourcc: "MJPG".into(),
                sizes: SizesProbe::Stepwise,
            },
            FormatProbe {
                fourcc: "YUYV".into(),
                sizes: SizesProbe::Failed("EINVAL".into()),
            },
            discrete(
                "NV12",
                vec![size(640, 480, IntervalProbe::Failed("EIO".into()))],
            ),
        ]);
        assert!(result.capabilities.modes().is_empty());
        assert_eq!(result.skipped.len(), 3);
        assert!(result.skipped[1].reason.contains("EINVAL"));
        assert!(result.skipped[2].reason.contains("EIO"));
    }

    #[test]
    fn invalid_entries_are_skipped_without_dropping_valid_ones() {
        let result = map_probe(&[
            discrete(
                "MJPG",
                vec![
                    size(0, 720, IntervalProbe::Discrete(vec![(1, 30)])),
                    size(1280, 720, IntervalProbe::Discrete(vec![(1, 0), (1, 30)])),
                ],
            ),
            discrete(
                "BAD",
                vec![size(640, 480, IntervalProbe::Discrete(vec![(1, 30)]))],
            ),
        ]);
        assert_eq!(
            result.capabilities.modes(),
            &[mode("MJPG", 1280, 720, 1, 30)]
        );
        assert_eq!(result.skipped.len(), 3);
    }
}
