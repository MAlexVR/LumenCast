//! Scaling a frame up with a Lanczos-3 filter. NVIDIA Broadcast's
//! `video_super_resolution` (output size plus a performance or quality mode)
//! adds detail with a neural model; this is the classical alternative: it
//! cannot invent detail, but it keeps edges sharper than bilinear scaling does.

use crate::frame::Frame;
use rayon::prelude::*;

const LOBES: f32 = 3.0;

fn sinc(x: f32) -> f32 {
    if x.abs() < 1e-6 {
        1.0
    } else {
        let px = std::f32::consts::PI * x;
        px.sin() / px
    }
}

fn lanczos(x: f32) -> f32 {
    if x.abs() >= LOBES {
        0.0
    } else {
        sinc(x) * sinc(x / LOBES)
    }
}

/// For one output coordinate: the first source index and the weights.
struct Taps {
    first: usize,
    weights: Vec<f32>,
}

/// Taps for scaling `source_len` samples to `out_len` (pixel centres aligned,
/// edges clamped). For a reduction the filter widens so nothing aliases.
fn taps(source_len: usize, out_len: usize) -> Vec<Taps> {
    let scale = source_len as f32 / out_len as f32;
    let stretch = scale.max(1.0);
    let support = LOBES * stretch;
    (0..out_len)
        .map(|index| {
            let centre = (index as f32 + 0.5) * scale - 0.5;
            let first = (centre - support).ceil().max(0.0) as usize;
            let last = ((centre + support).floor() as isize).min(source_len as isize - 1);
            let last = last.max(first as isize) as usize;
            let mut weights: Vec<f32> = (first..=last)
                .map(|i| lanczos((i as f32 - centre) / stretch))
                .collect();
            let total: f32 = weights.iter().sum();
            if total.abs() > 1e-6 {
                weights.iter_mut().for_each(|w| *w /= total);
            }
            Taps { first, weights }
        })
        .collect()
}

/// `frame` scaled to `width` x `height` (at least 1 each).
pub fn resize_lanczos(frame: &Frame, width: u32, height: u32) -> Frame {
    let (out_w, out_h) = (width.max(1) as usize, height.max(1) as usize);
    let (src_w, src_h) = (frame.width() as usize, frame.height() as usize);
    let columns = taps(src_w, out_w);
    let rows = taps(src_h, out_h);
    let src = frame.data();

    // Horizontal pass: every source row becomes `out_w` pixels, kept as floats.
    let mut middle = vec![0.0f32; src_h * out_w * 3];
    middle
        .par_chunks_exact_mut(out_w * 3)
        .enumerate()
        .for_each(|(y, row)| {
            let line = &src[y * src_w * 3..(y + 1) * src_w * 3];
            for (x, tap) in columns.iter().enumerate() {
                let mut sum = [0.0f32; 3];
                for (k, weight) in tap.weights.iter().enumerate() {
                    let at = (tap.first + k) * 3;
                    sum[0] += weight * f32::from(line[at]);
                    sum[1] += weight * f32::from(line[at + 1]);
                    sum[2] += weight * f32::from(line[at + 2]);
                }
                row[x * 3..x * 3 + 3].copy_from_slice(&sum);
            }
        });

    // Vertical pass.
    let mut data = vec![0u8; out_w * out_h * 3];
    data.par_chunks_exact_mut(out_w * 3)
        .zip(rows.par_iter())
        .for_each(|(out_row, tap)| {
            let mut sums = vec![0.0f32; out_w * 3];
            for (k, weight) in tap.weights.iter().enumerate() {
                let line = &middle[(tap.first + k) * out_w * 3..(tap.first + k + 1) * out_w * 3];
                for (sum, value) in sums.iter_mut().zip(line) {
                    *sum += weight * value;
                }
            }
            for (out, sum) in out_row.iter_mut().zip(&sums) {
                *out = (sum + 0.5).clamp(0.0, 255.0) as u8;
            }
        });
    Frame::from_fn_data(out_w as u32, out_h as u32, data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_picture_stays_flat_at_any_scale() {
        let frame = Frame::from_fn(8, 6, |_, _| [90, 150, 210]);
        for (w, h) in [(16, 12), (12, 9), (4, 3), (8, 6)] {
            let out = resize_lanczos(&frame, w, h);
            assert_eq!((out.width(), out.height()), (w, h));
            assert!(out.data().chunks(3).all(|p| p == [90, 150, 210]), "{w}x{h}");
        }
    }

    #[test]
    fn the_same_size_returns_the_same_picture() {
        let frame = Frame::from_fn(9, 7, |x, y| [(x * 25) as u8, (y * 30) as u8, 100]);
        assert_eq!(resize_lanczos(&frame, 9, 7), frame);
    }

    #[test]
    fn a_gradient_stays_in_order_when_scaled_up() {
        let frame = Frame::from_fn(8, 2, |x, _| [(x * 30) as u8; 3]);
        let out = resize_lanczos(&frame, 24, 6);
        let row: Vec<u8> = (0..24).map(|x| out.pixel(x, 3)[0]).collect();
        assert!(row.windows(2).all(|w| w[0] <= w[1] + 1), "{row:?}");
    }

    #[test]
    fn a_hard_edge_gets_no_wider_than_a_few_pixels() {
        let frame = Frame::from_fn(8, 1, |x, _| if x < 4 { [0; 3] } else { [255; 3] });
        let out = resize_lanczos(&frame, 32, 1);
        let transition = (0..32)
            .filter(|x| (10..245).contains(&out.pixel(*x, 0)[0]))
            .count();
        assert!(transition <= 5, "{transition} pixels in transition");
    }

    #[test]
    fn tiny_and_odd_sizes_do_not_panic() {
        let one = Frame::from_fn(1, 1, |_, _| [7; 3]);
        assert_eq!(resize_lanczos(&one, 5, 3).pixel(2, 1), [7; 3]);
        let odd = Frame::from_fn(5, 3, |x, y| [(x + y) as u8; 3]);
        let out = resize_lanczos(&odd, 7, 11);
        assert_eq!((out.width(), out.height()), (7, 11));
    }
}
