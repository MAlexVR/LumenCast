//! JPEG in and out. Decoding doubles as the check that catches frames whose
//! scan data is damaged even though their marker structure is valid.

use crate::error::FxError;
use crate::frame::Frame;
use jpeg_encoder::{ColorType, Encoder};
use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;
use zune_jpeg::JpegDecoder;

pub fn decode_jpeg(data: &[u8]) -> Result<Frame, FxError> {
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(data), options);
    let pixels = decoder
        .decode()
        .map_err(|error| FxError::Decode(format!("{error:?}")))?;
    let info = decoder
        .info()
        .ok_or_else(|| FxError::Decode("no image header".into()))?;
    Frame::new(u32::from(info.width), u32::from(info.height), pixels)
        .map_err(|error| FxError::Decode(error.to_string()))
}

/// Pixel formats the preview can show: JPEG frames and uncompressed YUYV.
pub fn can_decode(fourcc: &str) -> bool {
    matches!(fourcc, "MJPG" | "YUYV")
}

/// Converts packed YUYV (YUY2, 4:2:2) to RGB with the BT.601 limited-range
/// matrix UVC cameras use. Two pixels share one pair of chroma samples.
pub fn yuyv_to_rgb(data: &[u8], width: u32, height: u32) -> Result<Frame, FxError> {
    if width == 0 || height == 0 || !width.is_multiple_of(2) {
        return Err(FxError::Decode(format!(
            "YUYV needs a non-zero even width, got {width}x{height}"
        )));
    }
    let expected = width as usize * height as usize * 2;
    if data.len() != expected {
        return Err(FxError::Decode(format!(
            "YUYV {width}x{height} needs {expected} bytes, got {}",
            data.len()
        )));
    }
    let mut rgb = Vec::with_capacity(expected / 2 * 3);
    for pair in data.chunks_exact(4) {
        let (first, u, second, v) = (pair[0], pair[1], pair[2], pair[3]);
        rgb.extend(yuv_to_rgb(first, u, v));
        rgb.extend(yuv_to_rgb(second, u, v));
    }
    Ok(Frame::from_fn_data(width, height, rgb))
}

/// BT.601 limited range, 8-bit fixed point.
fn yuv_to_rgb(y: u8, u: u8, v: u8) -> [u8; 3] {
    let c = i32::from(y) - 16;
    let d = i32::from(u) - 128;
    let e = i32::from(v) - 128;
    let channel = |value: i32| (value >> 8).clamp(0, 255) as u8;
    [
        channel(298 * c + 409 * e + 128),
        channel(298 * c - 100 * d - 208 * e + 128),
        channel(298 * c + 516 * d + 128),
    ]
}

