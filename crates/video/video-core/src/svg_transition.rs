use glam::Vec2;
use skia_safe::{
    BlendMode, Canvas, Color, Matrix, Paint, PaintStyle, Path, PathBuilder, Point, RRect, Rect,
    canvas::SaveLayerRec,
    svg::{self, Node, TypedNode},
};

use crate::generated::GeneratedTransition;

pub(crate) struct SvgPath {
    pub path: Path,
    pub fill: bool,
    pub fill_color: Color,
    pub fill_opacity: f32,
    pub stroke_color: Color,
    pub stroke_opacity: f32,
    pub stroke_width: f32,
}

#[derive(Clone, Copy)]
struct SvgStyle {
    fill: bool,
    fill_color: Color,
    fill_opacity: f32,
    stroke: bool,
    stroke_color: Color,
    stroke_opacity: f32,
    stroke_width: f32,
    current_color: Color,
    opacity: f32,
}

pub(crate) fn draw(
    dom: &svg::Dom,
    root: &svg::Svg,
    canvas: &Canvas,
    transition: GeneratedTransition,
    width: f32,
    height: f32,
    path_effect: Option<&skia_safe::PathEffect>,
) -> bool {
    let mut paths = svg_paths(root, width, height);
    if paths.is_empty() {
        return false;
    }
    if let Some(effect) = path_effect {
        apply_path_effect(&mut paths, effect, width, height);
    }

    let fallback_trace_width = (height * (2.0 / 1080.0)).max(1.0);
    if transition.kind == shrimply_project::project::VisualTransitionKind::FacetAssembly {
        for (index, path) in paths.iter().enumerate() {
            let progress =
                crate::path_transition::submobject_progress(transition, index, paths.len());
            draw_facet_path(dom, canvas, path, progress);
        }
        return true;
    }
    if matches!(
        transition.kind,
        shrimply_project::project::VisualTransitionKind::SoftRefraction
            | shrimply_project::project::VisualTransitionKind::MorphologicalResolve
    ) {
        for (index, path) in paths.iter().enumerate() {
            let progress =
                crate::path_transition::submobject_progress(transition, index, paths.len());
            draw_filtered_path(dom, canvas, path, transition, progress);
        }
        return true;
    }

    canvas.save_layer(&SaveLayerRec::default());
    for (index, path) in paths.iter().enumerate() {
        let progress = crate::path_transition::submobject_progress(transition, index, paths.len());
        match transition.kind {
            shrimply_project::project::VisualTransitionKind::Write if progress < 0.5 => {
                let partial = crate::path_transition::partial_path(
                    &path.path,
                    progress * 2.0,
                    transition.side == shrimply_project::project::TransitionSide::Outro,
                );
                draw_mask(canvas, &partial, false, fallback_trace_width, 1.0);
            }
            shrimply_project::project::VisualTransitionKind::Write => {
                draw_mask(canvas, &path.path, false, fallback_trace_width, 1.0);
                draw_mask(
                    canvas,
                    &path.path,
                    path.fill,
                    path.stroke_width,
                    (progress * 2.0 - 1.0).clamp(0.0, 1.0),
                );
            }
            shrimply_project::project::VisualTransitionKind::Create => {
                let partial = crate::path_transition::partial_path(&path.path, progress, false);
                draw_mask(
                    canvas,
                    &partial,
                    path.fill,
                    path.stroke_width.max(fallback_trace_width),
                    1.0,
                );
            }
            shrimply_project::project::VisualTransitionKind::Coalesce => {
                let pools = crate::path_transition::coalesce_mask(
                    *path.path.bounds(),
                    progress,
                    transition.effect_detail.round() as usize,
                );
                if let Some(filter) = crate::path_transition::coalesce_filter(
                    path.path.bounds().width().max(path.path.bounds().height()),
                    transition.effect_amount,
                ) {
                    let mut filtered = Paint::default();
                    filtered.set_image_filter(filter);
                    canvas.save_layer(&SaveLayerRec::default().paint(&filtered));
                    let mut mask = Paint::default();
                    mask.set_anti_alias(true);
                    mask.set_color(Color::WHITE);
                    canvas.draw_path(&pools, &mask);
                    canvas.restore();
                }
            }
            shrimply_project::project::VisualTransitionKind::ContourCurrent => {
                draw_mask(
                    canvas,
                    &path.path,
                    path.fill,
                    path.stroke_width,
                    shrimply_math_media::vector_reveal_opacity(progress),
                );
                canvas.draw_path(
                    &path.path,
                    &crate::path_transition::contour_current_paint(
                        progress,
                        fallback_trace_width * 1.6 * transition.effect_amount,
                        transition.effect_detail,
                    ),
                );
            }
            shrimply_project::project::VisualTransitionKind::LivingFill => {
                let geometry = mask_path(&path.path, path.fill, path.stroke_width);
                if let Some(mask) = crate::path_transition::living_fill_paint(
                    *path.path.bounds(),
                    progress,
                    transition.effect_amount,
                    transition.effect_detail,
                    transition.effect_angle_degrees,
                ) {
                    canvas.draw_path(&geometry, &mask);
                }
            }
            shrimply_project::project::VisualTransitionKind::Diffusion
            | shrimply_project::project::VisualTransitionKind::ReverseDiffusion => {
                let bounds = path
                    .path
                    .bounds()
                    .with_outset((path.path.bounds().width(), path.path.bounds().height()));
                let diffused = crate::path_transition::diffused_path(
                    &path.path,
                    bounds,
                    progress,
                    (transition.kind
                        == shrimply_project::project::VisualTransitionKind::ReverseDiffusion)
                        != (transition.side == shrimply_project::project::TransitionSide::Outro),
                    transition.effect_amount,
                    transition.effect_detail,
                    transition.effect_seed,
                );
                draw_mask(
                    canvas,
                    &diffused,
                    path.fill,
                    path.stroke_width,
                    if transition.effect_fade {
                        shrimply_math_media::vector_reveal_opacity(progress)
                    } else {
                        1.0
                    },
                );
            }
            _ => {}
        }
    }

    let mut source_in = Paint::default();
    source_in.set_blend_mode(BlendMode::SrcIn);
    canvas.save_layer(&SaveLayerRec::default().paint(&source_in));
    dom.render(canvas);
    canvas.restore();
    canvas.restore();
    true
}

