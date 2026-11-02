//! Loading the picture that replaces the background.

use lumencast_fx::codec::decode_jpeg;
use lumencast_fx::frame::Frame;
use std::path::Path;

/// Larger files are refused before they are read.
const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
/// Larger pictures are refused after their header is read.
const MAX_PIXELS: u64 = 40_000_000;

/// Reads a PNG or JPEG picture into an RGB frame (transparency is dropped).
pub fn load_image(path: &str) -> Result<Frame, String> {
    let extension = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    let metadata = std::fs::metadata(path).map_err(|error| format!("{path}: {error}"))?;
    if !metadata.is_file() {
        return Err(format!("{path} is not a file"));
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Err(format!("{path} is larger than 32 MB"));
    }
    let bytes = std::fs::read(path).map_err(|error| format!("{path}: {error}"))?;
    match extension.as_deref() {
        Some("png") => decode_png(&bytes),
        Some("jpg" | "jpeg") => decode_jpeg(&bytes).map_err(|error| error.to_string()),
        _ => Err("the background picture must be a PNG or JPEG file".into()),
    }
}

fn decode_png(bytes: &[u8]) -> Result<Frame, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|error| format!("not a valid PNG: {error}"))?;
    let info = reader.info();
    if u64::from(info.width) * u64::from(info.height) > MAX_PIXELS {
        return Err("the picture is too large".into());
    }
    let mut buffer = vec![0; reader.output_buffer_size()];
    let output = reader
        .next_frame(&mut buffer)
        .map_err(|error| format!("not a valid PNG: {error}"))?;
    let data = &buffer[..output.buffer_size()];
    let rgb: Vec<u8> = match output.color_type {
        png::ColorType::Rgb => data.to_vec(),
        png::ColorType::Rgba => data
            .chunks_exact(4)
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|g| [*g, *g, *g]).collect(),
        png::ColorType::GrayscaleAlpha => data
            .chunks_exact(2)
            .flat_map(|p| [p[0], p[0], p[0]])
            .collect(),
        png::ColorType::Indexed => return Err("indexed PNG was not expanded".into()),
    };
    Frame::new(output.width, output.height, rgb).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumencast_fx::codec::encode_jpeg;

    fn write_png(path: &Path, color: png::ColorType, data: &[u8]) {
        let file = std::fs::File::create(path).unwrap();
        let mut encoder = png::Encoder::new(file, 2, 1);
        encoder.set_color(color);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(data)
            .unwrap();
    }

    #[test]
    fn a_png_with_and_without_transparency_loads_as_rgb() {
        let dir = tempfile::tempdir().unwrap();
        let rgb = dir.path().join("rgb.png");
        write_png(&rgb, png::ColorType::Rgb, &[1, 2, 3, 4, 5, 6]);
        let frame = load_image(rgb.to_str().unwrap()).unwrap();
        assert_eq!((frame.width(), frame.height()), (2, 1));
        assert_eq!(frame.pixel(1, 0), [4, 5, 6]);
        let rgba = dir.path().join("rgba.png");
        write_png(&rgba, png::ColorType::Rgba, &[9, 8, 7, 0, 6, 5, 4, 255]);
        assert_eq!(
            load_image(rgba.to_str().unwrap()).unwrap().pixel(0, 0),
            [9, 8, 7]
        );
        let grey = dir.path().join("grey.png");
        write_png(&grey, png::ColorType::Grayscale, &[10, 20]);
        assert_eq!(
            load_image(grey.to_str().unwrap()).unwrap().pixel(1, 0),
            [20; 3]
        );
    }

    #[test]
    fn a_jpeg_loads_by_its_extension() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wall.JPG");
        let frame = Frame::from_fn(16, 8, |_, _| [200, 100, 50]);
        std::fs::write(&path, encode_jpeg(&frame, 95).unwrap()).unwrap();
        let loaded = load_image(path.to_str().unwrap()).unwrap();
        assert_eq!((loaded.width(), loaded.height()), (16, 8));
    }

    #[test]
    fn missing_unsupported_and_broken_files_are_refused_with_a_reason() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_image("/no/such/file.png").is_err());
        let text = dir.path().join("notes.txt");
        std::fs::write(&text, "hello").unwrap();
        assert!(load_image(text.to_str().unwrap())
            .unwrap_err()
            .contains("PNG or JPEG"));
        let broken = dir.path().join("broken.png");
        std::fs::write(&broken, "not a png").unwrap();
        assert!(load_image(broken.to_str().unwrap())
            .unwrap_err()
            .contains("PNG"));
        assert!(load_image(dir.path().to_str().unwrap()).is_err());
    }
}
