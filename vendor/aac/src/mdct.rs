use std::{f64::consts::PI, sync::Arc};

use rustfft::{Fft, FftPlanner, num_complex::Complex32};

use crate::{FRAME_SAMPLES, Window};

const SHORT: usize = FRAME_SAMPLES / 8;
const OFFSET: usize = (FRAME_SAMPLES - SHORT) / 2;

pub(crate) struct Analysis {
    long: Transform,
    short: Transform,
    start: Vec<f32>,
    stop: Vec<f32>,
}

impl Analysis {
    pub fn new() -> Self {
        let mut planner = FftPlanner::new();
        let long = Transform::new(FRAME_SAMPLES, 4.0, &mut planner);
        let short = Transform::new(SHORT, 6.0, &mut planner);
        let mut start = long.window.clone();
        start[FRAME_SAMPLES..FRAME_SAMPLES + OFFSET].fill(1.0);
        start[FRAME_SAMPLES + OFFSET..FRAME_SAMPLES + OFFSET + SHORT]
            .copy_from_slice(&short.window[SHORT..]);
        start[FRAME_SAMPLES + OFFSET + SHORT..].fill(0.0);
        let stop = start.iter().rev().copied().collect();
        Self {
            long,
            short,
            start,
            stop,
        }
    }

    pub fn transform(
        &mut self,
        window: Window,
        samples: &[[f32; 2]],
        channel: usize,
        spectrum: &mut [f32; FRAME_SAMPLES],
    ) {
        match window {
            Window::Short => {
                for (index, lines) in spectrum.chunks_exact_mut(SHORT).enumerate() {
                    let start = OFFSET + index * SHORT;
                    self.short
                        .run(&samples[start..start + 2 * SHORT], channel, None, lines);
                }
            }
            Window::Long => self.long.run(samples, channel, None, spectrum),
            Window::Start => self.long.run(samples, channel, Some(&self.start), spectrum),
            Window::Stop => self.long.run(samples, channel, Some(&self.stop), spectrum),
        }
    }
}

struct Transform {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    input_rotation: Vec<Complex32>,
    output_rotation: Vec<Complex32>,
    spectrum: Vec<Complex32>,
    scratch: Vec<Complex32>,
}

impl Transform {
    fn new(size: usize, alpha: f64, planner: &mut FftPlanner<f32>) -> Self {
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
            window: kbd_window(size, alpha),
            input_rotation,
            output_rotation,
            spectrum: vec![Complex32::ZERO; size / 2],
            scratch,
        }
    }

    fn run(
        &mut self,
        samples: &[[f32; 2]],
        channel: usize,
        window: Option<&[f32]>,
        lines: &mut [f32],
    ) {
        let window = window.unwrap_or(&self.window);
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

pub(crate) fn transient(
    samples: &[[f32; 2]],
    smooth: &mut [f32; 2],
    previous: &mut [f32; 2],
) -> [bool; 2] {
    let mut attack = [false; 2];
    for (index, block) in samples.chunks_exact(64).enumerate() {
        let mut energy = [0.0; 2];
        for sample in block {
            for ((energy, previous), current) in energy.iter_mut().zip(&mut *previous).zip(sample) {
                let difference = current - *previous;
                *energy += difference * difference;
                *previous = *current;
            }
        }
        for (long, current) in smooth.iter_mut().zip(energy) {
            attack[usize::from(index * 64 >= OFFSET)] |= current > 12.0 * *long + 0.0003;
            *long = *long * 0.8 + current * 0.2;
        }
    }
    attack
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folded_fft_matches_the_direct_mdct() {
        for size in [SHORT, FRAME_SAMPLES] {
            let mut transform = Transform::new(size, 4.0, &mut FftPlanner::new());
            let pcm: Vec<_> = (0..2 * size)
                .map(|n| {
                    [
                        ((n as f64 * 0.717).sin() * 0.25 + (n as f64 * 0.188).cos() * 0.52) as f32,
                        0.0,
                    ]
                })
                .collect();
            let mut output = vec![0.0; size];
            transform.run(&pcm, 0, None, &mut output);
            for (k, output) in output.into_iter().enumerate() {
                let direct = pcm
                    .iter()
                    .zip(&transform.window)
                    .enumerate()
                    .map(|(n, (sample, window))| {
                        let phase = PI / size as f64
                            * (n as f64 + 0.5 + size as f64 / 2.0)
                            * (k as f64 + 0.5);
                        f64::from(sample[0] * window) * phase.cos() * 65536.0
                    })
                    .sum::<f64>();
                assert!(
                    (f64::from(output) - direct).abs() < 0.65 + direct.abs() * 2.0e-7,
                    "{size} {k} {output} {direct}"
                );
            }
        }
    }
}
