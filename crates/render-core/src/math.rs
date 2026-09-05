use autodiff::F;
use nalgebra::SVector;
use num_traits::Float;

pub use shrimply_math_color::Color;
pub use shrimply_math_geometry::{
    Mat3, Vec2, inverse_bilinear_quad, projective_point, transform_point2,
};

pub type Float3 = [f32; 3];
pub type Float4 = [f32; 4];
pub type Float4x4 = [f32; 16];
pub type CubicControls<T> = SVector<T, 4>;

pub fn inverse_affine(matrix: glam::Mat3) -> Option<glam::Mat3> {
    let determinant = matrix.determinant();
    (determinant.is_finite() && determinant.abs() > f32::EPSILON).then(|| matrix.inverse())
}
type SepticControls<T> = SVector<T, 8>;

#[inline(always)]
pub fn lerp<T: Float>(left: T, right: T, amount: T) -> T {
    left + (right - left) * amount
}

#[inline(always)]
pub fn polar_degrees(distance: f32, direction_degrees: f32) -> glam::Vec2 {
    let radians = direction_degrees.to_radians();
    glam::Vec2::new(radians.cos() * distance, radians.sin() * distance)
}

#[inline(always)]
pub fn rotate_degrees(value: glam::Vec2, degrees: f32) -> glam::Vec2 {
    let (sin, cos) = degrees.to_radians().sin_cos();
    glam::Vec2::new(value.x * cos - value.y * sin, value.x * sin + value.y * cos)
}

#[inline(always)]
pub fn shape_alpha_mask_feather_half_width(size: glam::Vec2, feather: f32) -> f32 {
    size.max(glam::Vec2::ZERO).min_element() * feather.clamp(0.0, 1.0) * 0.25
}

#[inline(always)]
pub fn shape_alpha_mask_amount(point: glam::Vec2, params: &crate::ShapeAlphaMaskParams) -> f32 {
    let center = params.center;
    let size = params.size;
    let half_size = size.max(glam::Vec2::ZERO) * 0.5;
    let mut amount = if half_size.min_element() <= f32::EPSILON {
        0.0
    } else {
        let point = rotate_degrees(point - center, -params.rotation_degrees);
        let distance = match params.shape {
            crate::ShapeAlphaMaskKind::Rectangle => {
                rounded_rectangle_distance(point, half_size, params.rounding)
            }
            crate::ShapeAlphaMaskKind::Ellipse => {
                ((point / half_size).length() - 1.0) * half_size.min_element()
            }
            crate::ShapeAlphaMaskKind::Polygon => {
                polygon_signed_distance(point, params.vertices, params.vertex_count)
            }
        };
        let softness = shape_alpha_mask_feather_half_width(size, params.feather) * 2.0;
        if softness <= f32::EPSILON {
            (distance <= 0.0) as u8 as f32
        } else {
            let value = (0.5 - distance / softness).clamp(0.0, 1.0);
            value * value * (3.0 - 2.0 * value)
        }
    };
    if params.invert {
        amount = 1.0 - amount;
    }
    amount
}

#[inline(always)]
pub fn rounded_rectangle_distance(point: glam::Vec2, half_size: glam::Vec2, rounding: f32) -> f32 {
    let radius = half_size.min_element() * rounding.clamp(0.0, 1.0);
    let edge = point.abs() - half_size + glam::Vec2::splat(radius);
    edge.max(glam::Vec2::ZERO).length() + edge.max_element().min(0.0) - radius
}

#[inline(always)]
pub fn closest_point_on_segment(
    point: glam::Vec2,
    start: glam::Vec2,
    end: glam::Vec2,
) -> glam::Vec2 {
    let segment = end - start;
    let length_squared = segment.length_squared();
    if length_squared <= f32::EPSILON {
        start
    } else {
        start + segment * ((point - start).dot(segment) / length_squared).clamp(0.0, 1.0)
    }
}

