// SPDX-License-Identifier: GPL-2.0-or-later
// Port of Rubber Band 4.0.0 R3LiveShifter, Window, Peak, and GuidedPhaseAdvance,
// copyright 2007-2024 Particular Programs Ltd.

use std::f64::consts::{PI, TAU};
use std::sync::Arc;

use realfft::{ComplexToReal, RealFftPlanner, RealToComplex, num_complex::Complex64};

use crate::guide::{Classifier, Guidance};

pub(crate) struct Transform {
    forward: Arc<dyn RealToComplex<f64>>,
    inverse: Arc<dyn ComplexToReal<f64>>,
    window: Vec<f64>,
    window_power: f64,
    time: Vec<f64>,
    spectrum: Vec<Complex64>,
    scratch: Vec<Complex64>,
}

impl Transform {
    pub(crate) fn new(size: usize) -> Self {
        let mut planner = RealFftPlanner::new();
        let forward = planner.plan_fft_forward(size);
        let inverse = planner.plan_fft_inverse(size);
        let window: Vec<_> = (0..size)
            .map(|i| 0.5 - 0.5 * (TAU * i as f64 / size as f64).cos())
            .collect();
        Self {
            window_power: window.iter().map(|&x| x * x).sum(),
            window,
            time: forward.make_input_vec(),
            spectrum: forward.make_output_vec(),
            scratch: vec![
                Complex64::ZERO;
                forward.get_scratch_len().max(inverse.get_scratch_len())
            ],
            forward,
            inverse,
        }
    }

    pub(crate) fn analyse<const N: usize>(
        &mut self,
        frames: &[[f32; N]],
        channel: usize,
        dest: &mut Polar,
        gain: f64,
    ) {
        for ((out, &weight), input) in self.time.iter_mut().zip(&self.window).zip(frames) {
            *out = f64::from(input[channel]) * weight;
        }
        let half = self.time.len() / 2;
        self.time.rotate_left(half);
        self.forward();
        for ((mag, phase), value) in dest.mag.iter_mut().zip(&mut dest.phase).zip(&self.spectrum) {
            *mag = value.norm_sqr().sqrt() * gain;
            *phase = value.arg();
        }
    }

    pub(crate) fn formants(&mut self, channel: &mut Channel, rate: f64, factor: f64) {
        for (frequency, &mag) in self.spectrum.iter_mut().zip(&channel.current.mag) {
            *frequency = Complex64::new((mag + 1.0e-6).ln(), 0.0);
        }
        self.inverse();
        let cutoff = ((rate / 650.0) as usize).max(1);
        self.time[0] *= 0.5;
        self.time[cutoff - 1] *= 0.5;
        let scale = 1.0 / self.time.len() as f64;
        for value in &mut self.time[..cutoff] {
            *value *= scale;
        }
        self.time[cutoff..].fill(0.0);
        self.forward();

        for (gain, value) in channel.envelope.iter_mut().zip(&self.spectrum) {
            *gain = (2.0 * value.re).exp().min(1.0e10);
        }
        let high = ((self.time.len() as f64 * 10_000.0 / rate) as usize)
            .min(channel.current.mag.len() - 1);
        let inverse_factor = factor.recip();
        for (i, mag) in channel.current.mag[..high].iter_mut().enumerate() {
            let bin = i as f64 * inverse_factor;
            let base = bin as usize;
            let fraction = bin - base as f64;
            let source = channel.envelope.get(base).map_or(0.0, |&left| {
                channel
                    .envelope
                    .get(base + 1)
                    .map_or(left, |&right| left * (1.0 - fraction) + right * fraction)
            });
            let target = channel.envelope[i];
            if target > 0.0 {
                *mag *= (source / target).clamp(1.0 / 60.0, 60.0);
            }
        }
    }

