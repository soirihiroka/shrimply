use std::{f64::consts::PI, sync::Arc};

use rustfft::{Fft, FftPlanner, num_complex::Complex32};

use crate::{FRAME_SAMPLES, Window};

pub(crate) const SHORT: usize = FRAME_SAMPLES / 8;
pub(crate) const OFFSET: usize = (FRAME_SAMPLES - SHORT) / 2;

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowShape {
    Sine = 0,
    Kbd = 1,
}

pub(crate) struct Analysis {
    long: Transform,
    short: Transform,
    long_win: [[Vec<f32>; 2]; 2],
    start_win: [[Vec<f32>; 2]; 2],
    stop_win: [[Vec<f32>; 2]; 2],
    short_win: [[Vec<f32>; 2]; 2],
}

impl Analysis {
    pub fn new() -> Self {
        let mut planner = FftPlanner::new();
        let long = Transform::new(FRAME_SAMPLES, &mut planner);
        let short = Transform::new(SHORT, &mut planner);
        let long_base = [sine_window(FRAME_SAMPLES), kbd_window(FRAME_SAMPLES, 4.0)];
        let short_base = [sine_window(SHORT), kbd_window(SHORT, 6.0)];

        let long_win = stitch_windows(&long_base, FRAME_SAMPLES);
        let start_win = std::array::from_fn(|prev| {
            std::array::from_fn(|curr| {
                let mut w = vec![0.0; 2 * FRAME_SAMPLES];
                w[..FRAME_SAMPLES].copy_from_slice(&long_base[prev][..FRAME_SAMPLES]);
                w[FRAME_SAMPLES..FRAME_SAMPLES + OFFSET].fill(1.0);
                w[FRAME_SAMPLES + OFFSET..FRAME_SAMPLES + OFFSET + SHORT]
                    .copy_from_slice(&short_base[curr][SHORT..]);
                w
            })
        });
        let stop_win = std::array::from_fn(|prev| {
            std::array::from_fn(|curr| {
                let mut w = vec![0.0; 2 * FRAME_SAMPLES];
                w[OFFSET..OFFSET + SHORT].copy_from_slice(&short_base[prev][..SHORT]);
                w[OFFSET + SHORT..FRAME_SAMPLES].fill(1.0);
                w[FRAME_SAMPLES..].copy_from_slice(&long_base[curr][FRAME_SAMPLES..]);
                w
            })
        });
        let short_win = stitch_windows(&short_base, SHORT);

        Self {
            long,
            short,
            long_win,
            start_win,
            stop_win,
            short_win,
        }
    }

    pub fn transform(
        &mut self,
        window: Window,
        prev_shape: WindowShape,
        curr_shape: WindowShape,
        samples: &[[f32; 2]],
        channel: usize,
        spectrum: &mut [f32; FRAME_SAMPLES],
    ) {
        let p = prev_shape as usize;
        let c = curr_shape as usize;
        match window {
            Window::Short => {
                for (index, lines) in spectrum.as_chunks_mut::<SHORT>().0.iter_mut().enumerate() {
                    let start = OFFSET + index * SHORT;
                    let win = if index == 0 {
                        &self.short_win[p][c]
                    } else {
                        &self.short_win[c][c]
                    };
                    self.short
                        .run(&samples[start..start + 2 * SHORT], channel, win, lines);
                }
            }
            Window::Long => self
                .long
                .run(samples, channel, &self.long_win[p][c], spectrum),
            Window::Start => self
                .long
                .run(samples, channel, &self.start_win[p][c], spectrum),
            Window::Stop => self
                .long
                .run(samples, channel, &self.stop_win[p][c], spectrum),
        }
    }
}

struct Transform {
    fft: Arc<dyn Fft<f32>>,
    input_rotation: Vec<Complex32>,
    output_rotation: Vec<Complex32>,
    spectrum: Vec<Complex32>,
    scratch: Vec<Complex32>,
}

impl Transform {
    fn new(size: usize, planner: &mut FftPlanner<f32>) -> Self {
        let fft = planner.plan_fft_forward(size / 2);
        let scratch = vec![Complex32::ZERO; fft.get_inplace_scratch_len()];
        let length = size as f64;
        let rotation = |phase: f64, gain: f64| {
            let (sin, cos) = phase.sin_cos();
            Complex32::new((gain * cos) as f32, (gain * sin) as f32)
        };
        // Pack the even and reversed odd DCT-IV samples into a half-size FFT.
        let input_rotation = (0..size / 2)
            .map(|n| rotation(-PI * n as f64 / length, 1.0))
            .collect();
        let output_rotation = (0..size / 2)
            // AAC's factor of two and conversion from normalized PCM.
            .map(|k| rotation(-PI * (k as f64 + 0.25) / length, 65536.0))
            .collect();
        Self {
            fft,
            input_rotation,
            output_rotation,
            spectrum: vec![Complex32::ZERO; size / 2],
            scratch,
        }
    }

