use shrimply_background::{
    Background, BackgroundGenerator, CenteredLines, Checkerboard, ColorGradient, Curve,
    GradientMode, Grid, GridLineStyle, NoiseColorMode, NoiseDistribution, PerlinMode, PerlinNoise,
    Rainbow, RainbowBands, RainbowFill, SolidColor, Voronoi, VoronoiFill, VoronoiMetric,
    WhiteNoise,
};
use shrimply_project::project::Time;
use shrimply_render_core::background_spirv as shader;

pub fn uniforms(
    width: u32,
    height: u32,
    time: Time,
    background: &Background,
) -> shader::BackgroundUniforms {
    let mut value = shader::BackgroundUniforms::default();
    value.common.width = width;
    value.common.height = height;
    match &background.generator {
        BackgroundGenerator::SolidColor(config) => solid_color(&mut value, config, time),
        BackgroundGenerator::ColorGradient(config) => gradient(&mut value, config, time),
        BackgroundGenerator::Grid(config) => grid(&mut value, config, time),
        BackgroundGenerator::WhiteNoise(config) => noise(&mut value, config, time),
        BackgroundGenerator::PerlinNoise(config) => perlin(&mut value, config, time),
        BackgroundGenerator::CenteredLines(config) => centered_lines(&mut value, config, time),
        BackgroundGenerator::Rainbow(config) => rainbow(&mut value, config, time),
        BackgroundGenerator::Checkerboard(config) => checker(&mut value, config, time),
        BackgroundGenerator::Voronoi(config) => voronoi(&mut value, config, time),
        BackgroundGenerator::TestPattern => value.common.kind = shader::BackgroundKind::TestPattern,
    }
    value
}

fn solid_color(value: &mut shader::BackgroundUniforms, config: &SolidColor, time: Time) {
    *value = solid(
        value.common.width,
        value.common.height,
        config.color.value_at(time),
    );
}

pub fn solid(
    width: u32,
    height: u32,
    color: shrimply_render_core::Color<u8>,
) -> shader::BackgroundUniforms {
    let mut value = shader::BackgroundUniforms::default();
    value.common.width = width;
    value.common.height = height;
    value.common.kind = shader::BackgroundKind::ColorGradient;
    value.gradient.mode = 0;
    value.gradient.color_a = color.to_srgba();
    value
}

fn curve(value: Curve) -> u32 {
    match value {
        Curve::Step => 0,
        Curve::Linear => 1,
        Curve::Smooth => 2,
        Curve::Smoother => 3,
    }
}

fn gradient(value: &mut shader::BackgroundUniforms, config: &ColorGradient, time: Time) {
    value.common.kind = shader::BackgroundKind::ColorGradient;
    value.gradient.mode = match config.mode.value_at(time) {
        GradientMode::Solid => 0,
        GradientMode::Linear => 1,
        GradientMode::Radial => 2,
        GradientMode::Conic => 3,
    };
    value.gradient.curve = curve(config.curve.value_at(time));
    value.gradient.angle = config.angle_degrees.value_at(time);
    value.gradient.scale = config.scale.value_at(time);
    value.gradient.color_a = config.color_a.value_at(time).to_srgba();
    value.gradient.color_b = config.color_b.value_at(time).to_srgba();
    value.gradient.center = config.center.value_at(time).to_array();
    value.gradient.position = config.position.value_at(time).to_array();
    value.gradient.cycle_position = config.cycle_position.value_at(time);
}

fn grid(value: &mut shader::BackgroundUniforms, config: &Grid, time: Time) {
    value.common.kind = shader::BackgroundKind::Grid;
    value.grid.background = config.background_color.value_at(time).to_srgba();
    value.grid.horizontal = config.horizontal_color.value_at(time).to_srgba();
    value.grid.vertical = config.vertical_color.value_at(time).to_srgba();
    value.grid.spacing = config.spacing.value_at(time).to_array();
    value.grid.line_width = config.line_width.value_at(time).to_array();
    value.grid.position = config.position.value_at(time).to_array();
    value.grid.rotation = config.rotation_degrees.value_at(time);
    value.grid.line_style = match config.line_style.value_at(time) {
        GridLineStyle::Solid => 0,
        GridLineStyle::Dashed => 1,
        GridLineStyle::Dotted => 2,
    };
    value.grid.dash_length = config.dash_length.value_at(time);
    value.grid.dash_gap = config.dash_gap.value_at(time);
    value.grid.dash_position = config.dash_position.value_at(time);
    value.grid.wobble_amount = config.wobble_amount.value_at(time);
    value.grid.wobble_scale = config.wobble_scale.value_at(time);
    value.grid.wobble_position = config.wobble_position.value_at(time);
    value.grid.middle_padding = config.middle_padding.value_at(time).to_array();
    value.grid.padding_randomness = config.padding_randomness.value_at(time).to_array();
    value.grid.seed = config.seed.value_at(time);
}

fn noise(value: &mut shader::BackgroundUniforms, config: &WhiteNoise, time: Time) {
    value.common.kind = shader::BackgroundKind::WhiteNoise;
    value.noise.distribution = match config.distribution.value_at(time) {
        NoiseDistribution::Uniform => 0,
        NoiseDistribution::Gaussian => 1,
        NoiseDistribution::Binary => 2,
    };
    value.noise.color_mode = match config.color_mode.value_at(time) {
        NoiseColorMode::Monochrome => 0,
        NoiseColorMode::Rgb => 1,
        NoiseColorMode::Duotone => 2,
    };
    value.noise.pixel_size = config.pixel_size.value_at(time).max(1);
    value.noise.epoch = if config.animated.value_at(time).get() {
        shrimply_math_media::background_noise_epoch(time, config.refresh_interval.value_at(time))
    } else {
        0
    };
    value.noise.color_a = config.color_a.value_at(time).to_srgba();
    value.noise.color_b = config.color_b.value_at(time).to_srgba();
    value.noise.brightness = config.brightness.value_at(time);
    value.noise.contrast = config.contrast.value_at(time);
    value.noise.seed = config.seed.value_at(time);
}

