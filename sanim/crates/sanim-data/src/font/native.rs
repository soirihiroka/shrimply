//! Thin Sanim unit adapter for the application's shared native font pipeline.
use super::bounds;
use sanim_core::{Font, Fraction, math::to_f64};
pub use shrimply_fonts::native::shaping::ShapedText;
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
    validate_shape(shaped)
}
fn validate_shape(shaped: Arc<ShapedText>) -> Result<Arc<ShapedText>, String> {
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

/// Width and wrapping are part of the shared native cache recipe.
pub fn layout_options(
    size: Fraction,
    recipe: sanim_core::text::Recipe,
) -> Result<shrimply_fonts::native::shaping::Options, String> {
    let mut options = shrimply_fonts::native::shaping::Options::new(to_f64(size)?);
    options.width = recipe.width.map(to_f64).transpose()?;
    options.wrap = match recipe.overflow_wrap {
        sanim_core::text::OverflowWrap::Normal => pango::WrapMode::Word,
        sanim_core::text::OverflowWrap::BreakWord | sanim_core::text::OverflowWrap::Anywhere => {
            pango::WrapMode::WordChar
        }
    };
    Ok(options)
}

pub fn resolve_pending(element: &mut sanim_core::Element) -> Result<(), String> {
    let sanim_core::Geometry::Text {
        text, font, size, ..
    } = element.geometry.as_ref()
    else {
        return Ok(());
    };
    let recipe = element.layout.text.recipe(false);
    if !crate::requires_resolution(element) {
        return Ok(());
    }
    let normalized = crate::code::view::normalized_text(text.clone(), recipe.white_space)?;
    let shaped = validate_shape(
        shrimply_fonts::native::shaping::layout_shared(
            font,
            &normalized,
            &layout_options(*size, recipe)?,
        )?
        .shaped()?
        .clone(),
    )
    .map_err(|error| format!("Text '{}': {error}", element.id))?;
    element.layout.text.resolved = Some(recipe);
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
