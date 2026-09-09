use glam::{Mat3, Vec2};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub fn try_inverse(matrix: Mat3) -> Option<Mat3> {
    let determinant = matrix.determinant();
    (determinant.is_finite() && determinant.abs() > f32::EPSILON).then(|| matrix.inverse())
}

pub fn motion_sample_inverses(current: Mat3, relative: &[ComposedTransform2D]) -> Vec<Mat3> {
    relative
        .iter()
        .filter_map(|sample| try_inverse(sample.matrix * current))
        .collect()
}

pub fn vector_angle_degrees(vector: Vec2) -> Option<f32> {
    (vector.length_squared() > f32::EPSILON).then(|| vector.y.atan2(vector.x).to_degrees())
}

pub fn normalized_angle_degrees(angle: f32) -> f32 {
    let normalized = (angle + 180.0).rem_euclid(360.0) - 180.0;
    if normalized == -180.0 && angle > 0.0 {
        180.0
    } else {
        normalized
    }
}

#[inline(always)]
pub fn transform_point2(transform: glam::Mat3, point: glam::Vec2) -> glam::Vec2 {
    glam::Vec2::new(
        transform.x_axis.x * point.x + transform.y_axis.x * point.y + transform.z_axis.x,
        transform.x_axis.y * point.x + transform.y_axis.y * point.y + transform.z_axis.y,
    )
}

/// A size accepted by the standard transform constructors.
///
/// Applications can implement this for their own canvas type without coupling
/// this crate to their project model.
pub trait Size2D {
    fn size_2d(&self) -> Vec2;
}

impl Size2D for Vec2 {
    fn size_2d(&self) -> Vec2 {
        *self
    }
}

/// A possibly animated two-dimensional value.
pub trait Vector2Value {
    fn constant(value: Vec2) -> Self;
    fn fallback(&self) -> Vec2;
}

/// A possibly animated scalar value.
pub trait ScalarValue {
    fn constant(value: f32) -> Self;
    fn fallback(&self) -> f32;
}

/// Persistable transform data, generic over the application's value types.
///
/// This lets timeline applications use animated values while simpler clients
/// can use plain values, without either depending on the other.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Transform2D<V: Default, S> {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    pub position: V,
    pub anchor: V,
    pub scale: V,
    #[serde(default)]
    pub shear: V,
    pub rotation_degrees: S,
}

impl<V: Vector2Value + Default, S: ScalarValue> Transform2D<V, S> {
    pub fn fill(canvas: impl Size2D) -> Self {
        Self::from_resolved(ResolvedTransform2D::fill(canvas))
    }

    pub fn natural_size(canvas: impl Size2D, width: u32, height: u32) -> Self {
        Self::from_resolved(ResolvedTransform2D::natural_size(canvas, width, height))
    }

    pub fn contain(canvas: impl Size2D, width: u32, height: u32) -> Self {
        Self::from_resolved(ResolvedTransform2D::contain(canvas, width, height))
    }

    pub fn cover(canvas: impl Size2D, width: u32, height: u32) -> Self {
        Self::from_resolved(ResolvedTransform2D::cover(canvas, width, height))
    }

    pub fn stretch(canvas: impl Size2D, width: u32, height: u32) -> Self {
        Self::from_resolved(ResolvedTransform2D::stretch(canvas, width, height))
    }

    pub fn from_resolved(transform: ResolvedTransform2D) -> Self {
        Self {
            id: Uuid::new_v4(),
            position: V::constant(transform.position),
            anchor: V::constant(transform.anchor),
            scale: V::constant(transform.scale),
            shear: V::constant(transform.shear),
            rotation_degrees: S::constant(transform.rotation_degrees),
        }
    }

    pub fn fallback(&self) -> ResolvedTransform2D {
        ResolvedTransform2D {
            position: self.position.fallback(),
            anchor: self.anchor.fallback(),
            scale: self.scale.fallback(),
            shear: self.shear.fallback(),
            rotation_degrees: self.rotation_degrees.fallback(),
        }
    }
}

/// Concrete transform values ready for rendering.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedTransform2D {
    pub position: Vec2,
    pub anchor: Vec2,
    pub scale: Vec2,
    pub shear: Vec2,
    pub rotation_degrees: f32,
}

impl ResolvedTransform2D {
    pub const IDENTITY: Self = Self {
        position: Vec2::ZERO,
        anchor: Vec2::ZERO,
        scale: Vec2::ONE,
        shear: Vec2::ZERO,
        rotation_degrees: 0.0,
    };

    pub fn fill(canvas: impl Size2D) -> Self {
        let size = canvas.size_2d();
        let center = size * 0.5;
        Self {
            position: center,
            anchor: center,
            ..Self::IDENTITY
        }
    }

    pub fn natural_size(canvas: impl Size2D, width: u32, height: u32) -> Self {
        let canvas = canvas.size_2d().max(Vec2::ONE);
        let media = Vec2::new(width.max(1) as f32, height.max(1) as f32);
        Self {
            position: canvas * 0.5,
            anchor: media * 0.5,
            ..Self::IDENTITY
        }
    }

    pub fn contain(canvas: impl Size2D, width: u32, height: u32) -> Self {
        let canvas = canvas.size_2d().max(Vec2::ONE);
        let media = Vec2::new(width.max(1) as f32, height.max(1) as f32);
        Self {
            scale: Vec2::splat((canvas / media).min_element()),
            ..Self::natural_size(canvas, width, height)
        }
    }

    pub fn cover(canvas: impl Size2D, width: u32, height: u32) -> Self {
        let canvas = canvas.size_2d().max(Vec2::ONE);
        let media = Vec2::new(width.max(1) as f32, height.max(1) as f32);
        Self {
            scale: Vec2::splat((canvas / media).max_element()),
            ..Self::natural_size(canvas, width, height)
        }
    }

