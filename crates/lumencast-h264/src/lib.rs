//! H264 decoding through the system's OpenH264, loaded at runtime. LumenCast
//! ships no H264 code: the user's distribution provides Cisco's library.
//!
//! H264 is stateful: every picture depends on the previous ones, so a decoder
//! has to see every frame of a stream, even those that are not shown.

use lumencast_fx::frame::Frame;
use openh264::decoder::{Decoder, DecoderConfig};
use openh264::formats::YUVSource;
use openh264::OpenH264API;
use std::sync::OnceLock;

/// The library by soname: the soname encodes the ABI version, which is what
/// the bindings need to match, and it makes `dlopen` search the system
/// library paths instead of a file we chose.
const SONAME: &str = "libopenh264.so.8";

#[derive(Debug, thiserror::Error)]
pub enum H264Error {
    #[error("the OpenH264 library ({SONAME}) could not be loaded: {0}. Install OpenH264 (on Fedora: sudo dnf install openh264)")]
    Library(String),
    #[error("H264 decoding failed: {0}")]
    Decode(String),
}

#[allow(unsafe_code)]
fn load_api() -> Result<OpenH264API, H264Error> {
    // SAFETY: the only requirement of `from_blob_path_unchecked` is that the
    // library matches the bindings' version. `libopenh264.so.8` is the soname
    // of the OpenH264 2.5 and 2.6 ABI that `openh264-sys2` 0.9 is generated
    // from, and the name has no path, so only the system search path applies.
    unsafe { OpenH264API::from_blob_path_unchecked(SONAME) }
        .map_err(|error| H264Error::Library(error.to_string()))
}

/// Whether H264 can be decoded on this machine, with the reason when not.
pub fn availability() -> Result<(), String> {
    static STATUS: OnceLock<Result<(), String>> = OnceLock::new();
    STATUS
        .get_or_init(|| {
            H264Decoder::new()
                .map(drop)
                .map_err(|error| error.to_string())
        })
        .clone()
}

/// Length of the header some cameras put in front of every frame.
const VENDOR_HEADER: usize = 64;

/// The Annex B stream inside a camera frame.
///
/// Standard cameras send the stream as it is. The Samsung VD USB HDCAM (and,
/// by its descriptors, cameras of the same chip) sends each frame as a 64-byte
/// header, a `width` x `height` plane of 8-bit samples (a monochrome copy of
/// the picture, not part of the stream) and then the stream itself, which only
/// the first frame opens with its parameter sets and keyframe. A decoder fed
/// the whole frame never finds the stream. The size of the plane is written in
/// the header (two little-endian 16-bit numbers at bytes 6 and 8), and it is
/// the size of the stream, not necessarily the one that was asked for: the
/// camera answers 640x480 to any larger request. The prefix is recognised by
/// the stream starting exactly where that plane ends; anything else is left
/// alone. A frame with that header but nothing after the plane, or cut short
/// inside it (the camera sends such frames now and then), gives an empty slice.
pub fn elementary_stream(frame: &[u8]) -> &[u8] {
    const START_CODE: [u8; 4] = [0, 0, 0, 1];
    // A stream that begins with a start code is not a framed one.
    if frame.starts_with(&[0, 0, 1]) || frame.starts_with(&[0, 0, 0]) {
        return frame;
    }
    let number = |at: usize| {
        frame
            .get(at..at + 2)
            .map_or(0, |b| usize::from(u16::from_le_bytes([b[0], b[1]])))
    };
    let (width, height) = (number(6), number(8));
    if !(16..=8192).contains(&width) || !(16..=8192).contains(&height) {
        return frame;
    }
    match frame.get(VENDOR_HEADER + width * height..) {
        Some(rest) if rest.starts_with(&START_CODE) => rest,
        Some(rest) if rest.is_empty() => rest,
        // The plane itself is cut short: the frame was lost on the way.
        None => &[],
        _ => frame,
    }
}

pub struct H264Decoder {
    decoder: Decoder,
}

impl H264Decoder {
    pub fn new() -> Result<Self, H264Error> {
        let decoder = Decoder::with_api_config(load_api()?, DecoderConfig::new())
            .map_err(|error| H264Error::Decode(error.to_string()))?;
        Ok(Self { decoder })
    }

    /// Feeds one access unit in Annex B form (with start codes). The decoder
    /// state always advances. The picture is returned only when `want_picture`
    /// is set and one is ready: `None` means "not asked for" or "not yet".
    pub fn decode(&mut self, data: &[u8], want_picture: bool) -> Result<Option<Frame>, H264Error> {
        if data.is_empty() {
            return Ok(None);
        }
        let picture = self
            .decoder
            .decode(data)
            .map_err(|error| H264Error::Decode(error.to_string()))?;
        let Some(yuv) = picture.filter(|_| want_picture) else {
            return Ok(None);
        };
        let (width, height) = yuv.dimensions();
        let mut rgb = vec![0u8; width * height * 3];
        yuv.write_rgb8(&mut rgb);
        let frame = Frame::new(width as u32, height as u32, rgb)
            .map_err(|error| H264Error::Decode(error.to_string()))?;
        Ok(Some(frame))
    }
}