    fn run(&mut self, samples: &[[f32; 2]], channel: usize, window: &[f32], lines: &mut [f32]) {
        let size = lines.len();
        let sample = |n: usize| samples[n][channel] * window[n];
        // Fold the second and third quarters with the MDCT's cosine symmetry.
        let fold = |n: usize| {
            let reversed = sample(3 * size / 2 - 1 - n);
            if n < size / 2 {
                -reversed - sample(3 * size / 2 + n)
            } else {
                sample(n - size / 2) - reversed
            }
        };
        for (n, (bin, rotation)) in self
            .spectrum
            .iter_mut()
            .zip(&self.input_rotation)
            .enumerate()
        {
            *bin = Complex32::new(fold(2 * n), fold(size - 1 - 2 * n)) * rotation;
        }
        self.fft
            .process_with_scratch(&mut self.spectrum, &mut self.scratch);
        for (k, (bin, rotation)) in self.spectrum.iter().zip(&self.output_rotation).enumerate() {
            let value = bin * rotation;
            lines[2 * k] = value.re;
            lines[size - 1 - 2 * k] = -value.im;
        }
    }
}

fn stitch_windows(base: &[Vec<f32>; 2], size: usize) -> [[Vec<f32>; 2]; 2] {
    std::array::from_fn(|prev| {
        std::array::from_fn(|curr| {
            let mut w = Vec::with_capacity(2 * size);
            w.extend_from_slice(&base[prev][..size]);
            w.extend_from_slice(&base[curr][size..]);
            w
        })
    })
}

fn sine_window(size: usize) -> Vec<f32> {
    let denom = 2.0 * size as f64;
    (0..2 * size)
        .map(|n| (PI * (n as f64 + 0.5) / denom).sin() as f32)
        .collect()
}

// ISO/IEC 13818-7 §15.3.2. The cumulative Kaiser weights reduce leakage from
// strong tonal components while complementary halves preserve time-domain alias cancellation.
fn kbd_window(size: usize, alpha: f64) -> Vec<f32> {
    let weights: Vec<_> = (0..=size)
        .map(|index| {
            let coordinate = 2.0 * index as f64 / size as f64 - 1.0;
            let argument = (PI * alpha).powi(2) * (1.0 - coordinate.powi(2)) / 4.0;
            let mut term = 1.0;
            let mut sum = 1.0;
            for order in 1..=40 {
                term *= argument / f64::from(order * order);
                sum += term;
            }
            sum
        })
        .collect();
    let total = weights.iter().sum::<f64>();
    let mut window: Vec<_> = weights[..size]
        .iter()
        .scan(0.0, |sum, weight| {
            *sum += weight;
            Some((*sum / total).sqrt() as f32)
        })
        .collect();
    window.extend_from_within(..);
    window[size..].reverse();
    window
}

#[derive(Default)]
pub(crate) struct TransientDetector {
    smooth_hp: [f32; 2],
    smooth_full: [f32; 2],
    prev_sample: [f32; 2],
    prev_hp: [f32; 2],
    prev_full: [f32; 2],
}

impl TransientDetector {
    pub fn detect(&mut self, samples: &[[f32; 2]]) -> [bool; 2] {
        let mut attack = [false; 2];
        for (index, block) in samples.as_chunks::<64>().0.iter().enumerate() {
            let mut hp = [0.0_f32; 2];
            let mut full = [0.0_f32; 2];
            for sample in block {
                for c in 0..2 {
                    let current = sample[c];
                    let diff = current - self.prev_sample[c];
                    hp[c] += diff * diff;
                    full[c] += current * current;
                    self.prev_sample[c] = current;
                }
            }
            let half = usize::from(index * 64 >= FRAME_SAMPLES / 2);
            for c in 0..2 {
                let hp_jump = (hp[c] > 10.0 * self.smooth_hp[c] + 0.00025
                    && hp[c] > 3.5 * self.prev_hp[c])
                    || hp[c] > 14.0 * self.smooth_hp[c] + 0.0004;
                let full_jump = full[c] > 14.0 * self.smooth_full[c] + 0.0015
                    && full[c] > 4.0 * self.prev_full[c];
                attack[half] |= hp_jump || full_jump;
                self.smooth_hp[c] = self.smooth_hp[c] * 0.8 + hp[c] * 0.2;
                self.smooth_full[c] = self.smooth_full[c] * 0.85 + full[c] * 0.15;
                self.prev_hp[c] = hp[c];
                self.prev_full[c] = full[c];
            }
        }
        attack
    }
}

