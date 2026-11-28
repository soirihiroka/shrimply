use std::sync::LazyLock;

use crate::{FRAME_SAMPLES, Layout, bits::best_book};

const MAX_QUANTIZED: usize = 8191;

#[derive(Clone, Copy, Default)]
pub(crate) struct Choice {
    pub sf: u8,
    pub book: u8,
    pub cost: u16,
    pub error: f32,
}

pub(crate) struct Search {
    inverse: Vec<f32>,
    boundaries: Vec<f32>,
    pub bands: [Vec<Vec<Choice>>; 2],
    powers: [[f32; FRAME_SAMPLES]; 2],
}

impl Search {
    pub fn new() -> Self {
        let inverse: Vec<_> = (0..=MAX_QUANTIZED)
            .map(|value| (value as f32).powf(4.0 / 3.0))
            .collect();
        let boundaries = inverse
            .windows(2)
            .map(|adjacent| ((adjacent[0] + adjacent[1]) * 0.5).powf(0.75))
            .chain([f32::INFINITY])
            .collect();
        Self {
            inverse,
            boundaries,
            bands: std::array::from_fn(|_| (0..64).map(|_| Vec::with_capacity(24)).collect()),
            powers: [[0.0; FRAME_SAMPLES]; 2],
        }
    }

    pub fn analyze(
        &mut self,
        layout: Layout,
        spectra: &[[f32; FRAME_SAMPLES]; 2],
        masking: &[Vec<f32>; 2],
    ) -> f32 {
        let mut demand = 0.0;
        let mut quantized = [0_i16; FRAME_SAMPLES];
        for ((channel, powers), (source, masking)) in self
            .bands
            .iter_mut()
            .zip(&mut self.powers)
            .zip(spectra.iter().zip(masking))
        {
            for (power, sample) in powers.iter_mut().zip(source) {
                let root = sample.abs().sqrt();
                *power = root * root.sqrt();
            }
            for group in 0..layout.groups {
                for band in 0..layout.band_count() {
                    let index = group * layout.band_count() + band;
                    let choices = &mut channel[index];
                    choices.clear();
                    let mask = masking[index].max(1.0);
                    let mut energy = 0.0;
                    let mut peak = 0.0_f32;
                    let mut width = 0;
                    for i in layout.ranges(group, band).flatten() {
                        energy += source[i].powi(2);
                        peak = peak.max(powers[i]);
                        width += 1;
                    }
                    choices.push(Choice {
                        error: energy / mask,
                        ..Choice::default()
                    });
                    if energy == 0.0 {
                        continue;
                    }
                    // Approximate the scalefactor whose quantization error is masked,
                    // then measure exact distortion and Huffman rate around it.
                    let bins = width as f32;
                    demand += bins * (1.0 + energy / (mask * bins)).log2();
                    let rms = (energy / bins).sqrt();
                    let step = mask.sqrt() / (rms.sqrt().sqrt() * (4.0 / 3.0));
                    let masked = (100.0 + (16.0 / 3.0) * step.log2()) as i32;
                    let minimum =
                        (100.0 + (16.0 / 3.0) * (peak / MAX_QUANTIZED as f32).log2()).ceil() as i32;
                    let first = (masked - 24).max(minimum).clamp(0, 255);
                    let last = ((100.0 + (16.0 / 3.0) * (peak / 0.6).log2()).ceil() as i32)
                        .min(masked + 36)
                        .clamp(first, 255);
                    let rooted_rms = rms.sqrt();
                    let step = if peak > 2.0 * rooted_rms * rooted_rms.sqrt() {
                        1 // A few strong coefficients dominate the band's distortion.
                    } else {
                        2
                    };
                    for sf in (first..=last).step_by(step) {
                        let (scale, factor) = gains(sf as u8);
                        let mut error = 0.0;
                        let mut high = 0;
                        for (value, i) in quantized
                            .iter_mut()
                            .zip(layout.ranges(group, band).flatten())
                        {
                            *value = quantize(source[i], powers[i], factor, &self.boundaries);
                            let magnitude = value.unsigned_abs();
                            high = high.max(magnitude);
                            error += (source[i].abs()
                                - self.inverse[usize::from(magnitude)] * scale)
                                .powi(2);
                        }
                        if high == 0 {
                            break;
                        }
                        let (book, cost) = best_book(&quantized[..width], high);
                        choices.push(Choice {
                            sf: sf as u8,
                            book,
                            cost,
                            error: error / mask,
                        });
                    }
                }
            }
        }
        demand
    }

