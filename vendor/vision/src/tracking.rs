// Source-derived translation of OpenCV 4.13.0 featureselect.cpp, corner.cpp,
// pyramids.cpp and lkpyramid.cpp. Copyright Intel Corporation 2000 and
// OpenCV Foundation 2013. Distributed under the notice in ../LICENSE.
use crate::GrayImage;

#[derive(Clone, Copy, Debug)]
pub struct FeatureOptions {
    pub maximum: usize,
    pub quality: f64,
    pub minimum_distance: f64,
    pub block_size: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct FlowOptions {
    pub window: usize,
    pub maximum_level: usize,
    pub iterations: usize,
    pub epsilon: f64,
    pub minimum_eigenvalue: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct Track {
    pub previous: [f32; 2],
    pub current: [f32; 2],
    pub valid: bool,
    pub error: f32,
}

// borderInterpolate(..., BORDER_REFLECT_101), including one-pixel images.
fn reflect(mut position: i32, length: usize) -> usize {
    if length == 1 {
        return 0;
    }
    let length = length as i32;
    while position < 0 || position >= length {
        position = if position < 0 {
            -position
        } else {
            length * 2 - position - 2
        };
    }
    position as usize
}

fn pixel(image: &GrayImage, x: i32, y: i32) -> i32 {
    i32::from(image.data[reflect(y, image.height) * image.width + reflect(x, image.width)])
}

pub fn good_features(image: &GrayImage, options: FeatureOptions) -> Result<Vec<[f32; 2]>, String> {
    image.validate()?;
    if !options.quality.is_finite()
        || options.quality <= 0.0
        || !options.minimum_distance.is_finite()
        || options.minimum_distance < 0.0
        || options.block_size == 0
        || options.block_size > i32::MAX as usize
    {
        return Err("invalid feature detection parameters".into());
    }
    let width = image.width;
    let height = image.height;
    let mut covariance = vec![[0.0_f32; 3]; image.data.len()];
    // cornerEigenValsVecs: aperture_size=3, CV_8U, Sobel scale=1/(4*blockSize*255).
    let scale = (1.0 / (4.0 * options.block_size as f64 * 255.0)) as f32;
    for y in 0..height {
        for x in 0..width {
            let (x, y) = (x as i32, y as i32);
            // Separable Sobel: derivative row first, then the scaled smoothing column.
            let dx0 = (pixel(image, x + 1, y - 1) - pixel(image, x - 1, y - 1)) as f32;
            let dx1 = (pixel(image, x + 1, y) - pixel(image, x - 1, y)) as f32;
            let dx2 = (pixel(image, x + 1, y + 1) - pixel(image, x - 1, y + 1)) as f32;
            let dy0 = pixel(image, x - 1, y + 1) as f32 * scale
                + pixel(image, x, y + 1) as f32 * (2.0 * scale)
                + pixel(image, x + 1, y + 1) as f32 * scale;
            let dy1 = pixel(image, x - 1, y - 1) as f32 * scale
                + pixel(image, x, y - 1) as f32 * (2.0 * scale)
                + pixel(image, x + 1, y - 1) as f32 * scale;
            let dx = (dx0 + dx2) * scale + dx1 * (2.0 * scale);
            let dy = dy0 - dy1;
            covariance[y as usize * width + x as usize] = [dx * dx, dx * dy, dy * dy];
        }
    }
    // Unnormalized boxFilter: float input, double row/column sums, float output.
    let anchor = (options.block_size / 2) as i32;
    let mut eigen = vec![0.0_f32; image.data.len()];
    for y in 0..height {
        for x in 0..width {
            let mut sum = [0.0_f64; 3];
            for ky in 0..options.block_size {
                let sy = reflect(y as i32 + ky as i32 - anchor, height);
                for kx in 0..options.block_size {
                    let sx = reflect(x as i32 + kx as i32 - anchor, width);
                    for (sum, value) in sum.iter_mut().zip(covariance[sy * width + sx]) {
                        *sum += f64::from(value);
                    }
                }
            }
            let a = sum[0] as f32 * 0.5;
            let b = sum[1] as f32;
            let c = sum[2] as f32 * 0.5;
            eigen[y * width + x] = (a + c) - ((a - c) * (a - c) + b * b).sqrt();
        }
    }
    let maximum = eigen.iter().copied().fold(0.0_f32, f32::max);
    let threshold = (f64::from(maximum) * options.quality) as f32;
    for value in &mut eigen {
        if *value <= threshold {
            *value = 0.0;
        }
    }
    let mut candidates = Vec::new();
    for y in 1..height.saturating_sub(1) {
        for x in 1..width.saturating_sub(1) {
            let value = eigen[y * width + x];
            let mut local_maximum = 0.0_f32;
            for sy in y - 1..=y + 1 {
                for sx in x - 1..=x + 1 {
                    local_maximum = local_maximum.max(eigen[sy * width + sx]);
                }
            }
            if value != 0.0 && value == local_maximum {
                candidates.push(y * width + x);
            }
        }
    }
    // greaterThanPtr uses descending memory address to resolve equal responses.
    candidates.sort_unstable_by(|&a, &b| eigen[b].total_cmp(&eigen[a]).then(b.cmp(&a)));
    let mut corners = Vec::new();
    if options.minimum_distance < 1.0 {
        let count = if options.maximum == 0 {
            candidates.len()
        } else {
            options.maximum
        };
        corners.extend(
            candidates
                .into_iter()
                .take(count)
                .map(|index| [(index % width) as f32, (index / width) as f32]),
        );
        return Ok(corners);
    }
    let cell_size = options.minimum_distance.round_ties_even().max(1.0) as usize;
    let grid_width = width.div_ceil(cell_size);
    let grid_height = height.div_ceil(cell_size);
    let mut grid = vec![Vec::<[f32; 2]>::new(); grid_width * grid_height];
    let minimum_distance_squared = options.minimum_distance * options.minimum_distance;
    for index in candidates {
        let point = [(index % width) as f32, (index / width) as f32];
        let gx = index % width / cell_size;
        let gy = index / width / cell_size;
        let mut good = true;
        'neighbours: for cy in gy.saturating_sub(1)..=(gy + 1).min(grid_height - 1) {
            for cx in gx.saturating_sub(1)..=(gx + 1).min(grid_width - 1) {
                for other in &grid[cy * grid_width + cx] {
                    let dx = point[0] - other[0];
                    let dy = point[1] - other[1];
                    if f64::from(dx * dx + dy * dy) < minimum_distance_squared {
                        good = false;
                        break 'neighbours;
                    }
                }
            }
        }
        if good {
            grid[gy * grid_width + gx].push(point);
            corners.push(point);
            if options.maximum > 0 && corners.len() == options.maximum {
                break;
            }
        }
    }
    Ok(corners)
}

pub(crate) fn pyramid(image: &GrayImage, options: FlowOptions) -> Vec<GrayImage> {
    let mut levels = vec![image.clone()];
    for _ in 0..options.maximum_level {
        let previous = levels.last().unwrap();
        let width = previous.width.div_ceil(2);
        let height = previous.height.div_ceil(2);
        // buildOpticalFlowPyramid stops before creating a level <= the LK window.
        if width <= options.window || height <= options.window {
            break;
        }
        let mut next = GrayImage {
            width,
            height,
            data: vec![0; width * height],
        };
        // pyrDown: separable Gaussian [1,4,6,4,1], FixPtCast<int,uchar,8>.
        const GAUSSIAN: [i32; 5] = [1, 4, 6, 4, 1];
        for y in 0..height {
            for x in 0..width {
                let mut sum = 0;
                for (ky, &wy) in GAUSSIAN.iter().enumerate() {
                    for (kx, &wx) in GAUSSIAN.iter().enumerate() {
                        sum += wx
                            * wy
                            * pixel(previous, (2 * x + kx) as i32 - 2, (2 * y + ky) as i32 - 2);
                    }
                }
                next.data[y * width + x] = ((sum + 128) >> 8) as u8;
            }
        }
        levels.push(next);
    }
    levels
}

pub(crate) fn scharr(image: &GrayImage) -> Vec<[i16; 2]> {
    let mut result = vec![[0; 2]; image.data.len()];
    for y in 0..image.height {
        for x in 0..image.width {
            let (x, y) = (x as i32, y as i32);
            let dx = 3
                * (pixel(image, x + 1, y - 1) + pixel(image, x + 1, y + 1)
                    - pixel(image, x - 1, y - 1)
                    - pixel(image, x - 1, y + 1))
                + 10 * (pixel(image, x + 1, y) - pixel(image, x - 1, y));
            let dy = 3
                * (pixel(image, x - 1, y + 1) + pixel(image, x + 1, y + 1)
                    - pixel(image, x - 1, y - 1)
                    - pixel(image, x + 1, y - 1))
                + 10 * (pixel(image, x, y + 1) - pixel(image, x, y - 1));
            result[y as usize * image.width + x as usize] = [dx as i16, dy as i16];
        }
    }
    result
}

const WEIGHT_BITS: u32 = 14;
const INTENSITY_BITS: u32 = 5;
const COVARIANCE_SCALE: f32 = 1.0 / (1 << 20) as f32;

fn weights(point: [f32; 2]) -> (i32, i32, [i32; 4]) {
    let x = point[0].floor() as i32;
    let y = point[1].floor() as i32;
    let a = point[0] - point[0].floor();
    let b = point[1] - point[1].floor();
    let scale = (1 << WEIGHT_BITS) as f32;
    let w0 = ((1.0 - a) * (1.0 - b) * scale).round_ties_even() as i32;
    let w1 = (a * (1.0 - b) * scale).round_ties_even() as i32;
    let w2 = ((1.0 - a) * b * scale).round_ties_even() as i32;
    (x, y, [w0, w1, w2, (1 << WEIGHT_BITS) - w0 - w1 - w2])
}

fn interpolate(image: &GrayImage, x: i32, y: i32, weights: [i32; 4]) -> i32 {
    let value = pixel(image, x, y) * weights[0]
        + pixel(image, x + 1, y) * weights[1]
        + pixel(image, x, y + 1) * weights[2]
        + pixel(image, x + 1, y + 1) * weights[3];
    (value + (1 << (WEIGHT_BITS - INTENSITY_BITS - 1))) >> (WEIGHT_BITS - INTENSITY_BITS)
}

fn derivative(image: &GrayImage, derivatives: &[[i16; 2]], x: i32, y: i32) -> [i32; 2] {
    // buildOpticalFlowPyramid's derivative border is BORDER_CONSTANT, not reflected.
    if x < 0 || y < 0 || x >= image.width as i32 || y >= image.height as i32 {
        return [0; 2];
    }
    derivatives[y as usize * image.width + x as usize].map(i32::from)
}

fn in_bounds(image: &GrayImage, x: i32, y: i32, window: usize) -> bool {
    x >= -(window as i32)
        && y >= -(window as i32)
        && x < image.width as i32
        && y < image.height as i32
}

pub(crate) fn validate_flow(
    previous: &GrayImage,
    current: &GrayImage,
    points: &[[f32; 2]],
    options: FlowOptions,
) -> Result<(), String> {
    previous.validate()?;
    current.validate()?;
    if previous.width != current.width || previous.height != current.height {
        return Err("optical flow frame dimensions do not match".into());
    }
    if options.window <= 2
        || options.window > i16::MAX as usize
        || options.maximum_level > 30
        || !options.epsilon.is_finite()
        || !options.minimum_eigenvalue.is_finite()
        || points
            .iter()
            .flatten()
            .any(|coordinate| !coordinate.is_finite())
    {
        return Err("invalid optical flow parameters".into());
    }
    Ok(())
}

pub fn track(
    previous: &GrayImage,
    current: &GrayImage,
    points: &[[f32; 2]],
    options: FlowOptions,
) -> Result<Vec<Track>, String> {
    validate_flow(previous, current, points, options)?;
    if points.is_empty() {
        return Ok(Vec::new());
    }
    let early = pyramid(previous, options);
    let late = pyramid(current, options);
    let maximum_level = early.len().min(late.len()) - 1;
    let mut tracks: Vec<_> = points
        .iter()
        .copied()
        .map(|point| Track {
            previous: point,
            current: point,
            valid: true,
            error: 0.0,
        })
        .collect();
    let half_window = (options.window - 1) as f32 * 0.5;
    let epsilon = options.epsilon.clamp(0.0, 10.0).powi(2);
    let iterations = options.iterations.min(100);
    let area = options.window * options.window;
    let mut patch = vec![0_i32; area];
    let mut gradients = vec![[0_i32; 2]; area];
    for level in (0..=maximum_level).rev() {
        let image = &early[level];
        let next_image = &late[level];
        let derivatives = scharr(image);
        for track in &mut tracks {
            let scale = 1.0 / (1_u32 << level) as f32;
            let previous_point = track.previous.map(|value| value * scale);
            track.current = if level == maximum_level {
                previous_point
            } else {
                track.current.map(|value| value * 2.0)
            };
            let previous_point = previous_point.map(|value| value - half_window);
            let (px, py, weight) = weights(previous_point);
            if !in_bounds(image, px, py, options.window) {
                if level == 0 {
                    track.valid = false;
                    track.error = 0.0;
                }
                continue;
            }
            let (mut a11, mut a12, mut a22) = (0.0_f32, 0.0_f32, 0.0_f32);
            for y in 0..options.window {
                for x in 0..options.window {
                    let sx = px + x as i32;
                    let sy = py + y as i32;
                    let index = y * options.window + x;
                    patch[index] = interpolate(image, sx, sy, weight);
                    let d00 = derivative(image, &derivatives, sx, sy);
                    let d01 = derivative(image, &derivatives, sx + 1, sy);
                    let d10 = derivative(image, &derivatives, sx, sy + 1);
                    let d11 = derivative(image, &derivatives, sx + 1, sy + 1);
                    let mut gradient = [0; 2];
                    for axis in 0..2 {
                        gradient[axis] = (d00[axis] * weight[0]
                            + d01[axis] * weight[1]
                            + d10[axis] * weight[2]
                            + d11[axis] * weight[3]
                            + (1 << (WEIGHT_BITS - 1)))
                            >> WEIGHT_BITS;
                    }
                    gradients[index] = gradient;
                    a11 += (gradient[0] * gradient[0]) as f32;
                    a12 += (gradient[0] * gradient[1]) as f32;
                    a22 += (gradient[1] * gradient[1]) as f32;
                }
            }
            a11 *= COVARIANCE_SCALE;
            a12 *= COVARIANCE_SCALE;
            a22 *= COVARIANCE_SCALE;
            let determinant = a11 * a22 - a12 * a12;
            let minimum_eigenvalue = (a22 + a11
                - ((a11 - a22) * (a11 - a22) + 4.0 * a12 * a12).sqrt())
                / (2 * area) as f32;
            if minimum_eigenvalue < options.minimum_eigenvalue as f32 || determinant < f32::EPSILON
            {
                if level == 0 {
                    track.valid = false;
                }
                continue;
            }
            let inverse_determinant = 1.0 / determinant;
            let mut next_point = track.current.map(|value| value - half_window);
            let mut previous_delta = [0.0_f32; 2];
            for iteration in 0..iterations {
                let (nx, ny, weight) = weights(next_point);
                if !in_bounds(next_image, nx, ny, options.window) {
                    if level == 0 {
                        track.valid = false;
                    }
                    break;
                }
                let (mut b1, mut b2) = (0.0_f32, 0.0_f32);
                for y in 0..options.window {
                    for x in 0..options.window {
                        let index = y * options.window + x;
                        let difference =
                            interpolate(next_image, nx + x as i32, ny + y as i32, weight)
                                - patch[index];
                        b1 += (difference * gradients[index][0]) as f32;
                        b2 += (difference * gradients[index][1]) as f32;
                    }
                }
                b1 *= COVARIANCE_SCALE;
                b2 *= COVARIANCE_SCALE;
                let delta = [
                    (a12 * b2 - a22 * b1) * inverse_determinant,
                    (a12 * b1 - a11 * b2) * inverse_determinant,
                ];
                for axis in 0..2 {
                    next_point[axis] += delta[axis];
                    track.current[axis] = next_point[axis] + half_window;
                }
                if f64::from(delta[0]) * f64::from(delta[0])
                    + f64::from(delta[1]) * f64::from(delta[1])
                    <= epsilon
                {
                    break;
                }
                if iteration > 0
                    && (delta[0] + previous_delta[0]).abs() < 0.01
                    && (delta[1] + previous_delta[1]).abs() < 0.01
                {
                    for (coordinate, delta) in track.current.iter_mut().zip(delta) {
                        *coordinate -= delta * 0.5;
                    }
                    break;
                }
                previous_delta = delta;
            }
            if track.valid && level == 0 {
                let (nx, ny, weight) = weights(track.current.map(|value| value - half_window));
                if !in_bounds(next_image, nx, ny, options.window) {
                    track.valid = false;
                    continue;
                }
                let mut error = 0.0_f32;
                for y in 0..options.window {
                    for x in 0..options.window {
                        let index = y * options.window + x;
                        error += (interpolate(next_image, nx + x as i32, ny + y as i32, weight)
                            - patch[index])
                            .abs() as f32;
                    }
                }
                track.error = error / ((1 << INTENSITY_BITS) * area) as f32;
            }
        }
    }
    Ok(tracks)
}