/// Partition the 8 short windows into groups of similar energy and spectral tilt.
/// Splits at attack/decay transitions to eliminate pre-echo bleed while merging
/// quiet or stationary windows to save scalefactor and section header bits.
pub(crate) fn short_grouping(spectra: &[[f32; FRAME_SAMPLES]; 2]) -> (u8, usize, [usize; 8]) {
    let floor = 1.0e7_f32;
    let mut energies = [[floor; 8]; 2];
    let mut low = [[0.0_f32; 8]; 2];
    let mut high = [[0.0_f32; 8]; 2];
    for c in 0..2 {
        for (w, win) in spectra[c].chunks_exact(SHORT).enumerate() {
            let mut e_low = 0.0;
            let mut e_mid = 0.0;
            let mut e_high = 0.0;
            for (k, &v) in win.iter().enumerate() {
                let p = v * v;
                if k < 16 {
                    e_low += p;
                } else if k < 56 {
                    e_mid += p;
                } else {
                    e_high += p;
                }
            }
            low[c][w] = e_low;
            high[c][w] = e_high;
            energies[c][w] = (e_low + e_mid + e_high).max(floor);
        }
    }

    let mut grouping_bits = 0_u8;
    let mut group_lens = [0_usize; 8];
    let mut groups = 1_usize;
    group_lens[0] = 1;
    let mut group_min = [energies[0][0], energies[1][0]];
    let mut group_max = group_min;

    for w in 1..8 {
        let mut split = false;
        for c in 0..2 {
            let cur = energies[c][w];
            let new_min = group_min[c].min(cur);
            let new_max = group_max[c].max(cur);
            if new_max > 2.8 * new_min {
                split = true;
            }
            // Split if high-frequency share shifts dramatically on an energetic window.
            let prev = energies[c][w - 1];
            if cur > 10.0 * floor && prev > 10.0 * floor {
                let prev_hi = high[c][w - 1] / prev;
                let cur_hi = high[c][w] / cur;
                let prev_lo = low[c][w - 1] / prev;
                let cur_lo = low[c][w] / cur;
                if (cur_hi - prev_hi).abs() > 0.35 || (cur_lo - prev_lo).abs() > 0.35 {
                    split = true;
                }
            }
        }
        // Keep group lengths <= 4 so short-window dynamics stay responsive.
        if group_lens[groups - 1] >= 4 {
            split = true;
        }
        if split {
            group_lens[groups] = 1;
            groups += 1;
            group_min = [energies[0][w], energies[1][w]];
            group_max = group_min;
        } else {
            grouping_bits |= 1 << (7 - w);
            group_lens[groups - 1] += 1;
            for c in 0..2 {
                let cur = energies[c][w];
                group_min[c] = group_min[c].min(cur);
                group_max[c] = group_max[c].max(cur);
            }
        }
    }
    (grouping_bits, groups, group_lens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folded_fft_matches_the_direct_mdct() {
        for size in [SHORT, FRAME_SAMPLES] {
            let mut transform = Transform::new(size, &mut FftPlanner::new());
            let window = kbd_window(size, 4.0);
            let pcm: Vec<_> = (0..2 * size)
                .map(|n| {
                    [
                        ((n as f64 * 0.717).sin() * 0.25 + (n as f64 * 0.188).cos() * 0.52) as f32,
                        0.0,
                    ]
                })
                .collect();
            let mut output = vec![0.0; size];
            transform.run(&pcm, 0, &window, &mut output);
            let mut error = 0.0;
            let mut power = 0.0;
            for (k, output) in output.into_iter().enumerate() {
                let direct = pcm
                    .iter()
                    .zip(&window)
                    .enumerate()
                    .map(|(n, (sample, window))| {
                        let phase = PI / size as f64
                            * (n as f64 + 0.5 + size as f64 / 2.0)
                            * (k as f64 + 0.5);
                        f64::from(sample[0] * window) * phase.cos() * 65536.0
                    })
                    .sum::<f64>();
                error += (f64::from(output) - direct).powi(2);
                power += direct.powi(2);
            }
            assert!(
                error / power < 1.0e-12,
                "{size} bins, relative error {}",
                error / power
            );
        }
    }
}
