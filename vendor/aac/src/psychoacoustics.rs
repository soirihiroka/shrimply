use crate::{FRAME_SAMPLES, Layout};

pub(crate) struct Masking {
    quiet: Vec<f32>,
    spreading: Vec<Vec<f32>>,
}

impl Masking {
    pub fn new(rate: u32, layout: Layout) -> Self {
        let bins = (FRAME_SAMPLES / layout.windows) as f32;
        let frequencies: Vec<_> = layout
            .bands
            .windows(2)
            .map(|band| (band[0] + band[1]) as f32 * rate as f32 / (4.0 * bins))
            .collect();
        let bark: Vec<_> = frequencies
            .iter()
            .map(|&hz| 13.0 * (0.00076 * hz).atan() + 3.5 * (hz / 7500.0).powi(2).atan())
            .collect();
        let quiet = frequencies
            .iter()
            .map(|&hz| {
                let khz = (hz / 1000.0).max(0.02);
                let threshold = 3.64 * khz.powf(-0.8) - 6.5 * (-0.6 * (khz - 3.3).powi(2)).exp()
                    + 0.001 * khz.powi(4);
                // Place full scale at 100 dB SPL, with a cap on the ultrasonic threshold.
                (32768.0 * bins).powi(2) * 10.0_f32.powf((threshold.min(45.0) - 100.0) / 10.0)
            })
            .collect();
        // Noise spreads farther upward in frequency than downward.
        let spreading = bark
            .iter()
            .map(|&target| {
                bark.iter()
                    .map(|&source| {
                        let spread_db =
                            if target > source { -16.0 } else { -27.0 } * (target - source).abs();
                        10.0_f32.powf(spread_db / 10.0)
                    })
                    .collect()
            })
            .collect();
        Self { quiet, spreading }
    }

    pub fn thresholds(&self, layout: Layout, spectrum: &[f32; FRAME_SAMPLES]) -> Vec<f32> {
        let mut thresholds = Vec::with_capacity(layout.groups * layout.band_count());
        for group in 0..layout.groups {
            let masker: Vec<_> = (0..layout.band_count())
                .map(|band| {
                    let mut energy = 0.0;
                    let mut log_energy = 0.0;
                    let mut count = 0;
                    for index in layout.ranges(group, band).flatten() {
                        let power = spectrum[index].powi(2);
                        energy += power;
                        log_energy += (power + 1.0).ln();
                        count += 1;
                    }
                    // Johnston's geometric-to-arithmetic mean ratio estimates tonality.
                    let mean = energy / count as f32;
                    let flatness = ((log_energy / count as f32).exp() / (mean + 1.0)).min(1.0);
                    let tonal = (-flatness.log10() / 6.0).min(1.0);
                    energy * 10.0_f32.powf((-16.0 - 13.0 * tonal) / 10.0)
                })
                .collect();
            thresholds.extend(self.spreading.iter().zip(&self.quiet).enumerate().map(
                |(band, (spreading, quiet))| {
                    let masking: f32 = masker
                        .iter()
                        .zip(spreading)
                        .map(|(power, weight)| power * weight)
                        .sum();
                    let width = layout.bands[band + 1] - layout.bands[band];
                    (masking + quiet) / (width as f32).sqrt()
                },
            ));
        }
        thresholds
    }
}
