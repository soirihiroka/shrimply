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
