use glam::{Mat3, Vec2};

const PROJECTIVE_EPSILON: f32 = 0.000_001;
pub const UNIT_QUAD: [Vec2; 4] = [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y];

/// Returns the homography that maps a corner-pin destination back to the unit source rectangle.
/// Corners are ordered top-left, top-right, bottom-right, bottom-left.
pub fn corner_pin_inverse(corners: [Vec2; 4]) -> Option<Mat3> {
    if !is_convex(corners) {
        return None;
    }
    solve_projective(corners.into_iter().zip(UNIT_QUAD))
}

fn is_convex(points: [Vec2; 4]) -> bool {
    let mut winding = 0.0;
    for index in 0..4 {
        let a = points[index];
        let b = points[(index + 1) % 4];
        let c = points[(index + 2) % 4];
        let cross = (b - a).perp_dot(c - b);
        if cross.abs() <= PROJECTIVE_EPSILON {
            return false;
        }
        if winding == 0.0 {
            winding = cross;
        } else if winding.signum() != cross.signum() {
            return false;
        }
    }
    true
}

fn solve_projective(correspondences: impl IntoIterator<Item = (Vec2, Vec2)>) -> Option<Mat3> {
    let mut matrix = [[0.0; 9]; 8];
    for (index, (source, destination)) in correspondences.into_iter().enumerate() {
        if index >= 4 {
            return None;
        }
        let Vec2 { x, y } = source;
        let Vec2 {
            x: destination_x,
            y: destination_y,
        } = destination;
        matrix[index * 2] = [
            x,
            y,
            1.0,
            0.0,
            0.0,
            0.0,
            -x * destination_x,
            -y * destination_x,
            destination_x,
        ];
        matrix[index * 2 + 1] = [
            0.0,
            0.0,
            0.0,
            x,
            y,
            1.0,
            -x * destination_y,
            -y * destination_y,
            destination_y,
        ];
    }
    for column in 0..8 {
        let pivot = (column..8).max_by(|left, right| {
            matrix[*left][column]
                .abs()
                .total_cmp(&matrix[*right][column].abs())
        })?;
        if matrix[pivot][column].abs() <= PROJECTIVE_EPSILON {
            return None;
        }
        matrix.swap(column, pivot);
        let divisor = matrix[column][column];
        for value in &mut matrix[column][column..] {
            *value /= divisor;
        }
        let pivot_row = matrix[column];
        for (row, values) in matrix.iter_mut().enumerate() {
            if row == column {
                continue;
            }
            let factor = values[column];
            for (value, pivot) in values[column..].iter_mut().zip(&pivot_row[column..]) {
                *value -= factor * pivot;
            }
        }
    }
    let values: [f32; 8] = std::array::from_fn(|index| matrix[index][8]);
    Some(Mat3::from_cols_array(&[
        values[0], values[3], values[6], values[1], values[4], values[7], values[2], values[5], 1.0,
    ]))
}

#[inline(always)]
pub fn projective_point(transform: Mat3, point: Vec2) -> Option<Vec2> {
    let denominator =
        transform.x_axis.z * point.x + transform.y_axis.z * point.y + transform.z_axis.z;
    if denominator.abs() <= f32::EPSILON {
        return None;
    }
    Some(Vec2::new(
        (transform.x_axis.x * point.x + transform.y_axis.x * point.y + transform.z_axis.x)
            / denominator,
        (transform.x_axis.y * point.x + transform.y_axis.y * point.y + transform.z_axis.y)
            / denominator,
    ))
}

const BILINEAR_INVERSE_ITERATIONS: usize = 6;
const BILINEAR_INVERSE_EPSILON: f32 = 0.000_001;

#[inline(always)]
pub fn inverse_bilinear_quad(corners: [Vec2; 4], point: Vec2, initial: Vec2) -> Option<Vec2> {
    let [top_left, top_right, bottom_right, bottom_left] = corners;
    let horizontal = top_right - top_left;
    let vertical = bottom_left - top_left;
    let diagonal = top_left - top_right + bottom_right - bottom_left;
    let Vec2 { mut x, mut y } = initial;
    for _ in 0..BILINEAR_INVERSE_ITERATIONS {
        let error = top_left + horizontal * x + vertical * y + diagonal * x * y - point;
        let derivative_x = horizontal + diagonal * y;
        let derivative_y = vertical + diagonal * x;
        let determinant = derivative_x.perp_dot(derivative_y);
        if determinant.abs() <= BILINEAR_INVERSE_EPSILON {
            return None;
        }
        x -= error.perp_dot(derivative_y) / determinant;
        y -= derivative_x.perp_dot(error) / determinant;
    }
    (x.is_finite() && y.is_finite()).then_some(Vec2::new(x, y))
}
