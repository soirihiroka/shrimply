use crate::{FRAME_SAMPLES, Layout, bits::best_book};

const MAX_QUANTIZED: usize = 8191;

#[derive(Clone, Copy, Default)]
pub(crate) struct Choice {
    pub sf: u8,
    pub book: u8,
    pub cost: u16,
    error: f32,
}

pub(crate) struct Search {
    inverse: Vec<f32>,
    bands: [Vec<Vec<Choice>>; 2],
    powers: [[f32; FRAME_SAMPLES]; 2],
}

impl Search {
    pub fn new() -> Self {
        Self {
            inverse: (0..=MAX_QUANTIZED)
                .map(|value| (value as f32).powf(4.0 / 3.0))
                .collect(),
            bands: std::array::from_fn(|_| (0..64).map(|_| Vec::with_capacity(24)).collect()),
            powers: [[0.0; FRAME_SAMPLES]; 2],
        }
    }

    pub fn analyze(
        &mut self,
        layout: Layout,
        spectra: &[[f32; FRAME_SAMPLES]; 2],
        masking: &[Vec<f32>; 2],
    ) {
        for ((channel, powers), (source, masking)) in self
            .bands
            .iter_mut()
            .zip(&mut self.powers)
            .zip(spectra.iter().zip(masking))
        {
            for (power, sample) in powers.iter_mut().zip(source) {
                *power = sample.abs().sqrt().sqrt() * sample.abs().sqrt();
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
                    let rms = (energy / width as f32).sqrt();
                    let step = mask.sqrt() / (rms.sqrt().sqrt() * (4.0 / 3.0));
                    let masked = (100.0 + (16.0 / 3.0) * step.log2()) as i32;
                    let minimum =
                        (100.0 + (16.0 / 3.0) * (peak / MAX_QUANTIZED as f32).log2()).ceil() as i32;
                    let first = (masked - 20).max(minimum).clamp(0, 255);
                    let last = ((100.0 + (16.0 / 3.0) * (peak / 0.6).log2()).ceil() as i32)
                        .min(masked + 36)
                        .clamp(first, 255);
                    let mut quantized = [0_i16; FRAME_SAMPLES];
                    for sf in (first..=last).step_by(3) {
                        let scale = gain(sf as u8);
                        let factor = scale.powf(-0.75);
                        let mut error = 0.0;
                        let mut high = 0;
                        for (value, i) in quantized
                            .iter_mut()
                            .zip(layout.ranges(group, band).flatten())
                        {
                            *value = quantize(source[i], powers[i], factor);
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
    }

    pub fn select(&self, layout: Layout, lambda: f32) -> [Vec<Choice>; 2] {
        std::array::from_fn(|channel| {
            let mut previous = None::<u8>;
            let mut previous_book = None;
            let mut result = Vec::with_capacity(layout.groups * layout.band_count());
            for group in 0..layout.groups {
                for band in 0..layout.band_count() {
                    let choices = &self.bands[channel][group * layout.band_count() + band];
                    let mut best = choices[0];
                    let mut score = f32::INFINITY;
                    for &choice in choices {
                        let mut cost = choice.cost;
                        if choice.book != 0 {
                            let delta =
                                previous.map_or(0, |sf| i16::from(choice.sf) - i16::from(sf));
                            if delta.abs() > 60 {
                                continue;
                            }
                            cost +=
                                (crate::tables::SCALEFACTORS[(delta + 60) as usize] & 31) as u16;
                        }
                        if band == 0 || previous_book != Some(choice.book) {
                            cost += (4 + layout.section_bits()) as u16;
                        }
                        let candidate = choice.error + lambda * f32::from(cost);
                        if candidate < score {
                            best = choice;
                            score = candidate;
                        }
                    }
                    if best.book != 0 {
                        previous = Some(best.sf);
                    }
                    previous_book = Some(best.book);
                    result.push(best);
                }
            }
            result
        })
    }

    pub fn coefficient(&self, channel: usize, index: usize, sample: f32, factor: f32) -> i16 {
        quantize(sample, self.powers[channel][index], factor)
    }
}

pub(crate) fn gain(sf: u8) -> f32 {
    ((f32::from(sf) - 100.0) * 0.25).exp2()
}

fn quantize(sample: f32, power: f32, factor: f32) -> i16 {
    let magnitude = (power * factor + 0.4054).min(MAX_QUANTIZED as f32) as i16;
    if sample < 0.0 { -magnitude } else { magnitude }
}
