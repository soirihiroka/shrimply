//! Deterministic scalar noise for frame-sampled motion.
use crate::Fraction;

pub fn next_seed(seed: u32) -> u32 {
    seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223)
}

pub fn random_sample(seed: u32) -> Fraction {
    Fraction::new(u64::from(seed), 1_u64 << 31) - Fraction::from(1)
}

pub fn shake_sample(item_seed: u32, phase: f64, stream: u32) -> f64 {
    let index = phase.floor();
    let progress = phase - index;
    let progress = progress * progress * (3.0 - 2.0 * progress);
    let index = index as i64 as u32;
    let sample = |index: u32| {
        let value = item_seed
            .wrapping_add(index.wrapping_mul(374_761_393))
            .wrapping_add(stream.wrapping_mul(668_265_263));
        let value = (value ^ (value >> 13)).wrapping_mul(1_274_126_177);
        f64::from(value ^ (value >> 16)) / f64::from(1_u32 << 31) - 1.0
    };
    let left = sample(index);
    left + (sample(index.wrapping_add(1)) - left) * progress
}
