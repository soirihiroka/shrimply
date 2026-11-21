pub mod effects;
pub mod math;
pub const PREVIEW_FRAMES_IN_FLIGHT: usize = 2;
extern crate self as shrimply_render_core;

pub use shrimply_math_color::{Color, ColorCorrectionParams, LayerBlendMode};

include!(concat!(env!("OUT_DIR"), "/abi.rs"));
include!(concat!(env!("OUT_DIR"), "/background.rs"));

#[cfg(all(feature = "cuda", windows))]
unsafe impl shrimply_gpu_cuda::DeviceCopy for ConvolutionTerm {}
