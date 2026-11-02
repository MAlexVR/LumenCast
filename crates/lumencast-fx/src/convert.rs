//! Colour conversion for sending frames out as video.

use crate::frame::Frame;
use rayon::prelude::*;

/// The frame as planar YUV 4:2:0 (what V4L2 calls `YU12`, "I420"): the full
/// luma plane, then the quarter-size U plane, then the V plane. BT.601, limited
/// range, as video devices expect. An odd last row or column is left out
/// because chroma needs pairs; the size reported is that of the planes.
pub fn rgb_to_i420(frame: &Frame) -> (u32, u32, Vec<u8>) {
    let width = (frame.width() & !1) as usize;
    let height = (frame.height() & !1) as usize;
    let stride = frame.width() as usize * 3;
    let data = frame.data();
    let mut out = vec![0u8; width * height * 3 / 2];
    let (luma, chroma) = out.split_at_mut(width * height);
    let (u_plane, v_plane) = chroma.split_at_mut(width * height / 4);
    luma.par_chunks_exact_mut(width.max(1))
        .enumerate()
        .for_each(|(y, out_row)| {
            let row = &data[y * stride..y * stride + width * 3];
            for (out, pixel) in out_row.iter_mut().zip(row.chunks_exact(3)) {
                let (r, g, b) = (
                    i32::from(pixel[0]),
                    i32::from(pixel[1]),
                    i32::from(pixel[2]),
                );
                *out = (((66 * r + 129 * g + 25 * b + 128) >> 8) + 16) as u8;
            }
        });
    u_plane
        .par_chunks_mut((width / 2).max(1))
        .zip(v_plane.par_chunks_mut((width / 2).max(1)))
        .enumerate()
        .for_each(|(pair, (u_row, v_row))| {
            let y = pair * 2;
            for (index, (u_out, v_out)) in u_row.iter_mut().zip(v_row.iter_mut()).enumerate() {
                let x = index * 2;
                let (mut r, mut g, mut b) = (0, 0, 0);
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let at = (y + dy) * stride + (x + dx) * 3;
                    r += i32::from(data[at]);
                    g += i32::from(data[at + 1]);
                    b += i32::from(data[at + 2]);
                }
                let (r, g, b) = (r / 4, g / 4, b / 4);
                *u_out = ((((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128).clamp(0, 255)) as u8;
                *v_out = ((((112 * r - 94 * g - 18 * b + 128) >> 8) + 128).clamp(0, 255)) as u8;
            }
        });
    (width as u32, height as u32, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(r: u8, g: u8, b: u8) -> (u32, u32, Vec<u8>) {
        rgb_to_i420(&Frame::from_fn(4, 4, |_, _| [r, g, b]))
    }

    #[test]
    fn the_planes_have_the_i420_sizes() {
        let (w, h, data) = solid(0, 0, 0);
        assert_eq!((w, h, data.len()), (4, 4, 4 * 4 * 3 / 2));
    }

    #[test]
    fn black_white_and_the_primaries_land_on_their_bt601_values() {
        let near = |a: u8, b: u8| a.abs_diff(b) <= 2;
        let (_, _, black) = solid(0, 0, 0);
        assert!(near(black[0], 16) && near(black[16], 128) && near(black[20], 128));
        let (_, _, white) = solid(255, 255, 255);
        assert!(near(white[0], 235) && near(white[16], 128));
        let (_, _, red) = solid(255, 0, 0);
        assert!(
            near(red[0], 82) && near(red[16], 90) && near(red[20], 240),
            "{:?}",
            [red[0], red[16], red[20]]
        );
        let (_, _, blue) = solid(0, 0, 255);
        assert!(near(blue[0], 41) && near(blue[16], 240) && near(blue[20], 110));
    }

    #[test]
    fn an_odd_size_drops_the_unpaired_row_and_column() {
        let (w, h, data) = rgb_to_i420(&Frame::from_fn(5, 3, |_, _| [9; 3]));
        assert_eq!((w, h, data.len()), (4, 2, 4 * 2 * 3 / 2));
        let (w, h, data) = rgb_to_i420(&Frame::from_fn(1, 1, |_, _| [9; 3]));
        assert_eq!((w, h, data.len()), (0, 0, 0));
    }

    #[test]
    fn each_chroma_sample_averages_its_two_by_two_block() {
        // Left pair red, right pair blue: two chroma samples per row pair.
        let frame = Frame::from_fn(4, 2, |x, _| if x < 2 { [255, 0, 0] } else { [0, 0, 255] });
        let (_, _, data) = rgb_to_i420(&frame);
        let (u, v) = (&data[8..10], &data[10..12]);
        assert!(u[1] > u[0] && v[0] > v[1]);
    }
}
