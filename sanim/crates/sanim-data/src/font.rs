//! Font descriptors and immutable results stay portable; shaping is an explicit capability.
#[cfg(feature = "native-text")]
mod native;
#[cfg(feature = "native-text")]
pub use native::layout_options;
use sanim_core::{Font, Fraction, TextSegment, Unit, Vec2, verify::Bounds};
pub use shrimply_fonts::{FontChange, FontEpoch, ShapedText, subscribe};
use std::sync::Arc;

#[cfg(not(feature = "native-text"))]
const UNAVAILABLE: &str = "native text layout is unavailable in this build; enable Sanim's native-text feature to shape or validate font assets";

pub fn system_font_epoch() -> Option<FontEpoch> {
    #[cfg(feature = "native-text")]
    {
        shrimply_fonts::native::system_font_epoch()
    }
    #[cfg(not(feature = "native-text"))]
    {
        None
    }
}
pub fn register_application_font(path: impl AsRef<std::path::Path>) -> Result<(), String> {
    #[cfg(feature = "native-text")]
    {
        shrimply_fonts::native::register_application_font(path)
    }
    #[cfg(not(feature = "native-text"))]
    {
        let _ = path;
        Err(UNAVAILABLE.into())
    }
}
pub fn validate(bytes: &[u8]) -> Result<(), String> {
    #[cfg(feature = "native-text")]
    {
        shrimply_fonts::native::validate_asset(bytes)
    }
    #[cfg(not(feature = "native-text"))]
    {
        let _ = bytes;
        Err(UNAVAILABLE.into())
    }
}
pub fn cached_shape(
    font: &Font,
    text: &str,
    size: Fraction,
    bold: bool,
    italic: bool,
) -> Result<Arc<ShapedText>, String> {
    #[cfg(feature = "native-text")]
    {
        native::cached_shape(font, text, size, bold, italic)
    }
    #[cfg(not(feature = "native-text"))]
    {
        let _ = (font, text, size, bold, italic);
        Err(UNAVAILABLE.into())
    }
}
pub fn bounds(shaped: &ShapedText) -> Bounds {
    Bounds {
        min: Vec2 {
            values: [shaped.bounds[0].floor(), shaped.bounds[1].floor()],
            unit: Unit::Length,
        },
        max: Vec2 {
            values: [shaped.bounds[2].ceil(), shaped.bounds[3].ceil()],
            unit: Unit::Length,
        },
    }
}
pub fn shape(
    font: &Font,
    text: &str,
    size: Fraction,
) -> Result<(Bounds, Vec<TextSegment>), String> {
    let shaped = cached_shape(font, text, size, false, false)?;
    Ok((bounds(&shaped), shaped.outline.as_ref().clone()))
}
pub fn shape_styled(
    font: &Font,
    text: &str,
    size: Fraction,
    bold: bool,
    italic: bool,
) -> Result<(Bounds, Vec<TextSegment>, f64), String> {
    let shaped = cached_shape(font, text, size, bold, italic)?;
    Ok((
        bounds(&shaped),
        shaped.outline.as_ref().clone(),
        shaped.advance,
    ))
}
pub fn resolve_pending(element: &mut sanim_core::Element) -> Result<(), String> {
    if !matches!(element.geometry.as_ref(), sanim_core::Geometry::Text { .. })
        || !crate::requires_resolution(element)
    {
        return Ok(());
    }
    #[cfg(feature = "native-text")]
    {
        native::resolve_pending(element)
    }
    #[cfg(not(feature = "native-text"))]
    {
        Err(UNAVAILABLE.into())
    }
}
