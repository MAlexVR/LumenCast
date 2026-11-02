#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum FxError {
    #[error("invalid frame: {0}")]
    InvalidFrame(String),
    #[error("{name} must be between {min} and {max}, got {value}")]
    OutOfRange {
        name: &'static str,
        min: f32,
        max: f32,
        value: f32,
    },
    #[error("jpeg decode failed: {0}")]
    Decode(String),
    #[error("jpeg encode failed: {0}")]
    Encode(String),
}