pub fn point_in_polygon(point: glam::Vec2, vertices: &[glam::Vec2]) -> bool {
    if vertices.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut previous = vertices[vertices.len() - 1];
    for &current in vertices {
        if (previous.y > point.y) != (current.y > point.y)
            && point.x
                < (current.x - previous.x) * (point.y - previous.y) / (current.y - previous.y)
                    + previous.x
        {
            inside = !inside;
        }
        previous = current;
    }
    inside
}

#[inline(always)]
fn polygon_signed_distance(
    point: glam::Vec2,
    vertices: *const glam::Vec2,
    vertex_count: u32,
) -> f32 {
    if vertices.is_null() || vertex_count < 3 {
        return f32::MAX;
    }
    let mut inside = false;
    let mut distance_squared = f32::MAX;
    let mut previous = unsafe { *vertices.add(vertex_count as usize - 1) };
    let mut index = 0;
    while index < vertex_count {
        let current = unsafe { *vertices.add(index as usize) };
        let nearest = closest_point_on_segment(point, previous, current);
        distance_squared = distance_squared.min((point - nearest).length_squared());
        if (previous.y > point.y) != (current.y > point.y)
            && point.x
                < (current.x - previous.x) * (point.y - previous.y) / (current.y - previous.y)
                    + previous.x
        {
            inside = !inside;
        }
        previous = current;
        index += 1;
    }
    let distance = distance_squared.sqrt();
    if inside { -distance } else { distance }
}

#[inline(always)]
pub fn decoration_outset(
    outline_width: f32,
    shadow_offset: glam::Vec2,
    shadow_width: f32,
    shadow_sigma: f32,
) -> [f32; 4] {
    let outline = (outline_width * 0.5).max(0.0);
    let shadow = (shadow_width * 0.5).max(0.0) + shadow_sigma.max(0.0) * 3.0;
    [
        outline.max(shadow - shadow_offset.y).max(0.0),
        outline.max(shadow + shadow_offset.x).max(0.0),
        outline.max(shadow + shadow_offset.y).max(0.0),
        outline.max(shadow - shadow_offset.x).max(0.0),
    ]
}

#[inline(always)]
pub fn cubic<T>(t: T, p0: T, p1: T, p2: T, p3: T) -> T
where
    T: Float + nalgebra::Scalar,
{
    cubic_controls(t, CubicControls::new(p0, p1, p2, p3))
}

#[inline(always)]
pub fn cubic_controls<T>(t: T, controls: CubicControls<T>) -> T
where
    T: Float + nalgebra::Scalar,
{
    let three = T::one() + T::one() + T::one();
    let inv = T::one() - t;
    controls[0] * inv * inv * inv
        + controls[1] * three * inv * inv * t
        + controls[2] * three * inv * t * t
        + controls[3] * t * t * t
}

#[inline(always)]
pub fn cubic_derivative<T>(t: T, p0: T, p1: T, p2: T, p3: T) -> T
where
    T: Float + std::fmt::Debug,
{
    cubic_dual(
        F::<T, T>::var(t),
        F::<T, T>::cst(p0),
        F::<T, T>::cst(p1),
        F::<T, T>::cst(p2),
        F::<T, T>::cst(p3),
    )
    .deriv()
}

#[inline(always)]
pub fn cubic_bezier_t<T>(x: T, x1: T, x2: T) -> T
where
    T: Float + nalgebra::Scalar,
{
    let mut low = T::zero();
    let mut high = T::one();
    let mut t = x.max(low).min(high);
    for _ in 0..16 {
        let value = cubic(t, T::zero(), x1, x2, T::one());
        let delta = value - x;
        if value < x {
            low = t;
        } else {
            high = t;
        }
        let derivative = cubic_derivative(t, T::zero(), x1, x2, T::one());
        let next = if derivative.abs() > T::epsilon() {
            (t - delta / derivative).max(low).min(high)
        } else {
            (low + high) / two()
        };
        t = if next.is_finite() {
            next
        } else {
            (low + high) / two()
        };
    }
    t
}

