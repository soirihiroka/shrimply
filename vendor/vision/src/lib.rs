#![feature(portable_simd)]

//! The stabilization subset of OpenCV, translated from its scalar reference routines.
//! See README.md for source versions and the scope of the translation.

pub mod gpu;
pub mod math;
mod tracking;
pub use tracking::{FeatureOptions, FlowOptions, Track, good_features, track};

#[derive(Clone, Debug)]
pub struct GrayImage {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

impl GrayImage {
    pub fn validate(&self) -> Result<(), String> {
        if self.width == 0
            || self.height == 0
            || self.width.checked_mul(self.height) != Some(self.data.len())
            || self.width > i32::MAX as usize
            || self.height > i32::MAX as usize
        {
            return Err("invalid packed grayscale image dimensions".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct RgbaImage {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

impl RgbaImage {
    pub fn validate(&self) -> Result<(), String> {
        if self
            .width
            .checked_mul(self.height)
            .and_then(|n| n.checked_mul(4))
            != Some(self.data.len())
            || self.width == 0
            || self.height == 0
        {
            return Err("invalid packed RGBA image dimensions".into());
        }
        Ok(())
    }

    pub fn gray(&self) -> Result<GrayImage, String> {
        self.validate()?;
        // OpenCV 5.1 color_rgb.simd.hpp RGB2Gray<uchar>, color.simd_helpers.hpp.
        const RED: u32 = 9_798;
        const GREEN: u32 = 19_235;
        const BLUE: u32 = 3_735;
        const SHIFT: u32 = 15;
        let image = GrayImage {
            width: self.width,
            height: self.height,
            data: self
                .data
                .as_chunks::<4>()
                .0
                .iter()
                .map(|rgba| {
                    ((u32::from(rgba[0]) * RED
                        + u32::from(rgba[1]) * GREEN
                        + u32::from(rgba[2]) * BLUE
                        + (1 << (SHIFT - 1)))
                        >> SHIFT) as u8
                })
                .collect(),
        };
        image.validate()?;
        Ok(image)
    }
}