    pub(crate) fn synthesise(&mut self, channel: &mut Channel, hop: usize) {
        channel.prev_mag.copy_from_slice(&channel.current.mag);
        let gain = hop as f64 / self.window_power;
        let end = self.spectrum.len() - 2;
        for ((out, &mag), &phase) in self.spectrum[..end]
            .iter_mut()
            .zip(&channel.current.mag)
            .zip(&channel.output_phase)
        {
            let (sin, cos) = phase.sin_cos();
            let amplitude = mag * gain;
            *out = Complex64::new(amplitude * cos, amplitude * sin);
        }
        self.spectrum[0].im = 0.0;
        self.spectrum[end..].fill(Complex64::ZERO);
        self.inverse();
        let half = self.time.len() / 2;
        self.time.rotate_left(half);
        for ((out, &sample), &window) in channel
            .accumulator
            .iter_mut()
            .zip(&self.time)
            .zip(&self.window)
        {
            *out += sample * window;
        }
    }

    fn forward(&mut self) {
        self.forward
            .process_with_scratch(&mut self.time, &mut self.spectrum, &mut self.scratch)
            .expect("valid analysis FFT buffers");
    }

    fn inverse(&mut self) {
        self.inverse
            .process_with_scratch(&mut self.spectrum, &mut self.time, &mut self.scratch)
            .expect("valid synthesis FFT buffers");
    }
}

pub(crate) struct Channel {
    pub(crate) current: Polar,
    pub(crate) readahead: Polar,
    pub(crate) accumulator: Vec<f64>,
    pub(crate) classifier: Classifier,
    pub(crate) guide: Guidance,
    envelope: Vec<f64>,
    peaks: Vec<usize>,
    prev_peaks: Vec<usize>,
    prev_mag: Vec<f64>,
    prev_phase: Vec<f64>,
    prev_output_phase: Vec<f64>,
    output_phase: Vec<f64>,
    unlocked: Vec<f64>,
}

pub(crate) struct Polar {
    pub(crate) mag: Vec<f64>,
    phase: Vec<f64>,
}

impl Channel {
    pub(crate) fn new(fft_size: usize, rate: f64) -> Self {
        let bins = fft_size / 2 + 1;
        let polar = || Polar {
            mag: vec![0.0; bins],
            phase: vec![0.0; bins],
        };
        Self {
            current: polar(),
            readahead: polar(),
            accumulator: vec![0.0; fft_size],
            classifier: Classifier::new(
                (16_000.0_f64.min(rate / 2.0) * fft_size as f64 / rate) as usize,
            ),
            guide: Guidance {
                reset: Some(0.0..=rate / 2.0),
                unlocked: None,
                channel_lock: None,
            },
            envelope: vec![0.0; bins],
            peaks: vec![0; bins],
            prev_peaks: vec![0; bins],
            prev_mag: vec![0.0; bins],
            prev_phase: vec![0.0; bins],
            prev_output_phase: vec![0.0; bins],
            output_phase: vec![0.0; bins],
            unlocked: vec![0.0; bins],
        }
    }
}

pub(crate) struct PhaseAdvance {
    bin_hz: f64,
    peak_locations: Vec<usize>,
    strongest: Vec<usize>,
}

impl PhaseAdvance {
    pub(crate) fn new(size: usize, rate: f64) -> Self {
        Self {
            bin_hz: rate / size as f64,
            peak_locations: Vec::with_capacity(size / 2 + 1),
            strongest: vec![0; size / 2 + 1],
        }
    }

