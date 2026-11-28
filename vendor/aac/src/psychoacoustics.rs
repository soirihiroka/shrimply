use std::{f32::consts::PI, sync::Arc};

use rustfft::{Fft, FftPlanner, num_complex::Complex32};

use crate::{
    FRAME_SAMPLES, Layout,
    mdct::{OFFSET, SHORT},
};

/// Parseval scaling factor from positive-frequency Hann-FFT power |F[k]|^2
/// to AAC MDCT bin power X[k]^2: (32768^2) * (8 / 3).
const FFT_TO_MDCT_ENERGY: f32 = 32768.0 * 32768.0 * (8.0 / 3.0);
const MAX_SHORT_BANDS: usize = 16;

#[inline]
fn bark_of(hz: f32) -> f32 {
    13.0 * (0.00076 * hz).atan() + 3.5 * (hz / 7500.0).powi(2).atan()
}

#[inline]
fn bins_per_bark(start: usize, end: usize, hz_per_bin: f32) -> f32 {
    let dz = (bark_of(end as f32 * hz_per_bin) - bark_of(start as f32 * hz_per_bin)).max(0.05);
    ((end - start) as f32 / dz).max(1.0)
}

#[inline]
fn unit_vector(x: Complex32) -> (f32, Complex32) {
    let r = x.norm();
    let unit = if r > 1.0e-9 {
        x / r
    } else {
        Complex32::new(1.0, 0.0)
    };
    (r, unit)
}

#[inline]
fn spread_partition(part: &Partition, part_energy: &[f32], part_unpred: &[f32]) -> (f32, f32) {
    let mut eb = 0.0_f32;
    let mut ecb = 0.0_f32;
    let s0 = part.spread_start;
    for (offset, &w) in part.spread_weights.iter().enumerate() {
        eb += part_energy[s0 + offset] * w;
        ecb += part_unpred[s0 + offset] * w;
    }
    (eb, ecb)
}

#[inline]
fn clamped_band_threshold(
    bin_thr: &[f32],
    model: &FftModel,
    start: usize,
    end: usize,
    prev: &mut f32,
) -> f32 {
    let inv_sum: f32 = bin_thr[start..end].iter().map(|&t| 1.0 / t.max(1.0)).sum();
    let raw = ((end - start) as f32 / inv_sum.max(1.0e-30)).max(1.0);
    let ath_floor = model.partitions[model.bin_to_part[start]].ath_bin;
    let clamped = if *prev > 0.0 && raw > 2.0 * *prev {
        (2.0 * *prev).max(ath_floor)
    } else {
        raw
    };
    *prev = raw;
    clamped
}

struct Partition {
    start: usize,
    end: usize,
    ath_bin: f32,
    spread_start: usize,
    spread_weights: Vec<f32>,
}

struct FftModel {
    bins: usize,
    hz_per_bin: f32,
    fft: Arc<dyn Fft<f32>>,
    hann: Vec<f32>,
    buffer: Vec<Complex32>,
    scratch: Vec<Complex32>,
    partitions: Vec<Partition>,
    bin_to_part: Vec<usize>,
}

