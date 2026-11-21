use sanim_core::{math::to_f64, *};
use shrimply_drawing_skia::{
    path::PreparedPath,
    shape::{self, ShapeRoundingStrategy},
};
use skia_safe::{Canvas, Paint, PathBuilder, QuickReject, Rect};
use std::{
    rc::{Rc, Weak as PathWeak},
    sync::{Arc, Weak},
};

pub(super) const MAX_GEOMETRY_CACHE_BYTES: usize = 64 * 1024 * 1024;
// Many elements share a small number of paths (for example chart dots).
// Weak identity hints therefore have a separate, still bounded entry budget.
pub(super) const MAX_GEOMETRY_CACHE_ENTRIES: usize = 1024;
const MAX_GEOMETRY_HINT_ENTRIES: usize = 8192;
const LAYER_ANTIALIAS_PADDING: f32 = 1.;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum GeometryIdentity {
    Geometry(usize),
    TextOutline(usize),
}

enum GeometrySource {
    Geometry(Weak<Geometry>),
    TextOutline(Weak<Vec<TextSegment>>),
}
impl GeometrySource {
    fn identity(&self) -> GeometryIdentity {
        match self {
            Self::Geometry(source) => GeometryIdentity::Geometry(source.as_ptr() as usize),
            Self::TextOutline(source) => GeometryIdentity::TextOutline(source.as_ptr() as usize),
        }
    }
    fn alive(&self) -> bool {
        match self {
            Self::Geometry(source) => source.strong_count() > 0,
            Self::TextOutline(source) => source.strong_count() > 0,
        }
    }
}

pub(crate) struct GeometryHint {
    // The weak source keeps pointer identities safe without retaining scene payloads.
    source: GeometrySource,
    pub(super) key: String,
    pub(super) path: PathWeak<PreparedPath>,
    box_dimensions: Option<[Option<Number>; 2]>,
}
impl GeometryHint {
    pub(super) fn alive(&self) -> bool {
        self.source.alive() && self.path.strong_count() > 0
    }
}

