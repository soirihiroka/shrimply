//! Stateless, reproducible scalar noise. No mutable PRNG state or permutation allocation.
use super::{Binary, Instruction, Noise, NoiseKind, Unary};

const MIX_A: u64 = 0xbf58_476d_1ce4_e5b9;
const MIX_B: u64 = 0x94d0_49bb_1331_11eb;
const GOLDEN_STEP: u64 = 0x9e37_79b9_7f4a_7c15;
const LATTICE_LIMIT: f64 = (1u64 << 40) as f64;

pub fn random(seed: u64, index: u64) -> f64 {
    unit(mix(seed ^ mix(index.wrapping_add(GOLDEN_STEP))))
}
pub(super) fn scatter(seed: u64, point: [u32; 2], density: f64) -> bool {
    density > 0.0 && (density >= 1.0 || random(seed, column_index(point)) < density)
}

fn column_index(point: [u32; 2]) -> u64 {
    (u64::from(point[0]) << u32::BITS) | u64::from(point[1])
}

pub(super) fn surface_point(
    seed: u64,
    origin: [u32; 2],
    size: [u32; 2],
    step: u32,
    jitter: f64,
) -> [u32; 2] {
    if jitter == 0.0 {
        return origin;
    }
    let index = column_index(origin);
    std::array::from_fn(|axis| {
        let extent = step.min(size[axis] - origin[axis]);
        let stream = [MIX_A, MIX_B][axis];
        origin[axis]
            + (random(seed.wrapping_add(stream), index) * f64::from(extent) * jitter).floor() as u32
    })
}

pub(super) fn surface_slope(heights: &[u16], size: [u32; 2], point: [u32; 2]) -> f64 {
    let center = heights[(point[0] + size[0] * point[1]) as usize];
    let slope: [f64; 2] = std::array::from_fn(|axis| {
        let mut rise = 0;
        for neighbor in [
            point[axis].checked_sub(1),
            point[axis]
                .checked_add(1)
                .filter(|value| *value < size[axis]),
        ]
        .into_iter()
        .flatten()
        {
            let mut at = point;
            at[axis] = neighbor;
            rise = rise.max(center.abs_diff(heights[(at[0] + size[0] * at[1]) as usize]));
        }
        f64::from(rise)
    });
    slope[0].hypot(slope[1])
}

fn mix(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(MIX_A);
    value = (value ^ (value >> 27)).wrapping_mul(MIX_B);
    value ^ (value >> 31)
}
fn unit(value: u64) -> f64 {
    (value >> 11) as f64 / (1u64 << 53) as f64
}
fn hash(seed: u64, cell: [i64; 3]) -> u64 {
    cell.into_iter().fold(mix(seed), |state, coordinate| {
        mix(state ^ mix(coordinate as u64))
    })
}
fn fade(value: f64) -> f64 {
    value * value * value * (value * (value * 6.0 - 15.0) + 10.0)
}
fn lerp(a: f64, b: f64, amount: f64) -> f64 {
    a + (b - a) * amount
}

fn gradient(hash: u64, p: [f64; 3]) -> f64 {
    // The twelve edge directions avoid an allocated or seed-shuffled lookup table.
    let h = hash & 15;
    let u = if h < 8 { p[0] } else { p[1] };
    let v = if h < 4 {
        p[1]
    } else if h == 12 || h == 14 {
        p[0]
    } else {
        p[2]
    };
    (if h & 1 == 0 { u } else { -u }) + if h & 2 == 0 { v } else { -v }
}

fn planar_gradient(hash: u64, p: [f64; 3]) -> f64 {
    use std::f64::consts::FRAC_1_SQRT_2;
    match hash & 7 {
        0 => p[0],
        1 => -p[0],
        2 => p[1],
        3 => -p[1],
        4 => (p[0] + p[1]) * FRAC_1_SQRT_2,
        5 => (p[0] - p[1]) * FRAC_1_SQRT_2,
        6 => (-p[0] + p[1]) * FRAC_1_SQRT_2,
        _ => (-p[0] - p[1]) * FRAC_1_SQRT_2,
    }
}