impl FftModel {
    fn new(rate: u32, bitrate: u32, bins: usize, planner: &mut FftPlanner<f32>) -> Self {
        let fft_size = 2 * bins;
        let fft = planner.plan_fft_forward(fft_size);
        let scratch = vec![Complex32::ZERO; fft.get_inplace_scratch_len()];
        let hann = (0..fft_size)
            .map(|n| 0.5 * (1.0 - (2.0 * PI * n as f32 / fft_size as f32).cos()))
            .collect();

        let hz_per_bin = rate as f32 / fft_size as f32;
        let step_bark = if bins == SHORT { 0.55 } else { 0.33 };
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        let mut start = 0;
        while start < bins {
            let z0 = bark_of((start as f32 + 0.5) * hz_per_bin);
            let mut end = start + 1;
            while end < bins && bark_of((end as f32 + 0.5) * hz_per_bin) - z0 < step_bark {
                end += 1;
            }
            ranges.push((start, end));
            start = end;
        }

        let centers: Vec<f32> = ranges
            .iter()
            .map(|&(s, e)| bark_of((s + e) as f32 * 0.5 * hz_per_bin))
            .collect();

        let bits_per_sample = bitrate as f32 / rate as f32;
        let ath_cap_db = if bits_per_sample < 1.6 {
            54.0
        } else if bits_per_sample < 2.3 {
            46.0
        } else if bits_per_sample < 3.2 {
            38.0
        } else if bits_per_sample < 4.5 {
            30.0
        } else {
            24.0
        };

        let full_scale_energy = (32768.0 * bins as f32).powi(2);

        let mut bin_to_part = vec![0; bins];
        let mut partitions = Vec::with_capacity(ranges.len());
        for (p, &(s, e)) in ranges.iter().enumerate() {
            for item in &mut bin_to_part[s..e] {
                *item = p;
            }
            let center_hz = (s + e) as f32 * 0.5 * hz_per_bin;
            let khz = (center_hz / 1000.0).max(0.18);
            let ultra = 0.0005 * khz.powi(4);
            let ath_db = (3.64 * khz.powf(-0.8) - 6.5 * (-0.6 * (khz - 3.3).powi(2)).exp() + ultra)
                .min(ath_cap_db);
            let ath_cb = full_scale_energy * 10.0_f32.powf((ath_db - 112.0) / 10.0);
            let ath_bin = (ath_cb / bins_per_bark(s, e, hz_per_bin)).max(1.0);

            // Asymmetric Schroeder spreading in Bark space, normalized per target partition
            // (paired with `eb.max(part_energy[p])` so neither broadband noise is inflated
            // nor isolated tonal peaks are attenuated).
            let z_target = centers[p];
            let mut spread_start = p;
            let mut weights = Vec::new();
            let mut started = false;
            for (src, &z_src) in centers.iter().enumerate() {
                let dz_s = z_target - z_src;
                let spread_db = if dz_s >= 0.0 {
                    -15.0 * dz_s
                } else {
                    28.0 * dz_s
                };
                let w = 10.0_f32.powf(spread_db / 10.0);
                if w >= 1.0e-5 {
                    if !started {
                        spread_start = src;
                        started = true;
                    }
                    weights.push(w);
                } else if started {
                    break;
                }
            }
            let sum: f32 = weights.iter().sum::<f32>().max(1.0e-12);
            for w in &mut weights {
                *w /= sum;
            }
            partitions.push(Partition {
                start: s,
                end: e,
                ath_bin,
                spread_start,
                spread_weights: weights,
            });
        }

        Self {
            bins,
            hz_per_bin,
            fft,
            hann,
            buffer: vec![Complex32::ZERO; fft_size],
            scratch,
            partitions,
            bin_to_part,
        }
    }

    /// Run one complex FFT for stereo (L in real, R in imag) and unpack positive frequencies.
    fn fft_stereo(
        &mut self,
        samples: &[[f32; 2]],
        out_l: &mut [Complex32],
        out_r: &mut [Complex32],
    ) {
        let m = self.buffer.len();
        for ((dst, sample), &win) in self.buffer.iter_mut().zip(samples).zip(&self.hann) {
            *dst = Complex32::new(sample[0] * win, sample[1] * win);
        }
        self.fft
            .process_with_scratch(&mut self.buffer, &mut self.scratch);
        for k in 0..self.bins {
            let a = self.buffer[k];
            let b = self.buffer[if k == 0 { 0 } else { m - k }].conj();
            out_l[k] = Complex32::new(0.5 * (a.re + b.re), 0.5 * (a.im + b.im));
            out_r[k] = Complex32::new(0.5 * (a.im - b.im), -0.5 * (a.re - b.re));
        }
    }

    /// Bark-width weighting factor `((e - s) / dz)^0.85` for scalefactor band `[s, e)`.
    /// Sub-linear cochlear critical-band weighting (Zwicker/Melkote & Rose) protects narrow
    /// low/mid fundamentals without starving 7–14 kHz harmonics on complex stereo ensembles.
    #[inline]
    fn bark_weight(&self, s: usize, e: usize) -> f32 {
        bins_per_bark(s, e, self.hz_per_bin).powf(0.85)
    }
}