/// Prepare a leaf only when it survives drawing's visibility/reveal gates.
/// The bounded weak hint avoids rehashing immutable text outlines each frame.
pub(crate) fn prepared_path(
    element: &Element,
    renderer: &mut crate::Renderer,
) -> Result<Rc<PreparedPath>, String> {
    if matches!(element.geometry.as_ref(), Geometry::Voxel(_)) {
        return Err("VoxelScene uses the independent voxel renderer; use sanim voxel render, inspect, or video".into());
    }
    if matches!(
        element.geometry.as_ref(),
        Geometry::Text { pending: true, .. }
    ) {
        return Err(
            "text shaping is unresolved; sample native layout before preparing drawing paths"
                .into(),
        );
    }
    let box_dimensions = matches!(element.geometry.as_ref(), Geometry::Box { .. })
        .then_some([element.layout.width, element.layout.height]);
    // Layout can rebuild Text's presentation while preserving its immutable outline.
    let source = match element.geometry.as_ref() {
        Geometry::Text { outline, .. } => GeometrySource::TextOutline(Arc::downgrade(outline)),
        _ => GeometrySource::Geometry(Arc::downgrade(&element.geometry)),
    };
    let identity = source.identity();
    if let Some(path) = renderer
        .geometry_hints
        .get(&identity)
        .filter(|hint| hint.box_dimensions == box_dimensions)
        .and_then(|hint| hint.path.upgrade())
    {
        renderer.geometry_key_hits += 1;
        renderer.geometry_cache_hits += 1;
        return Ok(path);
    }
    // Child state does not change a Box surface; resolved layout dimensions do.
    let key = if let Geometry::Text { outline, .. } = element.geometry.as_ref() {
        digest(&serde_json::to_vec(outline.as_ref()).map_err(|error| error.to_string())?)
    } else if let Geometry::Box { radius, .. } = element.geometry.as_ref() {
        digest(
            &serde_json::to_vec(&("Box", box_dimensions, radius))
                .map_err(|error| error.to_string())?,
        )
    } else {
        digest(&serde_json::to_vec(element.geometry.as_ref()).map_err(|error| error.to_string())?)
    };
    let prepared = if let Some(path) = renderer.geometry_cache.get(&key) {
        renderer.geometry_cache_hits += 1;
        path
    } else {
        let path = match element.geometry.as_ref() {
            Geometry::Circle { radius } => {
                let radius = to_f64(*radius)? as f32;
                shape::ellipse_path(
                    glam::Vec2::splat(radius * 2.),
                    0.,
                    360.,
                    0.,
                    ShapeRoundingStrategy::Circular,
                )
                .with_transform(&skia_safe::Matrix::translate((-radius, -radius)))
            }
            Geometry::Box { radius, .. } => {
                let edges = sanim_core::scrolling::viewport(element)?;
                let width = element.layout.width.expect("validated Box width").value;
                let height = element.layout.height.expect("validated Box height").value;
                shape::rect_path(
                    Rect::new(
                        edges[0] as f32,
                        edges[1] as f32,
                        edges[2] as f32,
                        edges[3] as f32,
                    ),
                    to_f64(radius.resolve(width, height))? as f32,
                    ShapeRoundingStrategy::Circular,
                )
            }
            Geometry::Line { end, .. } => {
                let mut path = PathBuilder::new();
                path.move_to((0., 0.));
                path.line_to((end.values[0] as f32, end.values[1] as f32));
                path.detach()
            }
            Geometry::Polyline { points, .. } => {
                let mut path = PathBuilder::new();
                let first = points
                    .first()
                    .ok_or("Polyline requires at least two points")?;
                path.move_to((first.values[0] as f32, first.values[1] as f32));
                for point in points.iter().skip(1) {
                    path.line_to((point.values[0] as f32, point.values[1] as f32));
                }
                path.detach()
            }
            Geometry::Polygon { points } => shape::polygon_path(
                points
                    .iter()
                    .map(|p| glam::Vec2::new(p.values[0] as f32, p.values[1] as f32))
                    .collect(),
                0.,
                ShapeRoundingStrategy::Circular,
            ),
            Geometry::Text { outline, .. } => {
                let mut path = PathBuilder::new();
                let point = |p: [f64; 2]| (p[0] as f32, p[1] as f32);
                for segment in outline.iter() {
                    match segment {
                        TextSegment::Move(p) => {
                            path.move_to(point(*p));
                        }
                        TextSegment::Line(p) => {
                            path.line_to(point(*p));
                        }
                        TextSegment::Curve(a, b, c) => {
                            path.cubic_to(point(*a), point(*b), point(*c));
                        }
                        TextSegment::Close => {
                            path.close();
                        }
                    }
                }
                path.detach()
            }
            Geometry::Code { .. } | Geometry::Media { .. } | Geometry::Voxel(_) => {
                unreachable!("groups handled before geometry preparation")
            }
        };
        if !path.is_finite() {
            return Err(format!(
                "geometry '{}' exceeds Skia numeric range",
                element.id
            ));
        }
        let prepared = PreparedPath::new(path)
            .map_err(|error| format!("geometry '{}': {error}", element.id))?;
        renderer.geometries_prepared += 1;
        let prepared = Rc::new(prepared);
        let bytes = prepared.approximate_bytes_used();
        let (used_bytes, entries) = renderer.geometry_cache.usage();
        // Keep the admitted hot working set when one frame exceeds capacity.
        // Evicting or clearing on each new path makes cyclic scans miss everything.
        let bytes = bytes.saturating_add(key.len());
        if entries < MAX_GEOMETRY_CACHE_ENTRIES
            && bytes <= MAX_GEOMETRY_CACHE_BYTES.saturating_sub(used_bytes)
        {
            renderer
                .geometry_cache
                .insert(key.clone(), prepared.clone(), bytes);
        }
        prepared
    };
    // Keep admitted hot identities when a larger scene exceeds the hint budget.
    // Clearing everything here makes every subsequent frame rehash every leaf.
    if renderer.geometry_hints.len() < MAX_GEOMETRY_HINT_ENTRIES
        || renderer.geometry_hints.contains_key(&identity)
    {
        renderer.geometry_hints.insert(
            identity,
            GeometryHint {
                source,
                key,
                path: Rc::downgrade(&prepared),
                box_dimensions,
            },
        );
    }
    Ok(prepared)
}

