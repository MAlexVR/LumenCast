//! Level measurement.

/// The level of a block of samples in dB relative to full scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Level {
    pub peak_db: f32,
    pub rms_db: f32,
}

/// The quietest level reported, so silence is a number and not minus infinity.
pub const FLOOR_DB: f32 = -100.0;

fn to_db(linear: f32) -> f32 {
    if linear <= 1e-5 {
        FLOOR_DB
    } else {
        (20.0 * linear.log10()).max(FLOOR_DB)
    }
}

pub fn measure(samples: &[f32]) -> Level {
    if samples.is_empty() {
        return Level {
            peak_db: FLOOR_DB,
            rms_db: FLOOR_DB,
        };
    }
    let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let mean_square = samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32;
    Level {
        peak_db: to_db(peak),
        rms_db: to_db(mean_square.sqrt()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_scale_is_zero_db_and_silence_is_the_floor() {
        let loud = vec![1.0f32; 480];
        assert!((measure(&loud).peak_db).abs() < 1e-4);
        assert!((measure(&loud).rms_db).abs() < 1e-4);
        assert_eq!(measure(&[0.0; 100]).rms_db, FLOOR_DB);
        assert_eq!(measure(&[]).peak_db, FLOOR_DB);
    }

    #[test]
    fn half_the_amplitude_is_about_minus_six_db() {
        let half = vec![0.5f32; 480];
        assert!((measure(&half).rms_db + 6.02).abs() < 0.05);
    }
}
