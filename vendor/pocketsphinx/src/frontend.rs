use realfft::RealFftPlanner;
use std::f32::consts::PI;

use crate::model::{FEATURE_DIM, LDA_INPUT_DIM};

const FRAME_LENGTH: usize = 410;
const FRAME_SHIFT: usize = 160;
const FFT_LENGTH: usize = 512;
const FILTER_COUNT: usize = 25;
const CEPSTRUM_DIM: usize = 13;
const LOWER_FREQUENCY: f32 = 130.0;
const UPPER_FREQUENCY: f32 = 6_800.0;
const PREEMPHASIS: f32 = 0.97;
const LIFTER: f32 = 22.0;
const LOG_FLOOR: f32 = 1.0e-4;
const SMOOTH_WINDOW: usize = 4;

pub(crate) fn extract(samples: &[i16], lda: &[f32]) -> Result<Vec<Vec<f32>>, String> {
    if lda.len() != FEATURE_DIM * LDA_INPUT_DIM {
        return Err("invalid LDA transform".to_owned());
    }

    // PocketSphinx seeds MT19937 with -1 for this configuration. Keeping the
    // same generator also removes the native implementation's shared-state
    // race between decoder threads.
    let mut random = MersenneTwister::new(u32::MAX);
    let full_frame_count = if samples.len() < FRAME_LENGTH {
        0
    } else {
        1 + (samples.len() - FRAME_LENGTH) / FRAME_SHIFT
    };
    let regular_end = if full_frame_count == 0 {
        0
    } else {
        FRAME_LENGTH + (full_frame_count - 1) * FRAME_SHIFT
    };
    let tail_start = full_frame_count * FRAME_SHIFT;
    let mut regular = samples[..regular_end].to_vec();
    dither(&mut regular, &mut random);
    let mut tail = samples[tail_start..].to_vec();
    dither(&mut tail, &mut random);

    let filters = mel_filters();
    let mut planner = RealFftPlanner::<f32>::new();
    let transform = planner.plan_fft_forward(FFT_LENGTH);
    let mut fft_input = transform.make_input_vec();
    let mut spectrum = transform.make_output_vec();
    let frame_count = full_frame_count + 1;
    let mut cepstra = Vec::with_capacity(frame_count);
    let mut noise = NoiseStats::default();

    for frame_index in 0..frame_count {
        let start = frame_index * FRAME_SHIFT;
        let final_frame = frame_index == full_frame_count;
        let frame = if final_frame {
            tail.as_slice()
        } else {
            &regular[start..start + FRAME_LENGTH]
        };
        fft_input.fill(0.0);
        for (index, (current, output)) in frame.iter().zip(&mut fft_input).enumerate() {
            let previous = f32::from(if index > 0 {
                frame[index - 1]
            } else if start > 0 {
                regular[start - 1]
            } else {
                0
            });
            let window = 0.54 - 0.46 * (2.0 * PI * index as f32 / (FRAME_LENGTH - 1) as f32).cos();
            *output = (f32::from(*current) - PREEMPHASIS * previous) * window;
        }
        transform
            .process(&mut fft_input, &mut spectrum)
            .map_err(|error| format!("FFT failed: {error}"))?;
        let power: Vec<f32> = spectrum.iter().map(|value| value.norm_sqr()).collect();
        let mut mel_spectrum = [0.0; FILTER_COUNT];
        for (filter_index, filter) in filters.iter().enumerate() {
            mel_spectrum[filter_index] = filter
                .iter()
                .enumerate()
                .map(|(bin, weight)| power[bin] * weight)
                .sum();
        }
        noise.remove(&mut mel_spectrum);
        let logs = mel_spectrum.map(|value| (value + LOG_FLOOR).ln());

        let mut cepstrum = [0.0; CEPSTRUM_DIM];
        for (coefficient, output) in cepstrum.iter_mut().enumerate() {
            let normalizer = if coefficient == 0 {
                (1.0 / FILTER_COUNT as f32).sqrt()
            } else {
                (2.0 / FILTER_COUNT as f32).sqrt()
            };
            *output = logs
                .iter()
                .enumerate()
                .map(|(filter, value)| {
                    value
                        * (PI * coefficient as f32 * (filter as f32 + 0.5) / FILTER_COUNT as f32)
                            .cos()
                })
                .sum::<f32>()
                * normalizer;
            *output *= 1.0 + LIFTER / 2.0 * (coefficient as f32 * PI / LIFTER).sin();
        }
        cepstra.push(cepstrum);
    }

    batch_normalize(&mut cepstra);
    let mut output = Vec::with_capacity(cepstra.len());
    for frame in 0..cepstra.len() {
        let mut combined = [0.0; LDA_INPUT_DIM];
        for coefficient in 0..CEPSTRUM_DIM {
            combined[coefficient] = cepstra[frame][coefficient];
            combined[CEPSTRUM_DIM + coefficient] = cepstra
                [clamp_frame(frame as isize + 2, cepstra.len())][coefficient]
                - cepstra[clamp_frame(frame as isize - 2, cepstra.len())][coefficient];
            combined[CEPSTRUM_DIM * 2 + coefficient] = cepstra
                [clamp_frame(frame as isize + 3, cepstra.len())][coefficient]
                - cepstra[clamp_frame(frame as isize - 1, cepstra.len())][coefficient]
                - cepstra[clamp_frame(frame as isize + 1, cepstra.len())][coefficient]
                + cepstra[clamp_frame(frame as isize - 3, cepstra.len())][coefficient];
        }
        let mut feature = vec![0.0; FEATURE_DIM];
        for row in 0..FEATURE_DIM {
            feature[row] = lda[row * LDA_INPUT_DIM..(row + 1) * LDA_INPUT_DIM]
                .iter()
                .zip(combined)
                .map(|(weight, value)| weight * value)
                .sum();
        }
        output.push(feature);
    }
    Ok(output)
}

