//! Using NVIDIA's own background model through a container the user builds
//! from their own Blucast image: this crate holds the client side only. No
//! NVIDIA code or model is here.

pub mod client;
pub mod protocol;
pub mod sidecar;
