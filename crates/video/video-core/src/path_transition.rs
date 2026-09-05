use skia_safe::{
    Color, Color4f, ColorChannel, ImageFilter, Paint, PaintStyle, Path, PathBuilder, PathEffect,
    PathFillType, PathMeasure, Point, Rect, TileMode, color_filters, gradient, image_filters,
    shaders,
};

use crate::generated::GeneratedTransition;
use shrimply_project::project::{TransitionSide, VisualTransitionKind, WriteOrdering};

pub fn partial_path(path: &Path, progress: f32, reverse: bool) -> Path {
    if progress <= 0.0 {
        return Path::default();
    }
    if progress >= 1.0 {
        return path.clone();
    }
    let contours = contours(path);
    let lengths = contours
        .iter()
        .map(|contour| PathMeasure::new(contour, false, None).length())
        .collect::<Vec<_>>();
    let mut remaining = lengths.iter().sum::<f32>() * progress.clamp(0.0, 1.0);
    let mut output = PathBuilder::new();
    let indices: Box<dyn Iterator<Item = usize>> = if reverse {
        Box::new((0..contours.len()).rev())
    } else {
        Box::new(0..contours.len())
    };
    for index in indices {
        let length = lengths[index];
        let visible = remaining.clamp(0.0, length);
        remaining -= visible;
        if visible <= 0.0 {
            continue;
        }
        let mut measure = PathMeasure::new(&contours[index], false, None);
        let (start, end) = if reverse {
            (length - visible, length)
        } else {
            (0.0, visible)
        };
        measure.get_segment(start, end, &mut output, true);
        if visible >= length && measure.is_closed() {
            output.close();
        }
    }
    output.detach()
}

pub use crate::path::contours;

pub fn morphed_path(from: &Path, to: &Path, progress: f32) -> Path {
    const MIN_CONTOUR_KEYPOINTS: usize = 4;

    let progress = progress.clamp(0.0, 1.0);
    if progress <= f32::EPSILON {
        return from.clone();
    }
    if progress >= 1.0 - f32::EPSILON {
        return to.clone();
    }

    let from_contours = contours(from);
    let to_contours = contours(to);
    let contour_count = from_contours.len().max(to_contours.len());
    let mut output = PathBuilder::new_with_fill_type(PathFillType::Winding);
    for index in 0..contour_count {
        let from_contour = from_contours.get(index);
        let to_contour = to_contours.get(index);
        let keypoint_count = from_contour
            .map_or(0, |path| path.points().len())
            .max(to_contour.map_or(0, |path| path.points().len()))
            .max(MIN_CONTOUR_KEYPOINTS);
        let from_points = virtual_keypoints(from_contour, keypoint_count, from.bounds().center());
        let mut to_points = virtual_keypoints(to_contour, keypoint_count, to.bounds().center());
        let from_vectors = from_points
            .iter()
            .map(|point| glam::Vec2::new(point.x, point.y))
            .collect::<Vec<_>>();
        let to_vectors = to_points
            .iter()
            .map(|point| glam::Vec2::new(point.x, point.y))
            .collect::<Vec<_>>();
        to_points.rotate_left(shrimply_math_media::closest_cyclic_shift(
            &from_vectors,
            &to_vectors,
        ));

        for (point_index, (from, to)) in from_points.into_iter().zip(to_points).enumerate() {
            let point = Point::new(
                shrimply_math_media::lerp(from.x, to.x, progress),
                shrimply_math_media::lerp(from.y, to.y, progress),
            );
            if point_index == 0 {
                output.move_to(point);
            } else {
                output.line_to(point);
            }
        }
        output.close();
    }
    output.detach()
}

fn virtual_keypoints(contour: Option<&Path>, count: usize, fallback: Point) -> Vec<Point> {
    let Some(contour) = contour else {
        return vec![fallback; count];
    };
    let mut measure = PathMeasure::new(contour, true, None);
    let length = measure.length();
    if length <= f32::EPSILON {
        return vec![contour.bounds().center(); count];
    }
    (0..count)
        .map(|index| {
            measure
                .pos_tan(length * index as f32 / count as f32)
                .map_or(fallback, |(point, _)| point)
        })
        .collect()
}

