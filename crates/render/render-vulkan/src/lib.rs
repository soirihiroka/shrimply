#![cfg(any(target_os = "linux", target_os = "android"))]

pub mod device;
#[cfg(feature = "renderer")]
mod renderer;
pub mod surface;
#[cfg(feature = "renderer")]
pub use renderer::*;