pub(crate) trait Output {
    fn enter(&mut self, _element: &Element) {}
    fn leave(&mut self) {}
    fn clip(&mut self, _canvas: &Canvas, _bounds: &Rect) {}
    fn media(
        &mut self,
        canvas: &Canvas,
        prepared: &Arc<shrimply_drawing_skia::media::Prepared>,
        bounds: &Rect,
        opacity: f32,
    ) -> Result<(), String> {
        prepared.draw(canvas, bounds, opacity)
    }
    fn path(
        &mut self,
        canvas: &Canvas,
        path: &skia_safe::Path,
        paint: &Paint,
        fill: &sanim_core::Paint,
        bounds: &Rect,
    ) -> Result<(), String>;
}
#[derive(Default)]
pub(crate) struct RecordingOutput {
    media: std::collections::BTreeMap<usize, Arc<shrimply_drawing_skia::media::Prepared>>,
    pub media_bytes: usize,
}
impl RecordingOutput {
    fn retain(
        &mut self,
        prepared: &Arc<shrimply_drawing_skia::media::Prepared>,
    ) -> Result<(), String> {
        let identity = Arc::as_ptr(prepared) as usize;
        if let std::collections::btree_map::Entry::Vacant(entry) = self.media.entry(identity) {
            self.media_bytes = self
                .media_bytes
                .checked_add(prepared.retained_bytes())
                .ok_or("media frame byte budget overflow")?;
            if self.media_bytes > sanim_data::Limits::default().max_bytes {
                return Err("native media frame exceeds its retained byte budget".into());
            }
            entry.insert(prepared.clone());
        }
        Ok(())
    }
    pub(super) fn merge(&mut self, other: &Self) -> Result<(), String> {
        for prepared in other.media.values() {
            self.retain(prepared)?;
        }
        Ok(())
    }
    pub(super) fn retained_bytes(&self) -> usize {
        self.media_bytes
            + self.media.len()
                * (std::mem::size_of::<(usize, Arc<shrimply_drawing_skia::media::Prepared>)>()
                    + 3 * std::mem::size_of::<usize>())
    }
}
impl Output for RecordingOutput {
    fn media(
        &mut self,
        canvas: &Canvas,
        prepared: &Arc<shrimply_drawing_skia::media::Prepared>,
        bounds: &Rect,
        opacity: f32,
    ) -> Result<(), String> {
        self.retain(prepared)?;
        prepared.draw(canvas, bounds, opacity)
    }
    fn path(
        &mut self,
        canvas: &Canvas,
        path: &skia_safe::Path,
        paint: &Paint,
        _fill: &sanim_core::Paint,
        _bounds: &Rect,
    ) -> Result<(), String> {
        canvas.draw_path(path, paint);
        Ok(())
    }
}
pub(crate) struct Drawing<'a, O> {
    pub renderer: &'a mut crate::Renderer,
    pub pixels: usize,
    pub work: usize,
    pub output: O,
}

fn clip_children<O: Output>(
    canvas: &Canvas,
    element: &Element,
    drawing: &mut Drawing<'_, O>,
) -> Result<(), String> {
    if element.layout.clips() {
        let [left, top, right, bottom] = sanim_core::scrolling::viewport(element)?;
        let Some(mut bounds) = canvas.local_clip_bounds() else {
            return Ok(());
        };
        if element.layout.overflow_x.clips() {
            bounds.left = left as f32;
            bounds.right = right as f32;
        }
        if element.layout.overflow_y.clips() {
            bounds.top = top as f32;
            bounds.bottom = bottom as f32;
        }
        canvas.clip_rect(bounds, None, true);
        drawing.output.clip(canvas, &bounds);
    }
    Ok(())
}

