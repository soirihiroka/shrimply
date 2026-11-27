// Gap closing follows the endpoint pairing described by Fourey, Tschumperle, and Revoy:
// https://gmic.eu/publications/vmv18.pdf
// and GIMP's GPL-3.0-or-later line-art bucket-fill implementation:
// https://gitlab.gnome.org/GNOME/gimp/-/blob/master/app/core/gimplineart.c

use std::{collections::HashMap, sync::LazyLock};

use crate::{Color, Oklab};

const MINIMUM_LINE_AREA: usize = 5;
const NORMAL_ESTIMATE_DISTANCE: usize = 5;
const ENDPOINT_RATE: f32 = 0.85;
const MAXIMUM_SPLINE_ANGLE_COS: f32 = 0.0;
const ENDPOINT_CONNECTIVITY: i32 = 2;
const SPLINE_ROUNDNESS: f32 = 1.0;
const SIGNIFICANT_CREATED_AREA: usize = 4;
const MINIMUM_ENDPOINT_BUCKET_SIZE: usize = 16;
const COLOR_QUANTIZATION_BITS: usize = 6;
const COLOR_QUANTIZATION_SHIFT: usize = u8::BITS as usize - COLOR_QUANTIZATION_BITS;
const COLOR_LEVELS: usize = 1 << COLOR_QUANTIZATION_BITS;

static OKLAB_LOOKUP: LazyLock<Box<[Oklab]>> = LazyLock::new(|| {
    let mut colors = Vec::with_capacity(COLOR_LEVELS.pow(3));
    let half_step = 1 << COLOR_QUANTIZATION_SHIFT.saturating_sub(1);
    for red in 0..COLOR_LEVELS {
        for green in 0..COLOR_LEVELS {
            for blue in 0..COLOR_LEVELS {
                colors.push(
                    Color::<u8>::from_rgb(
                        ((red << COLOR_QUANTIZATION_SHIFT) + half_step).min(u8::MAX as usize) as u8,
                        ((green << COLOR_QUANTIZATION_SHIFT) + half_step).min(u8::MAX as usize)
                            as u8,
                        ((blue << COLOR_QUANTIZATION_SHIFT) + half_step).min(u8::MAX as usize)
                            as u8,
                    )
                    .to_oklab(),
                );
            }
        }
    }
    colors.into_boxed_slice()
});

#[derive(Clone, Copy)]
struct Endpoint {
    index: usize,
    x: i32,
    y: i32,
    outward_x: f32,
    outward_y: f32,
}

pub fn transparent_fill_mask(
    rgba: &[u8],
    width: u32,
    height: u32,
    seeds: &[(u32, u32)],
    tolerance: f32,
    maximum_gap: u32,
) -> Result<Vec<u8>, String> {
    let width = usize::try_from(width).map_err(|_| "transparent fill width is too large")?;
    let height = usize::try_from(height).map_err(|_| "transparent fill height is too large")?;
    let pixels = width
        .checked_mul(height)
        .ok_or("transparent fill dimensions overflow")?;
    if rgba.len() != pixels.saturating_mul(4) {
        return Err("transparent fill RGBA frame has the wrong length".to_string());
    }
    let stride = width.div_ceil(8);
    let mut selected = vec![0_u8; stride.saturating_mul(height)];
    if width == 0 || height == 0 {
        return Ok(selected);
    }
    let tolerance = tolerance.clamp(0.0, 1.0);
    let tolerance_squared = tolerance * tolerance;
    let mut barrier = vec![0_u8; pixels];
    let mut visited = vec![0_u8; pixels];
    let mut flood_stack = Vec::with_capacity(4096);
    for &(seed_x, seed_y) in seeds {
        let seed_x = (seed_x as usize).min(width - 1);
        let seed_y = (seed_y as usize).min(height - 1);
        let seed_index = seed_y * width + seed_x;
        if rgba[seed_index * 4 + 3] == 0 {
            continue;
        }
        let seed = quantized_oklab(&rgba[seed_index * 4..]);
        for (barrier, pixel) in barrier.iter_mut().zip(rgba.as_chunks::<4>().0.iter()) {
            let color = quantized_oklab(pixel);
            let dl = color.l - seed.l;
            let da = color.a - seed.a;
            let db = color.b - seed.b;
            *barrier = u8::from(dl * dl + da * da + db * db > tolerance_squared);
        }
        if maximum_gap > 0 {
            close_line_art_gaps(&mut barrier, width, height, maximum_gap as usize);
        }
        if barrier[seed_index] != 0 {
            continue;
        }
        visited.fill(0);
        flood_stack.clear();
        flood_into(
            &barrier,
            width,
            seed_index,
            &mut selected,
            stride,
            &mut visited,
            &mut flood_stack,
        );
    }
    Ok(selected)
}

