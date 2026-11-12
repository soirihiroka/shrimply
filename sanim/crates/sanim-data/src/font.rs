//! Explicit font assets, Pango shaping, and vector outlines. No glyph bitmap rendering.
use pango::prelude::*;
use sanim_core::{Fraction, TextSegment, Unit, Vec2, math::to_f64, verify::Bounds};
use skia_safe::{Data, FontMgr, Typeface};
use std::io::Write;

fn typeface(bytes: &[u8]) -> Result<Typeface, String> {
    FontMgr::new()
        .new_from_data(Data::new_copy(bytes), None)
        .ok_or_else(|| "invalid font resource: Skia could not parse this font".into())
}
pub fn validate(bytes: &[u8]) -> Result<(), String> {
    typeface(bytes).map(|_| ())
}

pub fn shape(
    bytes: &[u8],
    text: &str,
    size: Fraction,
) -> Result<(Bounds, Vec<TextSegment>), String> {
    let size = to_f64(size)?;
    if size <= 0. || size * f64::from(pango::SCALE) > f64::from(i32::MAX) {
        return Err("font size exceeds Pango layout range".into());
    }
    let face = typeface(bytes)?;
    // Pango's portable asset API takes a filename. Keep the private snapshot alive
    // through shaping; never register into the process-wide/system font map.
    let mut file = tempfile::Builder::new()
        .suffix(".ttf")
        .tempfile()
        .map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    file.flush().map_err(|e| e.to_string())?;
    let map = pangocairo::FontMap::new();
    map.add_font_file(file.path())
        .map_err(|e| format!("Pango could not load font asset: {e}"))?;
    let context = map.create_context();
    context.set_round_glyph_positions(false);
    context.set_language(Some(&pango::Language::from_string("und")));
    let mut description = pango::FontDescription::new();
    description.set_family(&face.family_name());
    description.set_absolute_size(size * f64::from(pango::SCALE));
    let style = face.font_style();
    description.set_stretch(pango::Stretch::__Unknown((*style.width() - 1).clamp(0, 8)));
    description.set_weight(pango::Weight::__Unknown(*style.weight()));
    description.set_style(match style.slant() {
        skia_safe::font_style::Slant::Upright => pango::Style::Normal,
        skia_safe::font_style::Slant::Italic => pango::Style::Italic,
        skia_safe::font_style::Slant::Oblique => pango::Style::Oblique,
    });
    let layout = pango::Layout::new(&context);
    layout.set_font_description(Some(&description));
    let attributes = pango::AttrList::new();
    attributes.insert(pango::AttrInt::new_fallback(false));
    layout.set_attributes(Some(&attributes));
    layout.set_text(text);
    if layout.unknown_glyphs_count() != 0 {
        return Err(format!(
            "font asset cannot shape this text: {} missing glyph(s); supply a font covering the requested characters",
            layout.unknown_glyphs_count()
        ));
    }
    let mut runs = layout.iter();
    loop {
        if let Some(run) = runs.run_readonly() {
            const COLOR_GLYPH_BIT: u32 = 1 << 1;
            // pango-rs exposes this C bitfield storage, as in the editor text renderer.
            if run.glyph_string().glyph_info().iter().any(|glyph| unsafe {
                (*glyph.as_ptr()).attr.is_cluster_start & COLOR_GLYPH_BIT != 0
            }) {
                return Err("color/bitmap font glyphs require a color-text renderer; provide an outline font for Sanim Text".into());
            }
        }
        if !runs.next_run() {
            break;
        }
    }
    let surface =
        cairo::RecordingSurface::create(cairo::Content::Alpha, None).map_err(|e| e.to_string())?;
    let canvas = cairo::Context::new(&surface).map_err(|e| e.to_string())?;
    // Keep the existing first-baseline origin for element positions.
    let baseline = f64::from(layout.baseline()) / f64::from(pango::SCALE);
    canvas.move_to(0., -baseline);
    pangocairo::functions::layout_path(&canvas, &layout);
    let path = canvas.copy_path().map_err(|e| e.to_string())?;
    let outline = path
        .iter()
        .map(|segment| match segment {
            cairo::PathSegment::MoveTo(p) => TextSegment::Move([p.0, p.1]),
            cairo::PathSegment::LineTo(p) => TextSegment::Line([p.0, p.1]),
            cairo::PathSegment::CurveTo(a, b, c) => {
                TextSegment::Curve([a.0, a.1], [b.0, b.1], [c.0, c.1])
            }
            cairo::PathSegment::ClosePath => TextSegment::Close,
        })
        .collect::<Vec<_>>();
    let (ink, logical) = layout.extents();
    let (left, top, right, bottom) = canvas.path_extents().map_err(|e| e.to_string())?;
    let scale = f64::from(pango::SCALE);
    let min = [
        left.min(f64::from(ink.x().min(logical.x())) / scale),
        top.min(f64::from(ink.y().min(logical.y())) / scale - baseline),
    ];
    let max = [
        right.max(
            (f64::from(ink.x()) + f64::from(ink.width()))
                .max(f64::from(logical.x()) + f64::from(logical.width()))
                / scale,
        ),
        bottom.max(
            (f64::from(ink.y()) + f64::from(ink.height()))
                .max(f64::from(logical.y()) + f64::from(logical.height()))
                / scale
                - baseline,
        ),
    ];
    if min.iter().chain(&max).any(|v| !v.is_finite()) {
        return Err("Pango text bounds overflow".into());
    }
    Ok((
        Bounds {
            min: Vec2 {
                values: min.map(f64::floor),
                unit: Unit::Length,
            },
            max: Vec2 {
                values: max.map(f64::ceil),
                unit: Unit::Length,
            },
        },
        outline,
    ))
}