pub fn facet_clip(bounds: skia_safe::Rect, index: usize, count: usize) -> Path {
    let width = bounds.width() / count.max(1) as f32;
    let slant = bounds.height() * 0.16;
    let top_left = bounds.left + index as f32 * width + slant;
    let top_right = top_left + width;
    let bottom_left = bounds.left + index as f32 * width - slant;
    let bottom_right = bottom_left + width;
    Path::polygon(
        &[
            (top_left, bounds.top).into(),
            (top_right, bounds.top).into(),
            (bottom_right, bounds.bottom).into(),
            (bottom_left, bounds.bottom).into(),
        ],
        true,
        None,
        None,
    )
}

pub fn facet_glint_clip(bounds: skia_safe::Rect, progress: f32) -> Option<Path> {
    let progress = ((progress - 0.62) / 0.28).clamp(0.0, 1.0);
    (progress > 0.0 && progress < 1.0).then(|| {
        let width = bounds.width() * 0.1;
        let slant = bounds.height() * 0.18;
        let center = shrimply_math_media::lerp(bounds.left - width, bounds.right + width, progress);
        Path::polygon(
            &[
                (center - width + slant, bounds.top).into(),
                (center + width + slant, bounds.top).into(),
                (center + width - slant, bounds.bottom).into(),
                (center - width - slant, bounds.bottom).into(),
            ],
            true,
            None,
            None,
        )
    })
}

pub fn coalesce_mask(bounds: skia_safe::Rect, progress: f32, pool_count: usize) -> Path {
    let mut pools = PathBuilder::new();
    for index in 0..pool_count.clamp(2, 5) {
        let (center, radius) = shrimply_math_media::coalesce_pool(progress, index);
        if radius <= 0.0 {
            continue;
        }
        let center = (
            bounds.left + bounds.width() * center.x,
            bounds.top + bounds.height() * center.y,
        );
        pools.add_oval(
            skia_safe::Rect::from_xywh(
                center.0 - bounds.width() * radius,
                center.1 - bounds.height() * radius,
                bounds.width() * radius * 2.0,
                bounds.height() * radius * 2.0,
            ),
            None,
            None,
        );
    }
    pools.detach()
}

pub fn coalesce_filter(extent: f32, softness: f32) -> Option<ImageFilter> {
    let sigma = (extent * 0.018 * softness.clamp(0.25, 2.5)).clamp(0.5, 28.0);
    let blur = image_filters::blur(
        (sigma, sigma),
        TileMode::Decal,
        None,
        image_filters::CropRect::default(),
    )?;
    let alpha = std::array::from_fn(|index| {
        let value = ((index as f32 - 82.0) / 62.0).clamp(0.0, 1.0);
        (value * value * (3.0 - 2.0 * value) * 255.0).round() as u8
    });
    let threshold = color_filters::table_argb(Some(&alpha), None, None, None)?;
    image_filters::color_filter(threshold, blur, image_filters::CropRect::default())
}

pub fn soft_refraction_filter(
    progress: f32,
    extent: f32,
    strength: f32,
    texture_scale: f32,
) -> Option<ImageFilter> {
    let frequency = 0.015 / texture_scale.clamp(0.25, 3.0);
    let noise = shaders::turbulence((frequency, frequency * 1.2), 2, 7.0, None)?;
    let displacement = image_filters::shader(noise, image_filters::CropRect::default())?;
    image_filters::displacement_map(
        (ColorChannel::R, ColorChannel::G),
        extent * 0.035 * strength.clamp(0.0, 3.0) * (1.0 - progress.clamp(0.0, 1.0)).powi(2),
        displacement,
        None,
        image_filters::CropRect::default(),
    )
}

pub fn morphological_filter(
    progress: f32,
    extent: f32,
    amount: f32,
    softness: f32,
) -> Option<ImageFilter> {
    let remaining = 1.0 - progress.clamp(0.0, 1.0);
    let radius = (extent * 0.035 * amount.clamp(0.0, 3.0) * remaining).clamp(0.01, 42.0);
    let dilated =
        image_filters::dilate((radius, radius), None, image_filters::CropRect::default())?;
    let sigma = (extent * 0.006 * softness.clamp(0.0, 2.0) * remaining).clamp(0.0, 12.0);
    if sigma <= f32::EPSILON {
        Some(dilated)
    } else {
        image_filters::blur(
            (sigma, sigma),
            TileMode::Decal,
            dilated,
            image_filters::CropRect::default(),
        )
    }
}