fn scroll_children(canvas: &Canvas, element: &Element) {
    if let Some(offset) = element.layout.scroll {
        canvas.translate((-offset.values[0] as f32, -offset.values[1] as f32));
    }
}

fn draw_scrollbars<O: Output>(
    canvas: &Canvas,
    element: &Element,
    drawing: &mut Drawing<'_, O>,
) -> Result<(), String> {
    if !element.layout.clips() {
        return Ok(());
    }
    let viewport = sanim_core::scrolling::viewport(element)?;
    let bars = math::scroll::bars(
        viewport,
        element.layout.scroll_range,
        element
            .layout
            .scroll
            .map_or([0.; 2], |offset| offset.values),
        [element.layout.overflow_x, element.layout.overflow_y],
    );
    let full = Rect::new(
        viewport[0] as f32,
        viewport[1] as f32,
        viewport[2] as f32,
        viewport[3] as f32,
    );
    let fill = Color::parse("light1")?;
    let outline = Color::parse("black")?;
    for bar in bars {
        let bounds = bar.thumb;
        let rect = Rect::new(
            bounds[0] as f32,
            bounds[1] as f32,
            bounds[2] as f32,
            bounds[3] as f32,
        );
        let mut builder = PathBuilder::new();
        builder.add_rrect(
            skia_safe::RRect::new_rect_xy(
                rect,
                math::scroll::THICKNESS as f32 / 2.,
                math::scroll::THICKNESS as f32 / 2.,
            ),
            None,
            None,
        );
        let path = builder.detach();
        // The idle overlay has no painted trough. Its subtle outline remains
        // legible over code without painting a permanent rail across the view.
        let mut border = paint(&outline, Fraction::new(35u64, 100u64))?;
        border
            .set_style(skia_safe::paint::Style::Stroke)
            .set_stroke_width(math::scroll::OUTLINE as f32);
        for (color, paint) in [
            (&outline, border),
            (&fill, paint(&fill, Fraction::new(1u64, 5u64))?),
        ] {
            drawing.work = drawing
                .work
                .checked_sub(path.count_points())
                .ok_or("scrollbar rendering exceeds render work budget")?;
            drawing
                .output
                .path(canvas, &path, &paint, &color.clone().into(), &full)?;
        }
    }
    Ok(())
}

fn draw_box<O: Output>(
    canvas: &Canvas,
    element: &Element,
    reveal: Reveal,
    drawing: &mut Drawing<'_, O>,
) -> Result<(), String> {
    if matches!(&element.fill, sanim_core::Paint::Solid(color) if color.0[3] == Fraction::from(0))
        && element.stroke.is_none()
    {
        return Ok(());
    }
    let save = canvas.save();
    let result = draw_shape(canvas, element, reveal, Fraction::from(1), drawing);
    canvas.restore_to_count(save);
    result
}

