use super::{LightParameters, Lighting};
use glam::DVec3;

impl Lighting {
    pub fn parameters(&self) -> Result<LightParameters, String> {
        self.validate_fields()?;
        let direction = DVec3::from_array(self.key_direction.map(f64::from)).normalize();
        let helper = if direction.y.abs() > 0.9 {
            DVec3::X
        } else {
            DVec3::Y
        };
        let tangent = helper.cross(direction).normalize();
        let bitangent = direction.cross(tangent);
        let radius = match self.shadows.mode {
            crate::ShadowMode::Off | crate::ShadowMode::Hard => 0.0,
            crate::ShadowMode::Auto | crate::ShadowMode::Soft => {
                f64::from(self.sun_diameter_degrees).to_radians() * 0.5
            }
        };
        let cos_radius = radius.cos();
        let normalization = 2.0 / (1.0 + cos_radius);
        let key_radiance = scaled(self.key_color, f64::from(self.key_strength) * normalization)?;
        let sample_count = match self.shadows.mode {
            crate::ShadowMode::Auto | crate::ShadowMode::Soft => self.shadows.samples,
            _ => 1,
        };
        if key_radiance
            .iter()
            .any(|channel| f64::from(*channel) * f64::from(sample_count) > f64::from(f32::MAX))
        {
            return Err("voxel key-light radiance × shadow samples exceeds the GPU range; reduce light strength or shadow samples".into());
        }
        let environment_upper =
            scaled(self.environment_upper, f64::from(self.environment_strength))?;
        let environment_horizon = scaled(
            self.environment_horizon,
            f64::from(self.environment_strength),
        )?;
        let environment_lower =
            scaled(self.environment_lower, f64::from(self.environment_strength))?;
        Ok(LightParameters {
            direction: direction.to_array().map(|value| value as f32),
            tangent: tangent.to_array().map(|value| value as f32),
            bitangent: bitangent.to_array().map(|value| value as f32),
            cos_radius: cos_radius as f32,
            sin_radius: radius.sin() as f32,
            // The cone's mean cosine is (1 + cos(radius))/2. Keep normal-incidence
            // irradiance fixed while changing softness, including the delta-light limit.
            normalization: normalization as f32,
            key_radiance,
            environment_upper,
            environment_horizon,
            environment_lower,
        })
    }
}

pub(super) fn scaled(color: [f32; 3], strength: f64) -> Result<[f32; 3], String> {
    let color = color.map(|channel| f64::from(channel) * strength);
    if color
        .iter()
        .any(|channel| !channel.is_finite() || *channel > f64::from(f32::MAX))
    {
        return Err("voxel lighting color × strength exceeds the GPU radiance range; reduce the color or light strength".into());
    }
    Ok(color.map(|channel| channel as f32))
}
