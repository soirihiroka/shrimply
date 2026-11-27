// SPDX-License-Identifier: GPL-2.0-or-later
// Port of Rubber Band 4.0.0 BQResampler, copyright 2007-2024 Particular Programs Ltd.

use std::f64::consts::PI;
use std::sync::OnceLock;

const FILTER_WIDTH: usize = 62;
const CUTOFF: f64 = 0.975;
const ATTENUATION: f64 = 90.0;
const TRANSITION: f64 = 0.05;
const PROTOTYPE_SPACING: usize = 160;
// The lowest supported ratio is 1/4. Include the centre sample and rounding.
const MAX_TAPS: usize = ((FILTER_WIDTH as f64 * 4.0 / CUTOFF + 1.0) as usize) | 1;

pub(crate) struct Resampler<const N: usize> {
    ratio: f64,
    numerator: usize,
    denominator: usize,
    phase: usize,
    filter_length: usize,
    spacing: f64,
    gain: f64,
    kernel: &'static [f64],
    unity: &'static [f64],
    left: usize,
    length: usize,
    fill: usize,
    buffer: [[f32; N]; MAX_TAPS],
}

impl<const N: usize> Resampler<N> {
    pub(crate) fn new() -> Self {
        Self {
            ratio: 0.0,
            numerator: 1,
            denominator: 1,
            phase: 0,
            filter_length: 0,
            spacing: 0.0,
            gain: 0.0,
            kernel: prototype(),
            unity: unity_weights(),
            left: 0,
            length: 0,
            fill: 0,
            buffer: [[0.0; N]; MAX_TAPS],
        }
    }

    pub(crate) fn delay(ratio: f64) -> usize {
        let (numerator, denominator, length, _) = filter(ratio);
        (length / 2 / numerator * numerator).div_ceil(denominator)
    }

    pub(crate) fn process(
        &mut self,
        ratio: f64,
        input: &[[f32; N]],
        output: &mut [[f32; N]],
    ) -> (usize, usize) {
        if ratio != self.ratio {
            self.configure(ratio);
        }

        let mut consumed = 0;
        let mut written = 0;
        let mut weights = [0.0; MAX_TAPS];
        let unity = self.numerator == 1 && self.denominator == 1;
        for frame in output {
            let fill_count = (self.length - self.fill).min(input.len() - consumed);
            self.buffer[self.fill..self.fill + fill_count]
                .copy_from_slice(&input[consumed..consumed + fill_count]);
            self.fill += fill_count;
            consumed += fill_count;
            if self.fill < self.length {
                break;
            }

            let taps = (self.filter_length - self.phase)
                .div_ceil(self.numerator)
                .min(self.length - self.left);
            let weights = if unity {
                &self.unity[..taps]
            } else {
                let mut index = self.phase;
                for weight in &mut weights[..taps] {
                    *weight = interpolate(self.kernel, self.spacing * index as f64);
                    index += self.numerator;
                }
                &weights[..taps]
            };
            let mut sum = [0.0; N];
            for (samples, &weight) in self.buffer[self.left..self.left + taps].iter().zip(weights) {
                for (sum, &sample) in sum.iter_mut().zip(samples) {
                    *sum += weight * f64::from(sample);
                }
            }
            *frame = sum.map(|sample| (sample * self.gain) as f32);
            written += 1;

            let drop = self
                .denominator
                .saturating_sub(self.phase)
                .div_ceil(self.numerator);
            self.phase = self.phase + drop * self.numerator - self.denominator;
            self.buffer.copy_within(drop..self.length, 0);
            self.fill -= drop;
        }
        (consumed, written)
    }

    fn configure(&mut self, ratio: f64) {
        let (numerator, denominator, length, peak) = filter(ratio);
        let half = length / 2;
        let left = half / numerator;
        let old_centre = self.length / 2;
        let new_length = self.length.max(2 * left + 1);
        let centre = new_length / 2;

        if self.length == 0 {
            self.fill = centre;
            self.phase = half % numerator;
        } else {
            let offset = centre - old_centre;
            if offset > 0 {
                self.buffer.copy_within(..self.fill, offset);
                self.buffer[..offset].fill([0.0; N]);
                self.fill += offset;
            }
            // Convert the old phase to the new grid, with nearest rounding.
            self.phase = ((numerator as f64 * self.phase as f64 / self.numerator as f64).round()
                as usize)
                .min(numerator - 1);
        }
        self.length = new_length;
        self.left = centre - left;
        self.ratio = ratio;
        self.numerator = numerator;
        self.denominator = denominator;
        self.filter_length = length;
        self.spacing = (self.kernel.len() - 2) as f64 / (length - 1) as f64;
        self.gain = numerator as f64 / peak;
    }
}