#[inline]
fn quantized_oklab(pixel: &[u8]) -> Oklab {
    let red = pixel[0] as usize >> COLOR_QUANTIZATION_SHIFT;
    let green = pixel[1] as usize >> COLOR_QUANTIZATION_SHIFT;
    let blue = pixel[2] as usize >> COLOR_QUANTIZATION_SHIFT;
    OKLAB_LOOKUP[(red * COLOR_LEVELS + green) * COLOR_LEVELS + blue]
}

fn close_line_art_gaps(barrier: &mut [u8], width: usize, height: usize, maximum_gap: usize) {
    remove_small_components(barrier, width, height);
    let mut skeleton = barrier.to_vec();
    thin(&mut skeleton, width, height);
    let endpoints = endpoints(&skeleton, width, height);
    let mut pairs = Vec::new();
    let bucket_size = maximum_gap.max(MINIMUM_ENDPOINT_BUCKET_SIZE);
    let mut buckets: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for (index, endpoint) in endpoints.iter().enumerate() {
        buckets
            .entry((
                endpoint.x as usize / bucket_size,
                endpoint.y as usize / bucket_size,
            ))
            .or_default()
            .push(index);
    }
    for left in 0..endpoints.len() {
        let a = endpoints[left];
        let bucket_x = a.x as usize / bucket_size;
        let bucket_y = a.y as usize / bucket_size;
        for neighbor_y in bucket_y.saturating_sub(1)..=bucket_y.saturating_add(1) {
            for neighbor_x in bucket_x.saturating_sub(1)..=bucket_x.saturating_add(1) {
                let Some(candidates) = buckets.get(&(neighbor_x, neighbor_y)) else {
                    continue;
                };
                for &right in candidates.iter().filter(|&&right| right > left) {
                    let b = endpoints[right];
                    let dx = (b.x - a.x) as f32;
                    let dy = (b.y - a.y) as f32;
                    let distance = dx.hypot(dy);
                    if distance > maximum_gap as f32 || distance < ENDPOINT_CONNECTIVITY as f32 {
                        continue;
                    }
                    let direction_x = dx / distance;
                    let direction_y = dy / distance;
                    let facing_a = a.outward_x * direction_x + a.outward_y * direction_y;
                    let facing_b = -(b.outward_x * direction_x + b.outward_y * direction_y);
                    if facing_a < MAXIMUM_SPLINE_ANGLE_COS || facing_b < MAXIMUM_SPLINE_ANGLE_COS {
                        continue;
                    }
                    let quality = distance / (ENDPOINT_RATE + facing_a + facing_b);
                    pairs.push((quality, left, right));
                }
            }
        }
    }
    pairs.sort_by(|left, right| {
        left.0
            .total_cmp(&right.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
    });
    let mut used = vec![false; endpoints.len()];
    for (_, left, right) in pairs {
        if used[left] || used[right] {
            continue;
        }
        let a = endpoints[left];
        let b = endpoints[right];
        let bridge = spline(a, b);
        if bridge.len() < SIGNIFICANT_CREATED_AREA
            || intersects_barrier(&bridge, barrier, width, height, a.index, b.index)
        {
            continue;
        }
        for (x, y) in bridge {
            if x >= 0 && y >= 0 && (x as usize) < width && (y as usize) < height {
                barrier[y as usize * width + x as usize] = 1;
            }
        }
        used[left] = true;
        used[right] = true;
    }
    for (index, endpoint) in endpoints.into_iter().enumerate() {
        if used[index] {
            continue;
        }
        extend_endpoint(barrier, width, height, endpoint, maximum_gap);
    }
}

fn remove_small_components(barrier: &mut [u8], width: usize, height: usize) {
    let mut visited = vec![0_u8; barrier.len()];
    let mut component = Vec::new();
    let mut stack = Vec::new();
    for start in 0..barrier.len() {
        if barrier[start] == 0 || visited[start] != 0 {
            continue;
        }
        component.clear();
        stack.clear();
        stack.push(start);
        visited[start] = 1;
        while let Some(index) = stack.pop() {
            component.push(index);
            let x = index % width;
            let y = index / width;
            for_each_neighbor8(x, y, width, height, |neighbor| {
                if barrier[neighbor] != 0 && visited[neighbor] == 0 {
                    visited[neighbor] = 1;
                    stack.push(neighbor);
                }
            });
        }
        if component.len() < MINIMUM_LINE_AREA {
            for &index in &component {
                barrier[index] = 0;
            }
        }
    }
}

fn thin(image: &mut [u8], width: usize, height: usize) {
    if width < 3 || height < 3 {
        return;
    }
    let mut active = Vec::new();
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let index = y * width + x;
            if image[index] != 0 {
                active.push(index);
            }
        }
    }
    let mut second_candidates = Vec::new();
    let mut next_active = Vec::new();
    let mut first_removed = Vec::new();
    let mut second_removed = Vec::new();
    let mut candidate_generation = vec![0_u8; image.len()];
    let mut generation = 0_u8;
    loop {
        first_removed.clear();
        for &index in &active {
            if image[index] != 0 && thinning_candidate(image, width, index, false) {
                first_removed.push(index);
            }
        }
        for &index in &first_removed {
            image[index] = 0;
        }

        generation = generation.wrapping_add(1);
        if generation == 0 {
            candidate_generation.fill(0);
            generation = 1;
        }
        second_candidates.clear();
        for &index in &active {
            if image[index] != 0 && candidate_generation[index] != generation {
                candidate_generation[index] = generation;
                second_candidates.push(index);
            }
        }
        for &index in &first_removed {
            push_thinning_neighbors(
                index,
                image,
                width,
                height,
                &mut second_candidates,
                &mut candidate_generation,
                generation,
            );
        }
        second_removed.clear();
        for &index in &second_candidates {
            if thinning_candidate(image, width, index, true) {
                second_removed.push(index);
            }
        }
        for &index in &second_removed {
            image[index] = 0;
        }

        if first_removed.is_empty() && second_removed.is_empty() {
            break;
        }

        generation = generation.wrapping_add(1);
        if generation == 0 {
            candidate_generation.fill(0);
            generation = 1;
        }
        next_active.clear();
        for &index in first_removed.iter().chain(&second_removed) {
            push_thinning_neighbors(
                index,
                image,
                width,
                height,
                &mut next_active,
                &mut candidate_generation,
                generation,
            );
        }
        std::mem::swap(&mut active, &mut next_active);
    }
}

