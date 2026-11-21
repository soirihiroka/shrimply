//! Environment maps retain linear light, including HDR samples greater than one.
use super::Render3dError;
use shrimply_asset::Asset;
use shrimply_drawing_skia::media;
use shrimply_math_color::Color;
use skia_safe::{AlphaType, ColorSpace, ColorType};
use std::{path::Path, sync::Arc};

const MAX_ENVIRONMENT_EDGE: u32 = 2048;

pub struct DecodedEnvironment {
    pub pixels: Vec<Color>,
    pub width: u32,
    pub height: u32,
}

pub fn load_environment(path: impl AsRef<Path>) -> Result<DecodedEnvironment, Render3dError> {
    let bytes = Asset::new(path.as_ref())
        .snapshot()
        .and_then(|source| source.read_bounded(media::MAX_SOURCE_BYTES))
        .map_err(Render3dError::message)?;
    let image = media::prepare(Arc::new(bytes)).map_err(Render3dError::message)?;
    let mut surface = image
        .thumbnail(
            MAX_ENVIRONMENT_EDGE,
            ColorType::RGBAF32,
            ColorSpace::new_srgb_linear(),
        )
        .map_err(Render3dError::message)?;
    let info = surface.image_info();
    let (width, height) = (surface.width() as u32, surface.height() as u32);
    let mut rgba = vec![[0f32; 4]; width as usize * height as usize];
    let pixmap = surface
        .peek_pixels()
        .ok_or_else(|| Render3dError::message("Access environment pixels"))?;
    if !pixmap.read_pixels(
        &info.with_alpha_type(AlphaType::Unpremul),
        &mut rgba,
        info.min_row_bytes(),
        (0, 0),
    ) {
        return Err(Render3dError::message("Read linear environment pixels"));
    }
    let pixels = rgba
        .into_iter()
        .map(|[r, g, b, _]| Color::from_rgb(r, g, b))
        .collect();
    Ok(DecodedEnvironment {
        width,
        height,
        pixels,
    })
}