fn dither(samples: &mut [i16], random: &mut MersenneTwister) {
    for sample in samples {
        if (random.next() >> 1).is_multiple_of(4) {
            *sample = sample.wrapping_add(1);
        }
    }
}

fn clamp_frame(frame: isize, count: usize) -> usize {
    frame.clamp(0, count.saturating_sub(1) as isize) as usize
}

fn batch_normalize(cepstra: &mut [[f32; CEPSTRUM_DIM]]) {
    let mut mean = [0.0; CEPSTRUM_DIM];
    let mut count = 0_usize;
    for frame in cepstra.iter().filter(|frame| frame[0] >= 0.0) {
        for (sum, value) in mean.iter_mut().zip(frame) {
            *sum += value;
        }
        count += 1;
    }
    if count == 0 {
        return;
    }
    for value in &mut mean {
        *value /= count as f32;
    }
    for frame in cepstra {
        for (value, mean) in frame.iter_mut().zip(mean) {
            *value -= mean;
        }
    }
}

fn mel_filters() -> Vec<Vec<f32>> {
    let minimum = hz_to_mel(LOWER_FREQUENCY);
    let maximum = hz_to_mel(UPPER_FREQUENCY);
    let width = (maximum - minimum) / (FILTER_COUNT + 1) as f32;
    let frequencies: Vec<f32> = (0..FILTER_COUNT + 2)
        .map(|index| {
            let frequency = mel_to_hz(minimum + index as f32 * width);
            (frequency / bin_width()).round() * bin_width()
        })
        .collect();
    let bin_width = bin_width();
    (0..FILTER_COUNT)
        .map(|filter| {
            let area = 2.0 / (frequencies[filter + 2] - frequencies[filter]);
            (0..FFT_LENGTH / 2 + 1)
                .map(|bin| {
                    let frequency = bin as f32 * bin_width;
                    if frequency < frequencies[filter] || frequency > frequencies[filter + 2] {
                        0.0
                    } else {
                        ((frequency - frequencies[filter])
                            / (frequencies[filter + 1] - frequencies[filter]))
                            .min(
                                (frequencies[filter + 2] - frequency)
                                    / (frequencies[filter + 2] - frequencies[filter + 1]),
                            )
                            * area
                    }
                })
                .collect()
        })
        .collect()
}