fn base(kind: NoiseKind, seed: u64, point: [f64; 3], dimensions: usize) -> f64 {
    let cell = point.map(|value| value.floor() as i64);
    let local = std::array::from_fn::<_, 3, _>(|axis| point[axis] - cell[axis] as f64);
    if kind == NoiseKind::Cellular {
        let mut nearest = f64::INFINITY;
        let z_radius = i64::from(dimensions == 3);
        for z in -z_radius..=z_radius {
            for y in -1..=1 {
                for x in -1..=1 {
                    let at = [cell[0] + x, cell[1] + y, cell[2] + z];
                    let key = hash(seed, at);
                    let mut distance = 0.0;
                    for axis in 0..dimensions {
                        let delta = at[axis] as f64
                            + unit(mix(
                                key.wrapping_add((axis as u64).wrapping_mul(GOLDEN_STEP))
                            ))
                            - point[axis];
                        distance += delta * delta;
                    }
                    nearest = nearest.min(distance);
                }
            }
        }
        return 1.0 - 2.0 * nearest.sqrt().min(1.0);
    }
    let mut values = [0.0; 8];
    for (corner, value) in values.iter_mut().enumerate().take(1 << dimensions) {
        let at = std::array::from_fn(|axis| cell[axis] + i64::from(corner & (1 << axis) != 0));
        let key = hash(seed, at);
        *value = if kind == NoiseKind::Value {
            unit(key) * 2.0 - 1.0
        } else {
            let offset = std::array::from_fn(|axis| {
                local[axis] - if corner & (1 << axis) != 0 { 1.0 } else { 0.0 }
            });
            if dimensions == 3 {
                gradient(key, offset)
            } else {
                planar_gradient(key, offset)
            }
        };
    }
    let weight = local.map(fade);
    let a = lerp(
        lerp(values[0], values[1], weight[0]),
        lerp(values[2], values[3], weight[0]),
        weight[1],
    );
    let b = lerp(
        lerp(values[4], values[5], weight[0]),
        lerp(values[6], values[7], weight[0]),
        weight[1],
    );
    lerp(a, b, weight[2]).clamp(-1.0, 1.0)
}

pub(super) fn sample(options: &Noise, input: [f64; 3]) -> Result<f64, String> {
    let mut point = [0.0; 3];
    let mut dimensions = 0;
    for (axis, included) in options.axes.mask().into_iter().enumerate() {
        if included {
            point[dimensions] = input[axis] / options.scale;
            dimensions += 1;
        }
    }
    let mut sum = 0.0;
    let mut weight = 1.0;
    let mut total = 0.0;
    for octave in 0..options.octave_count() {
        if point
            .into_iter()
            .any(|value| !value.is_finite() || value.abs() > LATTICE_LIMIT)
        {
            return Err("noise coordinates exceed the finite lattice range; increase scale or reduce domain transforms".into());
        }
        let value = base(
            options.kind,
            mix(options.seed.wrapping_add(u64::from(octave))),
            point,
            dimensions,
        );
        sum += weight
            * if options.kind == NoiseKind::Ridged {
                1.0 - 2.0 * value.abs()
            } else {
                value
            };
        total += weight;
        weight *= options.persistence;
        // Rotating each octave avoids repeatedly aligning terrain detail with voxel axes.
        point = if dimensions == 3 {
            [
                point[1] * 0.8 - point[2] * 0.6,
                point[2] * 0.8 + point[1] * 0.6,
                point[0],
            ]
        } else {
            [
                point[0] * 0.8 - point[1] * 0.6,
                point[0] * 0.6 + point[1] * 0.8,
                0.0,
            ]
        }
        .map(|value| value * options.lacunarity);
    }
    Ok((sum / total).clamp(-1.0, 1.0))
}

pub(super) fn evaluate(
    instruction: &Instruction,
    values: &[f64],
    point: [f64; 3],
) -> Result<f64, String> {
    Ok(match instruction {
        Instruction::Axis(axis) => point[*axis as usize],
        Instruction::Constant(value) => *value,
        Instruction::Noise {
            options,
            coordinates,
        } => sample(options, coordinates.map(|at| values[at]))?,
        Instruction::Distance {
            coordinates,
            center,
            axes,
        } => (0..3)
            .filter(|axis| axes[*axis])
            .map(|axis| (values[coordinates[axis]] - center[axis]).powi(2))
            .sum::<f64>()
            .sqrt(),
        Instruction::Unary(operation, value) => match operation {
            Unary::Abs => values[*value].abs(),
            Unary::Negate => -values[*value],
            Unary::Floor => values[*value].floor(),
            Unary::Fract => {
                let fraction = values[*value] - values[*value].floor();
                if fraction >= 1.0 { 0.0 } else { fraction }
            }
            Unary::Sin => values[*value].sin(),
            Unary::Cos => values[*value].cos(),
            Unary::Smoothstep => {
                let value = values[*value].clamp(0.0, 1.0);
                value * value * (3.0 - 2.0 * value)
            }
        },
        Instruction::Binary(operation, a, b) => {
            let (a, b) = (values[*a], values[*b]);
            match operation {
                Binary::Add => a + b,
                Binary::Subtract => a - b,
                Binary::Multiply => a * b,
                Binary::Divide => a / b,
                Binary::Min => a.min(b),
                Binary::Max => a.max(b),
                Binary::Power => a.powf(b),
                Binary::Modulo => {
                    if b == 0.0 {
                        return Err("density modulo divisor must be nonzero".into());
                    }
                    let remainder = a.rem_euclid(b);
                    if remainder >= b.abs() { 0.0 } else { remainder }
                }
            }
        }
        Instruction::Clamp { value, min, max } => values[*value].clamp(*min, *max),
    })
}
