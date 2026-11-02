//! A person matte: how much of each pixel is the person, on a square grid that
//! is stretched over the whole frame.

/// `width` x `height` values, 0 (background) to 255 (person).
#[derive(Debug, Clone, PartialEq)]
pub struct Matte {
    width: usize,
    height: usize,
    alpha: Vec<u8>,
}

impl Matte {
    /// `None` when the values do not fill the grid.
    pub fn new(width: usize, height: usize, alpha: Vec<u8>) -> Option<Self> {
        (width > 0 && height > 0 && alpha.len() == width * height).then_some(Self {
            width,
            height,
            alpha,
        })
    }

    /// A matte that is `value` everywhere.
    pub fn uniform(width: usize, height: usize, value: u8) -> Self {
        Self {
            width,
            height,
            alpha: vec![value; width * height],
        }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn alpha(&self) -> &[u8] {
        &self.alpha
    }

    /// The value for a pixel of a `width` x `height` frame, bilinearly
    /// interpolated from the grid.
    pub fn sample(&self, x: u32, y: u32, frame_width: u32, frame_height: u32) -> u8 {
        let (gw, gh) = (self.width, self.height);
        let fx =
            ((x as f32 + 0.5) / frame_width as f32 * gw as f32 - 0.5).clamp(0.0, (gw - 1) as f32);
        let fy =
            ((y as f32 + 0.5) / frame_height as f32 * gh as f32 - 0.5).clamp(0.0, (gh - 1) as f32);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(gw - 1), (y0 + 1).min(gh - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let at = |x: usize, y: usize| f32::from(self.alpha[y * gw + x]);
        let top = at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx;
        let bottom = at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx;
        (top * (1.0 - ty) + bottom * ty).round() as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_grid_must_be_filled_exactly() {
        assert!(Matte::new(2, 2, vec![0; 4]).is_some());
        assert!(Matte::new(4, 2, vec![0; 8]).is_some());
        assert!(Matte::new(2, 2, vec![0; 3]).is_none());
        assert!(Matte::new(0, 2, vec![]).is_none());
    }

    #[test]
    fn sampling_maps_frame_pixels_onto_the_grid_and_stays_in_range() {
        let side = 256;
        let matte = Matte::new(
            side,
            side,
            (0..side * side).map(|i| (i % side) as u8).collect(),
        )
        .unwrap();
        assert_eq!(matte.sample(0, 0, 640, 360), 0);
        assert_eq!(matte.sample(639, 359, 640, 360), 255);
        let middle = matte.sample(320, 180, 640, 360);
        assert!((120..=136).contains(&middle), "{middle}");
    }

    #[test]
    fn a_wide_grid_maps_onto_the_whole_frame() {
        let matte = Matte::new(4, 2, vec![0, 0, 255, 255, 0, 0, 255, 255]).unwrap();
        assert_eq!(matte.sample(0, 0, 400, 200), 0);
        assert_eq!(matte.sample(399, 199, 400, 200), 255);
        assert!(matte.sample(200, 100, 400, 200) > 100);
    }
}