fn thinning_candidate(image: &[u8], width: usize, index: usize, second_pass: bool) -> bool {
    let x = index % width;
    let y = index / width;
    let p = neighbors_clockwise(image, width, x, y);
    let count = p.iter().filter(|value| **value != 0).count();
    let transitions = (0..8)
        .filter(|i| p[*i] == 0 && p[(*i + 1) % 8] != 0)
        .count();
    let clear = if second_pass {
        p[0] == 0 || p[6] == 0 || p[2] == 0 && p[4] == 0
    } else {
        p[2] == 0 || p[4] == 0 || p[0] == 0 && p[6] == 0
    };
    (2..=6).contains(&count) && transitions == 1 && clear
}

fn push_thinning_neighbors(
    index: usize,
    image: &[u8],
    width: usize,
    height: usize,
    candidates: &mut Vec<usize>,
    candidate_generation: &mut [u8],
    generation: u8,
) {
    let x = index % width;
    let y = index / width;
    for_each_neighbor8(x, y, width, height, |neighbor| {
        let neighbor_x = neighbor % width;
        let neighbor_y = neighbor / width;
        if neighbor_x > 0
            && neighbor_x + 1 < width
            && neighbor_y > 0
            && neighbor_y + 1 < height
            && image[neighbor] != 0
            && candidate_generation[neighbor] != generation
        {
            candidate_generation[neighbor] = generation;
            candidates.push(neighbor);
        }
    });
}

fn endpoints(skeleton: &[u8], width: usize, height: usize) -> Vec<Endpoint> {
    let mut result = Vec::new();
    for index in 0..skeleton.len() {
        if skeleton[index] == 0 {
            continue;
        }
        let x = index % width;
        let y = index / width;
        let mut neighbor_count = 0;
        let mut first_neighbor = 0;
        for_each_neighbor8(x, y, width, height, |neighbor| {
            if skeleton[neighbor] != 0 {
                if neighbor_count == 0 {
                    first_neighbor = neighbor;
                }
                neighbor_count += 1;
            }
        });
        if neighbor_count != 1 {
            continue;
        }
        let mut previous = index;
        let mut current = first_neighbor;
        for _ in 1..NORMAL_ESTIMATE_DISTANCE {
            let cx = current % width;
            let cy = current / width;
            let mut next = None;
            for_each_neighbor8(cx, cy, width, height, |neighbor| {
                if skeleton[neighbor] != 0 && neighbor != previous && next.is_none() {
                    next = Some(neighbor);
                }
            });
            let Some(next) = next else { break };
            previous = current;
            current = next;
        }
        let inner_x = (current % width) as f32;
        let inner_y = (current / width) as f32;
        let mut outward_x = x as f32 - inner_x;
        let mut outward_y = y as f32 - inner_y;
        let length = outward_x.hypot(outward_y).max(f32::EPSILON);
        outward_x /= length;
        outward_y /= length;
        result.push(Endpoint {
            index,
            x: x as i32,
            y: y as i32,
            outward_x,
            outward_y,
        });
    }
    result
}