fn draw_filtered_path(
    dom: &svg::Dom,
    canvas: &Canvas,
    path: &SvgPath,
    transition: GeneratedTransition,
    progress: f32,
) {
    let bounds = *path.path.bounds();
    let extent = bounds.width().max(bounds.height());
    let visible = shrimply_math_media::vector_reveal_opacity(progress);
    let filter = match transition.kind {
        shrimply_project::project::VisualTransitionKind::SoftRefraction => {
            crate::path_transition::soft_refraction_filter(
                progress,
                extent,
                transition.effect_amount,
                transition.effect_detail,
            )
        }
        _ => crate::path_transition::morphological_filter(
            progress,
            extent,
            transition.effect_amount,
            transition.effect_detail,
        ),
    };
    let Some(filter) = filter else { return };
    let mut filtered = Paint::default();
    filtered.set_alpha_f(visible);
    filtered.set_image_filter(filter);
    canvas.save_layer(&SaveLayerRec::default().paint(&filtered));
    draw_dom_path(dom, canvas, path, 1.0);
    canvas.restore();
}

fn draw_dom_path(dom: &svg::Dom, canvas: &Canvas, path: &SvgPath, opacity: f32) {
    canvas.save_layer(&SaveLayerRec::default());
    draw_mask(canvas, &path.path, path.fill, path.stroke_width, 1.0);
    let mut source_in = Paint::default();
    source_in.set_blend_mode(BlendMode::SrcIn);
    source_in.set_alpha_f(opacity.clamp(0.0, 1.0));
    canvas.save_layer(&SaveLayerRec::default().paint(&source_in));
    dom.render(canvas);
    canvas.restore();
    canvas.restore();
}

pub(crate) fn draw_shaky(
    dom: &svg::Dom,
    root: &svg::Svg,
    canvas: &Canvas,
    effect: &skia_safe::PathEffect,
    width: f32,
    height: f32,
) -> bool {
    let mut paths = svg_paths(root, width, height);
    if paths.is_empty() {
        return false;
    }
    apply_path_effect(&mut paths, effect, width, height);
    canvas.save_layer(&SaveLayerRec::default());
    for path in &paths {
        draw_mask(canvas, &path.path, path.fill, path.stroke_width, 1.0);
    }
    let mut source_in = Paint::default();
    source_in.set_blend_mode(BlendMode::SrcIn);
    canvas.save_layer(&SaveLayerRec::default().paint(&source_in));
    dom.render(canvas);
    canvas.restore();
    canvas.restore();
    true
}