/// `quality` is 1 to 100.
pub fn encode_jpeg(frame: &Frame, quality: u8) -> Result<Vec<u8>, FxError> {
    if !(1..=100).contains(&quality) {
        return Err(FxError::Encode(format!(
            "quality must be 1 to 100, got {quality}"
        )));
    }
    let size = |value: u32| {
        u16::try_from(value).map_err(|_| FxError::Encode(format!("{value} exceeds 65535")))
    };
    let mut out = Vec::new();
    Encoder::new(&mut out, quality)
        .encode(
            frame.data(),
            size(frame.width())?,
            size(frame.height())?,
            ColorType::Rgb,
        )
        .map_err(|error| FxError::Encode(error.to_string()))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn smooth() -> Frame {
        Frame::from_fn(64, 48, |x, y| [(x * 3) as u8, (y * 4) as u8, 128])
    }

    #[test]
    fn a_frame_survives_an_encode_decode_round_trip_within_jpeg_loss() {
        let jpeg = encode_jpeg(&smooth(), 92).unwrap();
        assert_eq!(&jpeg[..2], &[0xFF, 0xD8]);
        assert_eq!(&jpeg[jpeg.len() - 2..], &[0xFF, 0xD9]);
        let back = decode_jpeg(&jpeg).unwrap();
        assert_eq!((back.width(), back.height()), (64, 48));
        let worst = smooth()
            .data()
            .iter()
            .zip(back.data())
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(worst <= 14, "worst channel error {worst}");
    }

    fn yuyv(width: u32, height: u32, y: u8, u: u8, v: u8) -> Vec<u8> {
        (0..width * height / 2).flat_map(|_| [y, u, y, v]).collect()
    }

    #[test]
    fn only_jpeg_and_yuyv_can_be_previewed() {
        assert!(can_decode("MJPG"));
        assert!(can_decode("YUYV"));
        assert!(!can_decode("H264"));
        assert!(!can_decode("NV12"));
        assert!(!can_decode(""));
    }

    #[test]
    fn yuyv_black_and_white_use_the_limited_range() {
        let black = yuyv_to_rgb(&yuyv(4, 2, 16, 128, 128), 4, 2).unwrap();
        let white = yuyv_to_rgb(&yuyv(4, 2, 235, 128, 128), 4, 2).unwrap();
        assert!(black.data().iter().all(|value| *value == 0));
        assert!(white.data().iter().all(|value| *value == 255));
    }

    #[test]
    fn yuyv_primary_colours_land_near_their_rgb_values() {
        // BT.601 limited range: red is (Y 81, U 90, V 240), blue is (Y 41, U 240, V 110).
        let red = yuyv_to_rgb(&yuyv(2, 1, 81, 90, 240), 2, 1)
            .unwrap()
            .pixel(0, 0);
        assert!(red[0] >= 250 && red[1] <= 6 && red[2] <= 6, "{red:?}");
        let blue = yuyv_to_rgb(&yuyv(2, 1, 41, 240, 110), 2, 1)
            .unwrap()
            .pixel(1, 0);
        assert!(blue[2] >= 250 && blue[0] <= 6 && blue[1] <= 6, "{blue:?}");
    }

    #[test]
    fn yuyv_pixels_of_a_pair_share_chroma_but_keep_their_own_luma() {
        let frame = yuyv_to_rgb(&[16, 128, 235, 128], 2, 1).unwrap();
        assert_eq!(frame.pixel(0, 0), [0, 0, 0]);
        assert_eq!(frame.pixel(1, 0), [255, 255, 255]);
    }

    #[test]
    fn yuyv_with_the_wrong_length_or_an_odd_width_is_rejected() {
        assert!(matches!(
            yuyv_to_rgb(&[0; 7], 2, 2),
            Err(FxError::Decode(_))
        ));
        assert!(matches!(
            yuyv_to_rgb(&[0; 12], 3, 2),
            Err(FxError::Decode(_))
        ));
        assert!(matches!(yuyv_to_rgb(&[], 0, 2), Err(FxError::Decode(_))));
    }

    #[test]
    fn garbage_and_empty_input_are_decode_errors() {
        assert!(matches!(decode_jpeg(&[]), Err(FxError::Decode(_))));
        assert!(matches!(
            decode_jpeg(&[0, 1, 2, 3, 4, 5, 6, 7]),
            Err(FxError::Decode(_))
        ));
    }

    #[test]
    fn quality_outside_the_range_is_rejected() {
        assert!(matches!(encode_jpeg(&smooth(), 0), Err(FxError::Encode(_))));
        assert!(matches!(
            encode_jpeg(&smooth(), 101),
            Err(FxError::Encode(_))
        ));
    }

    #[test]
    fn a_lower_quality_makes_a_smaller_file() {
        let noisy = Frame::from_fn(64, 48, |x, y| {
            let n = (x * 31 + y * 17) % 255;
            [n as u8, (n * 3 % 255) as u8, (255 - n) as u8]
        });
        assert!(encode_jpeg(&noisy, 30).unwrap().len() < encode_jpeg(&noisy, 95).unwrap().len());
    }
}
