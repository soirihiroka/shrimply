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
    // cornerEigenValsVecs/deriv.cpp: scale the smoothing kernel only. RowFilter's
    // left-to-right f32 arithmetic is significant for nearly equal corner scores.
    let scale = (1.0 / (4.0 * options.block_size as f64 * 255.0)) as f32;
    let mut horizontal = vec![[0.0_f32; 2]; image.data.len()];
    let mut padded = vec![0_u8; width + 2];
    for (source, destination) in image
        .data
        .chunks_exact(width)
        .zip(horizontal.chunks_exact_mut(width))
    {
        padded[1..=width].copy_from_slice(source);
        padded[0] = source[reflect(-1, width)];
        padded[width + 1] = source[reflect(width as i32, width)];
        for (destination, source) in destination.iter_mut().zip(padded.windows(3)) {
            destination[0] = (i32::from(source[2]) - i32::from(source[0])) as f32;
            destination[1] = f32::from(source[0]) * scale
                + f32::from(source[1]) * (2.0 * scale)
                + f32::from(source[2]) * scale;
        }
    }
    let block = options.block_size;
    let anchor = block / 2;
    let mut covariance = vec![[0.0_f32; 3]; width + block - 1];
    let mut row_sum = |y: usize, destination: &mut [[f64; 3]]| {
        let top = reflect(y as i32 - 1, height) * width;
        let middle = y * width;
        let bottom = reflect(y as i32 + 1, height) * width;
        let rows = horizontal[top..top + width]
            .iter()
            .zip(&horizontal[middle..middle + width])
            .zip(&horizontal[bottom..bottom + width]);
        for (destination, ((top, middle), bottom)) in
            covariance[anchor..anchor + width].iter_mut().zip(rows)
        {
            let dx = (top[0] + bottom[0]) * scale + middle[0] * (2.0 * scale);
            let dy = bottom[1] - top[1];
            *destination = [dx * dx, dx * dy, dy * dy];
        }
        for x in 0..anchor {
            covariance[x] = covariance[anchor + reflect(x as i32 - anchor as i32, width)];
        }
        for x in anchor + width..covariance.len() {
            covariance[x] = covariance[anchor + reflect(x as i32 - anchor as i32, width)];
        }
        // box_filter.simd.hpp RowSum<float,double>: 3/5 taps are summed directly;
        // other widths use a sliding sum. No box normalization is applied.
        if block == 3 {
            for (destination, source) in destination.iter_mut().zip(covariance.windows(3)) {
                for axis in 0..3 {
                    destination[axis] = f64::from(source[0][axis])
                        + f64::from(source[1][axis])
                        + f64::from(source[2][axis]);
                }
            }
        } else if block == 5 {
            for (destination, source) in destination.iter_mut().zip(covariance.windows(5)) {
                for axis in 0..3 {
                    destination[axis] = f64::from(source[0][axis])
                        + f64::from(source[1][axis])
                        + f64::from(source[2][axis])
                        + f64::from(source[3][axis])
                        + f64::from(source[4][axis]);
                }
            }
        } else {
            let mut sum = [0.0; 3];
            for value in &covariance[..block] {
                for axis in 0..3 {
                    sum[axis] += f64::from(value[axis]);
                }
            }
            destination[0] = sum;
            for (x, destination) in destination.iter_mut().enumerate().skip(1) {
                for axis in 0..3 {
                    sum[axis] += f64::from(covariance[x + block - 1][axis])
                        - f64::from(covariance[x - 1][axis]);
                }
                *destination = sum;
            }
        }
    };
    let mut ring = vec![vec![[0.0_f64; 3]; width]; block];
    let mut column_sum = vec![[0.0_f64; 3]; width];
    for (y, row) in ring.iter_mut().enumerate().take(block - 1) {
        row_sum(reflect(y as i32 - anchor as i32, height), row);
        for (sum, value) in column_sum.iter_mut().zip(row) {
            for axis in 0..3 {
                sum[axis] += value[axis];
            }
        }
    }
    let mut eigen = vec![0.0_f32; image.data.len()];
    for (y, destination) in eigen.chunks_exact_mut(width).enumerate() {
        let entering = (y + block - 1) % block;
        row_sum(
            reflect((y + block - 1) as i32 - anchor as i32, height),
            &mut ring[entering],
        );
        // ColumnSum<double,float>: add entering row, emit f32, subtract leaving row.
        let rows = ring[entering].iter().zip(&ring[y % block]);
        for ((sum, destination), (entering, leaving)) in
            column_sum.iter_mut().zip(destination).zip(rows)
        {
            let mut covariance = [0.0; 3];
            for axis in 0..3 {
                covariance[axis] = sum[axis] + entering[axis];
                sum[axis] = covariance[axis] - leaving[axis];
            }
            let a = covariance[0] as f32 * 0.5;
            let b = covariance[1] as f32;
            let c = covariance[2] as f32 * 0.5;
            *destination = (a + c) - ((a - c) * (a - c) + b * b).sqrt();
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
        'columns: for x in 1..width.saturating_sub(1) {
            let value = eigen[y * width + x];
            if value == 0.0 {
                continue;
            }
            // Equivalent to equality with a 3x3 dilation; stop once a larger
            // neighbour rules out this candidate instead of finishing the max.
            for sy in y - 1..=y + 1 {
                for neighbour in &eigen[sy * width + x - 1..=sy * width + x + 1] {
                    if *neighbour > value {
                        continue 'columns;
                    }
                }
            }
            candidates.push(y * width + x);
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

pub(crate) fn pyramid_sizes(
    width: usize,
    height: usize,
    options: FlowOptions,
) -> Vec<(usize, usize)> {
    let mut sizes = vec![(width, height)];
    for _ in 0..options.maximum_level {
        let (width, height) = *sizes.last().expect("pyramid has a base level");
        let next = (width.div_ceil(2), height.div_ceil(2));
        // buildOpticalFlowPyramid stops before a level <= the LK window.
        if next.0 <= options.window || next.1 <= options.window {
            break;
        }
        sizes.push(next);
    }
    sizes
}

pub(crate) fn pyramid(image: &GrayImage, options: FlowOptions) -> Vec<GrayImage> {
    let mut levels = vec![image.clone()];
    for (width, height) in pyramid_sizes(image.width, image.height, options)
        .into_iter()
        .skip(1)
    {
        let previous = levels.last().expect("pyramid has a base level");
        let mut next = GrayImage {
            width,
            height,
            data: vec![0; width * height],
        };
        // pyramids.cpp PyrDownInvoker: five cached horizontal rows, decimated
        // by two, then the vertical [1,4,6,4,1] kernel and FixPtCast<uchar,8>.
        const TAPS: usize = 5;
        let mut ring = vec![vec![0_i32; width]; TAPS];
        let mut padded = vec![0_u8; previous.width + TAPS - 1];
        let mut source_y = -2_i32;
        for (y, destination) in next.data.chunks_exact_mut(width).enumerate() {
            while source_y <= (2 * y + 2) as i32 {
                let source_start = reflect(source_y, previous.height) * previous.width;
                let source = &previous.data[source_start..source_start + previous.width];
                padded[2..2 + previous.width].copy_from_slice(source);
                for x in 0..2 {
                    padded[x] = source[reflect(x as i32 - 2, previous.width)];
                    padded[2 + previous.width + x] =
                        source[reflect((previous.width + x) as i32, previous.width)];
                }
                let row = &mut ring[(source_y + 2) as usize % TAPS];
                for (destination, source) in row.iter_mut().zip(padded.windows(TAPS).step_by(2)) {
                    *destination = i32::from(source[2]) * 6
                        + (i32::from(source[1]) + i32::from(source[3])) * 4
                        + i32::from(source[0])
                        + i32::from(source[4]);
                }
                source_y += 1;
            }
            let rows = [0, 1, 2, 3, 4].map(|k| &ring[(2 * y + k) % TAPS]);
            for (x, destination) in destination.iter_mut().enumerate() {
                let sum = rows[2][x] * 6 + (rows[1][x] + rows[3][x]) * 4 + rows[0][x] + rows[4][x];
                *destination = ((sum + 128) >> 8) as u8;
            }
        }
        levels.push(next);
    }
    levels
}

pub(crate) fn scharr(image: &GrayImage) -> Vec<[i16; 2]> {
    // lkpyramid.cpp ScharrDerivInvoker: vertical convolutions share a padded
    // intermediate row, followed by horizontal derivative/smoothing passes.
    let mut result = vec![[0_i16; 2]; image.data.len()];
    let mut temporary = vec![[0_i16; 2]; image.width + 2];
    for (y, destination) in result.chunks_exact_mut(image.width).enumerate() {
        let upper = reflect(y as i32 - 1, image.height) * image.width;
        let lower = reflect(y as i32 + 1, image.height) * image.width;
        let rows = image.data[upper..upper + image.width]
            .iter()
            .zip(&image.data[y * image.width..(y + 1) * image.width])
            .zip(&image.data[lower..lower + image.width]);
        for (temporary, ((&upper, &middle), &lower)) in
            temporary[1..=image.width].iter_mut().zip(rows)
        {
            *temporary = [
                (i16::from(upper) + i16::from(lower)) * 3 + i16::from(middle) * 10,
                i16::from(lower) - i16::from(upper),
            ];
        }
        temporary[0] = temporary[1 + reflect(-1, image.width)];
        temporary[image.width + 1] = temporary[1 + reflect(image.width as i32, image.width)];
        for (destination, source) in destination.iter_mut().zip(temporary.windows(3)) {
            *destination = [
                source[2][0] - source[0][0],
                (source[2][1] + source[0][1]) * 3 + source[1][1] * 10,
            ];
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

// buildOpticalFlowPyramid/copyMakeBorder: reflected pixels and a zero
// derivative border. Preparing it once removes border mapping from LK's loops.
fn padded_image(image: &GrayImage, border: usize) -> (usize, Vec<u8>) {
    let stride = image.width + 2 * border;
    let mut data = vec![0; stride * (image.height + 2 * border)];
    let left: Vec<_> = (0..border)
        .map(|x| reflect(x as i32 - border as i32, image.width))
        .collect();
    let right: Vec<_> = (0..border)
        .map(|x| reflect((image.width + x) as i32, image.width))
        .collect();
    for (y, destination) in data.chunks_exact_mut(stride).enumerate() {
        let start = reflect(y as i32 - border as i32, image.height) * image.width;
        let source = &image.data[start..start + image.width];
        destination[border..border + image.width].copy_from_slice(source);
        for (value, &x) in destination[..border].iter_mut().zip(&left) {
            *value = source[x];
        }
        for (value, &x) in destination[border + image.width..].iter_mut().zip(&right) {
            *value = source[x];
        }
    }
    (stride, data)
}

fn interpolate(upper: &[u8], lower: &[u8], weights: [i32; 4]) -> i32 {
    let value = i32::from(upper[0]) * weights[0]
        + i32::from(upper[1]) * weights[1]
        + i32::from(lower[0]) * weights[2]
        + i32::from(lower[1]) * weights[3];
    (value + (1 << (WEIGHT_BITS - INTENSITY_BITS - 1))) >> (WEIGHT_BITS - INTENSITY_BITS)
}

// OpenCV's integer SIMD interpolation, followed by the original ordered f32
// reductions. Portable SIMD keeps identical integer lanes on every CPU backend.
#[inline]
fn interpolate_row(upper: &[u8], lower: &[u8], weights: [i32; 4], destination: &mut [i32]) {
    use std::simd::{Simd, num::SimdUint};
    const LANES: usize = 4;
    let coefficients = weights.map(Simd::<i32, LANES>::splat);
    let length = destination.len();
    let (upper_left, _) = upper[..length].as_chunks::<LANES>();
    let (upper_right, _) = upper[1..=length].as_chunks::<LANES>();
    let (lower_left, _) = lower[..length].as_chunks::<LANES>();
    let (lower_right, _) = lower[1..=length].as_chunks::<LANES>();
    let (blocks, tail) = destination.as_chunks_mut::<LANES>();
    let samples = upper_left
        .iter()
        .zip(upper_right)
        .zip(lower_left)
        .zip(lower_right);
    for (destination, (((upper_left, upper_right), lower_left), lower_right)) in
        blocks.iter_mut().zip(samples)
    {
        let upper_left = Simd::<u8, LANES>::from_array(*upper_left).cast::<i32>();
        let upper_right = Simd::<u8, LANES>::from_array(*upper_right).cast::<i32>();
        let lower_left = Simd::<u8, LANES>::from_array(*lower_left).cast::<i32>();
        let lower_right = Simd::<u8, LANES>::from_array(*lower_right).cast::<i32>();
        // The four integer weights sum to 2^14, so this is exactly the source
        // four-product sum with one multiply replaced by a shift.
        let value = (upper_left << WEIGHT_BITS as i32)
            + (upper_right - upper_left) * coefficients[1]
            + (lower_left - upper_left) * coefficients[2]
            + (lower_right - upper_left) * coefficients[3];
        let value = (value + Simd::splat(1 << (WEIGHT_BITS - INTENSITY_BITS - 1)))
            >> (WEIGHT_BITS - INTENSITY_BITS) as i32;
        *destination = value.to_array();
    }
    let count = length / LANES * LANES;
    for (x, destination) in tail.iter_mut().enumerate() {
        *destination = interpolate(&upper[count + x..], &lower[count + x..], weights);
    }
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
    let prepared: Vec<_> = early
        .iter()
        .zip(&late)
        .map(|(image, next_image)| {
            let (stride, pixels) = padded_image(image, options.window);
            let (_, next_pixels) = padded_image(next_image, options.window);
            let mut derivatives = vec![[0_i16; 2]; pixels.len()];
            for (y, source) in scharr(image).chunks_exact(image.width).enumerate() {
                let start = (y + options.window) * stride + options.window;
                derivatives[start..start + image.width].copy_from_slice(source);
            }
            (stride, pixels, next_pixels, derivatives)
        })
        .collect();
    let track_chunk = |tracks: &mut [Track]| {
        let mut patch = vec![0_i32; area];
        let mut gradients = vec![[0_i32; 2]; area];
        let mut sampled = vec![0_i32; options.window];
        for level in (0..=maximum_level).rev() {
            let image = &early[level];
            let next_image = &late[level];
            let (stride, pixels, next_pixels, derivatives) = &prepared[level];
            let stride = *stride;
            for track in &mut *tracks {
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
                let previous_start = (py + options.window as i32) as usize * stride
                    + (px + options.window as i32) as usize;
                for y in 0..options.window {
                    let start = previous_start + y * stride;
                    let upper = &pixels[start..start + options.window + 1];
                    let lower = &pixels[start + stride..start + stride + options.window + 1];
                    let upper_derivatives = &derivatives[start..start + options.window + 1];
                    let lower_derivatives =
                        &derivatives[start + stride..start + stride + options.window + 1];
                    let patch_row = &mut patch[y * options.window..(y + 1) * options.window];
                    let gradient_row = &mut gradients[y * options.window..(y + 1) * options.window];
                    interpolate_row(upper, lower, weight, patch_row);
                    let samples = upper_derivatives
                        .windows(2)
                        .zip(lower_derivatives.windows(2));
                    for (gradient, (du, dl)) in gradient_row.iter_mut().zip(samples) {
                        for (axis, value) in gradient.iter_mut().enumerate() {
                            *value = (i32::from(du[0][axis]) * weight[0]
                                + i32::from(du[1][axis]) * weight[1]
                                + i32::from(dl[0][axis]) * weight[2]
                                + i32::from(dl[1][axis]) * weight[3]
                                + (1 << (WEIGHT_BITS - 1)))
                                >> WEIGHT_BITS;
                        }
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
                if minimum_eigenvalue < options.minimum_eigenvalue as f32
                    || determinant < f32::EPSILON
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
                    let next_start = (ny + options.window as i32) as usize * stride
                        + (nx + options.window as i32) as usize;
                    for y in 0..options.window {
                        let start = next_start + y * stride;
                        let upper = &next_pixels[start..start + options.window + 1];
                        let lower =
                            &next_pixels[start + stride..start + stride + options.window + 1];
                        let patch_row = &patch[y * options.window..(y + 1) * options.window];
                        let gradient_row = &gradients[y * options.window..(y + 1) * options.window];
                        interpolate_row(upper, lower, weight, &mut sampled);
                        for ((old, gradient), current) in
                            patch_row.iter().zip(gradient_row).zip(&sampled)
                        {
                            let difference = current - old;
                            b1 += (difference * gradient[0]) as f32;
                            b2 += (difference * gradient[1]) as f32;
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
                    let next_start = (ny + options.window as i32) as usize * stride
                        + (nx + options.window as i32) as usize;
                    for y in 0..options.window {
                        let start = next_start + y * stride;
                        let upper = &next_pixels[start..start + options.window + 1];
                        let lower =
                            &next_pixels[start + stride..start + stride + options.window + 1];
                        let patch_row = &patch[y * options.window..(y + 1) * options.window];
                        interpolate_row(upper, lower, weight, &mut sampled);
                        for (old, current) in patch_row.iter().zip(&sampled) {
                            error += (current - old).abs() as f32;
                        }
                    }
                    track.error = error / ((1 << INTENSITY_BITS) * area) as f32;
                }
            }
        }
    };
    // LKTrackerInvoker's parallel_for_ partitions independent points. Each
    // scoped worker traverses all levels to avoid restarting threads per level.
    let workers = std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .min(tracks.len());
    if workers == 1 {
        track_chunk(&mut tracks);
    } else {
        let chunk_size = tracks.len().div_ceil(workers);
        std::thread::scope(|scope| {
            for chunk in tracks.chunks_mut(chunk_size) {
                scope.spawn(move || track_chunk(chunk));
            }
        });
    }
    Ok(tracks)
}
