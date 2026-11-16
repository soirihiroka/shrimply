//! Stateless shot composition: path framing, screen-space pan, dolly and lens zoom.
use crate::{Value, voxel::camera::Spec};
use glam::{DQuat, DVec3};
use sanim_voxel::{Camera, Projection, camera::path::resolve_position};

fn optional(value: &Value) -> Option<&Value> {
    if let Value::Some(value) = value {
        Some(value)
    } else {
        None
    }
}
fn position(value: &Value, dimensions: [u32; 3]) -> Result<DVec3, String> {
    resolve_position(crate::voxel::config::coordinates(value)?, dimensions).map(DVec3::from_array)
}
fn vector(value: &Value) -> Result<DVec3, String> {
    let values = value.list()?;
    let mut result = [0.0; 3];
    for (axis, value) in values.iter().enumerate() {
        result[axis] = super::to_f64(value.number()?.value)?;
    }
    Ok(DVec3::from_array(result))
}
pub fn resolve(spec: &Spec, dimensions: [u32; 3], viewport: [u32; 2]) -> Result<Camera, String> {
    let target = optional(spec.value_ref("target"))
        .map(|value| position(value, dimensions))
        .transpose()?;
    let mut up = vector(spec.value_ref("up"))?;
    let (mut eye, mut target) = if matches!(spec.value_ref("orbit"), Value::Bool(true)) {
        let target = target.ok_or("OrbitCamera requires a target")?;
        let azimuth = spec.number("azimuth")?.to_radians();
        let elevation = spec.number("elevation")?.to_radians();
        let direction = DVec3::new(
            azimuth.sin() * elevation.cos(),
            elevation.sin(),
            azimuth.cos() * elevation.cos(),
        );
        (target + direction * spec.number("radius")?, target)
    } else if let Some(curve) = spec.curve() {
        let sample = curve.sample_world(
            dimensions,
            spec.number("progress")?,
            spec.number("look_ahead")?,
        )?;
        let eye = DVec3::from_array(sample.position);
        let target = target.unwrap_or(DVec3::from_array(sample.target));
        // Parallel-transported path frames provide a stable fallback at vertical tangents.
        if up.cross(target - eye).length_squared() < f64::EPSILON {
            up = DVec3::from_array(sample.up);
        }
        (eye, target)
    } else {
        (
            position(
                optional(spec.value_ref("position"))
                    .ok_or("Camera requires a position or a path")?,
                dimensions,
            )?,
            target.ok_or("Camera requires a target")?,
        )
    };
    let delta = target - eye;
    let old_distance = delta.length();
    let forward = delta
        .try_normalize()
        .ok_or("Camera position and target must be distinct")?;
    let distance = old_distance - spec.number("dolly")?;
    if distance <= 0.0 || !distance.is_finite() {
        return Err(
            "Camera dolly would reach or pass its target; reduce by or move the target".into(),
        );
    }
    eye = target - forward * distance;
    let mut right = forward
        .cross(up)
        .try_normalize()
        .ok_or("Camera up cannot be zero or parallel to its view")?;
    up = right.cross(forward);
    let roll = DQuat::from_axis_angle(forward, spec.number("roll")?.to_radians());
    right = roll * right;
    up = roll * up;
    let pan = spec.value_ref("pan").list()?;
    let pan = right * super::to_f64(pan[0].number()?.value)?
        + up * super::to_f64(pan[1].number()?.value)?;
    let offset = pan + vector(spec.value_ref("offset"))?;
    eye += offset;
    target += offset;
    let zoom = spec.number("zoom")?;
    let projection = if spec.value_ref("projection").string()? == "orthographic" {
        Projection::Orthographic {
            height: (spec.number("height")? / zoom) as f32,
        }
    } else {
        let framing = if matches!(spec.value_ref("lock_framing"), Value::Bool(true)) {
            old_distance / distance
        } else {
            1.0
        };
        let fov = 2.0
            * ((0.5 * spec.number("fov")?.to_radians()).tan() * framing / zoom)
                .atan()
                .to_degrees();
        Projection::Perspective {
            vertical_fov_degrees: fov as f32,
        }
    };
    let camera = Camera {
        position: eye.to_array().map(|v| v as f32),
        target: target.to_array().map(|v| v as f32),
        up: up.to_array().map(|v| v as f32),
        projection,
    };
    camera.basis(viewport)?;
    Ok(camera)
}
