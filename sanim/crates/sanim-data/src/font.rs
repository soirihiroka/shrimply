//! Thin Sanim unit adapter for the application's shared native font pipeline.
use sanim_core::{Font, Fraction, TextSegment, Unit, Vec2, math::to_f64, verify::Bounds};
pub use shrimply_fonts::native::shaping::ShapedText;
pub use shrimply_fonts::native::{
    FontChange, FontEpoch, register_application_font, subscribe, system_font_epoch,
    validate_asset as validate,
};
use std::sync::Arc;

pub fn cached_shape(
    font: &Font,
    text: &str,
    size: Fraction,
    bold: bool,
    italic: bool,
) -> Result<Arc<ShapedText>, String> {
    let shaped = shrimply_fonts::native::shaping::shape(
        font,
        text,
        &shrimply_fonts::native::shaping::Options {
            weight: if bold { 700 } else { 400 },
            style: if italic {
                pango::Style::Italic
            } else {
                pango::Style::Normal
            },
            ..shrimply_fonts::native::shaping::Options::new(to_f64(size)?)
        },
    )?;
    if shaped.missing_glyphs != 0 {
        return Err(format!(
            "font cannot shape this text: {} missing glyph(s); choose a family/fallback or asset covering the requested characters",
            shaped.missing_glyphs
        ));
    }
    if shaped.color_glyphs {
        return Err("color/bitmap font glyphs require a color-text renderer; provide an outline font for Sanim Text".into());
    }
    Ok(shaped)
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
    let sanim_core::Geometry::Text {
        text,
        font,
        size,
        pending: true,
        ..
    } = element.geometry.as_ref()
    else {
        return Ok(());
    };
    let shaped = cached_shape(font, text, *size, false, false)
        .map_err(|error| format!("Text '{}': {error}", element.id))?;
    let sanim_core::Geometry::Text {
        bounds: current,
        outline,
        pending,
        ..
    } = Arc::make_mut(&mut element.geometry)
    else {
        unreachable!("resolved Text descriptor")
    };
    *current = bounds(&shaped);
    *outline = shaped.outline.clone();
    *pending = false;
    Ok(())
}
