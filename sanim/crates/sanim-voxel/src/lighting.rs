//! Linear-light illumination, independent of the visible background.
mod math;
pub mod scene;
use crate::Shadows;
pub use scene::{
    Environment, EnvironmentParameters, Light, LightKind, MAX_LIGHT_INTENSITY, MAX_LIGHTS,
    MIN_LIGHT_DISTANCE, PointLight, SceneLightParameters, SunLight, deserialize_lights,
    scene_light_parameters,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Lighting {
    pub key_direction: [f32; 3],
    /// Linear RGB multiplier for the key light.
    pub key_color: [f32; 3],
    /// Irradiance scale at normal incidence, preserved when the sun size changes.
    pub key_strength: f32,
    pub sun_diameter_degrees: f32,
    pub environment_strength: f32,
    /// Linear RGB environment radiance at zenith, horizon, and nadir.
    pub environment_upper: [f32; 3],
    pub environment_horizon: [f32; 3],
    pub environment_lower: [f32; 3],
    pub shadows: Shadows,
}

impl Default for Lighting {
    fn default() -> Self {
        Self {
            key_direction: [-0.4, 0.8, 0.5],
            key_color: [1.0, 0.95, 0.88],
            key_strength: 1.1,
            sun_diameter_degrees: 8.0,
            environment_strength: 1.0,
            environment_upper: [0.32, 0.38, 0.48],
            environment_horizon: [0.20, 0.22, 0.25],
            environment_lower: [0.10, 0.09, 0.08],
            shadows: Shadows::default(),
        }
    }
}

impl Lighting {
    pub fn validate(&self) -> Result<(), String> {
        self.parameters().map(drop)
    }

    fn validate_fields(&self) -> Result<(), String> {
        self.shadows.validate()?;
        if self.key_direction.iter().any(|value| !value.is_finite())
            || self.key_direction.iter().all(|value| *value == 0.0)
        {
            return Err("voxel key-light direction must be finite and nonzero".into());
        }
        if self
            .key_color
            .iter()
            .chain(&self.environment_upper)
            .chain(&self.environment_horizon)
            .chain(&self.environment_lower)
            .any(|value| !value.is_finite() || *value < 0.0)
            || !self.key_strength.is_finite()
            || self.key_strength < 0.0
            || !self.environment_strength.is_finite()
            || self.environment_strength < 0.0
        {
            return Err("voxel lighting colors and key strength must be finite and non-negative; environment colors are linear radiance".into());
        }
        if !self.sun_diameter_degrees.is_finite()
            || !(0.0..=180.0).contains(&self.sun_diameter_degrees)
        {
            return Err("voxel sun diameter must be finite and within 0..=180 degrees; zero selects hard shadows".into());
        }
        Ok(())
    }
}

/// Precomputed once per view, rather than rebuilding a light basis at each hit.
#[derive(Clone, Copy, Debug)]
pub struct LightParameters {
    pub direction: [f32; 3],
    pub tangent: [f32; 3],
    pub bitangent: [f32; 3],
    pub cos_radius: f32,
    pub sin_radius: f32,
    pub normalization: f32,
    pub key_radiance: [f32; 3],
    pub environment_upper: [f32; 3],
    pub environment_horizon: [f32; 3],
    pub environment_lower: [f32; 3],
}
