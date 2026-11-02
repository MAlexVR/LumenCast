//! Validated stage parameters. Ranges match the legacy settings: zoom 1.0 to
//! 2.0, pan -1.0 to 1.0, strengths 0.0 to 1.0.

use crate::error::FxError;

fn check(name: &'static str, value: f32, min: f32, max: f32) -> Result<f32, FxError> {
    if value.is_finite() && (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(FxError::OutOfRange {
            name,
            min,
            max,
            value,
        })
    }
}

macro_rules! ranged {
    ($(#[$doc:meta])* $type:ident, $name:literal, $min:expr, $max:expr) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq)]
        pub struct $type(f32);

        impl $type {
            pub fn new(value: f32) -> Result<Self, FxError> {
                check($name, value, $min, $max).map(Self)
            }

            pub fn get(self) -> f32 {
                self.0
            }
        }
    };
}

ranged!(
    /// 1.0 means no zoom.
    Zoom, "zoom", 1.0, 2.0
);

impl Default for Zoom {
    fn default() -> Self {
        Self(1.0)
    }
}

impl Default for Pan {
    fn default() -> Self {
        Self(0.0)
    }
}
ranged!(
    /// -1.0 shifts the crop window fully left or up, 1.0 fully right or down.
    Pan, "pan", -1.0, 1.0
);
ranged!(Strength, "strength", 0.0, 1.0);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_are_inclusive_and_values_are_kept() {
        assert_eq!(Zoom::new(1.0).unwrap().get(), 1.0);
        assert_eq!(Zoom::new(2.0).unwrap().get(), 2.0);
        assert_eq!(Pan::new(-1.0).unwrap().get(), -1.0);
        assert_eq!(Pan::new(1.0).unwrap().get(), 1.0);
        assert_eq!(Strength::new(0.0).unwrap().get(), 0.0);
        assert_eq!(Strength::new(0.35).unwrap().get(), 0.35);
    }

    #[test]
    fn out_of_range_and_non_finite_values_are_rejected_with_the_name() {
        for bad in [0.99, 2.01, f32::NAN, f32::INFINITY] {
            let error = Zoom::new(bad).unwrap_err();
            assert!(
                error
                    .to_string()
                    .starts_with("zoom must be between 1 and 2"),
                "{error}"
            );
        }
        assert!(Pan::new(-1.01).is_err());
        assert!(Pan::new(1.01).is_err());
        assert!(Strength::new(-0.01).is_err());
        assert!(Strength::new(1.01).is_err());
    }
}