fn septic_controls<T>(t: T, controls: SepticControls<T>) -> T
where
    T: Float + nalgebra::Scalar,
{
    let seven = T::from(7.0).unwrap();
    let twenty_one = T::from(21.0).unwrap();
    let thirty_five = T::from(35.0).unwrap();
    let inv = T::one() - t;
    let inv2 = inv * inv;
    let inv3 = inv2 * inv;
    let inv4 = inv3 * inv;
    let inv5 = inv4 * inv;
    let inv6 = inv5 * inv;
    let t2 = t * t;
    let t3 = t2 * t;
    let t4 = t3 * t;
    let t5 = t4 * t;
    let t6 = t5 * t;
    controls[0] * inv6 * inv
        + controls[1] * seven * inv6 * t
        + controls[2] * twenty_one * inv5 * t2
        + controls[3] * thirty_five * inv4 * t3
        + controls[4] * thirty_five * inv3 * t4
        + controls[5] * twenty_one * inv2 * t5
        + controls[6] * seven * inv * t6
        + controls[7] * t6 * t
}

fn septic_bezier_t<T>(x: T, controls: SepticControls<T>) -> T
where
    T: Float + nalgebra::Scalar,
{
    let mut low = T::zero();
    let mut high = T::one();
    let mut t = x.max(low).min(high);
    for _ in 0..28 {
        if septic_controls(t, controls) < x {
            low = t;
        } else {
            high = t;
        }
        t = (low + high) / two();
    }
    t
}

fn septic_bezier_y_at_x<T>(x: T, x_controls: SepticControls<T>, y_controls: SepticControls<T>) -> T
where
    T: Float + nalgebra::Scalar,
{
    septic_controls(septic_bezier_t(x, x_controls), y_controls)
}

#[inline(always)]
pub fn integrate_unit<T: Float>(steps: usize, progress: T, mut value_at: impl FnMut(T) -> T) -> T {
    let progress = progress.max(T::zero()).min(T::one());
    if progress <= T::zero() {
        return T::zero();
    }

    let steps = steps.max(1);
    let denominator = T::from(steps).unwrap();
    let mut area = T::zero();
    let mut last_progress = T::zero();
    let mut last_value = value_at(T::zero());
    for step in 1..=steps {
        let next_progress = progress * T::from(step).unwrap() / denominator;
        let next_value = value_at(next_progress);
        area = area + (last_value + next_value) / two() * (next_progress - last_progress);
        last_progress = next_progress;
        last_value = next_value;
    }
    area
}

#[derive(Clone, Copy)]
pub struct TemporalVelocityCurve<T> {
    start_speed: T,
    end_speed: T,
    peak_speed: T,
    out_x: T,
    out_reinforce_x: T,
    peak_left_x: T,
    peak_right_x: T,
    in_reinforce_x: T,
    in_x: T,
}

#[derive(Clone, Copy)]
pub enum TemporalVelocitySide<T> {
    Linear { speed: T },
    Curved { speed: T, influence: T },
}

#[inline(always)]
pub fn temporal_velocity_curve<T>(
    average_speed: T,
    start_speed: T,
    end_speed: T,
    out_influence: T,
    in_influence: T,
) -> TemporalVelocityCurve<T>
where
    T: Float + nalgebra::Scalar,
{
    let average_speed = average_speed.max(T::zero());
    let start_speed = start_speed.max(T::zero());
    let end_speed = end_speed.max(T::zero());
    let mut out_x = out_influence.max(T::zero()).min(T::one());
    let mut in_x = T::one() - in_influence.max(T::zero()).min(T::one());
    if out_x > in_x {
        let center = (out_x + in_x) / two();
        out_x = center;
        in_x = center;
    }
    let peak_x = (out_x + in_x) / two();
    let mut curve = TemporalVelocityCurve {
        start_speed,
        end_speed,
        peak_speed: T::zero(),
        out_x,
        out_reinforce_x: (out_x + (out_x + peak_x) / two()) / two(),
        peak_left_x: (out_x + peak_x) / two(),
        peak_right_x: (peak_x + in_x) / two(),
        in_reinforce_x: ((peak_x + in_x) / two() + in_x) / two(),
        in_x,
    };
    let base_area = temporal_velocity_area(curve, T::one());
    curve.peak_speed = T::one();
    let unit_area = temporal_velocity_area(curve, T::one());
    let peak_area = unit_area - base_area;
    curve.peak_speed = if peak_area.abs() > T::epsilon() {
        ((average_speed - base_area) / peak_area).max(T::zero())
    } else {
        average_speed
    };
    curve
}