fn draw_shape<O: Output>(
    canvas: &Canvas,
    element: &Element,
    reveal: Reveal,
    own_opacity: Fraction,
    drawing: &mut Drawing<'_, O>,
) -> Result<(), String> {
    let prepared = prepared_path(element, drawing.renderer)?;
    drawing.work = drawing
        .work
        .checked_sub(prepared.path.count_points())
        .ok_or("path rendering exceeds render work budget")?;
    let mut paint = fill_paint(&element.fill, own_opacity, prepared.path.bounds())?;
    let progress = to_f64(reveal.progress)? as f32;
    if paint.alpha_f() > 0.0 {
        if let Geometry::Line { width, .. } | Geometry::Polyline { width, .. } =
            element.geometry.as_ref()
        {
            paint
                .set_style(skia_safe::paint::Style::Stroke)
                .set_stroke_width(to_f64(*width)? as f32)
                .set_stroke_join(skia_safe::PaintJoin::Round);
            drawing.output.path(
                canvas,
                &prepared.partial(progress, false),
                &paint,
                &element.fill,
                prepared.path.bounds(),
            )?;
        } else if progress >= 1.0 {
            drawing.output.path(
                canvas,
                &prepared.path,
                &paint,
                &element.fill,
                prepared.path.bounds(),
            )?;
        } else {
            // Traces must stay inside the original bounds used by visual verification.
            canvas.clip_rect(prepared.path.bounds(), None, true);
            drawing.output.clip(canvas, prepared.path.bounds());
            let fill_layer = if paint.alpha_f() < 1.0 {
                drawing.work = drawing
                    .work
                    .checked_sub(drawing.pixels)
                    .ok_or("reveal compositing exceeds render work budget")?;
                let save = canvas.save_layer_alpha_f(None, paint.alpha_f());
                paint.set_alpha_f(1.0);
                Some(save)
            } else {
                None
            };
            let (trace_path, trace_opacity) = match reveal.kind {
                RevealKind::Create => {
                    let partial = prepared.partial(progress, false);
                    drawing.work = drawing
                        .work
                        .checked_sub(partial.count_points())
                        .ok_or("path rendering exceeds render work budget")?;
                    drawing.output.path(
                        canvas,
                        &partial,
                        &paint,
                        &element.fill,
                        prepared.path.bounds(),
                    )?;
                    (partial, 1.0)
                }
                RevealKind::Write => {
                    let fill_progress = (progress * 2.0 - 1.0).clamp(0.0, 1.0);
                    if fill_progress > 0.0 {
                        drawing.work = drawing
                            .work
                            .checked_sub(prepared.path.count_points())
                            .ok_or("path rendering exceeds render work budget")?;
                        let mut fill = paint.clone();
                        fill.set_alpha_f(fill.alpha_f() * fill_progress);
                        drawing.output.path(
                            canvas,
                            &prepared.path,
                            &fill,
                            &element.fill,
                            prepared.path.bounds(),
                        )?;
                    }
                    (
                        prepared.partial((progress * 2.0).min(1.0), false),
                        1.0 - fill_progress,
                    )
                }
            };
            const TRACE_WIDTH_PT: f32 = 1.0;
            paint
                .set_style(skia_safe::paint::Style::Stroke)
                .set_stroke_width(TRACE_WIDTH_PT);
            paint.set_alpha_f(paint.alpha_f() * trace_opacity);
            drawing.output.path(
                canvas,
                &trace_path,
                &paint,
                &element.fill,
                prepared.path.bounds(),
            )?;
            if let Some(save) = fill_layer {
                canvas.restore_to_count(save);
            }
        }
    }
    if let Some(stroke) = &element.stroke {
        let mut border = fill_paint(&stroke.paint, own_opacity, prepared.path.bounds())?;
        border
            .set_style(skia_safe::paint::Style::Stroke)
            .set_stroke_width(to_f64(stroke.width)? as f32);
        let border_path = prepared.partial(progress, false);
        drawing.work = drawing
            .work
            .checked_sub(border_path.count_points())
            .ok_or("border rendering exceeds render work budget")?;
        drawing.output.path(
            canvas,
            &border_path,
            &border,
            &stroke.paint,
            prepared.path.bounds(),
        )?;
    }
    Ok(())
}

