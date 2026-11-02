//! Cheap structural validation of MJPEG frames. It decides whether a packet
//! is a complete JPEG of the negotiated size; it does not decode pixels.

use lumencast_core::error::CaptureError;
use lumencast_core::mode::FrameSize;

/// Checks a frame and returns its valid length (up to and including the
/// end-of-image marker); anything after it is driver padding or junk.
pub fn validate_mjpeg(data: &[u8], expected: FrameSize) -> Result<usize, CaptureError> {
    if data.len() < 4 || data[..2] != [0xFF, 0xD8] {
        return Err(decode_error("missing start-of-image marker"));
    }
    let (declared, valid_len) = walk_markers(data)?;
    if declared != expected {
        return Err(decode_error(&format!(
            "frame is {}x{}, expected {}x{}",
            declared.width, declared.height, expected.width, expected.height
        )));
    }
    Ok(valid_len)
}

fn decode_error(message: &str) -> CaptureError {
    CaptureError::Decode(message.to_owned())
}

fn is_start_of_frame(marker: u8) -> bool {
    (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC)
}

/// Walks the real marker structure so that `FF D9` inside a table or inside
/// entropy-coded data is never mistaken for the end of the image. Returns the
/// declared size and the length through the end-of-image marker.
fn walk_markers(data: &[u8]) -> Result<(FrameSize, usize), CaptureError> {
    let truncated = || decode_error("missing end-of-image marker (truncated frame)");
    let mut size = None;
    let mut position = 2;
    loop {
        while data.get(position..position + 2) == Some(&[0xFF, 0xFF]) {
            position += 1; // fill byte before a marker
        }
        let [prefix, marker] = *data
            .get(position..position + 2)
            .and_then(|pair| <&[u8; 2]>::try_from(pair).ok())
            .ok_or_else(truncated)?;
        if prefix != 0xFF {
            return Err(decode_error("malformed marker segment"));
        }
        match marker {
            0xD9 => {
                let declared = size.ok_or_else(|| decode_error("no frame header found"))?;
                return Ok((declared, position + 2));
            }
            0x01 | 0xD0..=0xD8 => position += 2,
            _ => {
                let length = data
                    .get(position + 2..position + 4)
                    .map(|bytes| usize::from(u16::from_be_bytes([bytes[0], bytes[1]])))
                    .ok_or_else(truncated)?;
                if is_start_of_frame(marker) {
                    let header = data
                        .get(position + 5..position + 9)
                        .ok_or_else(|| decode_error("truncated frame header"))?;
                    let height = u32::from(u16::from_be_bytes([header[0], header[1]]));
                    let width = u32::from(u16::from_be_bytes([header[2], header[3]]));
                    size = Some(
                        FrameSize::new(width, height)
                            .map_err(|error| decode_error(&error.to_string()))?,
                    );
                }
                position += 2 + length;
                if marker == 0xDA {
                    position = skip_scan_data(data, position);
                }
            }
        }
    }
}

/// Entropy-coded data escapes `FF` as `FF 00` and may hold restart markers;
/// the first other marker ends the scan.
fn skip_scan_data(data: &[u8], mut position: usize) -> usize {
    while position + 1 < data.len() {
        if data[position] == 0xFF && !matches!(data[position + 1], 0x00 | 0xD0..=0xD7) {
            return position;
        }
        position += if data[position] == 0xFF { 2 } else { 1 };
    }
    data.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg(width: u16, height: u16) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08];
        bytes.extend(height.to_be_bytes());
        bytes.extend(width.to_be_bytes());
        bytes.extend([0x03, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        bytes.extend([0xFF, 0xD9]);
        bytes
    }

    fn size(width: u32, height: u32) -> FrameSize {
        FrameSize::new(width, height).unwrap()
    }

    #[test]
    fn complete_frame_of_the_expected_size_is_accepted() {
        let data = jpeg(1280, 720);
        assert_eq!(validate_mjpeg(&data, size(1280, 720)), Ok(data.len()));
    }

    #[test]
    fn trailing_zero_padding_after_the_end_marker_is_tolerated() {
        let mut data = jpeg(1280, 720);
        let valid_len = data.len();
        data.extend([0, 0, 0, 0]);
        assert_eq!(validate_mjpeg(&data, size(1280, 720)), Ok(valid_len));
    }

    /// Structure seen on a real UVC camera: complete JPEG, then non-zero junk.
    /// A DQT value pair looks like an end marker, and the scan contains byte
    /// stuffing and a restart marker, none of which may end the frame early.
    fn realistic_jpeg(width: u16, height: u16) -> (Vec<u8>, usize) {
        let mut bytes = vec![0xFF, 0xD8];
        bytes.extend([0xFF, 0xDB, 0x00, 0x04, 0xFF, 0xD9]);
        bytes.extend([0xFF, 0xC0, 0x00, 0x0B, 0x08]);
        bytes.extend(height.to_be_bytes());
        bytes.extend(width.to_be_bytes());
        bytes.extend([0x01, 0x01, 0x11, 0x00]);
        bytes.extend([0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00]);
        bytes.extend([0x12, 0xFF, 0x00, 0x34, 0xFF, 0xD0, 0x56, 0x78]);
        // Fill bytes are only legal right before a marker.
        bytes.extend([0xFF, 0xFF, 0xD9]);
        let valid_len = bytes.len();
        bytes.extend([0xAA, 0xBB, 0xFF, 0xD9, 0xCC, 0x01, 0x02]);
        (bytes, valid_len)
    }

    #[test]
    fn junk_after_the_real_end_marker_is_cut_off_not_rejected() {
        let (data, valid_len) = realistic_jpeg(1280, 720);
        assert_eq!(validate_mjpeg(&data, size(1280, 720)), Ok(valid_len));
    }

    #[test]
    fn marker_lookalikes_inside_tables_and_scan_data_do_not_end_the_frame() {
        let (mut data, valid_len) = realistic_jpeg(1280, 720);
        // Dropping the real end marker must read as truncated, not as the
        // `FF D9` that sits inside the quantization table.
        data.truncate(valid_len - 2);
        assert_eq!(
            validate_mjpeg(&data, size(1280, 720)).unwrap_err().code(),
            "decode_failed"
        );
    }

    #[test]
    fn empty_and_headerless_packets_are_decode_errors() {
        for data in [vec![], vec![0x00, 0x01, 0x02, 0x03]] {
            let error = validate_mjpeg(&data, size(1280, 720)).unwrap_err();
            assert_eq!(error.code(), "decode_failed");
        }
    }

    #[test]
    fn truncated_frames_without_end_marker_are_decode_errors() {
        let mut data = jpeg(1280, 720);
        data.truncate(data.len() - 2);
        assert_eq!(
            validate_mjpeg(&data, size(1280, 720)).unwrap_err().code(),
            "decode_failed"
        );
    }

    #[test]
    fn frames_of_another_size_are_decode_errors() {
        let error = validate_mjpeg(&jpeg(640, 480), size(1280, 720)).unwrap_err();
        assert_eq!(error.code(), "decode_failed");
        assert!(error.to_string().contains("640x480"));
    }

    #[test]
    fn frames_without_a_frame_header_are_decode_errors() {
        let data = vec![0xFF, 0xD8, 0xFF, 0xD9];
        assert_eq!(
            validate_mjpeg(&data, size(1280, 720)).unwrap_err().code(),
            "decode_failed"
        );
    }
}