#[inline(always)]
pub fn temporal_velocity_progress_between<T>(
    average_speed: T,
    start: TemporalVelocitySide<T>,
    end: TemporalVelocitySide<T>,
    progress: T,
) -> T
where
    T: Float + nalgebra::Scalar,
{
    let progress = progress.max(T::zero()).min(T::one());
    let average_speed = average_speed.max(T::zero());
    match (start, end) {
        (TemporalVelocitySide::Linear { .. }, TemporalVelocitySide::Linear { .. }) => progress,
        (
            TemporalVelocitySide::Curved {
                speed: start_speed,
                influence: out_influence,
            },
            TemporalVelocitySide::Curved {
                speed: end_speed,
                influence: in_influence,
            },
        ) => temporal_velocity_progress(
            temporal_velocity_curve(
                average_speed,
                start_speed,
                end_speed,
                out_influence,
                in_influence,
            ),
            progress,
        ),
        (
            TemporalVelocitySide::Curved {
                speed: start_speed,
                influence,
            },
            TemporalVelocitySide::Linear { speed: end_speed },
        ) => temporal_velocity_progress(
            temporal_velocity_curve(average_speed, start_speed, end_speed, influence, T::zero()),
            progress,
        ),
        (
            TemporalVelocitySide::Linear { speed: start_speed },
            TemporalVelocitySide::Curved {
                speed: end_speed,
                influence,
            },
        ) => temporal_velocity_progress(
            temporal_velocity_curve(average_speed, start_speed, end_speed, T::zero(), influence),
            progress,
        ),
    }
}

#[inline(always)]
pub fn temporal_velocity_speed_between<T>(
    average_speed: T,
    start: TemporalVelocitySide<T>,
    end: TemporalVelocitySide<T>,
    progress: T,
) -> T
where
    T: Float + nalgebra::Scalar,
{
    let progress = progress.max(T::zero()).min(T::one());
    let average_speed = average_speed.max(T::zero());
    match (start, end) {
        (TemporalVelocitySide::Linear { .. }, TemporalVelocitySide::Linear { .. }) => average_speed,
        (
            TemporalVelocitySide::Curved {
                speed: start_speed,
                influence: out_influence,
            },
            TemporalVelocitySide::Curved {
                speed: end_speed,
                influence: in_influence,
            },
        ) => temporal_velocity_speed(
            temporal_velocity_curve(
                average_speed,
                start_speed,
                end_speed,
                out_influence,
                in_influence,
            ),
            progress,
        ),
        (
            TemporalVelocitySide::Curved {
                speed: start_speed,
                influence,
            },
            TemporalVelocitySide::Linear { speed: end_speed },
        ) => temporal_velocity_speed(
            temporal_velocity_curve(average_speed, start_speed, end_speed, influence, T::zero()),
            progress,
        ),
        (
            TemporalVelocitySide::Linear { speed: start_speed },
            TemporalVelocitySide::Curved {
                speed: end_speed,
                influence,
            },
        ) => temporal_velocity_speed(
            temporal_velocity_curve(average_speed, start_speed, end_speed, T::zero(), influence),
            progress,
        ),
    }
}