pub(crate) struct Psychoacoustics {
    long: FftModel,
    short: FftModel,
    prev1_mag: [[f32; FRAME_SAMPLES]; 2],
    prev2_mag: [[f32; FRAME_SAMPLES]; 2],
    prev1_unit: [[Complex32; FRAME_SAMPLES]; 2],
    prev2_unit: [[Complex32; FRAME_SAMPLES]; 2],
    prev_long_thr: [Vec<f32>; 2],
    fft_l: Vec<Complex32>,
    fft_r: Vec<Complex32>,
    part_energy: Vec<f32>,
    part_unpred: Vec<f32>,
    bin_thr: Vec<f32>,
}

impl Psychoacoustics {
    pub fn new(rate: u32, bitrate: u32, long_bands: usize) -> Self {
        let mut planner = FftPlanner::new();
        let long = FftModel::new(rate, bitrate, FRAME_SAMPLES, &mut planner);
        let short = FftModel::new(rate, bitrate, SHORT, &mut planner);
        let max_parts = long.partitions.len().max(short.partitions.len());
        Self {
            long,
            short,
            prev1_mag: [[0.0; FRAME_SAMPLES]; 2],
            prev2_mag: [[0.0; FRAME_SAMPLES]; 2],
            prev1_unit: [[Complex32::new(1.0, 0.0); FRAME_SAMPLES]; 2],
            prev2_unit: [[Complex32::new(1.0, 0.0); FRAME_SAMPLES]; 2],
            prev_long_thr: [vec![0.0; long_bands], vec![0.0; long_bands]],
            fft_l: vec![Complex32::ZERO; FRAME_SAMPLES],
            fft_r: vec![Complex32::ZERO; FRAME_SAMPLES],
            part_energy: vec![0.0; max_parts],
            part_unpred: vec![0.0; max_parts],
            bin_thr: vec![0.0; FRAME_SAMPLES],
        }
    }

    /// Compute effective Bark-weighted masking denominators `T_eff = ((width / dz)^0.85) * T_bin`
    /// for L and R, plus frame Perceptual Entropy.
    pub fn analyze(
        &mut self,
        layout: Layout,
        samples: &[[f32; 2]],
        mdct_spectra: &[[f32; FRAME_SAMPLES]; 2],
    ) -> ([Vec<f32>; 2], f32) {
        self.long
            .fft_stereo(samples, &mut self.fft_l, &mut self.fft_r);

        if layout.windows == 1 {
            let mut thr_bin_sfb = [
                Vec::with_capacity(layout.band_count()),
                Vec::with_capacity(layout.band_count()),
            ];
            let mut thr_eff = [
                Vec::with_capacity(layout.band_count()),
                Vec::with_capacity(layout.band_count()),
            ];
            for c in 0..2 {
                self.compute_long_bin_thresholds(c);
                for band in 0..layout.band_count() {
                    let s = layout.bands[band];
                    let e = layout.bands[band + 1];
                    let clamped = clamped_band_threshold(
                        &self.bin_thr,
                        &self.long,
                        s,
                        e,
                        &mut self.prev_long_thr[c][band],
                    );
                    thr_bin_sfb[c].push(clamped);
                    thr_eff[c].push(clamped * self.long.bark_weight(s, e));
                }
            }
            let pe = perceptual_entropy(layout, mdct_spectra, &thr_bin_sfb);
            (thr_eff, pe)
        } else {
            for c in 0..2 {
                self.update_long_history_only(c);
            }
            let total = layout.groups * layout.band_count();
            let mut thr_bin_sfb = [vec![f32::INFINITY; total], vec![f32::INFINITY; total]];
            let mut prev_win_thr = [[0.0_f32; MAX_SHORT_BANDS]; 2];
            let mut short_l = [Complex32::ZERO; SHORT];
            let mut short_r = [Complex32::ZERO; SHORT];
            let mut group_idx = 0;
            let mut in_group = 0;
            for w in 0..8 {
                let start = OFFSET + w * SHORT;
                self.short.fft_stereo(
                    &samples[start..start + 2 * SHORT],
                    &mut short_l,
                    &mut short_r,
                );
                for (c, spec) in [&short_l[..], &short_r[..]].into_iter().enumerate() {
                    self.compute_short_bin_thresholds(spec);
                    for band in 0..layout.band_count() {
                        let s = layout.bands[band];
                        let e = layout.bands[band + 1];
                        let clamped = clamped_band_threshold(
                            &self.bin_thr,
                            &self.short,
                            s,
                            e,
                            &mut prev_win_thr[c][band],
                        );
                        let slot = group_idx * layout.band_count() + band;
                        thr_bin_sfb[c][slot] = thr_bin_sfb[c][slot].min(clamped);
                    }
                }
                in_group += 1;
                if in_group >= layout.group_lens[group_idx] {
                    group_idx += 1;
                    in_group = 0;
                }
            }
            for prev in &mut self.prev_long_thr {
                prev.fill(0.0);
            }
            let mut thr_eff = [Vec::with_capacity(total), Vec::with_capacity(total)];
            for c in 0..2 {
                for g in 0..layout.groups {
                    let g_len = layout.group_lens[g] as f32;
                    for band in 0..layout.band_count() {
                        let s = layout.bands[band];
                        let e = layout.bands[band + 1];
                        let slot = g * layout.band_count() + band;
                        // Scale by group_lens[g] so each short window group is weighted consistently.
                        thr_eff[c]
                            .push(thr_bin_sfb[c][slot] * self.short.bark_weight(s, e) * g_len);
                    }
                }
            }
            let pe = perceptual_entropy(layout, mdct_spectra, &thr_bin_sfb);
            (thr_eff, pe)
        }
    }

