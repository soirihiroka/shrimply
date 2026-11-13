//! Native font family fallback and explicit assets share one Pango outline pipeline.
use sanim_core::{Font, Fraction, TextSegment, Unit, Vec2, math::to_f64, verify::Bounds};
pub use shrimply_fonts::native::{
    FontChange, FontEpoch, register_application_font, subscribe, system_font_epoch,
    validate_asset as validate,
};

pub fn shape(
    font: &Font,
    text: &str,
    size: Fraction,
) -> Result<(Bounds, Vec<TextSegment>), String> {
    let prepared = shrimply_fonts::native::prepare(font, to_f64(size)?)?;
    let layout = &prepared.layout;
    layout.set_text(text);
    if layout.unknown_glyphs_count() != 0 {
        return Err(format!(
            "font cannot shape this text: {} missing glyph(s); choose a family/fallback or asset covering the requested characters",
            layout.unknown_glyphs_count()
        ));
    }
    let mut runs = layout.iter();
    loop {
        if let Some(run) = runs.run_readonly()
            && run
                .glyph_string()
                .glyph_info()
                .iter()
                .any(shrimply_fonts::native::glyph_is_color)
        {
            return Err("color/bitmap font glyphs require a color-text renderer; provide an outline font for Sanim Text".into());
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
    pangocairo::functions::layout_path(&canvas, layout);
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