    pub(crate) fn advance<const N: usize>(
        &mut self,
        channels: &mut [Channel; N],
        previous_hops: (usize, usize),
        pitch: f64,
        mid_side: bool,
    ) {
        let (input_hop, output_hop) = previous_hops;
        let ratio = output_hop as f64 / input_hop as f64;
        let last_bin = self.strongest.len() - 1;
        let scale = TAU * input_hop as f64 / (last_bin * 2) as f64;
        let mut ends = [1600.0, 7000.0].map(|hz| (hz / self.bin_hz).round() as usize);
        ends.iter_mut().for_each(|end| *end = (*end).min(last_bin));
        let ranges = [0..=ends[0], ends[0]..=ends[1], ends[1]..=last_bin];
        let radii = [1, 2, if output_hop > 256 { 4 } else { 5 }];
        let beta =
            [1200.0, 5000.0, 10_000.0].map(|hz| 1.0 + hz * ((2.0 + pitch) / 3.0 - 1.0) / 10_000.0);

        for channel in channels.iter_mut() {
            if input_hop != output_hop {
                for (range, radius) in ranges.clone().into_iter().zip(radii) {
                    nearest_peaks(
                        &channel.current.mag,
                        &mut channel.peaks,
                        range,
                        radius,
                        &mut self.peak_locations,
                    );
                }
                nearest_peaks(
                    &channel.prev_mag,
                    &mut channel.prev_peaks,
                    0..=last_bin,
                    1,
                    &mut self.peak_locations,
                );
            }

            for (i, output) in channel.unlocked.iter_mut().enumerate() {
                let omega = scale * i as f64;
                let error = principal(channel.current.phase[i] - (channel.prev_phase[i] + omega));
                *output = channel.prev_output_phase[i] + ratio * (omega + error);
            }
        }
        for (i, strongest) in self.strongest.iter_mut().enumerate() {
            *strongest = 0;
            for c in 1..N {
                // Upstream stores the best magnitude as a float.
                if channels[c].current.mag[i]
                    > f64::from(channels[*strongest].current.mag[i] as f32)
                {
                    *strongest = c;
                }
            }
        }
        for c in 0..N {
            let mut band = 0;
            for i in 0..=last_bin {
                let hz = self.bin_hz * i as f64;
                while band < 2 && hz > [1600.0, 7000.0][band] {
                    band += 1;
                }
                let state = &channels[c];
                let contains = |range: &Option<std::ops::RangeInclusive<f64>>| {
                    range.as_ref().is_some_and(|range| range.contains(&hz))
                };
                let phase = if contains(&state.guide.reset)
                    || (mid_side && c == 0 && contains(&channels[1].guide.reset))
                {
                    state.current.phase[i]
                } else if input_hop == output_hop || contains(&state.guide.unlocked) {
                    state.unlocked[i]
                } else {
                    let peak = state.peaks[i];
                    let previous_peak = state.prev_peaks[peak];
                    let other = self.strongest[i];
                    let peer = &channels[other];
                    let locked =
                        |channel: &Channel| channel.guide.channel_lock.is_some_and(|end| hz <= end);
                    let source = if other != c
                        && locked(state)
                        && locked(peer)
                        && peer.prev_peaks[peer.peaks[i]] == previous_peak
                    {
                        peer
                    } else {
                        state
                    };
                    let advance = source.unlocked[peak] - source.prev_output_phase[peak];
                    source.prev_output_phase[previous_peak]
                        + advance
                        + beta[band] * (state.current.phase[i] - source.current.phase[peak])
                };
                channels[c].output_phase[i] = principal(phase);
            }
        }
        for channel in channels {
            channel.prev_phase.copy_from_slice(&channel.current.phase);
            channel
                .prev_output_phase
                .copy_from_slice(&channel.output_phase);
        }
    }
}

fn principal(phase: f64) -> f64 {
    phase - TAU * ((phase - PI) / TAU).ceil()
}

fn nearest_peaks(
    values: &[f64],
    nearest: &mut [usize],
    range: std::ops::RangeInclusive<usize>,
    radius: usize,
    peaks: &mut Vec<usize>,
) {
    peaks.clear();
    let (first, last) = (*range.start(), *range.end());
    for i in range {
        let left = i.saturating_sub(radius).max(first);
        let right = (i + radius).min(last);
        let here = values[i];
        if values[left..i].iter().all(|&x| here > x)
            && values[i + 1..=right].iter().all(|&x| here >= x)
        {
            peaks.push(i);
        }
    }
    let mut next = 0;
    for (offset, peak) in nearest[first..=last].iter_mut().enumerate() {
        let i = first + offset;
        while next + 1 < peaks.len() && i > (peaks[next] + peaks[next + 1]) / 2 {
            next += 1;
        }
        // Upstream selects the upper peak when the distances tie.
        if next + 1 < peaks.len() && i * 2 == peaks[next] + peaks[next + 1] {
            next += 1;
        }
        *peak = peaks.get(next).copied().unwrap_or(i);
    }
}