fn spline(a: Endpoint, b: Endpoint) -> Vec<(i32, i32)> {
    let distance = ((b.x - a.x) as f32).hypot((b.y - a.y) as f32);
    let control_distance = distance * 0.5 * SPLINE_ROUNDNESS;
    let a_control = (
        a.x as f32 + a.outward_x * control_distance,
        a.y as f32 + a.outward_y * control_distance,
    );
    let b_control = (
        b.x as f32 + b.outward_x * control_distance,
        b.y as f32 + b.outward_y * control_distance,
    );
    let steps = distance.ceil().max(1.0) as usize;
    let mut result = Vec::with_capacity(steps + 1);
    for step in 0..=steps {
        let t = step as f32 / steps as f32;
        let one = 1.0 - t;
        let x = one.powi(3) * a.x as f32
            + 3.0 * one.powi(2) * t * a_control.0
            + 3.0 * one * t.powi(2) * b_control.0
            + t.powi(3) * b.x as f32;
        let y = one.powi(3) * a.y as f32
            + 3.0 * one.powi(2) * t * a_control.1
            + 3.0 * one * t.powi(2) * b_control.1
            + t.powi(3) * b.y as f32;
        let point = (x.round() as i32, y.round() as i32);
        if result.last().copied() != Some(point) {
            result.push(point);
        }
    }
    result
}

fn intersects_barrier(
    bridge: &[(i32, i32)],
    barrier: &[u8],
    width: usize,
    height: usize,
    start: usize,
    end: usize,
) -> bool {
    bridge
        .iter()
        .skip(ENDPOINT_CONNECTIVITY as usize)
        .take(
            bridge
                .len()
                .saturating_sub(ENDPOINT_CONNECTIVITY as usize * 2),
        )
        .any(|&(x, y)| {
            if x < 0 || y < 0 || x as usize >= width || y as usize >= height {
                return true;
            }
            let index = y as usize * width + x as usize;
            index != start && index != end && barrier[index] != 0
        })
}

fn extend_endpoint(
    barrier: &mut [u8],
    width: usize,
    height: usize,
    endpoint: Endpoint,
    maximum_gap: usize,
) {
    let mut pending = Vec::new();
    for step in 1..=maximum_gap {
        let x = (endpoint.x as f32 + endpoint.outward_x * step as f32).round() as i32;
        let y = (endpoint.y as f32 + endpoint.outward_y * step as f32).round() as i32;
        if x < 0 || y < 0 || x as usize >= width || y as usize >= height {
            return;
        }
        let index = y as usize * width + x as usize;
        if barrier[index] != 0 {
            if pending.len() >= SIGNIFICANT_CREATED_AREA {
                for index in pending {
                    barrier[index] = 1;
                }
            }
            return;
        }
        pending.push(index);
    }
}

fn flood_into(
    barrier: &[u8],
    width: usize,
    start: usize,
    selected: &mut [u8],
    stride: usize,
    visited: &mut [u8],
    stack: &mut Vec<usize>,
) {
    let height = barrier.len() / width;
    stack.push(start);
    while let Some(start) = stack.pop() {
        if barrier[start] != 0 || visited[start] != 0 {
            continue;
        }
        let y = start / width;
        let row = y * width;
        let mut x = start - row;
        while x > 0 && barrier[row + x - 1] == 0 && visited[row + x - 1] == 0 {
            x -= 1;
        }
        let mut open_above = false;
        let mut open_below = false;
        while x < width && barrier[row + x] == 0 && visited[row + x] == 0 {
            let index = row + x;
            visited[index] = 1;
            selected[y * stride + x / 8] |= 0x80 >> (x % 8);
            if y > 0 {
                let above = index - width;
                let open = barrier[above] == 0 && visited[above] == 0;
                if open && !open_above {
                    stack.push(above);
                }
                open_above = open;
            }
            if y + 1 < height {
                let below = index + width;
                let open = barrier[below] == 0 && visited[below] == 0;
                if open && !open_below {
                    stack.push(below);
                }
                open_below = open;
            }
            x += 1;
        }
    }
}

