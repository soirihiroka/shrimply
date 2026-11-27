// SPDX-License-Identifier: GPL-2.0-or-later
// Port of Rubber Band 4.0.0 R3LiveShifter, copyright 2007-2024 Particular Programs Ltd.

mod guide;
mod resample;
mod spectrum;

use std::collections::VecDeque;

use guide::GuideParams;
use resample::Resampler;
use spectrum::{Channel, PhaseAdvance, Transform};

pub const BLOCK_SIZE: usize = 512;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Window {
    Short,
    Medium,
}

#[derive(Clone, Copy)]
pub struct Shift {
    pub pitch_scale: f64,
    pub formant_scale: f64,
    pub preserve_formants: bool,
}

pub struct LiveShifter<const N: usize> {
    rate: f64,
    fft_size: usize,
    window_source: Vec<[f32; N]>,
    medium: bool,
    link_channels: bool,
    mid_side: bool,
    max_input_hop: usize,
    max_output_hop: usize,
    previous_hops: (usize, usize),
    first_block: bool,
    channels: [Channel; N],
    transform: Transform,
    phase: PhaseAdvance,
    input_resampler: Resampler<N>,
    output_resampler: Resampler<N>,
    input: VecDeque<[f32; N]>,
    output: VecDeque<[f32; N]>,
    resampled: [[f32; N]; BLOCK_SIZE],
}

impl<const N: usize> LiveShifter<N> {
    pub fn new(sample_rate: u32, window: Window, link_channels: bool) -> Self {
        assert!(N > 0, "LiveShifter requires at least one channel");
        let rate = sample_rate.clamp(8000, 192_000) as usize;
        let fft_size = rate.div_ceil(32).next_power_of_two().max(1024);
        let max_input_hop = rate.div_ceil(128).next_power_of_two();
        let max_output_hop = max_input_hop * 5 / 4;
        let medium = window == Window::Medium;
        let source_size = fft_size + if medium { max_input_hop } else { 0 };
        let rate = rate as f64;
        Self {
            rate,
            fft_size,
            window_source: vec![[0.0; N]; source_size],
            medium,
            link_channels,
            mid_side: link_channels && N == 2,
            max_input_hop,
            max_output_hop,
            previous_hops: (max_input_hop / 2, max_input_hop / 2),
            first_block: true,
            channels: std::array::from_fn(|_| Channel::new(fft_size, rate)),
            transform: Transform::new(fft_size),
            phase: PhaseAdvance::new(fft_size, rate),
            input_resampler: Resampler::new(),
            output_resampler: Resampler::new(),
            // At +24 semitones the startup pad spans four windows.
            input: VecDeque::with_capacity(source_size * 4 + BLOCK_SIZE),
            output: VecDeque::with_capacity(source_size * 4),
            resampled: [[0.0; N]; BLOCK_SIZE],
        }
    }

    /// Startup delay when initialized at unity, in frames.
    pub fn start_delay(&self) -> usize {
        self.window_source.len() + 2 * Resampler::<N>::delay(1.0)
    }

    pub fn shift(
        &mut self,
        input: &[[f32; N]; BLOCK_SIZE],
        output: &mut [[f32; N]; BLOCK_SIZE],
        settings: Shift,
    ) {
        let pitch = if settings.pitch_scale.is_finite() {
            settings.pitch_scale.clamp(0.25, 4.0)
        } else {
            1.0
        };
        // LiveShifter 4.0 gates cepstral correction on the preservation option.
        let formant_factor = if settings.preserve_formants {
            Some(
                if settings.formant_scale.is_finite() && settings.formant_scale > 0.0 {
                    settings.formant_scale
                } else {
                    pitch.recip()
                },
            )
        } else {
            None
        };
        let input_ratio = pitch.max(1.0).recip();
        let output_ratio = pitch.min(1.0).recip();
        self.read_input(input, input_ratio);
        self.generate(
            (BLOCK_SIZE as f64 / output_ratio).ceil() as usize,
            pitch,
            formant_factor,
        );
        self.read_output(output, output_ratio);
        self.first_block = false;
    }

    fn read_input(&mut self, input: &[[f32; N]; BLOCK_SIZE], ratio: f64) {
        if self.first_block {
            let pad = (self.window_source.len() as f64 / ratio).ceil() as usize;
            self.input.resize(pad, [0.0; N]);
        }
        let mut mid_side;
        let input = if self.mid_side {
            mid_side = *input;
            for frame in &mut mid_side {
                let (left, right) = (frame[0], frame[1]);
                frame[0] = (left + right) * 0.5;
                frame[1] = (left - right) * 0.5;
            }
            &mid_side
        } else {
            input
        };
        let (consumed, written) = self
            .input_resampler
            .process(ratio, input, &mut self.resampled);
        assert_eq!(consumed, BLOCK_SIZE);
        if self.first_block {
            let expected = (BLOCK_SIZE as f64 * ratio).floor() as usize;
            self.input.resize(
                self.input.len() + expected.saturating_sub(written),
                [0.0; N],
            );
        }
        self.input.extend(self.resampled[..written].iter().copied());
    }

