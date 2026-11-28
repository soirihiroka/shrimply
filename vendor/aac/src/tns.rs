//! AAC-LC temporal noise shaping using frequency-domain linear prediction.

use std::f64::consts::FRAC_PI_2;

use crate::{FRAME_SAMPLES, Layout, bits::Bits};

// ISO/IEC 13818-7 §7.1.6, long windows. Short windows already localize noise in time.
const MAX_BANDS: [usize; 12] = [31, 31, 34, 40, 42, 51, 46, 46, 42, 42, 42, 39];
const ORDER: usize = 6;

pub(crate) struct Filter {
    length: usize,
    codes: [i8; ORDER],
    order: usize,
}

impl Filter {
    pub fn apply(
        layout: Layout,
        rate_index: usize,
        rate: u32,
        spectrum: &mut [f32; FRAME_SAMPLES],
    ) -> Option<Self> {
        let end_band = MAX_BANDS[rate_index].min(layout.band_count());
        let start_band = layout.bands[..end_band]
            .partition_point(|&bin| bin * (rate as usize) < 1500 * 2 * FRAME_SAMPLES);
        let lines = &mut spectrum[layout.bands[start_band]..layout.bands[end_band]];
        if lines.len() < 4 * ORDER {
            return None;
        }

        let correlation: [f64; ORDER + 1] = std::array::from_fn(|lag| {
            lines
                .iter()
                .zip(&lines[lag..])
                .map(|(&before, &after)| f64::from(before) * f64::from(after))
                .sum()
        });
        if correlation[0] <= f32::EPSILON as f64 {
            return None;
        }
        let mut residual = correlation[0];
        let mut lpc = [0.0; ORDER];
        let mut codes = [0; ORDER];
        for index in 0..ORDER {
            let prediction: f64 = lpc[..index]
                .iter()
                .zip(correlation[1..index + 1].iter().rev())
                .map(|(coefficient, correlation)| coefficient * correlation)
                .sum();
            let reflection = (-(correlation[index + 1] + prediction) / residual).clamp(-0.85, 0.85);
            let scale = if reflection < 0.0 { 8.5 } else { 7.5 };
            let code = (reflection.asin() * scale / FRAC_PI_2).round() as i8;
            codes[index] = code;
            let reflection = (f64::from(code) * FRAC_PI_2 / scale).sin();
            let previous = lpc;
            for j in 0..index {
                lpc[j] += reflection * previous[index - 1 - j];
            }
            lpc[index] = reflection;
            residual *= 1.0 - reflection * reflection;
        }
        let order = codes.iter().rposition(|&code| code != 0)? + 1;
        let predict = |index: usize, lines: &[f32]| {
            let previous = &lines[index.saturating_sub(order)..index];
            f64::from(lines[index])
                + lpc
                    .iter()
                    .zip(previous.iter().rev())
                    .map(|(coefficient, &sample)| coefficient * f64::from(sample))
                    .sum::<f64>()
        };
        let residual: f64 = (0..lines.len())
            .map(|index| predict(index, lines).powi(2))
            .sum();
        if residual * 1.4 >= correlation[0] {
            return None;
        }
        // Write the FIR residual backwards to preserve preceding input samples.
        for index in (0..lines.len()).rev() {
            lines[index] = predict(index, lines) as f32;
        }
        Some(Self {
            length: layout.band_count() - start_band,
            codes,
            order,
        })
    }

    pub fn bits(&self) -> usize {
        16 + self.order * 4
    }

    pub fn write(&self, out: &mut Bits) {
        out.put(2, 1); // One long-window filter.
        out.put(1, 1); // Four-bit reflection coefficients.
        out.put(6, self.length as u32);
        out.put(5, self.order as u32);
        out.put(2, 0); // Low-to-high, uncompressed coefficients.
        for &code in &self.codes[..self.order] {
            out.put(4, u32::from(code as u8 & 15));
        }
    }
}