    pub fn select(&self, layout: Layout, lambda: f32, output: &mut [Vec<Choice>; 2]) {
        const PATHS: usize = 3;
        #[derive(Clone, Copy)]
        struct Step {
            score: f32,
            choice: Choice,
            sf: Option<u8>,
            run: usize,
            parent: usize,
        }
        let empty = Step {
            score: f32::INFINITY,
            choice: Choice::default(),
            sf: None,
            run: 0,
            parent: 0,
        };
        let section_bits = layout.section_bits();
        let section_escape = (1 << section_bits) - 1;
        let count = layout.groups * layout.band_count();
        for (channel, result) in self.bands.iter().zip(output) {
            let mut previous = [empty; PATHS];
            previous[0].score = 0.0;
            let mut history = [[empty; PATHS]; 64];
            for (index, (choices, next)) in channel.iter().zip(&mut history[..count]).enumerate() {
                let first_in_group = index % layout.band_count() == 0;
                let mut worst = 0;
                for (parent, from) in previous
                    .iter()
                    .enumerate()
                    .filter(|(_, from)| from.score.is_finite())
                {
                    for &choice in choices {
                        let mut bits = usize::from(choice.cost);
                        let mut sf = from.sf;
                        if choice.book != 0 {
                            let delta = sf.map_or(0, |sf| i16::from(choice.sf) - i16::from(sf));
                            if delta.abs() > 60 {
                                continue;
                            }
                            bits +=
                                (crate::tables::SCALEFACTORS[(delta + 60) as usize] & 31) as usize;
                            sf = Some(choice.sf);
                        }
                        let run = if first_in_group || from.choice.book != choice.book {
                            bits += 4 + section_bits;
                            1
                        } else {
                            let length = from.run + 1;
                            bits += usize::from(length % section_escape == 0) * section_bits;
                            length
                        };
                        let score = from.score + choice.error + lambda * bits as f32;
                        if score >= next[worst].score {
                            continue;
                        }
                        let slot = next
                            .iter()
                            .position(|step| {
                                step.sf == sf && step.run == run && step.choice.book == choice.book
                            })
                            .unwrap_or(worst);
                        if score < next[slot].score {
                            next[slot] = Step {
                                score,
                                choice,
                                sf,
                                run,
                                parent,
                            };
                            worst = (0..PATHS)
                                .max_by(|&a, &b| next[a].score.total_cmp(&next[b].score))
                                .unwrap();
                        }
                    }
                }
                previous = *next;
            }
            result.clear();
            result.resize(count, Choice::default());
            let mut path = (0..PATHS)
                .min_by(|&a, &b| previous[a].score.total_cmp(&previous[b].score))
                .unwrap();
            for (choice, states) in result.iter_mut().zip(&history[..count]).rev() {
                let step = states[path];
                *choice = step.choice;
                path = step.parent;
            }
        }
    }

    pub fn coefficient(&self, channel: usize, index: usize, sample: f32, factor: f32) -> i16 {
        quantize(
            sample,
            self.powers[channel][index],
            factor,
            &self.boundaries,
        )
    }
}

pub(crate) fn gains(sf: u8) -> (f32, f32) {
    static GAINS: LazyLock<[(f32, f32); 256]> = LazyLock::new(|| {
        std::array::from_fn(|sf| {
            let exponent = (sf as f32 - 100.0) * 0.25;
            (exponent.exp2(), (-0.75 * exponent).exp2())
        })
    });
    GAINS[usize::from(sf)]
}

fn quantize(sample: f32, power: f32, factor: f32, boundaries: &[f32]) -> i16 {
    let scaled = power * factor;
    let floor = (scaled as usize).min(MAX_QUANTIZED);
    let magnitude = (floor + usize::from(scaled > boundaries[floor])) as i16;
    if sample < 0.0 { -magnitude } else { magnitude }
}