fn for_each_neighbor8(
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    mut visit: impl FnMut(usize),
) {
    for offset_y in -1_i32..=1 {
        for offset_x in -1_i32..=1 {
            if offset_x == 0 && offset_y == 0 {
                continue;
            }
            let nx = x as i32 + offset_x;
            let ny = y as i32 + offset_y;
            if nx >= 0 && ny >= 0 && nx < width as i32 && ny < height as i32 {
                visit(ny as usize * width + nx as usize);
            }
        }
    }
}

fn neighbors_clockwise(image: &[u8], width: usize, x: usize, y: usize) -> [u8; 8] {
    [
        image[(y - 1) * width + x],
        image[(y - 1) * width + x + 1],
        image[y * width + x + 1],
        image[(y + 1) * width + x + 1],
        image[(y + 1) * width + x],
        image[(y + 1) * width + x - 1],
        image[y * width + x - 1],
        image[(y - 1) * width + x - 1],
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::{flood_into, neighbors_clockwise, thin};

    #[test]
    fn active_thinning_matches_full_image_passes() {
        let mut random = 0x93d7_65a4_u32;
        for size in 3..40 {
            for density in [2_u32, 3, 5, 8] {
                let mut actual = Vec::with_capacity(size * size);
                for _ in 0..size * size {
                    random = random.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    actual.push(u8::from(random % density != 0));
                }
                let mut expected = actual.clone();
                thin_reference(&mut expected, size, size);
                thin(&mut actual, size, size);
                assert_eq!(actual, expected, "size {size}, density 1/{density}");
            }
        }
    }

    #[test]
    fn scanline_flood_matches_pixel_flood() {
        const WIDTH: usize = 79;
        const HEIGHT: usize = 53;
        let stride = WIDTH.div_ceil(8);
        let mut random = 0x3a1f_95c7_u32;
        for _ in 0..40 {
            let mut barrier = Vec::with_capacity(WIDTH * HEIGHT);
            for _ in 0..WIDTH * HEIGHT {
                random = random.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                barrier.push(u8::from(random % 7 == 0));
            }
            let start = random as usize % barrier.len();
            barrier[start] = 0;
            let mut actual = vec![0; stride * HEIGHT];
            let mut visited = vec![0; barrier.len()];
            let mut stack = Vec::new();
            flood_into(
                &barrier,
                WIDTH,
                start,
                &mut actual,
                stride,
                &mut visited,
                &mut stack,
            );
            assert_eq!(actual, flood_reference(&barrier, WIDTH, HEIGHT, start));
        }
    }

    fn thin_reference(image: &mut [u8], width: usize, height: usize) {
        if width < 3 || height < 3 {
            return;
        }
        loop {
            let mut changed = false;
            for second_pass in [false, true] {
                let mut remove = Vec::new();
                for y in 1..height - 1 {
                    for x in 1..width - 1 {
                        let index = y * width + x;
                        if image[index] == 0 {
                            continue;
                        }
                        let p = neighbors_clockwise(image, width, x, y);
                        let count = p.iter().filter(|value| **value != 0).count();
                        let transitions = (0..8)
                            .filter(|i| p[*i] == 0 && p[(*i + 1) % 8] != 0)
                            .count();
                        let clear = if second_pass {
                            p[0] == 0 || p[6] == 0 || p[2] == 0 && p[4] == 0
                        } else {
                            p[2] == 0 || p[4] == 0 || p[0] == 0 && p[6] == 0
                        };
                        if (2..=6).contains(&count) && transitions == 1 && clear {
                            remove.push(index);
                        }
                    }
                }
                changed |= !remove.is_empty();
                for index in remove {
                    image[index] = 0;
                }
            }
            if !changed {
                break;
            }
        }
    }

    fn flood_reference(barrier: &[u8], width: usize, height: usize, start: usize) -> Vec<u8> {
        let stride = width.div_ceil(8);
        let mut selected = vec![0; stride * height];
        let mut visited = vec![false; barrier.len()];
        let mut queue = VecDeque::from([start]);
        visited[start] = true;
        while let Some(index) = queue.pop_front() {
            let x = index % width;
            let y = index / width;
            selected[y * stride + x / 8] |= 0x80 >> (x % 8);
            for neighbor in [
                x.checked_sub(1).map(|_| index - 1),
                (x + 1 < width).then_some(index + 1),
                y.checked_sub(1).map(|_| index - width),
                (y + 1 < height).then_some(index + width),
            ]
            .into_iter()
            .flatten()
            {
                if barrier[neighbor] == 0 && !visited[neighbor] {
                    visited[neighbor] = true;
                    queue.push_back(neighbor);
                }
            }
        }
        selected
    }
}