pub(crate) fn element_draw<O: Output>(
    canvas: &Canvas,
    element: &Element,
    inherited: Reveal,
    drawing: &mut Drawing<'_, O>,
) -> Result<(), String> {
    drawing.work = drawing
        .work
        .checked_sub(1)
        .ok_or("render work budget exceeded")?;
    let reveal = if inherited.progress < element.reveal.progress {
        inherited
    } else {
        element.reveal
    };
    if element.opacity == Fraction::from(0)
        || element.scale == Fraction::from(0)
        || element.horizontal.scale == Fraction::from(0)
        || reveal.progress == Fraction::from(0)
    {
        return Ok(());
    }
    drawing.output.enter(element);
    let save = canvas.save();
    let result = (|| {
        canvas.translate((
            element.position.values[0] as f32,
            element.position.values[1] as f32,
        ));
        canvas.rotate(to_f64(element.rotation)? as f32, None);
        let scale = to_f64(element.scale)? as f32;
        canvas.scale((scale, scale));
        if element.horizontal.scale != Fraction::from(1) {
            let origin = to_f64(element.horizontal.origin.value)? as f32;
            canvas.translate((origin, 0.));
            canvas.scale((to_f64(element.horizontal.scale)? as f32, 1.));
            canvas.translate((-origin, 0.));
        }
        if !canvas.local_to_device().is_finite() {
            return Err("composed transform exceeds Skia numeric range".into());
        }
        if let Geometry::Text {
            bounds,
            pending: false,
            ..
        } = element.geometry.as_ref()
        {
            let border = element
                .stroke
                .as_ref()
                .map(|stroke| to_f64(stroke.width))
                .transpose()?
                .unwrap_or(0.) as f32
                / 2.;
            let bounds = Rect::new(
                bounds.min.values[0] as f32,
                bounds.min.values[1] as f32,
                bounds.max.values[0] as f32,
                bounds.max.values[1] as f32,
            )
            .with_outset((
                border + LAYER_ANTIALIAS_PADDING,
                border + LAYER_ANTIALIAS_PADDING,
            ));
            if canvas.quick_reject(&bounds) {
                return Ok(());
            }
        }
        if let Geometry::Media {
            asset,
            width,
            height,
        } = element.geometry.as_ref()
        {
            let width = to_f64(*width)? as f32;
            let height = to_f64(*height)? as f32;
            let bounds = Rect::from_xywh(-width / 2., -height / 2., width, height);
            if canvas.quick_reject(&bounds) {
                return Ok(());
            }
            let prepared = match asset.source() {
                sanim_core::media::Source::Encoded(bytes) => {
                    shrimply_drawing_skia::media::prepare_hashed(bytes, asset.fingerprint())?
                }
                sanim_core::media::Source::Texture(texture) => {
                    shrimply_drawing_skia::media::prepare_rgba(
                        texture.pixels(),
                        [texture.width(), texture.height()],
                    )?
                }
            };
            if reveal.progress < Fraction::from(1) {
                let mut visible = bounds;
                visible.right = visible.left + visible.width() * to_f64(reveal.progress)? as f32;
                canvas.clip_rect(visible, None, true);
                drawing.output.clip(canvas, &visible);
            }
            drawing
                .output
                .media(canvas, &prepared, &bounds, to_f64(element.opacity)? as f32)?;
            return Ok(());
        }
        if let Geometry::Box { children, .. } | Geometry::Code { children, .. } =
            element.geometry.as_ref()
        {
            if element.opacity != Fraction::from(1) {
                let mut local = element.clone();
                local.position = Vec2::zero(Unit::Length);
                local.rotation = Fraction::from(0);
                local.scale = Fraction::from(1);
                local.horizontal = HorizontalTransform::default();
                let bounds = sanim_core::verify::bounds(&local).ok().map(|bounds| {
                    Rect::new(
                        bounds.min.values[0] as f32,
                        bounds.min.values[1] as f32,
                        bounds.max.values[0] as f32,
                        bounds.max.values[1] as f32,
                    )
                    .with_outset((LAYER_ANTIALIAS_PADDING, LAYER_ANTIALIAS_PADDING))
                });
                let pixels = if let Some(bounds) = &bounds {
                    let (mut device, _) = canvas.local_to_device_as_3x3().map_rect(bounds);
                    let clipped = canvas
                        .device_clip_bounds()
                        .and_then(|clip| device.intersect(Rect::from(clip)).then_some(device));
                    clipped
                        .map_or(0, |rect| {
                            (rect.width().ceil().max(0.) as usize)
                                .saturating_mul(rect.height().ceil().max(0.) as usize)
                        })
                        .min(drawing.pixels)
                } else {
                    drawing.pixels
                };
                drawing.work = drawing
                    .work
                    .checked_sub(pixels)
                    .ok_or("group compositing exceeds render work budget")?;
                canvas.save_layer_alpha_f(bounds, to_f64(element.opacity)? as f32);
            }
            if matches!(element.geometry.as_ref(), Geometry::Box { .. }) {
                draw_box(canvas, element, reveal, drawing)?;
            }
            clip_children(canvas, element, drawing)?;
            let content = canvas.save();
            scroll_children(canvas, element);
            // A group reveals its children in parallel, retaining each child's own reveal limit.
            for child in children {
                element_draw(canvas, child, reveal, drawing)?;
            }
            canvas.restore_to_count(content);
            draw_scrollbars(canvas, element, drawing)?;
            return Ok(());
        }
        draw_shape(canvas, element, reveal, element.opacity, drawing)?;
        Ok(())
    })();
    canvas.restore_to_count(save);
    drawing.output.leave();
    result
}
pub(crate) fn paint(color: &Color, opacity: Fraction) -> Result<Paint, String> {
    let mut channels = [0.; 4];
    for (i, value) in color.0.iter().enumerate() {
        let value = to_f64(*value)?;
        if !(0.0..=1.0).contains(&value) {
            return Err("color channel outside [0,1]".into());
        }
        channels[i] = value as f32;
    }
    channels[3] *= to_f64(opacity)? as f32;
    Ok(shape::paint(
        skia_safe::Color4f::new(channels[0], channels[1], channels[2], channels[3]),
        1.,
    ))
}