    #[inline]
    fn advance_long_history(&mut self, channel: usize, k: usize, r: f32, unit: Complex32) {
        self.prev2_mag[channel][k] = self.prev1_mag[channel][k];
        self.prev2_unit[channel][k] = self.prev1_unit[channel][k];
        self.prev1_mag[channel][k] = r;
        self.prev1_unit[channel][k] = unit;
    }

    fn update_long_history_only(&mut self, channel: usize) {
        for k in 0..FRAME_SAMPLES {
            let x = if channel == 0 {
                self.fft_l[k]
            } else {
                self.fft_r[k]
            };
            let (r, unit) = unit_vector(x);
            self.advance_long_history(channel, k, r, unit);
        }
    }

    fn compute_long_bin_thresholds(&mut self, channel: usize) {
        let fft = if channel == 0 {
            &self.fft_l
        } else {
            &self.fft_r
        };
        let n_parts = self.long.partitions.len();
        self.part_energy[..n_parts].fill(0.0);
        self.part_unpred[..n_parts].fill(0.0);

        // Precompute per-bin power so local spectral peakiness uses a uniform 7-bin neighborhood
        // across all frequencies (no discontinuity where 1/3-Bark partitions narrow to 1 bin).
        let mut bin_pow = [0.0_f32; FRAME_SAMPLES];
        for (k, &x) in fft.iter().enumerate() {
            bin_pow[k] = x.norm_sqr() * FFT_TO_MDCT_ENERGY;
        }

        for p in 0..n_parts {
            let start = self.long.partitions[p].start;
            let end = self.long.partitions[p].end;
            let mut energy = 0.0_f32;
            let mut unpred_energy = 0.0_f32;
            for k in start..end {
                let x = if channel == 0 {
                    self.fft_l[k]
                } else {
                    self.fft_r[k]
                };
                let (r, unit) = unit_vector(x);
                let r1 = self.prev1_mag[channel][k];
                let r2 = self.prev2_mag[channel][k];
                let u1 = self.prev1_unit[channel][k];
                let u2 = self.prev2_unit[channel][k];
                let r_pred = (2.0 * r1 - r2).max(0.0);
                let pred = (u1 * u1 * u2.conj()) * r_pred;
                let c_phase = ((x - pred).norm() / (r + r_pred + 1.0e-6)).min(1.0);
                self.advance_long_history(channel, k, r, unit);

                // Uniform 7-bin local spectral peakiness (using 2-bin peak pair so tones split
                // between adjacent bins or close harmonic chords still register as tonal).
                let lo = k.saturating_sub(3);
                let hi = (k + 4).min(FRAME_SAMPLES);
                let local_sum: f32 = bin_pow[lo..hi].iter().sum();
                let left = if k > 0 { bin_pow[k - 1] } else { 0.0 };
                let right = if k + 1 < FRAME_SAMPLES {
                    bin_pow[k + 1]
                } else {
                    0.0
                };
                let pair_peak = bin_pow[k] + left.max(right);
                let local_mean = local_sum / (hi - lo) as f32;
                let peak_ratio = (pair_peak / (2.0 * local_mean + 1.0)).clamp(1.0, 3.5);
                let c_spec = (1.0 - (peak_ratio - 1.0) / 2.5).clamp(0.08, 1.0);
                let c = c_phase.min(c_spec);

                let bin_power = bin_pow[k];
                energy += bin_power;
                unpred_energy += bin_power * c;
            }
            self.part_energy[p] = energy;
            self.part_unpred[p] = unpred_energy;
        }

        let frame_peak_eb = self
            .part_energy
            .iter()
            .take(n_parts)
            .copied()
            .fold(1.0_f32, f32::max);
        for (p, part) in self.long.partitions.iter().enumerate() {
            let (eb, ecb) = spread_partition(part, &self.part_energy, &self.part_unpred);
            let cb = (ecb / (eb + 1.0e-12)).clamp(1.0e-4, 1.0);
            let tonality = (-0.299 - 0.43 * cb.ln()).clamp(0.0, 1.0);
            let eff_eb = eb.max(self.part_energy[p]);
            // Gentle nonlinear cochlear level-dependent masking: dominant loud bands get
            // slightly stricter SNR without starving quieter 7-14 kHz ensemble harmonics.
            let rel_db = 10.0 * ((eff_eb + 1.0) / frame_peak_eb).log10();
            let level_boost = (rel_db * 0.07).clamp(-3.0, 0.0);
            let snr_db = 16.0 + 13.0 * tonality + level_boost;
            let width = (part.end - part.start) as f32;
            let masked_bin = (eff_eb * 10.0_f32.powf(-snr_db / 10.0)) / width;
            self.bin_thr[part.start..part.end].fill(masked_bin.max(part.ath_bin));
        }
    }

