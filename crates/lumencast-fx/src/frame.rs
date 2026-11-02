use crate::error::FxError;

/// Interleaved RGB, 8 bits per channel, row-major.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl Frame {
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Result<Self, FxError> {
        if width == 0 || height == 0 {
            return Err(FxError::InvalidFrame(format!(
                "dimensions must be non-zero, got {width}x{height}"
            )));
        }
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(3));
        if expected != Some(data.len()) {
            return Err(FxError::InvalidFrame(format!(
                "{width}x{height} RGB needs {expected:?} bytes, got {}",
                data.len()
            )));
        }
        Ok(Self {
            width,
            height,
            data,
        })
    }

    /// Builds a frame from a per-pixel function; for tests and fixtures.
    pub fn from_fn(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 3]) -> Self {
        let mut data = Vec::with_capacity(width as usize * height as usize * 3);
        for y in 0..height {
            for x in 0..width {
                data.extend(pixel(x, y));
            }
        }
        Self {
            width,
            height,
            data,
        }
    }

    /// Wraps a buffer this crate just built with the right length.
    pub(crate) fn from_fn_data(width: u32, height: u32, data: Vec<u8>) -> Self {
        debug_assert_eq!(data.len(), width as usize * height as usize * 3);
        Self {
            width,
            height,
            data,
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Requires `x < width` and `y < height`.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 3] {
        let at = (y as usize * self.width as usize + x as usize) * 3;
        [self.data[at], self.data[at + 1], self.data[at + 2]]
    }

    /// The pixels, to change in place; the frame keeps its size.
    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_checks_dimensions_and_buffer_length() {
        assert!(Frame::new(2, 2, vec![0; 12]).is_ok());
        assert!(Frame::new(2, 2, vec![0; 11]).is_err());
        assert!(Frame::new(0, 2, vec![]).is_err());
        assert!(Frame::new(2, 0, vec![]).is_err());
    }

    #[test]
    fn from_fn_and_pixel_agree_on_layout() {
        let frame = Frame::from_fn(3, 2, |x, y| [x as u8, y as u8, (x + y) as u8]);
        assert_eq!(frame.width(), 3);
        assert_eq!(frame.height(), 2);
        assert_eq!(frame.pixel(2, 1), [2, 1, 3]);
        assert_eq!(&frame.data()[..6], &[0, 0, 0, 1, 0, 1]);
    }
}