fn apply_path_effect(
    paths: &mut [SvgPath],
    effect: &skia_safe::PathEffect,
    width: f32,
    height: f32,
) {
    let cull = Rect::from_xywh(-width, -height, width * 3.0, height * 3.0);
    for path in paths {
        path.path = crate::shaky_path::apply(&path.path, effect, cull);
    }
}

fn draw_facet_path(dom: &svg::Dom, canvas: &Canvas, path: &SvgPath, progress: f32) {
    const FACETS: usize = 7;
    let bounds = path.path.bounds();
    let extent = bounds.width().max(bounds.height());
    let center = Point::new(bounds.center_x(), bounds.center_y());
    for facet in 0..FACETS {
        let (offset, rotation, scale, opacity) =
            shrimply_math_media::facet_transform(progress, facet, FACETS, extent);
        if opacity <= 0.0 {
            continue;
        }
        let clip = crate::path_transition::facet_clip(*bounds, facet, FACETS);
        canvas.save_layer(&SaveLayerRec::default());
        canvas.translate((offset.x, offset.y));
        canvas.rotate(rotation, Some(center));
        canvas.translate((center.x, center.y));
        canvas.scale((scale, scale));
        canvas.translate((-center.x, -center.y));
        canvas.clip_path(&clip, None, true);
        draw_mask(canvas, &path.path, path.fill, path.stroke_width, opacity);

        let mut source_in = Paint::default();
        source_in.set_blend_mode(BlendMode::SrcIn);
        canvas.save_layer(&SaveLayerRec::default().paint(&source_in));
        dom.render(canvas);
        canvas.restore();
        canvas.restore();
    }

    if let Some(glint) = crate::path_transition::facet_glint_clip(*bounds, progress) {
        canvas.save();
        canvas.clip_path(&glint, None, true);
        let glint_progress = ((progress - 0.62) / 0.28).clamp(0.0, 1.0);
        let mut highlight = Paint::default();
        highlight.set_anti_alias(true);
        highlight.set_color(Color::WHITE);
        highlight.set_alpha_f((std::f32::consts::PI * glint_progress).sin() * 0.42);
        canvas.draw_path(&path.path, &highlight);
        canvas.restore();
    }
}

pub(crate) fn svg_paths(root: &svg::Svg, width: f32, height: f32) -> Vec<SvgPath> {
    let view_box = root.view_box().copied();
    let viewport = view_box
        .map(|rect| Vec2::new(rect.width(), rect.height()))
        .unwrap_or_else(|| Vec2::new(width, height));
    let transform = Matrix::concat(
        root.transform(),
        &view_box_matrix(view_box, width, height, root.preserve_aspect_ratio()),
    );
    let style = node_style(
        root,
        SvgStyle {
            fill: true,
            fill_color: Color::BLACK,
            fill_opacity: 1.0,
            stroke: false,
            stroke_color: Color::BLACK,
            stroke_opacity: 1.0,
            stroke_width: 1.0,
            current_color: Color::BLACK,
            opacity: 1.0,
        },
        viewport,
    );
    let mut paths = Vec::new();
    collect_paths(
        root.children_typed(),
        &transform,
        viewport,
        style,
        &mut paths,
    );
    paths
}