    pub fn stretch(canvas: impl Size2D, width: u32, height: u32) -> Self {
        let canvas = canvas.size_2d().max(Vec2::ONE);
        let media = Vec2::new(width.max(1) as f32, height.max(1) as f32);
        Self {
            scale: canvas / media,
            ..Self::natural_size(canvas, width, height)
        }
    }

    /// Maps source-local coordinates into the parent's coordinate system.
    ///
    /// Composition is `translation(position) * rotation * shear * scale *
    /// translation(-anchor)`, so rotation and scale occur around the anchor.
    pub fn matrix(self) -> Mat3 {
        Mat3::from_translation(self.position)
            * Mat3::from_angle(self.rotation_degrees.to_radians())
            * Mat3::from_cols_array(&[
                1.0,
                self.shear.y,
                0.0,
                self.shear.x,
                1.0,
                0.0,
                0.0,
                0.0,
                1.0,
            ])
            * Mat3::from_scale(self.scale)
            * Mat3::from_translation(-self.anchor)
    }

    pub fn composed(self) -> ComposedTransform2D {
        ComposedTransform2D {
            matrix: self.matrix(),
        }
    }

    /// Applies `child` first and this transform second.
    pub fn compose(self, child: Self) -> ComposedTransform2D {
        self.composed().compose(child.composed())
    }
}

/// A lossless affine composition. It retains matrices rather than decomposing
/// them back into scale and rotation, which would discard shear information.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComposedTransform2D {
    pub matrix: Mat3,
}

impl ComposedTransform2D {
    pub const IDENTITY: Self = Self {
        matrix: Mat3::IDENTITY,
    };

    /// Applies `child` first and this transform second.
    pub fn compose(self, child: Self) -> Self {
        Self {
            matrix: self.matrix * child.matrix,
        }
    }

    pub fn transform_point(self, point: Vec2) -> Vec2 {
        self.matrix.transform_point2(point)
    }

    pub fn transform_vector(self, vector: Vec2) -> Vec2 {
        self.matrix.transform_vector2(vector)
    }

    pub fn inverse(self) -> Self {
        let determinant = self.matrix.determinant();
        assert!(
            determinant.is_finite() && determinant != 0.0,
            "cannot invert a singular 2D transform"
        );
        Self {
            matrix: self.matrix.inverse(),
        }
    }
}
pub fn relative_motion_transforms(
    current: ComposedTransform2D,
    samples: Vec<ComposedTransform2D>,
) -> Option<Vec<ComposedTransform2D>> {
    let determinant = current.matrix.determinant();
    if !determinant.is_finite()
        || determinant == 0.0
        || samples.iter().all(|sample| *sample == current)
    {
        return None;
    }
    let inverse = current.inverse();
    Some(
        samples
            .into_iter()
            .map(|sample| sample.compose(inverse))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(left: Vec2, right: Vec2) -> bool {
        (left - right).abs().max_element() <= 1e-5
    }

    #[test]
    fn shear_is_applied_after_scale() {
        // scale (2, 4) then an x-shear of 0.5 maps (1, 1) to (1, 1) * scale = (2, 4),
        // then (2 + 0.5 * 4, 4) = (4, 4). Composing the other way round shears the
        // unscaled point first and reaches (3, 4) instead.
        let transform = ResolvedTransform2D {
            scale: Vec2::new(2.0, 4.0),
            shear: Vec2::new(0.5, 0.0),
            ..ResolvedTransform2D::IDENTITY
        };
        let point = transform.matrix().transform_point2(Vec2::ONE);
        assert!(approx(point, Vec2::new(4.0, 4.0)), "{point:?}");
    }

    #[test]
    fn the_matrix_is_the_documented_composition() {
        let transform = ResolvedTransform2D {
            position: Vec2::new(30.0, -10.0),
            anchor: Vec2::new(4.0, 7.0),
            scale: Vec2::new(3.0, 0.5),
            shear: Vec2::new(0.25, -0.75),
            rotation_degrees: 35.0,
        };
        let shear = Mat3::from_cols_array(&[
            1.0,
            transform.shear.y,
            0.0,
            transform.shear.x,
            1.0,
            0.0,
            0.0,
            0.0,
            1.0,
        ]);
        let documented = Mat3::from_translation(transform.position)
            * Mat3::from_angle(transform.rotation_degrees.to_radians())
            * shear
            * Mat3::from_scale(transform.scale)
            * Mat3::from_translation(-transform.anchor);
        for point in [Vec2::ZERO, Vec2::X, Vec2::Y, Vec2::new(-6.0, 11.0)] {
            let actual = transform.matrix().transform_point2(point);
            let expected = documented.transform_point2(point);
            assert!(
                approx(actual, expected),
                "{point:?}: {actual:?} {expected:?}"
            );
        }
    }

    #[test]
    fn a_transform_without_shear_is_unchanged() {
        let transform = ResolvedTransform2D {
            position: Vec2::new(120.0, 80.0),
            anchor: Vec2::new(16.0, 9.0),
            scale: Vec2::new(2.0, 3.0),
            shear: Vec2::ZERO,
            rotation_degrees: 90.0,
        };
        // Rotating a quarter turn about the anchor, then translating to the position.
        let anchor = transform.matrix().transform_point2(transform.anchor);
        assert!(approx(anchor, transform.position), "{anchor:?}");
        let corner = transform
            .matrix()
            .transform_point2(transform.anchor + Vec2::X);
        assert!(
            approx(corner, transform.position + Vec2::Y * 2.0),
            "{corner:?}"
        );
    }
}