fn perlin(value: &mut shader::BackgroundUniforms, config: &PerlinNoise, time: Time) {
    value.common.kind = shader::BackgroundKind::PerlinNoise;
    value.perlin.mode = match config.mode.value_at(time) {
        PerlinMode::Fbm => 0,
        PerlinMode::Turbulence => 1,
        PerlinMode::Ridged => 2,
    };
    value.perlin.octaves = config.octaves.value_at(time).clamp(1, 8);
    value.perlin.seed = config.seed.value_at(time);
    value.perlin.scale = config.scale.value_at(time);
    value.perlin.color_a = config.color_a.value_at(time).to_srgba();
    value.perlin.color_b = config.color_b.value_at(time).to_srgba();
    value.perlin.lacunarity = config.lacunarity.value_at(time);
    value.perlin.persistence = config.persistence.value_at(time);
    value.perlin.contrast = config.contrast.value_at(time);
    value.perlin.evolution = config.evolution.value_at(time);
    value.perlin.position = config.position.value_at(time).to_array();
    value.perlin.warp_amount = config.warp_amount.value_at(time);
    value.perlin.warp_scale = config.warp_scale.value_at(time);
}

fn centered_lines(value: &mut shader::BackgroundUniforms, config: &CenteredLines, time: Time) {
    value.common.kind = shader::BackgroundKind::CenteredLines;
    value.centered_lines.background = config.background_color.value_at(time).to_srgba();
    value.centered_lines.line = config.line_color.value_at(time).to_srgba();
    value.centered_lines.center = config.center.value_at(time).to_array();
    value.centered_lines.rotation_degrees = config.rotation_degrees.value_at(time);
    value.centered_lines.line_count = config.line_count.value_at(time);
    value.centered_lines.line_width = config.line_width.value_at(time);
    value.centered_lines.line_width_randomness = config.line_width_randomness.value_at(time);
    value.centered_lines.line_length = config.line_length.value_at(time);
    value.centered_lines.line_length_randomness = config.line_length_randomness.value_at(time);
    value.centered_lines.line_offset = config.line_offset.value_at(time);
    value.centered_lines.line_offset_randomness = config.line_offset_randomness.value_at(time);
    value.centered_lines.angular_randomness = config.angular_randomness.value_at(time);
    value.centered_lines.fade_length = config.fade_length.value_at(time);
    value.centered_lines.seed = config.seed.value_at(time);
}

fn rainbow(value: &mut shader::BackgroundUniforms, config: &Rainbow, time: Time) {
    value.common.kind = shader::BackgroundKind::Rainbow;
    value.rainbow.fill = match config.fill.value_at(time) {
        RainbowFill::Linear => 0,
        RainbowFill::Radial => 1,
        RainbowFill::Conic => 2,
    };
    value.rainbow.bands = match config.bands.value_at(time) {
        RainbowBands::Smooth => 0,
        RainbowBands::Stepped => 1,
    };
    value.rainbow.band_count = config.band_count.value_at(time);
    value.rainbow.angle = config.angle_degrees.value_at(time);
    value.rainbow.center = config.center.value_at(time).to_array();
    value.rainbow.scale = config.scale.value_at(time);
    value.rainbow.saturation = config.saturation.value_at(time);
    value.rainbow.brightness = config.brightness.value_at(time);
    value.rainbow.alpha = config.alpha.value_at(time);
    value.rainbow.position = config.position.value_at(time).to_array();
    value.rainbow.hue_position = config.hue_position.value_at(time);
}

fn checker(value: &mut shader::BackgroundUniforms, config: &Checkerboard, time: Time) {
    value.common.kind = shader::BackgroundKind::Checkerboard;
    value.checker.color_a = config.color_a.value_at(time).to_srgba();
    value.checker.color_b = config.color_b.value_at(time).to_srgba();
    value.checker.cell_size = config.cell_size.value_at(time).to_array();
    value.checker.edge_softness = config.edge_softness.value_at(time);
    value.checker.position = config.position.value_at(time).to_array();
    value.checker.rotation = config.rotation_degrees.value_at(time);
}

fn voronoi(value: &mut shader::BackgroundUniforms, config: &Voronoi, time: Time) {
    value.common.kind = shader::BackgroundKind::Voronoi;
    value.voronoi.fill = match config.fill.value_at(time) {
        VoronoiFill::Distance => 0,
        VoronoiFill::Cells => 1,
        VoronoiFill::Edges => 2,
    };
    value.voronoi.metric = match config.metric.value_at(time) {
        VoronoiMetric::Euclidean => 0,
        VoronoiMetric::Manhattan => 1,
        VoronoiMetric::Chebyshev => 2,
    };
    value.voronoi.seed = config.seed.value_at(time);
    value.voronoi.cell_size = config.cell_size.value_at(time);
    value.voronoi.color_a = config.color_a.value_at(time).to_srgba();
    value.voronoi.color_b = config.color_b.value_at(time).to_srgba();
    value.voronoi.edge_color = config.edge_color.value_at(time).to_srgba();
    value.voronoi.jitter = config.jitter.value_at(time);
    value.voronoi.edge_width = config.edge_width.value_at(time);
    value.voronoi.position = config.position.value_at(time).to_array();
    value.voronoi.motion_amount = config.motion_amount.value_at(time);
    value.voronoi.motion_position = config.motion_position.value_at(time);
}
