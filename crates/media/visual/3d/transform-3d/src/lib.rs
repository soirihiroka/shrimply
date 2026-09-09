mod math;

use glam::{EulerRot, Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use math::{
    MAX_EXPOSURE_EV, MAX_F_STOP, MIN_EXPOSURE_EV, MIN_F_STOP, focal_length_mm, vertical_fov_degrees,
};

pub trait Vector3Value {
    fn constant(value: Vec3) -> Self;
    fn fallback(&self) -> Vec3;
}

pub trait RotationOrderValue {
    fn constant(value: RotationOrder) -> Self;
    fn fallback(&self) -> RotationOrder;
}

pub trait ScalarValue {
    fn constant(value: f32) -> Self;
    fn fallback(&self) -> f32;
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CameraSource {
    #[default]
    Custom,
    #[serde(alias = "colmap")]
    Tracking(TrackingCameraSource),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TrackingCameraSource {
    pub track_id: Uuid,
    #[serde(default)]
    pub settings: TrackingSettings,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TrackingSettings {
    #[serde(default = "default_tracking_model")]
    pub model: String,
    #[serde(default)]
    pub quality: ColmapQuality,
    #[serde(default = "default_analysis_fps")]
    pub analysis_fps: u32,
    #[serde(default)]
    pub camera_model: ColmapCameraModel,
}

impl Default for TrackingSettings {
    fn default() -> Self {
        Self {
            model: default_tracking_model(),
            quality: ColmapQuality::Medium,
            analysis_fps: default_analysis_fps(),
            camera_model: ColmapCameraModel::SimpleRadial,
        }
    }
}

pub const COLMAP_TRACKING_MODEL: &str = "colmap/colmap";
pub const VGGT_SLAM_TRACKING_MODEL: &str = "MIT-SPARK/VGGT-SLAM";

fn default_tracking_model() -> String {
    COLMAP_TRACKING_MODEL.to_string()
}

const fn default_analysis_fps() -> u32 {
    10
}

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    PartialEq,
    Serialize,
    Deserialize,
    strum::Display,
    strum::EnumIter,
)]
#[serde(rename_all = "snake_case")]
pub enum ColmapQuality {
    Low,
    #[default]
    Medium,
    High,
    Extreme,
}

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    PartialEq,
    Serialize,
    Deserialize,
    strum::Display,
    strum::EnumIter,
)]
#[serde(rename_all = "snake_case")]
pub enum ColmapCameraModel {
    #[default]
    #[strum(to_string = "Simple Radial")]
    SimpleRadial,
    Pinhole,
    #[strum(to_string = "OpenCV")]
    OpenCv,
    #[strum(to_string = "OpenCV Fisheye")]
    OpenCvFisheye,
    Equirectangular,
}

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    PartialEq,
    Serialize,
    Deserialize,
    strum::Display,
    strum::EnumIter,
)]
#[serde(rename_all = "snake_case")]
pub enum Projection {
    #[default]
    Perspective,
    Orthographic,
    Equirectangular,
    Cylindrical,
    Fisheye,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RotationOrder {
    #[default]
    Xyz,
    Xzy,
    Yxz,
    Yzx,
    Zxy,
    Zyx,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Transform3D<V: Default, R: Default> {
    pub position: V,
    #[serde(default)]
    pub anchor: V,
    pub rotation_degrees: V,
    #[serde(default)]
    pub rotation_order: R,
    pub scale: V,
}

impl<V: Vector3Value + Default, R: RotationOrderValue + Default> Default for Transform3D<V, R> {
    fn default() -> Self {
        Self {
            position: V::constant(Vec3::ZERO),
            anchor: V::constant(Vec3::ZERO),
            rotation_degrees: V::constant(Vec3::ZERO),
            rotation_order: R::constant(RotationOrder::Xyz),
            scale: V::constant(Vec3::ONE),
        }
    }
}

impl<V: Vector3Value + Default, R: RotationOrderValue + Default> Transform3D<V, R> {
    pub fn fallback(&self) -> ResolvedTransform3D {
        ResolvedTransform3D {
            position: self.position.fallback(),
            anchor: self.anchor.fallback(),
            rotation_degrees: self.rotation_degrees.fallback(),
            rotation_order: self.rotation_order.fallback(),
            scale: self.scale.fallback(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Camera3D<V: Default, S: ScalarValue> {
    #[serde(default)]
    pub source: CameraSource,
    pub projection: Projection,
    pub position: V,
    pub rotation_degrees: V,
    pub vertical_fov_degrees: S,
    pub orthographic_height: S,
    #[serde(default = "default_focus_distance")]
    pub focus_distance: S,
    #[serde(default = "default_f_stop")]
    pub f_stop: S,
    pub exposure_ev: S,
}

fn default_focus_distance<S: ScalarValue>() -> S {
    S::constant(0.0)
}

fn default_f_stop<S: ScalarValue>() -> S {
    S::constant(2.8)
}

impl<V: Vector3Value + Default, S: ScalarValue> Default for Camera3D<V, S> {
    fn default() -> Self {
        let vertical_fov_degrees = 50.0f32;
        let distance = 1.1 / (vertical_fov_degrees * 0.5).to_radians().sin();
        Self {
            source: CameraSource::Custom,
            projection: Projection::Perspective,
            position: V::constant(Vec3::new(0.0, 0.0, distance)),
            rotation_degrees: V::constant(Vec3::ZERO),
            vertical_fov_degrees: S::constant(vertical_fov_degrees),
            orthographic_height: S::constant(2.2),
            focus_distance: default_focus_distance(),
            f_stop: default_f_stop(),
            exposure_ev: S::constant(0.0),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ResolvedTransform3D {
    pub position: Vec3,
    pub anchor: Vec3,
    pub rotation_degrees: Vec3,
    pub rotation_order: RotationOrder,
    pub scale: Vec3,
}

impl ResolvedTransform3D {
    pub fn matrix(self) -> Mat4 {
        self.matrix_with_source(Mat4::IDENTITY)
    }

    pub fn matrix_with_source(self, source: Mat4) -> Mat4 {
        Mat4::from_scale_rotation_translation(
            self.scale,
            rotation(self.rotation_degrees, self.rotation_order),
            self.position,
        ) * source
            * Mat4::from_translation(-self.anchor)
    }
}

pub fn rotation(degrees: Vec3, order: RotationOrder) -> Quat {
    let [first, second, third] = axes(order);
    Quat::from_euler(
        euler(order),
        degrees[first].to_radians(),
        degrees[second].to_radians(),
        degrees[third].to_radians(),
    )
}

pub fn rotation_degrees(rotation: Quat, order: RotationOrder) -> Vec3 {
    let (first_angle, second_angle, third_angle) = rotation.to_euler(euler(order));
    let [first, second, third] = axes(order);
    let mut degrees = Vec3::ZERO;
    degrees[first] = first_angle.to_degrees();
    degrees[second] = second_angle.to_degrees();
    degrees[third] = third_angle.to_degrees();
    degrees
}

pub fn camera_world(position: Vec3, rotation_degrees: Vec3) -> Mat4 {
    Mat4::from_rotation_translation(rotation(rotation_degrees, RotationOrder::Xyz), position)
}

/// The `degrees` components that the euler sequence takes its angles from, in
/// sequence order, so that each component always turns about its own axis.
fn axes(order: RotationOrder) -> [usize; 3] {
    match order {
        RotationOrder::Xyz => [0, 1, 2],
        RotationOrder::Xzy => [0, 2, 1],
        RotationOrder::Yxz => [1, 0, 2],
        RotationOrder::Yzx => [1, 2, 0],
        RotationOrder::Zxy => [2, 0, 1],
        RotationOrder::Zyx => [2, 1, 0],
    }
}

fn euler(order: RotationOrder) -> EulerRot {
    match order {
        RotationOrder::Xyz => EulerRot::XYZ,
        RotationOrder::Xzy => EulerRot::XZY,
        RotationOrder::Yxz => EulerRot::YXZ,
        RotationOrder::Yzx => EulerRot::YZX,
        RotationOrder::Zxy => EulerRot::ZXY,
        RotationOrder::Zyx => EulerRot::ZYX,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORDERS: [RotationOrder; 6] = [
        RotationOrder::Xyz,
        RotationOrder::Xzy,
        RotationOrder::Yxz,
        RotationOrder::Yzx,
        RotationOrder::Zxy,
        RotationOrder::Zyx,
    ];

    const PROBE: Vec3 = Vec3::new(1.0, 2.0, 3.0);

    fn close(left: Vec3, right: Vec3) -> bool {
        (left - right).abs().max_element() <= 1e-4
    }

    #[test]
    fn one_nonzero_angle_turns_about_its_own_axis_whatever_the_order() {
        let quarter = 90f32.to_radians();
        for (degrees, expected) in [
            (Vec3::new(90.0, 0.0, 0.0), Quat::from_rotation_x(quarter)),
            (Vec3::new(0.0, 90.0, 0.0), Quat::from_rotation_y(quarter)),
            (Vec3::new(0.0, 0.0, 90.0), Quat::from_rotation_z(quarter)),
        ] {
            let expected = expected * PROBE;
            for order in ORDERS {
                let actual = rotation(degrees, order) * PROBE;
                assert!(
                    close(actual, expected),
                    "{degrees:?} {order:?}: {actual:?} {expected:?}"
                );
            }
        }
    }

    #[test]
    fn degrees_round_trip_through_every_order() {
        let degrees = Vec3::new(10.0, 20.0, 30.0);
        for order in ORDERS {
            let actual = rotation_degrees(rotation(degrees, order), order);
            assert!(close(actual, degrees), "{order:?}: {actual:?}");
        }
    }

    #[test]
    fn the_order_still_changes_a_three_axis_rotation() {
        let degrees = Vec3::new(10.0, 20.0, 30.0);
        let first = rotation(degrees, RotationOrder::Xyz) * PROBE;
        let second = rotation(degrees, RotationOrder::Zyx) * PROBE;
        assert!(!close(first, second), "{first:?} {second:?}");
    }
}
