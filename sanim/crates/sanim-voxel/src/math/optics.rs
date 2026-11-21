//! Validated optical coefficients are shared by CPU tools and the GPU material uploader.
use crate::OpticalProperties;

pub const MIN_WAVE_LENGTH: f32 = 0.01;
pub const MAX_WAVE_LENGTH: f32 = 4096.;
pub const MAX_WAVE_SPEED: f32 = 64.;
const MIN_TRANSMITTANCE: f64 = 1e-6;

impl OpticalProperties {
    /// Conservative shared bound for finite shader wave phase, including harmonic headroom.
    pub const MAX_WAVE_TIME: f64 = f32::MAX as f64
        / (4. * std::f64::consts::TAU * MAX_WAVE_SPEED as f64 / MIN_WAVE_LENGTH as f64);
    pub fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            ("transmission", self.transmission),
            ("roughness", self.roughness),
            ("wave_strength", self.wave_strength),
        ] {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(format!(
                    "voxel material {name} must be finite and within 0..=1"
                ));
            }
        }
        if !self.ior.is_finite() || !(1.0..=4.0).contains(&self.ior) {
            return Err(
                "voxel material ior must be finite and within 1..=4 (water:1.333, glass:1.5)"
                    .into(),
            );
        }
        if self
            .attenuation_color
            .iter()
            .any(|channel| !channel.is_finite() || !(0.0..=1.0).contains(channel))
        {
            return Err("voxel material attenuation_color requires finite linear RGB transmittance within 0..=1".into());
        }
        if !self.attenuation_distance.is_finite() || self.attenuation_distance <= 0. {
            return Err("voxel material attenuation_distance must be finite and greater than zero voxel cells".into());
        }
        if self
            .absorption()
            .iter()
            .any(|coefficient| !coefficient.is_finite())
        {
            return Err("voxel material attenuation exceeds the GPU numeric range; increase attenuation_distance or use brighter attenuation_color".into());
        }
        if !self.wave_length.is_finite()
            || !(MIN_WAVE_LENGTH..=MAX_WAVE_LENGTH).contains(&self.wave_length)
        {
            return Err(format!(
                "voxel material wave_length must be finite and within {MIN_WAVE_LENGTH}..={MAX_WAVE_LENGTH} voxel cells"
            ));
        }
        if !self.wave_speed.is_finite() || self.wave_speed.abs() > MAX_WAVE_SPEED {
            return Err(format!(
                "voxel material wave_speed must be finite and within -{MAX_WAVE_SPEED}..={MAX_WAVE_SPEED} voxel cells per second"
            ));
        }
        Ok(())
    }
    /// Beer–Lambert absorption per voxel-cell distance. Black is clamped to a finite near-opaque transmittance.
    pub fn absorption(&self) -> [f32; 3] {
        self.attenuation_color.map(|channel| {
            if channel >= 1. {
                0.
            } else {
                (-f64::from(channel).max(MIN_TRANSMITTANCE).ln()
                    / f64::from(self.attenuation_distance)) as f32
            }
        })
    }
    pub fn transmittance(&self, distance: f64) -> Result<[f64; 3], String> {
        self.validate()?;
        if !distance.is_finite() || distance < 0. {
            return Err("voxel optical distance must be finite and non-negative".into());
        }
        Ok(self
            .absorption()
            .map(|coefficient| (-f64::from(coefficient) * distance).exp()))
    }
}
