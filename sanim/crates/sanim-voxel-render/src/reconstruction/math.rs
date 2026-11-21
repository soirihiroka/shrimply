//! Bounded host-side change classification; light resolution stays canonical.
use super::History;
use crate::{RenderSettings, Uniforms};
use sanim_voxel::LightKind;

const MAX_RELATIVE_LIGHT_CHANGE: f32 = 0.1;
const MAX_DIRECTION_CHANGE: f32 = 0.03;
const MAX_POINT_TRAVEL: f32 = 1.0;

fn relative(a: f32, b: f32) -> f32 {
    (a - b).abs() / a.abs().max(b.abs()).max(0.01)
}

/// None rejects history; a nonzero bounded change shortens its useful lifetime.
pub(super) fn reactivity(
    previous: &History,
    settings: &RenderSettings,
    camera: &Uniforms,
) -> Result<Option<f32>, String> {
    if previous.settings.lighting.shadows != settings.lighting.shadows
        || previous.settings.environment.map(|value| value.visible)
            != settings.environment.map(|value| value.visible)
    {
        return Ok(None);
    }
    let mut old = previous.settings.clone();
    old.lighting = settings.lighting;
    old.lights = settings.lights.clone();
    old.environment = settings.environment;
    if old != *settings {
        return Ok(None);
    }
    let mut change = 0f32;
    let old_environment = [
        previous.camera.environment_upper,
        previous.camera.environment_horizon,
        previous.camera.environment_lower,
    ];
    let new_environment = [
        camera.environment_upper,
        camera.environment_horizon,
        camera.environment_lower,
    ];
    if old_environment.iter().flatten().any(|value| *value > 0.)
        != new_environment.iter().flatten().any(|value| *value > 0.)
    {
        return Ok(None);
    }
    for (a, b) in old_environment
        .into_iter()
        .flatten()
        .zip(new_environment.into_iter().flatten())
    {
        change = change.max(relative(a, b) / MAX_RELATIVE_LIGHT_CHANGE);
    }
    if previous.settings.lights != settings.lights
        || previous.settings.lighting != settings.lighting
    {
        let dimensions = [
            camera.dimensions[0],
            camera.dimensions[1],
            camera.dimensions[2],
        ];
        let old = crate::lights::resolved(&previous.settings, dimensions)?;
        let current = crate::lights::resolved(settings, dimensions)?;
        if old.len() != current.len() {
            return Ok(None);
        }
        for (a, b) in old.iter().zip(&current) {
            if a.kind != b.kind {
                return Ok(None);
            }
            if a.radiance.iter().any(|value| *value > 0.)
                != b.radiance.iter().any(|value| *value > 0.)
            {
                return Ok(None);
            }
            for (&a, &b) in a.radiance.iter().zip(&b.radiance) {
                change = change.max(relative(a, b) / MAX_RELATIVE_LIGHT_CHANGE);
            }
            let travel = if a.kind == LightKind::Sun {
                MAX_DIRECTION_CHANGE
            } else {
                MAX_POINT_TRAVEL
            };
            for (&a, &b) in a.position_or_direction.iter().zip(&b.position_or_direction) {
                change = change.max((a - b).abs() / travel);
            }
            change = change.max(relative(a.radius, b.radius) / MAX_RELATIVE_LIGHT_CHANGE);
            change = change.max((a.cos_radius - b.cos_radius).abs() / MAX_DIRECTION_CHANGE);
        }
    }
    Ok((change <= 1.).then_some(change))
}