    fn generate(&mut self, required: usize, pitch: f64, formant_factor: Option<f64>) {
        let needed = required.saturating_sub(self.output.len());
        let available = self.input.len().saturating_sub(self.window_source.len());
        if needed == 0 || available == 0 {
            return;
        }
        let hops = available
            .div_ceil(self.max_input_hop)
            .max(needed.div_ceil(self.max_output_hop));
        let input_hop = available.div_ceil(hops);
        let output_hop = needed.div_ceil(hops);
        for index in 0..hops {
            let count = copy_front(&self.input, &mut self.window_source);
            self.window_source[count..].fill([0.0; N]);
            let reuse_readahead = self.medium
                && (!self.first_block || index > 0)
                && input_hop == self.previous_hops.0;
            let normalise = 1.0 / self.fft_size as f64;
            for (c, channel) in self.channels.iter_mut().enumerate() {
                if reuse_readahead {
                    std::mem::swap(&mut channel.current, &mut channel.readahead);
                    for magnitude in &mut channel.current.mag {
                        *magnitude *= normalise;
                    }
                } else {
                    self.transform.analyse(
                        &self.window_source[..self.fft_size],
                        c,
                        &mut channel.current,
                        normalise,
                    );
                }
                if self.medium {
                    self.transform.analyse(
                        &self.window_source[input_hop..input_hop + self.fft_size],
                        c,
                        &mut channel.readahead,
                        1.0,
                    );
                }
                if let Some(factor) = formant_factor.filter(|&factor| factor != 1.0) {
                    self.transform.formants(channel, self.rate, factor);
                }
                let magnitudes = if self.medium {
                    &channel.readahead.mag
                } else {
                    &channel.current.mag
                };
                let mean =
                    channel.current.mag[1..].iter().sum::<f64>() / (self.fft_size / 2) as f64;
                channel.classifier.update(
                    &mut channel.guide,
                    magnitudes,
                    mean,
                    GuideParams {
                        pitch,
                        bin_hz: self.rate / self.fft_size as f64,
                        link_channels: self.link_channels,
                        reset_on_silence: !self.mid_side || c == 0,
                    },
                );
            }
            self.phase
                .advance(&mut self.channels, self.previous_hops, pitch, self.mid_side);
            for channel in &mut self.channels {
                self.transform.synthesise(channel, output_hop);
            }
            self.output.extend((0..output_hop).map(|i| {
                self.channels
                    .each_ref()
                    .map(|channel| channel.accumulator[i] as f32)
            }));
            for channel in &mut self.channels {
                channel.accumulator.copy_within(output_hop.., 0);
                channel.accumulator[self.fft_size - output_hop..].fill(0.0);
            }
            self.input.drain(..input_hop.min(self.input.len()));
            self.previous_hops = (input_hop, output_hop);
        }
    }

    fn read_output(&mut self, output: &mut [[f32; N]; BLOCK_SIZE], ratio: f64) {
        let mut requested = ((BLOCK_SIZE as f64 / ratio).floor() as usize).max(1);
        let mut written = 0;
        while written < BLOCK_SIZE {
            let count = copy_front(&self.output, &mut self.resampled[..requested]);
            let (consumed, produced) = self.output_resampler.process(
                ratio,
                &self.resampled[..count],
                &mut output[written..],
            );
            self.output.drain(..consumed);
            if consumed == 0 && produced == 0 {
                break;
            }
            written += produced;
            requested = 1;
        }
        if self.mid_side {
            for frame in &mut output[..written] {
                let (mid, side) = (frame[0], frame[1]);
                frame[0] = mid + side;
                frame[1] = mid - side;
            }
        }
        if self.first_block {
            let pad = BLOCK_SIZE - written;
            output.copy_within(..written, pad);
            output[..pad].fill([0.0; N]);
        } else {
            let mut last = if written > 0 {
                output[written - 1]
            } else {
                [0.0; N]
            };
            for frame in &mut output[written..] {
                last = last.map(|sample| sample * 0.9);
                *frame = last;
            }
        }
    }
}

fn copy_front<const N: usize>(source: &VecDeque<[f32; N]>, dest: &mut [[f32; N]]) -> usize {
    let (first, second) = source.as_slices();
    let split = first.len().min(dest.len());
    let tail = second.len().min(dest.len() - split);
    dest[..split].copy_from_slice(&first[..split]);
    dest[split..split + tail].copy_from_slice(&second[..tail]);
    split + tail
}