fn fill_paint(fill: &sanim_core::Paint, opacity: Fraction, bounds: &Rect) -> Result<Paint, String> {
    match fill {
        sanim_core::Paint::Solid(color) => paint(color, opacity),
        sanim_core::Paint::LinearGradient { angle, stops } => {
            let points = math::paint::gradient_endpoints(
                [
                    bounds.left as f64,
                    bounds.top as f64,
                    bounds.right as f64,
                    bounds.bottom as f64,
                ],
                *angle,
            )?;
            let colors = stops
                .iter()
                .map(|stop| {
                    let values = stop
                        .color
                        .0
                        .map(|channel| to_f64(channel).map(|channel| channel as f32));
                    Ok(skia_safe::Color4f::new(
                        values[0].clone()?,
                        values[1].clone()?,
                        values[2].clone()?,
                        values[3].clone()?,
                    ))
                })
                .collect::<Result<Vec<_>, String>>()?;
            let offsets = stops
                .iter()
                .map(|stop| to_f64(stop.offset).map(|offset| offset as f32))
                .collect::<Result<Vec<_>, _>>()?;
            let gradient_colors = skia_safe::gradient::Colors::new(
                &colors,
                Some(&offsets),
                skia_safe::TileMode::Clamp,
                None,
            );
            let gradient = skia_safe::gradient::Gradient::new(
                gradient_colors,
                skia_safe::gradient::Interpolation::default(),
            );
            let shader = skia_safe::gradient::shaders::linear_gradient(
                (
                    (points[0][0] as f32, points[0][1] as f32),
                    (points[1][0] as f32, points[1][1] as f32),
                ),
                &gradient,
                None,
            )
            .ok_or("Skia could not create the linear gradient shader")?;
            let mut paint = paint(&Color::parse("white")?, opacity)?;
            paint.set_shader(shader);
            Ok(paint)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn painted_box(width: i64, height: i64) -> Element {
        Element {
            id: "box".into(),
            geometry: Arc::new(Geometry::Box {
                children: Default::default(),
                radius: CornerRadius::Pill,
            }),
            position: Vec2::zero(Unit::Length),
            rotation: 0.into(),
            scale: 1.into(),
            opacity: Fraction::new(1u64, 2u64),
            fill: Color::parse("blue3").unwrap().into(),
            stroke: None,
            reveal: Default::default(),
            horizontal: Default::default(),
            layout: Layout {
                width: Some(Number {
                    value: width.into(),
                    unit: Unit::Length,
                }),
                height: Some(Number {
                    value: height.into(),
                    unit: Unit::Length,
                }),
                x_align: Align::Start,
                y_align: Align::Start,
                ..Default::default()
            },
        }
    }

    #[test]
    fn box_paths_reuse_geometry_but_rekey_on_layout_size_changes() {
        let mut renderer = crate::Renderer::default();
        let mut element = painted_box(40, 20);
        let geometry = element.geometry.clone();
        let first = prepared_path(&element, &mut renderer).unwrap();
        assert_eq!(first.path.bounds().width(), 40.);
        element.layout.width.as_mut().unwrap().value = 80.into();
        let resized = prepared_path(&element, &mut renderer).unwrap();
        assert!(Arc::ptr_eq(&geometry, &element.geometry));
        assert_eq!(resized.path.bounds().width(), 80.);
        assert!(!Rc::ptr_eq(&first, &resized));
        assert!(Rc::ptr_eq(
            &resized,
            &prepared_path(&element, &mut renderer).unwrap()
        ));
        assert_eq!(renderer.geometries_prepared, 2);
    }

    #[test]
    #[ignore = "microbenchmark: shared Box surface preparation against direct path rebuilding"]
    fn box_path_benchmark() {
        const ITERATIONS: u64 = 100_000;
        let element = painted_box(40, 20);
        let mut renderer = crate::Renderer::default();
        prepared_path(&element, &mut renderer).unwrap();
        let start = std::time::Instant::now();
        for _ in 0..ITERATIONS {
            std::hint::black_box(prepared_path(&element, &mut renderer).unwrap());
        }
        let cached = start.elapsed();
        let start = std::time::Instant::now();
        for _ in 0..ITERATIONS {
            std::hint::black_box(shape::rect_path(
                Rect::from_xywh(0., 0., 40., 20.),
                10.,
                ShapeRoundingStrategy::Circular,
            ));
        }
        let rebuilt = start.elapsed();
        assert_eq!(renderer.geometries_prepared, 1);
        assert_eq!(renderer.geometry_key_hits, ITERATIONS);
        eprintln!("Box paths ({ITERATIONS}): cached={cached:?}, rebuilt={rebuilt:?}");
    }

    fn rendered_alphas(element: &Element) -> Vec<u8> {
        let mut surface = skia_safe::surfaces::raster_n32_premul((64, 32)).unwrap();
        surface.canvas().clear(skia_safe::Color::TRANSPARENT);
        let mut renderer = crate::Renderer::default();
        let mut drawing = Drawing {
            renderer: &mut renderer,
            pixels: 64 * 32,
            work: 1_000_000,
            output: RecordingOutput::default(),
        };
        element_draw(surface.canvas(), element, Reveal::default(), &mut drawing).unwrap();
        let pixels = surface.peek_pixels().unwrap();
        (0..32)
            .flat_map(|y| (0..64).map(move |x| (x, y)))
            .map(|point| pixels.get_color(point).a())
            .collect()
    }

    #[test]
    fn box_reveal_uses_the_shared_contour_pipeline_and_preserves_opacity() {
        let render = |progress| {
            let mut element = painted_box(40, 20);
            element.reveal.progress = progress;
            let alphas = rendered_alphas(&element);
            assert!(alphas.iter().all(|alpha| *alpha <= 128));
            alphas.into_iter().filter(|alpha| *alpha > 0).count()
        };
        let partial = render(Fraction::new(1u64, 4u64));
        let complete = render(1.into());
        assert!(
            partial > 0 && partial < complete,
            "partial={partial}, complete={complete}"
        );
    }

    #[test]
    fn transparent_box_fill_does_not_hide_the_revealed_border() {
        let mut element = painted_box(40, 20);
        element.fill = Color::parse("transparent").unwrap().into();
        element.stroke = Some(sanim_core::paint::Stroke {
            paint: Color::parse("white").unwrap().into(),
            width: 2.into(),
        });
        element.reveal.progress = Fraction::new(1u64, 4u64);
        for kind in [RevealKind::Create, RevealKind::Write] {
            element.reveal.kind = kind;
            let alphas = rendered_alphas(&element);
            assert!(alphas.iter().any(|alpha| *alpha > 0));
            assert!(alphas.iter().all(|alpha| *alpha <= 128));
        }
    }
}