#[inline(always)]
pub fn temporal_velocity_progress<T>(curve: TemporalVelocityCurve<T>, progress: T) -> T
where
    T: Float + nalgebra::Scalar,
{
    let total = temporal_velocity_area(curve, T::one());
    if total <= T::epsilon() {
        return T::zero();
    }
    (temporal_velocity_area(curve, progress) / total)
        .max(T::zero())
        .min(T::one())
}

#[inline(always)]
pub fn temporal_velocity_speed<T>(curve: TemporalVelocityCurve<T>, progress: T) -> T
where
    T: Float + nalgebra::Scalar,
{
    septic_bezier_y_at_x(
        progress.max(T::zero()).min(T::one()),
        curve.x_controls(),
        curve.y_controls(),
    )
    .max(T::zero())
}

fn temporal_velocity_area<T>(curve: TemporalVelocityCurve<T>, progress: T) -> T
where
    T: Float + nalgebra::Scalar,
{
    integrate_unit(64, progress, |progress| {
        temporal_velocity_speed(curve, progress)
    })
}

impl<T> TemporalVelocityCurve<T>
where
    T: Float + nalgebra::Scalar,
{
    fn x_controls(self) -> SepticControls<T> {
        SepticControls::from_row_slice(&[
            T::zero(),
            self.out_x,
            self.out_reinforce_x,
            self.peak_left_x,
            self.peak_right_x,
            self.in_reinforce_x,
            self.in_x,
            T::one(),
        ])
    }

    fn y_controls(self) -> SepticControls<T> {
        SepticControls::from_row_slice(&[
            self.start_speed,
            self.start_speed,
            self.peak_speed,
            self.peak_speed,
            self.peak_speed,
            self.peak_speed,
            self.end_speed,
            self.end_speed,
        ])
    }
}

fn two<T: Float>() -> T {
    T::one() + T::one()
}

fn cubic_dual<T>(t: F<T, T>, p0: F<T, T>, p1: F<T, T>, p2: F<T, T>, p3: F<T, T>) -> F<T, T>
where
    T: Float + std::fmt::Debug,
{
    let three = F::<T, T>::cst(T::one() + T::one() + T::one());
    let inv = F::<T, T>::cst(T::one()) - t;
    inv * inv * inv * p0 + three * inv * inv * t * p1 + three * inv * t * t * p2 + t * t * t * p3
}

/// Bilinearly samples an in-bounds RGBA coordinate.
///
/// # Safety
///
/// `input` must reference a readable buffer of at least `width * height` pixels, both dimensions
/// must be nonzero, and `x` and `y` must be within the image bounds.
#[inline(always)]
pub unsafe fn sample_bilinear_rgba(
    input: *const u32,
    width: u32,
    height: u32,
    x: f32,
    y: f32,
) -> u32 {
    let x0 = floor_f32(x) as usize;
    let y0 = floor_f32(y) as usize;
    let x1 = (x0 + 1).min(width.saturating_sub(1) as usize);
    let y1 = (y0 + 1).min(height.saturating_sub(1) as usize);
    let horizontal = x - x0 as f32;
    let vertical = y - y0 as f32;
    let top_left = Color::from_rgba_u32(unsafe { *input.add(y0 * width as usize + x0) });
    let top_right = Color::from_rgba_u32(unsafe { *input.add(y0 * width as usize + x1) });
    let bottom_left = Color::from_rgba_u32(unsafe { *input.add(y1 * width as usize + x0) });
    let bottom_right = Color::from_rgba_u32(unsafe { *input.add(y1 * width as usize + x1) });
    top_left
        .premultiply()
        .lerp(top_right.premultiply(), horizontal)
        .lerp(
            bottom_left
                .premultiply()
                .lerp(bottom_right.premultiply(), horizontal),
            vertical,
        )
        .unpremultiply()
        .to_rgba_u32()
}

