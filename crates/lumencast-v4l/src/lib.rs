//! Linux V4L2 adapter. Pure mapping and validation live here so they are
//! testable offline; device access is kept at the edge.

pub mod convert;
pub mod device;
pub mod jpeg;
pub mod output;
pub mod probe;