pub fn contour_current_paint(progress: f32, width: f32, trail: f32) -> Paint {
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(Color::WHITE);
    paint.set_alpha_f(shrimply_math_media::transition_accent(progress) * 0.72);
    paint.set_style(PaintStyle::Stroke);
    paint.set_stroke_width(width.max(1.0));
    let stop = progress.clamp(0.0, 1.0);
    let start = (stop - trail.clamp(0.04, 0.7)).max(0.0);
    paint.set_path_effect(PathEffect::trim(start, stop, None));
    paint
}

pub fn living_fill_paint(
    bounds: Rect,
    progress: f32,
    band_width: f32,
    softness: f32,
    angle_degrees: f32,
) -> Option<Paint> {
    let band = bounds.width().max(bounds.height()) * band_width.clamp(0.03, 0.6);
    let direction = shrimply_math_media::polar_degrees(1.0, angle_degrees);
    let span =
        direction.x.abs() * bounds.width() * 0.5 + direction.y.abs() * bounds.height() * 0.5 + band;
    let center = glam::Vec2::new(bounds.center_x(), bounds.center_y())
        + direction * shrimply_math_media::lerp(-span, span, progress.clamp(0.0, 1.0));
    let colors = [
        Color4f::new(1.0, 1.0, 1.0, 1.0),
        Color4f::new(1.0, 1.0, 1.0, 0.78),
        Color4f::new(1.0, 1.0, 1.0, 0.0),
    ];
    let positions = [0.0, (1.0 - softness.clamp(0.05, 1.0) * 0.7), 1.0];
    let colors = gradient::Colors::new(&colors, Some(&positions), TileMode::Clamp, None);
    let gradient = gradient::Gradient::new(colors, gradient::Interpolation::default());
    let start = center - direction * band;
    let end = center + direction * band;
    let shader = shaders::linear_gradient(((start.x, start.y), (end.x, end.y)), &gradient, None)?;
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_shader(shader);
    Some(paint)
}

pub fn diffused_path(
    path: &Path,
    bounds: Rect,
    progress: f32,
    reverse: bool,
    amount: f32,
    detail: f32,
    seed: u32,
) -> Path {
    let roughness = if reverse { 1.0 - progress } else { progress }.clamp(0.0, 1.0);
    if roughness <= f32::EPSILON {
        return path.clone();
    }
    let extent = bounds.width().max(bounds.height());
    let Some(effect) = PathEffect::discrete(
        (extent * 0.035 / detail.clamp(0.25, 3.0)).max(1.0),
        extent * 0.032 * amount.clamp(0.0, 3.0) * roughness,
        seed,
    ) else {
        return path.clone();
    };
    crate::shaky_path::apply(path, &effect, bounds)
}

pub fn submobject_progress(transition: GeneratedTransition, index: usize, count: usize) -> f32 {
    let lag_ratio = match transition.ordering {
        WriteOrdering::Simultaneous => 0.0,
        WriteOrdering::Sequential => match transition.kind {
            VisualTransitionKind::Write => (4.0 / count.max(1) as f32).min(0.2),
            VisualTransitionKind::Create => 1.0,
            VisualTransitionKind::FacetAssembly => (2.0 / count.max(1) as f32).min(0.12),
            VisualTransitionKind::Coalesce => (1.5 / count.max(1) as f32).min(0.08),
            VisualTransitionKind::ContourCurrent
            | VisualTransitionKind::SoftRefraction
            | VisualTransitionKind::MorphologicalResolve
            | VisualTransitionKind::LivingFill => (1.0 / count.max(1) as f32).min(0.06),
            VisualTransitionKind::Diffusion | VisualTransitionKind::ReverseDiffusion => {
                (1.0 / count.max(1) as f32).min(0.06)
            }
            _ => 0.0,
        },
    };
    let index = if transition.side == TransitionSide::Outro
        && transition.kind == VisualTransitionKind::Write
    {
        count.saturating_sub(index + 1)
    } else {
        index
    };
    let progress = shrimply_math_media::lagged_transition_progress(
        transition.progress,
        index,
        count,
        lag_ratio,
    );
    transition
        .interpolation
        .value(f64::from(match transition.side {
            TransitionSide::Intro => progress,
            TransitionSide::Outro => 1.0 - progress,
        })) as f32
}
