use super::*;
use crate::{ShadowMode, Shadows};
use num_traits::ToPrimitive;

const DISTANCE_HEADROOM: f64 = 16.;
const UNIFORM_SAMPLING_WEIGHT: f64 = 0.5;

fn color(color: [f32; 3], intensity: f32) -> Result<[f32; 3], String> {
    if !intensity.is_finite()
        || !(0.0..=MAX_LIGHT_INTENSITY).contains(&intensity)
        || color
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
    {
        return Err(
            "voxel light colors must be finite linear RGB within 0..=1; intensity must be finite and within 0..=1e12 (use intensity for HDR brightness)"
                .into(),
        );
    }
    crate::lighting::math::scaled(color, f64::from(intensity))
}
fn position(values: [Coordinate; 3], dimensions: [u32; 3]) -> Result<[f32; 3], String> {
    let mut result = [0.; 3];
    for axis in 0..3 {
        let value = match values[axis] {
            Coordinate::Cells(value) => value.to_f64(),
            Coordinate::Percent(value) => value
                .to_f64()
                .map(|value| value * f64::from(dimensions[axis]) / 100.),
        }
        .ok_or("point-light position is outside the numeric range")?;
        if !value.is_finite() || value.abs() > f64::from(f32::MAX).sqrt() / DISTANCE_HEADROOM {
            return Err("point-light position exceeds the finite GPU distance range; use closer coordinates".into());
        }
        result[axis] = value as f32;
    }
    Ok(result)
}
impl Light {
    pub fn validate(&self) -> Result<(), String> {
        self.parameters([1; 3], ShadowMode::Soft).map(drop)
    }
    pub fn parameters(
        &self,
        dimensions: [u32; 3],
        mode: ShadowMode,
    ) -> Result<SceneLightParameters, String> {
        let mut output = SceneLightParameters {
            kind: LightKind::Sun,
            position_or_direction: [0.; 3],
            radiance: [0.; 3],
            radius: 0.,
            cos_radius: 1.,
            tangent: [0.; 3],
            bitangent: [0.; 3],
            pdf: 0.,
            cdf: 0.,
        };
        match self {
            Light::Sun(light) => {
                color(light.color, light.intensity)?;
                let parameters = Lighting {
                    key_direction: light.direction,
                    key_color: light.color,
                    key_strength: light.intensity,
                    sun_diameter_degrees: light.angular_diameter_degrees,
                    shadows: Shadows {
                        mode,
                        ..Default::default()
                    },
                    ..Default::default()
                }
                .parameters()?;
                output.position_or_direction = parameters.direction;
                output.radiance = parameters.key_radiance;
                output.cos_radius = parameters.cos_radius;
                output.tangent = parameters.tangent;
                output.bitangent = parameters.bitangent;
            }
            Light::Point(light) => {
                if !light.radius.is_finite()
                    || light.radius < 0.
                    || f64::from(light.radius) > f64::from(f32::MAX).sqrt() / DISTANCE_HEADROOM
                {
                    return Err("point-light radius must be finite, non-negative, and within the GPU distance range; zero selects a point source".into());
                }
                output.kind = LightKind::Point;
                output.position_or_direction = position(light.position, dimensions)?;
                output.radiance = color(light.color, light.intensity)?;
                output.radius = light.radius;
            }
        }
        Ok(output)
    }
}
impl Environment {
    pub fn validate(&self) -> Result<(), String> {
        self.parameters().map(drop)
    }
    pub fn parameters(&self) -> Result<EnvironmentParameters, String> {
        Ok(EnvironmentParameters {
            top: color(self.top, self.intensity)?,
            horizon: color(self.horizon, self.intensity)?,
            ground: color(self.ground, self.intensity)?,
            visible: self.visible,
        })
    }
}

pub fn scene_light_parameters(
    lights: &[Light],
    dimensions: [u32; 3],
    mode: ShadowMode,
) -> Result<Vec<SceneLightParameters>, String> {
    if lights.len() > MAX_LIGHTS {
        return Err(format!(
            "voxel scene has {} lights; maximum {MAX_LIGHTS}. Merge lights or reduce their count",
            lights.len()
        ));
    }
    let mut parameters = lights
        .iter()
        .map(|light| light.parameters(dimensions, mode))
        .collect::<Result<Vec<_>, _>>()?;
    parameters.retain(|light| light.radiance.iter().any(|value| *value > 0.));
    let power =
        |light: &SceneLightParameters| light.radiance.into_iter().map(f64::from).fold(0., f64::max);
    let total = parameters.iter().map(power).sum::<f64>();
    let count = parameters.len();
    let mut cdf = 0.;
    for light in &mut parameters {
        let power = power(light);
        let pdf =
            UNIFORM_SAMPLING_WEIGHT / count as f64 + (1. - UNIFORM_SAMPLING_WEIGHT) * power / total;
        let distance_headroom = if light.kind == LightKind::Point {
            1. / f64::from(MIN_LIGHT_DISTANCE).powi(2)
        } else {
            1.
        };
        if light.radiance.iter().any(|value| {
            f64::from(*value) * distance_headroom / pdf
                > f64::from(f32::MAX) / f64::from(crate::appearance::MAX_SHADOW_SAMPLES)
        }) {
            return Err("voxel light intensity exceeds the finite GPU sampling range; reduce intensity or increase the source distance".into());
        }
        cdf += pdf;
        light.pdf = pdf as f32;
        light.cdf = cdf as f32;
    }
    if let Some(last) = parameters.last_mut() {
        last.cdf = 1.;
    }
    Ok(parameters)
}