    fn compute_short_bin_thresholds(&mut self, fft: &[Complex32]) {
        let n_parts = self.short.partitions.len();
        self.part_energy[..n_parts].fill(0.0);
        self.part_unpred[..n_parts].fill(0.0);

        for (p, part) in self.short.partitions.iter().enumerate() {
            let mut energy = 0.0_f32;
            let mut log_sum = 0.0_f32;
            let width = (part.end - part.start) as f32;
            for &x in &fft[part.start..part.end] {
                let bin_power = x.norm_sqr() * FFT_TO_MDCT_ENERGY;
                energy += bin_power;
                log_sum += (bin_power + 1.0).ln();
            }
            let mean = energy / width;
            let sfm = ((log_sum / width).exp() / (mean + 1.0)).clamp(1.0e-4, 1.0);
            self.part_energy[p] = energy;
            self.part_unpred[p] = energy * sfm;
        }

        for (p, part) in self.short.partitions.iter().enumerate() {
            let (eb, ecb) = spread_partition(part, &self.part_energy, &self.part_unpred);
            let flatness = (ecb / (eb + 1.0e-12)).clamp(1.0e-4, 1.0);
            let tonality = (-flatness.log10() / 4.0).clamp(0.0, 1.0);
            let snr_db = 12.0 + 14.0 * tonality;
            let eff_eb = eb.max(self.part_energy[p]);
            let width = (part.end - part.start) as f32;
            let masked_bin = (eff_eb * 10.0_f32.powf(-snr_db / 10.0)) / width;
            self.bin_thr[part.start..part.end].fill(masked_bin.max(part.ath_bin));
        }
    }
}

fn perceptual_entropy(
    layout: Layout,
    spectra: &[[f32; FRAME_SAMPLES]; 2],
    thr_bin: &[Vec<f32>; 2],
) -> f32 {
    let mut pe = 0.0_f32;
    for (spectrum, masks) in spectra.iter().zip(thr_bin) {
        for group in 0..layout.groups {
            for band in 0..layout.band_count() {
                let mut energy = 0.0_f32;
                let mut width = 0_usize;
                for i in layout.ranges(group, band).flatten() {
                    energy += spectrum[i] * spectrum[i];
                    width += 1;
                }
                let band_mask = masks[group * layout.band_count() + band] * width as f32;
                if energy > band_mask {
                    pe += width as f32 * 0.5 * (energy / band_mask).log2();
                }
            }
        }
    }
    pe
}
