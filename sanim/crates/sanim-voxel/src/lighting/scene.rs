//! Declarative scene lights retain relative coordinates until a containing volume is known.
mod math;
use crate::{Lighting, layout::Coordinate};
pub use math::scene_light_parameters;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const MAX_LIGHTS: usize = 32;
/// Ample HDR range with headroom for inverse-square falloff, light PDFs, and glossy BSDFs.
pub const MAX_LIGHT_INTENSITY: f32 = 1e12;
pub const MIN_LIGHT_DISTANCE: f32 = 0.001;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum LightKind {
    Sun = 0,
    Point = 1,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SunLight {
    pub direction: [f32; 3],
    pub color: [f32; 3],
    /// Irradiance at normal incidence; changing angular size preserves this scale.
    pub intensity: f32,
    pub angular_diameter_degrees: f32,
}
impl Default for SunLight {
    fn default() -> Self {
        let lighting = Lighting::default();
        Self {
            direction: lighting.key_direction,
            color: lighting.key_color,
            intensity: lighting.key_strength,
            angular_diameter_degrees: lighting.sun_diameter_degrees,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PointLight {
    pub position: [Coordinate; 3],
    pub color: [f32; 3],
    /// Normal-incidence irradiance at one voxel-cell distance (inverse-square falloff).
    pub intensity: f32,
    pub radius: f32,
}
impl Default for PointLight {
    fn default() -> Self {
        Self {
            position: [Coordinate::Percent(50.into()); 3],
            color: [1.; 3],
            intensity: 1.,
            radius: 0.25,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Light {
    Sun(SunLight),
    Point(PointLight),
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Environment {
    pub top: [f32; 3],
    pub horizon: [f32; 3],
    pub ground: [f32; 3],
    pub intensity: f32,
    pub visible: bool,
}
impl Default for Environment {
    fn default() -> Self {
        let lighting = Lighting::default();
        Self {
            top: lighting.environment_upper,
            horizon: lighting.environment_horizon,
            ground: lighting.environment_lower,
            intensity: lighting.environment_strength,
            visible: true,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvironmentParameters {
    pub top: [f32; 3],
    pub horizon: [f32; 3],
    pub ground: [f32; 3],
    pub visible: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct SceneLightParameters {
    pub kind: LightKind,
    pub position_or_direction: [f32; 3],
    pub radiance: [f32; 3],
    pub radius: f32,
    pub cos_radius: f32,
    pub tangent: [f32; 3],
    pub bitangent: [f32; 3],
    pub pdf: f32,
    pub cdf: f32,
}

/// A bounded optional list also protects project deserialization before allocating arbitrary light arrays.
pub fn deserialize_lights<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Arc<[Light]>>, D::Error> {
    struct Optional;
    impl<'de> serde::de::Visitor<'de> for Optional {
        type Value = Option<Arc<[Light]>>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "null or at most {MAX_LIGHTS} scene lights")
        }
        fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_some<D: serde::Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> Result<Self::Value, D::Error> {
            let lights = crate::bounded::<_, Light, MAX_LIGHTS>(deserializer)?;
            for light in &lights {
                light.validate().map_err(serde::de::Error::custom)?;
            }
            Ok(Some(lights.into()))
        }
    }
    deserializer.deserialize_option(Optional)
}

/// Validate a fixed-size environment record while loading retained project state.
pub fn deserialize_environment<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Environment>, D::Error> {
    let environment = Option::<Environment>::deserialize(deserializer)?;
    if let Some(environment) = &environment {
        environment.validate().map_err(serde::de::Error::custom)?;
    }
    Ok(environment)
}