/// Real H264 streams for tests, built with the same library.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    use super::{load_api, H264Error};
    use openh264::encoder::{Encoder, EncoderConfig};
    use openh264::formats::{RgbSliceU8, YUVBuffer};

    pub const WIDTH: usize = 128;
    pub const HEIGHT: usize = 64;

    /// `count` access units of a picture with a black left half and a white
    /// right half, sized `WIDTH` x `HEIGHT`. The first one is a keyframe.
    pub fn stream(count: usize) -> Result<Vec<Vec<u8>>, H264Error> {
        let to_error = |error: openh264::Error| H264Error::Decode(error.to_string());
        let mut encoder =
            Encoder::with_api_config(load_api()?, EncoderConfig::new()).map_err(to_error)?;
        let rgb: Vec<u8> = (0..WIDTH * HEIGHT)
            .flat_map(|index| {
                if index % WIDTH < WIDTH / 2 {
                    [0, 0, 0]
                } else {
                    [255; 3]
                }
            })
            .collect();
        let yuv = YUVBuffer::from_rgb_source(RgbSliceU8::new(&rgb, (WIDTH, HEIGHT)));
        (0..count)
            .map(|_| {
                encoder
                    .encode(&yuv)
                    .map(|stream| stream.to_vec())
                    .map_err(to_error)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{stream as make_stream, HEIGHT as H, WIDTH as W};
    use super::*;

    /// `None` (with a note) when OpenH264 is not installed on this machine.
    fn library() -> Option<OpenH264API> {
        match load_api() {
            Ok(api) => Some(api),
            Err(error) => {
                eprintln!("SKIPPED, OpenH264 is not available: {error}");
                None
            }
        }
    }

    fn left_dark_right_bright(frame: &Frame) -> bool {
        frame.pixel(8, 32)[0] < 60 && frame.pixel(W as u32 - 8, 32)[0] > 190
    }

    #[test]
    fn availability_matches_whether_the_library_loads() {
        assert_eq!(availability().is_ok(), load_api().is_ok());
        if let Err(reason) = availability() {
            assert!(reason.contains("libopenh264"), "{reason}");
        }
    }

    #[test]
    fn a_stream_from_the_same_library_decodes_to_the_same_picture() {
        if library().is_none() {
            return;
        }
        let mut decoder = H264Decoder::new().unwrap();
        let mut last = None;
        for access_unit in make_stream(6).unwrap() {
            if let Some(frame) = decoder.decode(&access_unit, true).unwrap() {
                last = Some(frame);
            }
        }
        let frame = last.expect("a picture");
        assert_eq!((frame.width(), frame.height()), (W as u32, H as u32));
        assert!(left_dark_right_bright(&frame));
    }

    #[test]
    fn skipping_the_pictures_keeps_the_decoder_state_for_the_ones_that_are_shown() {
        if library().is_none() {
            return;
        }
        let units = make_stream(8).unwrap();
        let mut decoder = H264Decoder::new().unwrap();
        for unit in &units[..7] {
            assert!(
                decoder.decode(unit, false).unwrap().is_none(),
                "no picture was asked for"
            );
        }
        let shown = decoder
            .decode(&units[7], true)
            .unwrap()
            .expect("the last picture");
        assert!(left_dark_right_bright(&shown));
    }

    #[test]
    fn garbage_and_a_stream_without_its_first_frame_never_panic() {
        if library().is_none() {
            return;
        }
        let mut decoder = H264Decoder::new().unwrap();
        let _ = decoder.decode(&[1, 2, 3, 4, 5, 6, 7, 8], true);
        let _ = decoder.decode(&[], true);
        // Without its keyframe the rest of a stream has nothing to build on.
        let mut midway = H264Decoder::new().unwrap();
        for unit in make_stream(4).unwrap().iter().skip(1) {
            let _ = midway.decode(unit, true);
        }
        // A fresh keyframe makes it work again.
        let keyframe = &make_stream(1).unwrap()[0];
        let recovered = decoder.decode(keyframe, true).unwrap();
        assert!(recovered.is_some_and(|frame| left_dark_right_bright(&frame)));
    }

    /// A frame as the Samsung sends it: header carrying the plane size, plane, stream.
    fn camera_frame(width: u16, height: u16, stream: &[u8]) -> Vec<u8> {
        let mut frame = vec![0xAA; VENDOR_HEADER + usize::from(width) * usize::from(height)];
        frame[6..8].copy_from_slice(&width.to_le_bytes());
        frame[8..10].copy_from_slice(&height.to_le_bytes());
        frame.extend_from_slice(stream);
        frame
    }

    #[test]
    fn the_stream_is_found_after_the_vendor_header_and_plane() {
        let stream = [0, 0, 0, 1, 0x67, 1, 2, 3];
        assert_eq!(elementary_stream(&camera_frame(16, 16, &stream)), stream);
        // The size comes from the header, whatever size was asked for.
        assert_eq!(elementary_stream(&camera_frame(640, 480, &stream)), stream);
        // An empty frame (header and plane only) has no stream to give.
        assert!(elementary_stream(&camera_frame(64, 48, &[])).is_empty());
        // So is one cut short inside its plane.
        let mut cut = camera_frame(64, 48, &[]);
        cut.truncate(VENDOR_HEADER + 64 * 48 - 32);
        assert!(elementary_stream(&cut).is_empty());
    }

    #[test]
    fn a_frame_that_is_already_a_stream_is_left_alone() {
        let frame = [0, 0, 0, 1, 0x65, 9, 9, 9];
        assert_eq!(elementary_stream(&frame), frame);
        let short = [1u8, 2, 3];
        assert_eq!(elementary_stream(&short), short);
        // A header with no start code where its plane ends is not the prefix.
        let mut other = camera_frame(16, 16, &[7, 7, 7, 7, 7, 7]);
        other[6] = 15;
        assert_eq!(elementary_stream(&other), other.as_slice());
    }
}
