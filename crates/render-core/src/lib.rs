pub mod math;

pub use shrimply_math_color::{Color, ColorCorrectionParams, LayerBlendMode};

include!(concat!(env!("OUT_DIR"), "/abi.rs"));
