mod uniforms;
pub use uniforms::{solid, uniforms};

use shrimply_core::timeline_value::{TimelineBase, TimelineExpressionValue, TimelineValue};
use shrimply_evaluation::{TransformExpressionCache, VisualEvaluation};

pub fn resolve(
    source: &shrimply_background::Background,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> shrimply_background::Background {
    let mut output = source.clone();
    macro_rules! resolve_fields {
        ($source:expr, $output:expr; $($field:ident),* $(,)?) => {{
            $(resolve_value(&$source.$field, &mut $output.$field, evaluation, expressions);)*
        }};
    }
    match (&source.generator, &mut output.generator) {
        (
            shrimply_background::BackgroundGenerator::SolidColor(source),
            shrimply_background::BackgroundGenerator::SolidColor(output),
        ) => resolve_fields!(source, output; color),
        (
            shrimply_background::BackgroundGenerator::ColorGradient(source),
            shrimply_background::BackgroundGenerator::ColorGradient(output),
        ) => {
            resolve_fields!(source, output; color_a, color_b, center, angle_degrees, scale, position, cycle_position)
        }
        (
            shrimply_background::BackgroundGenerator::Grid(source),
            shrimply_background::BackgroundGenerator::Grid(output),
        ) => {
            resolve_fields!(source, output; background_color, horizontal_color, vertical_color, spacing, line_width, position, rotation_degrees, dash_length, dash_gap, dash_position, wobble_amount, wobble_scale, wobble_position, middle_padding, padding_randomness);
            resolve_seed(&source.seed, &mut output.seed, evaluation, expressions);
        }
        (
            shrimply_background::BackgroundGenerator::WhiteNoise(source),
            shrimply_background::BackgroundGenerator::WhiteNoise(output),
        ) => {
            resolve_fields!(source, output; color_a, color_b, pixel_size, brightness, contrast, animated, refresh_interval);
            resolve_seed(&source.seed, &mut output.seed, evaluation, expressions);
        }
        (
            shrimply_background::BackgroundGenerator::PerlinNoise(source),
            shrimply_background::BackgroundGenerator::PerlinNoise(output),
        ) => {
            resolve_fields!(source, output; color_a, color_b, scale, octaves, lacunarity, persistence, contrast, position, evolution, warp_amount, warp_scale);
            resolve_seed(&source.seed, &mut output.seed, evaluation, expressions);
        }
        (
            shrimply_background::BackgroundGenerator::CenteredLines(source),
            shrimply_background::BackgroundGenerator::CenteredLines(output),
        ) => {
            resolve_fields!(source, output; background_color, line_color, center, rotation_degrees, line_count, line_width, line_width_randomness, line_length, line_length_randomness, line_offset, line_offset_randomness, angular_randomness, fade_length);
            resolve_seed(&source.seed, &mut output.seed, evaluation, expressions);
        }
        (
            shrimply_background::BackgroundGenerator::Rainbow(source),
            shrimply_background::BackgroundGenerator::Rainbow(output),
        ) => {
            resolve_fields!(source, output; band_count, center, angle_degrees, scale, saturation, brightness, alpha, position, hue_position)
        }
        (
            shrimply_background::BackgroundGenerator::Checkerboard(source),
            shrimply_background::BackgroundGenerator::Checkerboard(output),
        ) => {
            resolve_fields!(source, output; color_a, color_b, cell_size, edge_softness, position, rotation_degrees)
        }
        (
            shrimply_background::BackgroundGenerator::Voronoi(source),
            shrimply_background::BackgroundGenerator::Voronoi(output),
        ) => {
            resolve_fields!(source, output; color_a, color_b, edge_color, cell_size, jitter, edge_width, position, motion_amount, motion_position);
            resolve_seed(&source.seed, &mut output.seed, evaluation, expressions);
        }
        (
            shrimply_background::BackgroundGenerator::TestPattern,
            shrimply_background::BackgroundGenerator::TestPattern,
        ) => {}
        _ => unreachable!("cloned background generator kind must match"),
    }
    output
}

fn resolve_seed(
    source: &TimelineValue<u32>,
    output: &mut TimelineValue<u32>,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) {
    let mut stepped = source.clone();
    if let TimelineBase::Keyframes(keyframes) = &source.base {
        let value = keyframes
            .iter()
            .rev()
            .find(|keyframe| keyframe.time <= evaluation.local_time())
            .map(|keyframe| keyframe.value)
            .unwrap_or_else(|| source.fallback());
        stepped.base = TimelineBase::Const(value);
    }
    resolve_value(&stepped, output, evaluation, expressions);
}

fn resolve_value<T>(
    source: &TimelineValue<T>,
    output: &mut TimelineValue<T>,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) where
    T: TimelineExpressionValue,
{
    output.base = TimelineBase::Const(shrimply_evaluation::resolve(
        source,
        evaluation,
        expressions,
    ));
    output.expression = None;
}