#[inline(always)]
pub fn smoothstep(start: f32, end: f32, value: f32) -> f32 {
    let t = ((value - start) / (end - start).max(0.000_01)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline(always)]
pub fn hash_unit(x: i32, y: i32, seed: u32) -> f32 {
    let mut hash = (x as u32)
        .wrapping_mul(0x8da6_b343)
        .wrapping_add((y as u32).wrapping_mul(0xd816_3841))
        .wrapping_add(seed.wrapping_mul(0xcb1a_b31f));
    hash ^= hash >> 16;
    hash = hash.wrapping_mul(0x7feb_352d);
    hash ^= hash >> 15;
    hash = hash.wrapping_mul(0x846c_a68b);
    hash ^= hash >> 16;
    hash as f32 / 4_294_967_296.0
}

const STREAK_WIPE_HASH_SEED: u32 = 37;

#[inline(always)]
pub fn streak_wipe_alpha(
    point: Vec2,
    canvas_size: Vec2,
    visibility: f32,
    direction_degrees: f32,
    line_width: f32,
    variation: f32,
    softness: f32,
) -> f32 {
    let visibility = visibility.clamp(0.0, 1.0);
    if visibility <= 0.0 {
        return 0.0;
    }
    if visibility >= 1.0 {
        return 1.0;
    }

    let radians = direction_degrees.to_radians();
    let direction = Vec2::new(
        sin_f32(radians + core::f32::consts::FRAC_PI_2),
        sin_f32(radians),
    );
    let centered = point - canvas_size * 0.5;
    let extent = direction.abs().dot(canvas_size).max(0.000_01);
    let position = (centered.dot(direction) / extent + 0.5).clamp(0.0, 1.0);
    let perpendicular = centered.dot(Vec2::new(-direction.y, direction.x));
    let band = floor_f32(perpendicular / line_width.max(1.0)) as i32;
    let variation = variation.clamp(0.0, 1.0);
    let edge =
        visibility * (1.0 + variation) - hash_unit(band, 0, STREAK_WIPE_HASH_SEED) * variation;
    let softness = softness.clamp(0.0, 128.0) / extent;
    1.0 - smoothstep(edge - softness, edge + softness + 0.000_01, position)
}

#[inline(always)]
pub fn wrap_unit(value: f32) -> f32 {
    value - floor_f32(value)
}

#[inline(always)]
pub fn floor_f32(value: f32) -> f32 {
    let truncated = value as i32 as f32;
    if truncated > value {
        truncated - 1.0
    } else {
        truncated
    }
}

#[inline(always)]
pub fn ceil_f32(value: f32) -> f32 {
    -floor_f32(-value)
}

#[inline(always)]
pub fn repeat_index(value: i32, start: i32, length: i32) -> i32 {
    let length = length.max(1);
    let offset = (value - start) % length;
    start + if offset < 0 { offset + length } else { offset }
}

#[inline(always)]
pub fn mirror_repeat_index(value: i32, start: i32, length: i32) -> i32 {
    let length = length.max(1);
    let period = length.saturating_mul(2);
    let offset = repeat_index(value - start, 0, period);
    start
        + if offset < length {
            offset
        } else {
            period - offset - 1
        }
}

#[inline(always)]
pub fn sin_f32(value: f32) -> f32 {
    let tau = core::f32::consts::PI * 2.0;
    let mut x = value - floor_f32((value + core::f32::consts::PI) / tau) * tau;
    if x > core::f32::consts::PI {
        x -= tau;
    }
    let x2 = x * x;
    x * (1.0 + x2 * (-1.0 / 6.0 + x2 * (1.0 / 120.0 + x2 * (-1.0 / 5_040.0 + x2 / 362_880.0))))
}

/// Fast quadrant-safe atan2 approximation for CUDA geometry kernels.
#[inline(always)]
pub fn atan2_f32(y: f32, x: f32) -> f32 {
    let abs_y = if y < 0.0 { -y } else { y } + 0.000_000_1;
    let (base, ratio) = if x >= 0.0 {
        (core::f32::consts::FRAC_PI_4, (x - abs_y) / (x + abs_y))
    } else {
        (
            3.0 * core::f32::consts::FRAC_PI_4,
            (x + abs_y) / (abs_y - x),
        )
    };
    let angle = base + (0.1963 * ratio * ratio - 0.9817) * ratio;
    if y < 0.0 { -angle } else { angle }
}

#[inline(always)]
pub fn gaussian_weight(distance: i32, radius: u32) -> f32 {
    let sigma = (radius as f32 * 0.5).max(0.5);
    let exponent = -((distance * distance) as f32) / (2.0 * sigma * sigma);
    let mut result = 1.0 + exponent / 256.0;
    for _ in 0..8 {
        result *= result;
    }
    result.max(0.0)
}

#[inline(always)]
pub fn normalized_crop(mut crop: [f32; 4]) -> [f32; 4] {
    crop = crop.map(|edge| edge.clamp(0.0, 0.999_99));
    if crop[0] + crop[2] >= 1.0 {
        crop[2] = (0.999_99 - crop[0]).max(0.0);
    }
    if crop[1] + crop[3] >= 1.0 {
        crop[3] = (0.999_99 - crop[1]).max(0.0);
    }
    crop
}

#[inline(always)]
pub fn compose_fractional_crop(
    current_fraction: [f32; 4],
    current_pixels: [f32; 4],
    added: [f32; 4],
) -> ([f32; 4], [f32; 4]) {
    let added = normalized_crop(added);
    let vertical_fraction = (1.0 - current_fraction[0] - current_fraction[2]).max(0.0);
    let horizontal_fraction = (1.0 - current_fraction[1] - current_fraction[3]).max(0.0);
    let vertical_pixels = -(current_pixels[0] + current_pixels[2]);
    let horizontal_pixels = -(current_pixels[1] + current_pixels[3]);
    (
        [
            current_fraction[0] + added[0] * vertical_fraction,
            current_fraction[1] + added[1] * horizontal_fraction,
            current_fraction[2] + added[2] * vertical_fraction,
            current_fraction[3] + added[3] * horizontal_fraction,
        ],
        [
            current_pixels[0] + added[0] * vertical_pixels,
            current_pixels[1] + added[1] * horizontal_pixels,
            current_pixels[2] + added[2] * vertical_pixels,
            current_pixels[3] + added[3] * horizontal_pixels,
        ],
    )
}

#[inline(always)]
pub fn signed_edges_bounds(
    edges: [f32; 4],
    modifier_crop: [f32; 4],
    modifier_crop_pixels: [f32; 4],
    source_size: glam::Vec2,
) -> ([f32; 4], [f32; 4]) {
    let source_size = source_size.max(glam::Vec2::ONE);
    let [top, right, bottom, left] = edges;
    let base = normalized_crop([
        (-top).max(0.0) / source_size.y,
        (-right).max(0.0) / source_size.x,
        (-bottom).max(0.0) / source_size.y,
        (-left).max(0.0) / source_size.x,
    ]);
    let horizontal = (1.0 - base[1] - base[3]).max(0.0);
    let vertical = (1.0 - base[0] - base[2]).max(0.0);
    let crop = normalized_crop([
        base[0] + modifier_crop[0] * vertical + modifier_crop_pixels[0] / source_size.y,
        base[1] + modifier_crop[1] * horizontal + modifier_crop_pixels[1] / source_size.x,
        base[2] + modifier_crop[2] * vertical + modifier_crop_pixels[2] / source_size.y,
        base[3] + modifier_crop[3] * horizontal + modifier_crop_pixels[3] / source_size.x,
    ]);
    (
        crop,
        [top.max(0.0), right.max(0.0), bottom.max(0.0), left.max(0.0)],
    )
}

#[inline(always)]
pub fn source_size_for_signed_frame(
    edges: [f32; 4],
    modifier_crop: [f32; 4],
    modifier_crop_pixels: [f32; 4],
    frame_size: glam::Vec2,
) -> glam::Vec2 {
    let axis = |frame: f32,
                start: f32,
                end: f32,
                crop_start: f32,
                crop_end: f32,
                pixel_start: f32,
                pixel_end: f32| {
        let content = (frame - start.max(0.0) - end.max(0.0)).max(0.000_01);
        let modifier_remaining = (1.0 - crop_start - crop_end).max(0.000_01);
        let inset = (-start).max(0.0) + (-end).max(0.0);
        let uncropped = (content + pixel_start + pixel_end).max(0.000_01);
        let excessive_limit = inset * 0.000_01 * modifier_remaining;
        if inset > 0.0 && uncropped <= excessive_limit {
            (uncropped / (0.000_01 * modifier_remaining)).max(1.0)
        } else {
            (uncropped / modifier_remaining + inset).max(1.0)
        }
    };
    glam::Vec2::new(
        axis(
            frame_size.x,
            edges[3],
            edges[1],
            modifier_crop[3],
            modifier_crop[1],
            modifier_crop_pixels[3],
            modifier_crop_pixels[1],
        ),
        axis(
            frame_size.y,
            edges[0],
            edges[2],
            modifier_crop[0],
            modifier_crop[2],
            modifier_crop_pixels[0],
            modifier_crop_pixels[2],
        ),
    )
}

#[inline(always)]
/// Horizontally blurs the pixel at `index`.
///
/// # Safety
///
/// `input` must reference a readable RGBA buffer containing `index` and every
/// horizontally clamped sample in its row, and `width` must be nonzero.
pub unsafe fn gaussian_horizontal_rgba(
    input: *const u32,
    index: usize,
    width: u32,
    radius: u32,
) -> u32 {
    let x = (index as u32 % width) as i32;
    let y = (index as u32 / width) as usize;
    let radius = radius.min(100) as i32;
    let mut sum = [0.0; 4];
    let mut total = 0.0;
    for distance in -radius..=radius {
        let sample_x = (x + distance).clamp(0, width as i32 - 1) as usize;
        let color = Color::from_rgba_u32(unsafe { *input.add(y * width as usize + sample_x) });
        let weight = gaussian_weight(distance, radius as u32);
        sum[0] += color.r * color.a * weight;
        sum[1] += color.g * color.a * weight;
        sum[2] += color.b * color.a * weight;
        sum[3] += color.a * weight;
        total += weight;
    }
    Color::new(
        sum[0] / total,
        sum[1] / total,
        sum[2] / total,
        sum[3] / total,
    )
    .to_rgba_u32()
}

#[inline(always)]
/// Vertically blurs the pixel at `index`.
///
/// # Safety
///
/// `input` must reference a readable RGBA buffer of at least `width * height`
/// pixels, `index` must address that buffer, and both dimensions must be nonzero.
pub unsafe fn gaussian_vertical_rgba(
    input: *const u32,
    index: usize,
    width: u32,
    height: u32,
    radius: u32,
) -> u32 {
    let x = (index as u32 % width) as usize;
    let y = (index as u32 / width) as i32;
    let radius = radius.min(100) as i32;
    let mut sum = [0.0; 4];
    let mut total = 0.0;
    for distance in -radius..=radius {
        let sample_y = (y + distance).clamp(0, height as i32 - 1) as usize;
        let color = Color::from_rgba_u32(unsafe { *input.add(sample_y * width as usize + x) });
        let weight = gaussian_weight(distance, radius as u32);
        sum[0] += color.r * weight;
        sum[1] += color.g * weight;
        sum[2] += color.b * weight;
        sum[3] += color.a * weight;
        total += weight;
    }
    let alpha = sum[3] / total;
    let divisor = alpha.max(0.000_01);
    Color::new(
        sum[0] / total / divisor,
        sum[1] / total / divisor,
        sum[2] / total / divisor,
        alpha,
    )
    .to_rgba_u32()
}