const fn bin_width() -> f32 {
    crate::SAMPLE_RATE as f32 / FFT_LENGTH as f32
}

fn hz_to_mel(frequency: f32) -> f32 {
    2_595.0 * (1.0 + frequency / 700.0).log10()
}

fn mel_to_hz(mel: f32) -> f32 {
    700.0 * (10.0_f32.powf(mel / 2_595.0) - 1.0)
}

struct MersenneTwister {
    state: [u32; 624],
    index: usize,
}

impl MersenneTwister {
    fn new(seed: u32) -> Self {
        let mut state = [0; 624];
        state[0] = seed;
        for index in 1..state.len() {
            state[index] = 1_812_433_253_u32
                .wrapping_mul(state[index - 1] ^ (state[index - 1] >> 30))
                .wrapping_add(index as u32);
        }
        Self { state, index: 624 }
    }

    fn next(&mut self) -> u32 {
        if self.index == self.state.len() {
            for index in 0..self.state.len() {
                let combined = (self.state[index] & 0x8000_0000)
                    | (self.state[(index + 1) % self.state.len()] & 0x7fff_ffff);
                self.state[index] = self.state[(index + 397) % self.state.len()]
                    ^ (combined >> 1)
                    ^ if combined & 1 == 0 { 0 } else { 0x9908_b0df };
            }
            self.index = 0;
        }
        let mut value = self.state[self.index];
        self.index += 1;
        value ^= value >> 11;
        value ^= (value << 7) & 0x9d2c_5680;
        value ^= (value << 15) & 0xefc6_0000;
        value ^ (value >> 18)
    }
}

#[derive(Default)]
struct NoiseStats {
    power: [f32; FILTER_COUNT],
    noise: [f32; FILTER_COUNT],
    floor: [f32; FILTER_COUNT],
    peak: [f32; FILTER_COUNT],
    initialized: bool,
}

impl NoiseStats {
    fn remove(&mut self, spectrum: &mut [f32; FILTER_COUNT]) {
        if !self.initialized {
            self.power = *spectrum;
            self.noise = spectrum.map(|value| value / 20.0);
            self.floor = self.noise;
            self.initialized = true;
        }
        for (power, value) in self.power.iter_mut().zip(*spectrum) {
            *power = 0.7 * *power + 0.3 * value;
        }
        lower_envelope(&self.power, &mut self.noise);

        let mut signal: [f32; FILTER_COUNT] =
            std::array::from_fn(|index| (self.power[index] - self.noise[index]).max(1.0));
        lower_envelope(&signal, &mut self.floor);
        for (signal, peak) in signal.iter_mut().zip(&mut self.peak) {
            let current = *signal;
            *peak *= 0.85;
            if *signal < 0.85 * *peak {
                *signal = 0.2 * *peak;
            }
            *peak = (*peak).max(current);
        }

        let gains: [f32; FILTER_COUNT] = std::array::from_fn(|index| {
            let signal = signal[index].max(self.floor[index]);
            (signal / self.power[index]).clamp(0.05, 20.0)
        });
        for (index, value) in spectrum.iter_mut().enumerate() {
            let start = index.saturating_sub(SMOOTH_WINDOW);
            let end = (index + SMOOTH_WINDOW + 1).min(FILTER_COUNT);
            *value *= gains[start..end].iter().sum::<f32>() / (end - start) as f32;
        }
    }
}

fn lower_envelope(input: &[f32; FILTER_COUNT], envelope: &mut [f32; FILTER_COUNT]) {
    for (input, envelope) in input.iter().zip(envelope) {
        if *input >= *envelope {
            *envelope = 0.995 * *envelope + 0.005 * *input;
        } else {
            *envelope = 0.5 * *envelope + 0.5 * *input;
        }
    }
}
