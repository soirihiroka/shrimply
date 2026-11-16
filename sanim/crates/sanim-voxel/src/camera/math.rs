//! Shared camera fitting and pixel-ray calculations for GPU rendering and inspection.
use super::{Camera, CameraBasis, CameraRig, Projection, Ray, View};
use glam::{DVec3, Quat, Vec3};

const FIT_MARGIN: f32 = 1.08;
const MAX_VERTICAL_FOV: f32 = 179.0;

impl CameraRig {
    pub fn resolve(&self, dimensions: [u32; 3], viewport: [u32; 2]) -> Result<Camera, String> {
        if dimensions.contains(&0) || viewport.contains(&0) {
            return Err("voxel camera dimensions and viewport must be nonzero".into());
        }
        if !self.yaw_degrees.is_finite()
            || !self.pitch_degrees.is_finite()
            || !self.zoom.is_finite()
            || self.zoom <= 0.0
        {
            return Err(
                "voxel camera yaw and pitch must be finite, and zoom must be finite and positive"
                    .into(),
            );
        }
        let (direction, up) = match self.view {
            View::Front => (Vec3::Z, Vec3::Y),
            View::Back => (-Vec3::Z, Vec3::Y),
            View::Left => (-Vec3::X, Vec3::Y),
            View::Right => (Vec3::X, Vec3::Y),
            View::Top => (Vec3::Y, -Vec3::Z),
            View::Bottom => (-Vec3::Y, Vec3::Z),
            View::Isometric => (Vec3::ONE.normalize(), Vec3::Y),
        };
        let yaw = Quat::from_axis_angle(Vec3::Y, self.yaw_degrees.to_radians());
        let right = (-direction).cross(up).normalize();
        let pitch = Quat::from_axis_angle(right, self.pitch_degrees.to_radians());
        let rotation = yaw * pitch;
        let direction = rotation * direction;
        let up = rotation * up;
        let forward = -direction;
        let right = forward.cross(up).normalize();
        let up = right.cross(forward).normalize();
        let extent = Vec3::new(
            dimensions[0] as f32,
            dimensions[1] as f32,
            dimensions[2] as f32,
        );
        let center = extent * 0.5;
        let aspect = viewport[0] as f32 / viewport[1] as f32;
        let half_width = right.abs().dot(center);
        let half_height = up.abs().dot(center);
        let half_depth = direction.abs().dot(center);
        let fitted_half_height = half_height.max(half_width / aspect) * FIT_MARGIN;
        let (distance, projection) = if let Some(fov) = self.vertical_fov_degrees {
            if !fov.is_finite() || fov <= 0.0 || fov >= MAX_VERTICAL_FOV {
                return Err(
                    "voxel camera field of view must be finite and between 0 and 179 degrees"
                        .into(),
                );
            }
            let distance =
                half_depth + fitted_half_height / (0.5 * fov.to_radians()).tan() / self.zoom;
            (
                distance,
                Projection::Perspective {
                    vertical_fov_degrees: fov,
                },
            )
        } else {
            (
                extent.length() + 1.0,
                Projection::Orthographic {
                    height: 2.0 * fitted_half_height / self.zoom,
                },
            )
        };
        let camera = Camera {
            position: (center + direction * distance).to_array(),
            target: center.to_array(),
            up: up.to_array(),
            projection,
        };
        camera.basis(viewport)?;
        Ok(camera)
    }
}

impl Camera {
    pub fn fit(view: View, dimensions: [u32; 3], viewport: [u32; 2]) -> Result<Self, String> {
        CameraRig {
            view,
            ..Default::default()
        }
        .resolve(dimensions, viewport)
    }

    pub fn basis(&self, viewport: [u32; 2]) -> Result<CameraBasis, String> {
        if viewport.contains(&0)
            || self
                .position
                .iter()
                .chain(&self.target)
                .chain(&self.up)
                .any(|value| !value.is_finite())
        {
            return Err(
                "voxel camera vectors must be finite and viewport dimensions must be nonzero"
                    .into(),
            );
        }
        let position = DVec3::from_array(self.position.map(f64::from));
        let direction = DVec3::from_array(self.target.map(f64::from)) - position;
        let up = DVec3::from_array(self.up.map(f64::from));
        let Some(forward) = direction.try_normalize() else {
            return Err("voxel camera must have distinct position and target, and an up vector not parallel to its view".into());
        };
        let Some(right) = forward.cross(up).try_normalize() else {
            return Err("voxel camera must have an up vector not parallel to its view".into());
        };
        let up = right.cross(forward);
        let (half_height, perspective) = match self.projection {
            Projection::Orthographic { height } if height.is_finite() && height > 0.0 => (height * 0.5, false),
            Projection::Perspective { vertical_fov_degrees } if vertical_fov_degrees.is_finite() && vertical_fov_degrees > 0.0 && vertical_fov_degrees < MAX_VERTICAL_FOV => ((0.5 * vertical_fov_degrees.to_radians()).tan(), true),
            _ => return Err("voxel camera projection must have positive finite height or a field of view between 0 and 179 degrees".into()),
        };
        let aspect = viewport[0] as f32 / viewport[1] as f32;
        if !(half_height * aspect).is_finite() {
            return Err("voxel camera projection exceeds the representable viewport size".into());
        }
        Ok(CameraBasis {
            position: self.position,
            right: right.to_array().map(|value| value as f32),
            up: up.to_array().map(|value| value as f32),
            forward: forward.to_array().map(|value| value as f32),
            half_height,
            aspect,
            perspective,
        })
    }

    pub fn ray_for_pixel(&self, viewport: [u32; 2], pixel: [u32; 2]) -> Result<Ray, String> {
        if pixel[0] >= viewport[0] || pixel[1] >= viewport[1] {
            return Err("voxel inspection pixel is outside the output viewport".into());
        }
        let basis = self.basis(viewport)?;
        let x = ((f64::from(pixel[0]) + 0.5) / f64::from(viewport[0]) * 2.0 - 1.0)
            * f64::from(basis.half_height)
            * f64::from(basis.aspect);
        let y = (1.0 - (f64::from(pixel[1]) + 0.5) / f64::from(viewport[1]) * 2.0)
            * f64::from(basis.half_height);
        let offset = DVec3::from_array(basis.right.map(f64::from)) * x
            + DVec3::from_array(basis.up.map(f64::from)) * y;
        Ok(if basis.perspective {
            Ray {
                origin: basis.position.map(f64::from),
                direction: (DVec3::from_array(basis.forward.map(f64::from)) + offset)
                    .normalize()
                    .to_array(),
            }
        } else {
            Ray {
                origin: (DVec3::from_array(basis.position.map(f64::from)) + offset).to_array(),
                direction: basis.forward.map(f64::from),
            }
        })
    }
}
