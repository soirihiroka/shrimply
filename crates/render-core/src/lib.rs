pub mod effects;
pub mod math;
extern crate self as shrimply_render_core;

pub use shrimply_math_color::{Color, ColorCorrectionParams, LayerBlendMode};

include!(concat!(env!("OUT_DIR"), "/abi.rs"));
include!(concat!(env!("OUT_DIR"), "/background.rs"));
