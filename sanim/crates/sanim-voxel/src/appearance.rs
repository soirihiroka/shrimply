//! Bounded presentation controls; automatic modes keep preview cheap and path tracing physical.
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

pub const MAX_SHADOW_SAMPLES: u32 = 8;
pub const MAX_AO_SAMPLES: u32 = 16;
pub const MAX_AO_DISTANCE: f32 = crate::MAX_DIMENSION as f32 * 2.0;

macro_rules! modes {
    ($name:ident { $default:ident => $default_name:literal, $($variant:ident => $label:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
        pub enum $name {
            #[default]
            #[serde(rename = $default_name)]
            $default,
            $(#[serde(rename = $label)] $variant),+
        }
        impl $name {
            pub const ALL: &'static [Self] = &[Self::$default, $(Self::$variant),+];
            pub const fn name(self) -> &'static str {
                match self { Self::$default => $default_name, $(Self::$variant => $label),+ }
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result { formatter.write_str(self.name()) }
        }
        impl FromStr for $name {
            type Err = String;
            fn from_str(value: &str) -> Result<Self, String> {
                Self::ALL.iter().copied().find(|mode| mode.name() == value).ok_or_else(|| format!("unknown {} `{value}`; use {}", stringify!($name), Self::ALL.iter().map(|mode| mode.name()).collect::<Vec<_>>().join(", ")))
            }
        }
    };
}

modes!(ShadowMode { Auto => "auto", Off => "off", Hard => "hard", Soft => "soft" });
modes!(AoMode { Auto => "auto", Off => "off", Local => "local", RayTraced => "ray-traced" });

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shadows {
    pub mode: ShadowMode,
    pub strength: f32,
    /// Independent visibility samples, used for soft shadows only.
    pub samples: u32,
}

impl Default for Shadows {
    fn default() -> Self {
        Self {
            mode: ShadowMode::Auto,
            strength: 1.0,
            samples: 1,
        }
    }
}

impl Shadows {
    pub fn validate(&self) -> Result<(), String> {
        if !self.strength.is_finite() || !(0.0..=1.0).contains(&self.strength) {
            return Err("voxel shadow strength must be finite and within 0..=1".into());
        }
        if self.samples == 0 || self.samples > MAX_SHADOW_SAMPLES {
            return Err(format!(
                "voxel soft shadows require 1..={MAX_SHADOW_SAMPLES} visibility samples"
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AmbientOcclusion {
    pub mode: AoMode,
    pub strength: f32,
    /// Maximum visibility-ray distance in voxel-cell units.
    pub distance: f32,
    pub samples: u32,
}

impl Default for AmbientOcclusion {
    fn default() -> Self {
        Self {
            mode: AoMode::Auto,
            strength: 0.4,
            distance: 8.0,
            samples: 4,
        }
    }
}

impl AmbientOcclusion {
    pub fn validate(&self) -> Result<(), String> {
        if !self.strength.is_finite() || !(0.0..=1.0).contains(&self.strength) {
            return Err("voxel ambient-occlusion strength must be finite and within 0..=1".into());
        }
        if !self.distance.is_finite() || self.distance <= 0.0 || self.distance > MAX_AO_DISTANCE {
            return Err(format!(
                "voxel ambient-occlusion distance must be finite and within 0..={MAX_AO_DISTANCE} voxel cells (excluding zero)"
            ));
        }
        if self.samples == 0 || self.samples > MAX_AO_SAMPLES {
            return Err(format!(
                "voxel ray-traced ambient occlusion requires 1..={MAX_AO_SAMPLES} visibility samples"
            ));
        }
        Ok(())
    }
}