fn collect_paths(
    nodes: Vec<TypedNode>,
    parent_transform: &Matrix,
    viewport: Vec2,
    inherited_style: SvgStyle,
    paths: &mut Vec<SvgPath>,
) {
    for node in nodes {
        match node {
            TypedNode::G(group) => {
                let transform = Matrix::concat(parent_transform, group.transform());
                let style = node_style(&group, inherited_style, viewport);
                collect_paths(group.children_typed(), &transform, viewport, style, paths);
            }
            TypedNode::Svg(svg) => {
                let width = length(svg.width(), viewport.x).max(0.0);
                let height = length(svg.height(), viewport.y).max(0.0);
                if width == 0.0 || height == 0.0 {
                    continue;
                }
                let view_box = svg.view_box().copied();
                let child_viewport = view_box
                    .map(|rect| Vec2::new(rect.width(), rect.height()))
                    .unwrap_or_else(|| Vec2::new(width, height));
                let placement = Matrix::concat(
                    &Matrix::translate((length(svg.x(), viewport.x), length(svg.y(), viewport.y))),
                    &view_box_matrix(view_box, width, height, svg.preserve_aspect_ratio()),
                );
                let transform = Matrix::concat(
                    parent_transform,
                    &Matrix::concat(svg.transform(), &placement),
                );
                let style = node_style(&svg, inherited_style, child_viewport);
                collect_paths(
                    svg.children_typed(),
                    &transform,
                    child_viewport,
                    style,
                    paths,
                );
            }
            TypedNode::Path(node) => push_path(
                node.path().clone(),
                &node,
                node.transform(),
                parent_transform,
                viewport,
                inherited_style,
                paths,
            ),
            TypedNode::Line(node) => push_path(
                Path::line(
                    (length(node.x1(), viewport.x), length(node.y1(), viewport.y)),
                    (length(node.x2(), viewport.x), length(node.y2(), viewport.y)),
                ),
                &node,
                node.transform(),
                parent_transform,
                viewport,
                inherited_style,
                paths,
            ),
            TypedNode::Rect(node) => {
                let rect = Rect::from_xywh(
                    length(node.x(), viewport.x),
                    length(node.y(), viewport.y),
                    length(node.width(), viewport.x).max(0.0),
                    length(node.height(), viewport.y).max(0.0),
                );
                let rx = node.rx().map(|value| length(value, viewport.x));
                let ry = node.ry().map(|value| length(value, viewport.y));
                let path = match (rx, ry) {
                    (None, None) => Path::rect(rect, None),
                    (rx, ry) => Path::rrect(
                        RRect::new_rect_xy(
                            rect,
                            rx.or(ry).unwrap_or(0.0),
                            ry.or(rx).unwrap_or(0.0),
                        ),
                        None,
                    ),
                };
                push_path(
                    path,
                    &node,
                    node.transform(),
                    parent_transform,
                    viewport,
                    inherited_style,
                    paths,
                );
            }
            TypedNode::Circle(node) => push_path(
                Path::circle(
                    (length(node.cx(), viewport.x), length(node.cy(), viewport.y)),
                    length(node.r(), viewport.min_element()).max(0.0),
                    None,
                ),
                &node,
                node.transform(),
                parent_transform,
                viewport,
                inherited_style,
                paths,
            ),
            TypedNode::Ellipse(node) => {
                let rx = node
                    .rx()
                    .map(|value| length(value, viewport.x))
                    .unwrap_or(0.0)
                    .max(0.0);
                let ry = node
                    .ry()
                    .map(|value| length(value, viewport.y))
                    .unwrap_or(0.0)
                    .max(0.0);
                push_path(
                    Path::oval(
                        Rect::from_xywh(
                            length(node.cx(), viewport.x) - rx,
                            length(node.cy(), viewport.y) - ry,
                            rx * 2.0,
                            ry * 2.0,
                        ),
                        None,
                    ),
                    &node,
                    node.transform(),
                    parent_transform,
                    viewport,
                    inherited_style,
                    paths,
                );
            }
            TypedNode::Polygon(node) => push_path(
                Path::polygon(node.points(), true, None, None),
                &node,
                node.transform(),
                parent_transform,
                viewport,
                inherited_style,
                paths,
            ),
            TypedNode::Polyline(node) => push_path(
                Path::polygon(node.points(), false, None, None),
                &node,
                node.transform(),
                parent_transform,
                viewport,
                inherited_style,
                paths,
            ),
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn push_path(
    path: Path,
    node: &Node,
    local_transform: &Matrix,
    parent_transform: &Matrix,
    viewport: Vec2,
    inherited_style: SvgStyle,
    paths: &mut Vec<SvgPath>,
) {
    let transform = Matrix::concat(parent_transform, local_transform);
    let style = node_style(node, inherited_style, viewport);
    if path.is_empty() || (!style.fill && !style.stroke) {
        return;
    }
    paths.push(SvgPath {
        path: path.with_transform(&transform),
        fill: style.fill,
        fill_color: style.fill_color,
        fill_opacity: style.fill_opacity * style.opacity,
        stroke_color: style.stroke_color,
        stroke_opacity: style.stroke_opacity * style.opacity,
        stroke_width: if style.stroke {
            transform
                .map_radius(style.stroke_width)
                .unwrap_or(style.stroke_width)
        } else {
            0.0
        },
    });
}

fn node_style(node: &Node, inherited: SvgStyle, viewport: Vec2) -> SvgStyle {
    let current_color = *node.color().unwrap_or(&inherited.current_color);
    let paint = |value: Option<&svg::Paint>, inherited_enabled: bool, inherited_color: Color| {
        value.map_or((inherited_enabled, inherited_color), |paint| {
            if paint.is_none() {
                (false, inherited_color)
            } else {
                (
                    true,
                    paint.color().unwrap_or(if paint.is_current_color() {
                        current_color
                    } else {
                        inherited_color
                    }),
                )
            }
        })
    };
    let (fill, fill_color) = paint(node.fill(), inherited.fill, inherited.fill_color);
    let (stroke, stroke_color) = paint(node.stroke(), inherited.stroke, inherited.stroke_color);
    SvgStyle {
        fill,
        fill_color,
        fill_opacity: node.fill_opacity().unwrap_or(inherited.fill_opacity),
        stroke,
        stroke_color,
        stroke_opacity: node.stroke_opacity().unwrap_or(inherited.stroke_opacity),
        stroke_width: node.stroke_width().map_or(inherited.stroke_width, |value| {
            length(value, viewport.min_element())
        }),
        current_color,
        opacity: inherited.opacity * node.opacity().unwrap_or(1.0),
    }
}

fn length(length: &svg::Length, basis: f32) -> f32 {
    use svg::LengthUnit;

    match length.unit {
        LengthUnit::Percentage => length.value * basis / 100.0,
        LengthUnit::IN => length.value * 96.0,
        LengthUnit::CM => length.value * 96.0 / 2.54,
        LengthUnit::MM => length.value * 96.0 / 25.4,
        LengthUnit::PT => length.value * 96.0 / 72.0,
        LengthUnit::PC => length.value * 16.0,
        LengthUnit::EMS => length.value * 16.0,
        LengthUnit::EXS => length.value * 8.0,
        _ => length.value,
    }
}

fn view_box_matrix(
    view_box: Option<Rect>,
    width: f32,
    height: f32,
    aspect: &svg::PreserveAspectRatio,
) -> Matrix {
    let Some(view_box) = view_box.filter(|rect| !rect.is_empty()) else {
        return Matrix::new_identity();
    };
    use svg::preserve_aspect_ratio::{Align, Scale};

    let sx = width / view_box.width();
    let sy = height / view_box.height();
    if aspect.align == Align::None {
        return Matrix::scale_translate((sx, sy), (-view_box.left * sx, -view_box.top * sy));
    }
    let scale = if aspect.scale == Scale::Slice {
        sx.max(sy)
    } else {
        sx.min(sy)
    };
    let remaining_x = width - view_box.width() * scale;
    let remaining_y = height - view_box.height() * scale;
    let align_x = match aspect.align {
        Align::XMidYMin | Align::XMidYMid | Align::XMidYMax => remaining_x * 0.5,
        Align::XMaxYMin | Align::XMaxYMid | Align::XMaxYMax => remaining_x,
        _ => 0.0,
    };
    let align_y = match aspect.align {
        Align::XMinYMid | Align::XMidYMid | Align::XMaxYMid => remaining_y * 0.5,
        Align::XMinYMax | Align::XMidYMax | Align::XMaxYMax => remaining_y,
        _ => 0.0,
    };
    Matrix::scale_translate(
        (scale, scale),
        (
            align_x - view_box.left * scale,
            align_y - view_box.top * scale,
        ),
    )
}

fn draw_mask(canvas: &Canvas, path: &Path, fill: bool, stroke_width: f32, alpha: f32) {
    if path.is_empty() || alpha <= 0.0 || (!fill && stroke_width <= 0.0) {
        return;
    }
    let mask = mask_path(path, fill, stroke_width);
    if mask.is_empty() {
        return;
    }
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(Color::WHITE);
    paint.set_alpha_f(alpha.clamp(0.0, 1.0));
    canvas.draw_path(&mask, &paint);
}

fn mask_path(path: &Path, fill: bool, stroke_width: f32) -> Path {
    let mut mask = PathBuilder::new();
    if fill {
        mask.add_path(path, None);
    }
    if stroke_width > 0.0 {
        let mut stroke = Paint::default();
        stroke.set_style(PaintStyle::Stroke);
        stroke.set_stroke_width(stroke_width);
        skia_safe::path_utils::fill_path_with_paint(path, &stroke, &mut mask, None, None);
    }
    mask.detach()
}