fn filter(ratio: f64) -> (usize, usize, usize, f64) {
    let max_denominator = (96_000.0 / ratio.max(1.0)).ceil() as usize;
    let (numerator, denominator) = nearest_fraction(ratio, max_denominator);
    let peak = numerator.max(denominator) as f64 / CUTOFF;
    let length = ((peak * FILTER_WIDTH as f64 + 1.0) as usize) | 1;
    (numerator, denominator, length, peak)
}

// Continued fractions reach the same Farey neighbours without scanning
// thousands of fractions when an automated pitch is close to unity.
fn nearest_fraction(ratio: f64, max_denominator: usize) -> (usize, usize) {
    let (mut previous, mut current) = ((0, 1), (1, 0));
    let mut remainder = ratio;
    loop {
        let term = remainder.floor();
        if current.1 > 0 && term > ((max_denominator - previous.1) / current.1) as f64 {
            let count = (max_denominator - previous.1) / current.1;
            let boundary = (
                previous.0 + count * current.0,
                previous.1 + count * current.1,
            );
            let error = |(n, d): (usize, usize)| (n as f64 / d as f64 - ratio).abs();
            return if error(boundary) < error(current) {
                boundary
            } else {
                current
            };
        }
        let term = term as usize;
        let next = (previous.0 + term * current.0, previous.1 + term * current.1);
        if (next.0 as f64 / next.1 as f64 - ratio).abs() < 1.0e-9 {
            return next;
        }
        (previous, current) = (current, next);
        remainder = (remainder - term as f64).recip();
    }
}

fn prototype() -> &'static [f64] {
    static PROTOTYPE: OnceLock<Vec<f64>> = OnceLock::new();
    PROTOTYPE.get_or_init(|| {
        let length = PROTOTYPE_SPACING * FILTER_WIDTH + 1;
        let kaiser_length = (1 + ((ATTENUATION - 7.95) / (2.285 * TRANSITION)).ceil() as usize) | 1;
        let beta = 0.1102 * (ATTENUATION - 8.7);
        let denominator = bessel(beta);
        let mut window: Vec<_> = (0..kaiser_length)
            .map(|i| {
                let x = 2.0 * i as f64 / (kaiser_length - 1) as f64 - 1.0;
                bessel(beta * (1.0 - x * x).sqrt()) / denominator
            })
            .collect();
        window.push(0.0);
        let interpolation = (kaiser_length - 1) as f64 / (length - 1) as f64;
        let mut kernel: Vec<_> = (0..length)
            .map(|i| {
                let ix = i as f64 * interpolation;
                let base = ix as usize;
                let fraction = ix - base as f64;
                let position = PI * (i as f64 - (length / 2) as f64) / PROTOTYPE_SPACING as f64;
                let sinc = if i == length / 2 {
                    1.0
                } else {
                    position.sin() / position
                };
                (window[base] * (1.0 - fraction) + window[base + 1] * fraction) * sinc
            })
            .collect();
        kernel.push(0.0);
        kernel
    })
}

fn bessel(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    for n in 1..20 {
        term *= (x / (2.0 * f64::from(n))).powi(2);
        sum += term;
    }
    sum
}

fn unity_weights() -> &'static [f64] {
    static UNITY: OnceLock<Vec<f64>> = OnceLock::new();
    UNITY.get_or_init(|| {
        let kernel = prototype();
        let (_, _, length, _) = filter(1.0);
        let spacing = (kernel.len() - 2) as f64 / (length - 1) as f64;
        (0..length)
            .map(|i| interpolate(kernel, spacing * i as f64))
            .collect()
    })
}

fn interpolate(kernel: &[f64], index: f64) -> f64 {
    let base = index as usize;
    let fraction = index - base as f64;
    kernel[base] * (1.0 - fraction) + kernel[base + 1] * fraction
}
