use std::sync::LazyLock;

use crate::{
    FRAME_SAMPLES, Layout,
    bits::{ACTIVE_BOOKS, CODEBOOK_COUNT, Codebook, code_len, codebook_costs, escape_bits},
    tables::{SCALEFACTORS, SPECTRUM},
};

pub(crate) const SF_DELTA_MAX: i16 = 60;
pub(crate) const SF_OFFSET: u8 = 100;

const MAX_QUANTIZED: usize = 8191;
const RDOQ_DEADZONE: f32 = 0.12;
const ZERO_STAY: u16 = u16::MAX;
const FROM_NONE: u16 = u16::MAX;
const FROM_ZERO_FLAG: u16 = 0x8000;

/// ISO/IEC 13818-7 Table 4.42 `ms_mask_present` stereo coding mode.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum MsMode {
    #[default]
    None = 0,
    PerBand = 1,
    All = 2,
}

#[derive(Clone, Copy)]
pub(crate) struct StereoFrame<'a> {
    pub layout: Layout,
    pub spectra_lr: &'a [[f32; FRAME_SAMPLES]; 2],
    pub spectra_ms: &'a [[f32; FRAME_SAMPLES]; 2],
    pub thr_lr: &'a [Vec<f32>; 2],
    pub thr_ms: &'a [f32],
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Choice {
    pub sf: u8,
    pub book: Codebook,
    pub cost: u16,
    pub error: f32,
}

#[derive(Clone, Copy)]
struct SfCandidate {
    sf: u8,
    errors: [f32; CODEBOOK_COUNT],
    costs: [u16; CODEBOOK_COUNT],
    min_cost: u16,
    min_error: f32,
}

impl SfCandidate {
    fn new(sf: u8) -> Self {
        Self {
            sf,
            errors: [f32::INFINITY; CODEBOOK_COUNT],
            costs: [u16::MAX; CODEBOOK_COUNT],
            min_cost: u16::MAX,
            min_error: f32::INFINITY,
        }
    }

    #[inline]
    fn record(&mut self, index: usize, err: f32, bits: u16) {
        if bits != u16::MAX {
            self.errors[index] = err;
            self.costs[index] = bits;
            self.min_cost = self.min_cost.min(bits);
            self.min_error = self.min_error.min(err);
        }
    }
}

#[derive(Default, Clone)]
struct BandCandidates {
    zero_error: f32,
    min_sf: u8,
    max_sf: u8,
    cands: Vec<SfCandidate>,
}

/// Reusable scratch buffers for exact 2D Viterbi DP (zero per-call heap churn).
struct ViterbiScratch {
    suffix_zero: Vec<f32>,
    active_back: Vec<Vec<[(u16, Codebook); CODEBOOK_COUNT]>>,
    // Packs `(prev_k << 4) | prev_cb.id()` when entering zero_dp from active, or ZERO_STAY.
    zero_back: Vec<[u16; 256]>,
    zero_dp: [f32; 256],
    next_zero_dp: [f32; 256],
    zero_sfs: Vec<u8>,
    next_zero_sfs: Vec<u8>,
    switch_cost: [f32; 256],
    switch_src: [(u16, Codebook); 256],
    switch_sfs: Vec<u8>,
    // Per-codebook compact frontier: (prev_k, prev_sf, dp_cost).
    prev_by_cb: [Vec<(u16, u8, f32)>; CODEBOOK_COUNT],
    cur_by_cb: [Vec<(u16, u8, f32)>; CODEBOOK_COUNT],
    prev_sf: Vec<u8>,
    prev_best: Vec<(f32, Codebook)>,
    cur_sf: Vec<u8>,
    cur_best: Vec<(f32, Codebook)>,
}

impl ViterbiScratch {
    fn new() -> Self {
        Self {
            suffix_zero: Vec::with_capacity(128),
            active_back: Vec::with_capacity(128),
            zero_back: Vec::with_capacity(128),
            zero_dp: [f32::INFINITY; 256],
            next_zero_dp: [f32::INFINITY; 256],
            zero_sfs: Vec::with_capacity(64),
            next_zero_sfs: Vec::with_capacity(64),
            switch_cost: [f32::INFINITY; 256],
            switch_src: [(FROM_NONE, Codebook::Zero); 256],
            switch_sfs: Vec::with_capacity(96),
            prev_by_cb: std::array::from_fn(|_| Vec::with_capacity(40)),
            cur_by_cb: std::array::from_fn(|_| Vec::with_capacity(40)),
            prev_sf: Vec::with_capacity(40),
            prev_best: Vec::with_capacity(40),
            cur_sf: Vec::with_capacity(40),
            cur_best: Vec::with_capacity(40),
        }
    }
}

pub(crate) struct Search {
    inverse: Vec<f32>,
    boundaries: Vec<f32>,
    // Pre-analyzed candidates for L, R (0, 1) and M, S (2, 3).
    bands: [Vec<BandCandidates>; 4],
    // Stage 2 tuple-RDOQ candidates for active stereo mode (0, 1).
    rdoq_bands: [Vec<BandCandidates>; 2],
    powers: [[f32; FRAME_SAMPLES]; 4],
    viterbi: ViterbiScratch,
    pub quantized: [[i16; FRAME_SAMPLES]; 2],
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
            bands: std::array::from_fn(|_| (0..128).map(|_| BandCandidates::default()).collect()),
            rdoq_bands: std::array::from_fn(|_| {
                (0..128).map(|_| BandCandidates::default()).collect()
            }),
            powers: [[0.0; FRAME_SAMPLES]; 4],
            viterbi: ViterbiScratch::new(),
            quantized: [[0; FRAME_SAMPLES]; 2],
        }
    }

    /// Analyze all four representations (L, R, M, S) with full per-codebook costs.
    pub fn analyze_stereo(&mut self, frame: StereoFrame<'_>) {
        let layout = frame.layout;
        let sources = [
            (&frame.spectra_lr[0], &frame.thr_lr[0][..]),
            (&frame.spectra_lr[1], &frame.thr_lr[1][..]),
            (&frame.spectra_ms[0], frame.thr_ms),
            (&frame.spectra_ms[1], frame.thr_ms),
        ];
        let mut qbuf = [0_i16; FRAME_SAMPLES];
        for (slot_ch, (source, masks)) in sources.into_iter().enumerate() {
            let powers = &mut self.powers[slot_ch];
            for (power, &sample) in powers.iter_mut().zip(source) {
                let root = sample.abs().sqrt();
                *power = root * root.sqrt();
            }
            let total_slots = layout.groups * layout.band_count();
            if self.bands[slot_ch].len() < total_slots {
                self.bands[slot_ch].resize_with(total_slots, BandCandidates::default);
            }
            for group in 0..layout.groups {
                for band in 0..layout.band_count() {
                    let idx = group * layout.band_count() + band;
                    let entry = &mut self.bands[slot_ch][idx];
                    entry.cands.clear();
                    let mask = masks[idx].max(1.0);
                    let mut energy = 0.0_f32;
                    let mut peak = 0.0_f32;
                    let mut width = 0_usize;
                    for i in layout.ranges(group, band).flatten() {
                        energy += source[i] * source[i];
                        peak = peak.max(powers[i]);
                        width += 1;
                    }
                    // Smooth energy-envelope distortion weight: penalizes both spectral holes
                    // and severe low-bitrate band energy collapse continuously (matches 1.85x at E_hat = 0).
                    let raw_zero_err = energy / mask;
                    let env_weight = band_env_weight(raw_zero_err);
                    entry.zero_error = raw_zero_err * (1.0 + env_weight);
                    if energy == 0.0 {
                        entry.min_sf = SF_OFFSET;
                        entry.max_sf = SF_OFFSET;
                        continue;
                    }
                    let orig_norm = energy.sqrt();
                    let sf_offset = f32::from(SF_OFFSET);
                    let rms = (energy / width as f32).sqrt();
                    let bin_mask = mask / width as f32;
                    let est_scale = (6.75 * bin_mask / rms.sqrt().max(1.0e-6)).powf(2.0 / 3.0);
                    let masked_sf = (sf_offset + 4.0 * est_scale.log2()) as i32;
                    let min_sf = (sf_offset + (16.0 / 3.0) * (peak / MAX_QUANTIZED as f32).log2())
                        .ceil() as i32;
                    let max_sf = (sf_offset + (16.0 / 3.0) * (peak / 0.45).log2()).ceil() as i32;
                    let min_sf = min_sf.clamp(0, 255);
                    let max_sf = max_sf.clamp(min_sf, 255);
                    entry.min_sf = min_sf as u8;
                    entry.max_sf = max_sf as u8;

                    // Stage 1 uses step = 2 around the masked scalefactor (~20 candidates);
                    // Stage 2 refines with exact step = 1 tuple-RDOQ around the Viterbi path.
                    let fine_lo = (masked_sf - 24).max(min_sf).min(max_sf);
                    let fine_hi = (masked_sf + 24).min(max_sf).max(fine_lo);
                    let coarse_lo = (fine_lo - 24).max(min_sf);
                    let coarse_hi = (fine_hi + 24).min(max_sf);

                    let mut sfs = [0_u8; 38];
                    let mut n_sfs = 0;
                    let mut sf = coarse_lo;
                    while sf <= coarse_hi && n_sfs < sfs.len() {
                        sfs[n_sfs] = sf as u8;
                        n_sfs += 1;
                        sf += if sf >= fine_lo && sf < fine_hi { 2 } else { 4 };
                    }
                    if n_sfs > 0 && sfs[n_sfs - 1] < coarse_hi as u8 && n_sfs < sfs.len() {
                        sfs[n_sfs] = coarse_hi as u8;
                        n_sfs += 1;
                    }

                    for &sf in &sfs[..n_sfs] {
                        let (scale, factor) = gains(sf);
                        let mut sq_err = 0.0_f32;
                        let mut hat_energy = 0.0_f32;
                        let mut high = 0_u16;
                        for (value, i) in qbuf.iter_mut().zip(layout.ranges(group, band).flatten())
                        {
                            *value =
                                quantize_scalar(source[i], powers[i], factor, &self.boundaries);
                            let mag = value.unsigned_abs();
                            high = high.max(mag);
                            let deq = self.inverse[usize::from(mag)] * scale;
                            let diff = source[i].abs() - deq;
                            sq_err += diff * diff;
                            hat_energy += deq * deq;
                        }
                        if high == 0 {
                            break;
                        }
                        let costs = codebook_costs(&qbuf[..width], high);
                        let min_cost = costs[1..].iter().copied().min().unwrap_or(u16::MAX);
                        let env_diff = orig_norm - hat_energy.sqrt();
                        let err = (sq_err + env_weight * env_diff * env_diff) / mask;
                        let mut errors = [f32::INFINITY; CODEBOOK_COUNT];
                        for cb in ACTIVE_BOOKS {
                            let ci = cb.index();
                            if costs[ci] != u16::MAX {
                                errors[ci] = err;
                            }
                        }
                        entry.cands.push(SfCandidate {
                            sf,
                            errors,
                            costs,
                            min_cost,
                            min_error: err,
                        });
                    }
                }
            }
        }
    }

    /// Choose stereo coding mode (None, PerBand, or All M/S) via exact R-D comparison at `lambda`.
    pub fn choose_ms(&self, layout: Layout, lambda: f32, ms_mask: &mut Vec<bool>) -> MsMode {
        let total = layout.groups * layout.band_count();
        ms_mask.clear();
        ms_mask.resize(total, false);
        let mut sum_lr = 0.0_f32;
        let mut sum_ms = 0.0_f32;
        let mut sum_hybrid = 0.0_f32;
        let mut active_bands = 0_usize;
        for idx in 0..total {
            let lr_cost = band_best_rd(&self.bands[0][idx], lambda)
                + band_best_rd(&self.bands[1][idx], lambda);
            let ms_cost = band_best_rd(&self.bands[2][idx], lambda)
                + band_best_rd(&self.bands[3][idx], lambda);
            sum_lr += lr_cost;
            sum_ms += ms_cost;
            if ms_cost < lr_cost {
                ms_mask[idx] = true;
                sum_hybrid += ms_cost;
            } else {
                sum_hybrid += lr_cost;
            }
            if self.bands[0][idx].zero_error > 0.0 || self.bands[1][idx].zero_error > 0.0 {
                active_bands = active_bands.max((idx % layout.band_count()) + 1);
            }
        }
        let hybrid_total = sum_hybrid + lambda * (active_bands * layout.groups) as f32;
        if sum_ms <= sum_lr && sum_ms <= hybrid_total {
            ms_mask.fill(true);
            MsMode::All
        } else if hybrid_total < sum_lr {
            MsMode::PerBand
        } else {
            ms_mask.fill(false);
            MsMode::None
        }
    }

    /// Stage 1 exact 2D Viterbi DP over (sf, cb, max_sfb) using the active stereo mode.
    pub fn select_stage1(
        &mut self,
        layout: Layout,
        ms_mask: &[bool],
        ms_mode: MsMode,
        lambda: f32,
        output: &mut [Vec<Choice>; 2],
    ) {
        let ms_penalty = if ms_mode == MsMode::PerBand {
            0.5 * lambda
        } else {
            0.0
        };
        let bands = &self.bands;
        let viterbi = &mut self.viterbi;
        for (ch, out) in output.iter_mut().enumerate() {
            viterbi_channel(
                viterbi,
                layout,
                |idx| &bands[if ms_mask[idx] { 2 + ch } else { ch }][idx],
                lambda,
                ms_penalty,
                out,
            );
        }
    }

    /// Stage 2: build exact tuple-RDOQ candidate tables around `center` choices at `lambda`,
    /// including smaller codebooks that clamp outlier peaks to save Huffman bits.
    pub fn build_rdoq(
        &mut self,
        frame: StereoFrame<'_>,
        ms_mask: &[bool],
        center: &[Vec<Choice>; 2],
        lambda: f32,
        radius: i32,
    ) {
        let layout = frame.layout;
        let total = layout.groups * layout.band_count();
        let mut xbuf = [0.0_f32; FRAME_SAMPLES];
        let mut pbuf = [0.0_f32; FRAME_SAMPLES];
        for ch in 0..2 {
            if self.rdoq_bands[ch].len() < total {
                self.rdoq_bands[ch].resize_with(total, BandCandidates::default);
            }
            for idx in 0..total {
                let is_ms = ms_mask[idx];
                let src_ch = if is_ms { 2 + ch } else { ch };
                let base = &self.bands[src_ch][idx];
                let (min_sf, max_sf, zero_error, has_cands) = (
                    base.min_sf,
                    base.max_sf,
                    base.zero_error,
                    !base.cands.is_empty(),
                );
                let entry = &mut self.rdoq_bands[ch][idx];
                entry.zero_error = zero_error;
                entry.min_sf = min_sf;
                entry.max_sf = max_sf;
                entry.cands.clear();
                if zero_error == 0.0 || !has_cands {
                    continue;
                }

                let pick = center[ch][idx];
                let (lo, hi) = if pick.book != Codebook::Zero {
                    let c = i32::from(pick.sf);
                    (
                        (c - radius).max(i32::from(min_sf)),
                        (c + radius).min(i32::from(max_sf)),
                    )
                } else {
                    let hi = i32::from(max_sf);
                    let lo = (hi - radius.max(3)).max(i32::from(min_sf));
                    (lo, hi)
                };

                let (width, inv_mask) =
                    self.gather_band(frame, ch, idx, is_ms, &mut xbuf, &mut pbuf);
                let entry = &mut self.rdoq_bands[ch][idx];
                for sf in lo..=hi {
                    if let Some(cand) = rdoq_all_books(
                        &xbuf[..width],
                        &pbuf[..width],
                        sf as u8,
                        inv_mask,
                        lambda,
                        &self.inverse,
                    ) {
                        entry.cands.push(cand);
                    }
                }
            }
        }
    }

    /// Run exact 2D Viterbi DP on Stage 2 RDOQ tables.
    pub fn select_stage2(
        &mut self,
        layout: Layout,
        ms_mode: MsMode,
        lambda: f32,
        output: &mut [Vec<Choice>; 2],
    ) {
        let ms_penalty = if ms_mode == MsMode::PerBand {
            0.5 * lambda
        } else {
            0.0
        };
        let rdoq_bands = &self.rdoq_bands;
        let viterbi = &mut self.viterbi;
        for (ch, out) in output.iter_mut().enumerate() {
            viterbi_channel(
                viterbi,
                layout,
                |idx| &rdoq_bands[ch][idx],
                lambda,
                ms_penalty,
                out,
            );
        }
    }

    /// Materialize the exact RDOQ-quantized `i16` spectral lines at the exact `build_lambda`
    /// used to construct `rdoq_bands`, guaranteeing bit-exact agreement with `packet_bytes()`.
    pub fn materialize_quantized(
        &mut self,
        frame: StereoFrame<'_>,
        ms_mask: &[bool],
        picks: &mut [Vec<Choice>; 2],
        build_lambda: f32,
    ) {
        let layout = frame.layout;
        let total = layout.groups * layout.band_count();
        self.quantized = [[0; FRAME_SAMPLES]; 2];
        let mut xbuf = [0.0_f32; FRAME_SAMPLES];
        let mut pbuf = [0.0_f32; FRAME_SAMPLES];
        let mut qbuf = [0_i16; FRAME_SAMPLES];

        for ch in 0..2 {
            for idx in 0..total {
                let choice = &mut picks[ch][idx];
                if choice.book == Codebook::Zero {
                    continue;
                }
                let (width, inv_mask) =
                    self.gather_band(frame, ch, idx, ms_mask[idx], &mut xbuf, &mut pbuf);
                let (err, bits, nonzero) = rdoq_single_book(
                    &RdoqCtx::new(
                        &xbuf[..width],
                        &pbuf[..width],
                        choice.sf,
                        inv_mask,
                        build_lambda,
                        &self.inverse,
                    ),
                    choice.book,
                    &mut qbuf[..width],
                );
                debug_assert!(nonzero);
                if nonzero {
                    choice.error = err;
                    choice.cost = bits;
                    self.scatter_band(layout, ch, idx, &qbuf[..width]);
                } else {
                    *choice = Choice {
                        sf: 0,
                        book: Codebook::Zero,
                        cost: 0,
                        error: self.rdoq_bands[ch][idx].zero_error,
                    };
                }
            }
        }
    }

    /// Spend leftover bytes below `limit` on the highest-distortion bands using exact RDOQ.
    pub fn polish_residual<F>(
        &mut self,
        frame: StereoFrame<'_>,
        ms_mask: &[bool],
        picks: &mut [Vec<Choice>; 2],
        lambda: f32,
        limit: usize,
        size_fn: F,
    ) where
        F: Fn(&[Vec<Choice>; 2]) -> usize,
    {
        if size_fn(picks) >= limit {
            return;
        }
        let mut order: Vec<(usize, usize, f32)> = picks
            .iter()
            .enumerate()
            .flat_map(|(ch, vec)| {
                vec.iter()
                    .enumerate()
                    .map(move |(idx, c)| (ch, idx, c.error))
            })
            .filter(|&(_, _, err)| err > 0.0)
            .collect();
        order.sort_unstable_by(|a, b| b.2.total_cmp(&a.2));

        let mut xbuf = [0.0_f32; FRAME_SAMPLES];
        let mut pbuf = [0.0_f32; FRAME_SAMPLES];
        let mut qbuf = [0_i16; FRAME_SAMPLES];
        let polish_lambda = lambda * 0.4;
        let max_delta = SF_DELTA_MAX as u8;

        for (ch, idx, _) in order.into_iter().take(8) {
            if size_fn(picks) >= limit {
                break;
            }
            let prev_sf = picks[ch][..idx]
                .iter()
                .rfind(|c| c.book != Codebook::Zero)
                .map(|c| c.sf);
            let next_sf = picks[ch][idx + 1..]
                .iter()
                .find(|c| c.book != Codebook::Zero)
                .map(|c| c.sf);

            let is_ms = ms_mask[idx];
            let src_ch = if is_ms { 2 + ch } else { ch };
            let (width, inv_mask) = self.gather_band(frame, ch, idx, is_ms, &mut xbuf, &mut pbuf);

            let base = &self.bands[src_ch][idx];
            let cur = picks[ch][idx];
            let mut best = cur;
            let mut updated = false;

            let sf_lo = if cur.book != Codebook::Zero {
                i32::from(cur.sf)
                    .saturating_sub(3)
                    .max(i32::from(base.min_sf))
            } else {
                i32::from(base.max_sf)
                    .saturating_sub(4)
                    .max(i32::from(base.min_sf))
            };
            let sf_hi = if cur.book != Codebook::Zero {
                i32::from(cur.sf)
            } else {
                i32::from(base.max_sf)
            };

            for sf in sf_lo..=sf_hi {
                let sf = sf as u8;
                if prev_sf.is_some_and(|p| p.abs_diff(sf) > max_delta)
                    || next_sf.is_some_and(|n| n.abs_diff(sf) > max_delta)
                {
                    continue;
                }
                if let Some(cand) = rdoq_all_books(
                    &xbuf[..width],
                    &pbuf[..width],
                    sf,
                    inv_mask,
                    polish_lambda,
                    &self.inverse,
                ) {
                    for cb in ACTIVE_BOOKS {
                        let ci = cb.index();
                        let err = cand.errors[ci];
                        let cost = cand.costs[ci];
                        if cost == u16::MAX || err >= best.error {
                            continue;
                        }
                        let trial = Choice {
                            sf,
                            book: cb,
                            cost,
                            error: err,
                        };
                        picks[ch][idx] = trial;
                        if size_fn(picks) <= limit {
                            best = trial;
                            updated = true;
                        }
                    }
                }
            }
            picks[ch][idx] = best;
            if updated {
                let (err, bits, nonzero) = rdoq_single_book(
                    &RdoqCtx::new(
                        &xbuf[..width],
                        &pbuf[..width],
                        best.sf,
                        inv_mask,
                        polish_lambda,
                        &self.inverse,
                    ),
                    best.book,
                    &mut qbuf[..width],
                );
                if nonzero {
                    picks[ch][idx].error = err;
                    picks[ch][idx].cost = bits;
                    self.scatter_band(frame.layout, ch, idx, &qbuf[..width]);
                } else {
                    picks[ch][idx] = cur;
                }
            }
        }
    }

    fn gather_band(
        &self,
        frame: StereoFrame<'_>,
        ch: usize,
        idx: usize,
        is_ms: bool,
        xs: &mut [f32; FRAME_SAMPLES],
        ps: &mut [f32; FRAME_SAMPLES],
    ) -> (usize, f32) {
        let group = idx / frame.layout.band_count();
        let band = idx % frame.layout.band_count();
        let src_ch = if is_ms { 2 + ch } else { ch };
        let source = if is_ms {
            &frame.spectra_ms[ch]
        } else {
            &frame.spectra_lr[ch]
        };
        let mask = if is_ms {
            frame.thr_ms[idx]
        } else {
            frame.thr_lr[ch][idx]
        }
        .max(1.0);
        let mut width = 0;
        for i in frame.layout.ranges(group, band).flatten() {
            xs[width] = source[i];
            ps[width] = self.powers[src_ch][i];
            width += 1;
        }
        (width, 1.0 / mask)
    }

    fn scatter_band(&mut self, layout: Layout, ch: usize, idx: usize, qbuf: &[i16]) {
        let group = idx / layout.band_count();
        let band = idx % layout.band_count();
        for (pos, i) in layout.ranges(group, band).flatten().enumerate() {
            self.quantized[ch][i] = qbuf[pos];
        }
    }
}

fn band_best_rd(band: &BandCandidates, lambda: f32) -> f32 {
    let mut best = band.zero_error;
    for cand in &band.cands {
        if cand.min_cost != u16::MAX {
            let j = cand.min_error + lambda * f32::from(cand.min_cost);
            if j < best {
                best = j;
            }
        }
    }
    best
}

/// Exact 2D Viterbi DP over (scalefactor, codebook, zero-runs, and max_sfb truncation).
fn viterbi_channel<'a>(
    sc: &mut ViterbiScratch,
    layout: Layout,
    band_at: impl Fn(usize) -> &'a BandCandidates,
    lambda: f32,
    ms_penalty: f32,
    output: &mut Vec<Choice>,
) {
    let b_count = layout.band_count();
    let total = layout.groups * b_count;
    let sec_hdr_bits = (4 + layout.section_bits()) as u16;
    let sec_hdr = lambda * f32::from(sec_hdr_bits);
    let sf_bits_lut: [f32; 121] =
        std::array::from_fn(|i| lambda * f32::from(code_len(SCALEFACTORS[i])));

    sc.suffix_zero.resize(total + 1, 0.0);
    sc.suffix_zero[total] = 0.0;
    for idx in (0..total).rev() {
        sc.suffix_zero[idx] = sc.suffix_zero[idx + 1] + band_at(idx).zero_error;
    }

    if sc.active_back.len() < total {
        sc.active_back.resize_with(total, Vec::new);
    }
    if sc.zero_back.len() < total {
        sc.zero_back.resize(total, [ZERO_STAY; 256]);
    }

    let mut zero_none = 0.0_f32;
    for &s in &sc.zero_sfs {
        sc.zero_dp[usize::from(s)] = f32::INFINITY;
    }
    sc.zero_sfs.clear();
    sc.prev_sf.clear();
    sc.prev_best.clear();
    for v in &mut sc.prev_by_cb {
        v.clear();
    }

    let mut best_term_cost = sc.suffix_zero[0];
    let mut best_term_slot: Option<(usize, usize, Codebook)> = None;
    let sf_delta_max = i32::from(SF_DELTA_MAX);

    for slot in 0..total {
        let band_data = band_at(slot);
        let new_group = slot % b_count == 0;
        let cands = &band_data.cands;
        let k_len = cands.len();

        // Unified switch frontier: best cost leaving previous slot with last_sf = s.
        sc.switch_sfs.clear();
        for &s in &sc.zero_sfs {
            let si = usize::from(s);
            sc.switch_cost[si] = sc.zero_dp[si];
            sc.switch_src[si] = (FROM_ZERO_FLAG | u16::from(s), Codebook::Zero);
            sc.switch_sfs.push(s);
        }
        for (pk, (&(cost, cb), &s)) in sc.prev_best.iter().zip(&sc.prev_sf).enumerate() {
            let si = usize::from(s);
            if cost < sc.switch_cost[si] {
                if sc.switch_cost[si] == f32::INFINITY {
                    sc.switch_sfs.push(s);
                }
                sc.switch_cost[si] = cost;
                sc.switch_src[si] = (pk as u16, cb);
            }
        }

        let cur_back = &mut sc.active_back[slot];
        cur_back.resize(k_len, [(FROM_NONE, Codebook::Zero); CODEBOOK_COUNT]);
        sc.cur_sf.clear();
        sc.cur_best.clear();
        for v in &mut sc.cur_by_cb {
            v.clear();
        }

        for (k, cand) in cands.iter().enumerate() {
            let sf = cand.sf;
            sc.cur_sf.push(sf);

            let mut best_sw_cost = zero_none + sec_hdr;
            let mut best_sw_src = (FROM_NONE, Codebook::Zero);
            for &ps in &sc.switch_sfs {
                let delta = i32::from(sf) - i32::from(ps);
                if delta.abs() <= sf_delta_max {
                    let c = sc.switch_cost[usize::from(ps)]
                        + sf_bits_lut[(delta + sf_delta_max) as usize]
                        + sec_hdr;
                    if c < best_sw_cost {
                        best_sw_cost = c;
                        best_sw_src = sc.switch_src[usize::from(ps)];
                    }
                }
            }

            let prune_limit = cand.min_cost.saturating_add(2 * sec_hdr_bits);
            let mut k_best_cost = f32::INFINITY;
            let mut k_best_cb = Codebook::Zero;

            for cb in ACTIVE_BOOKS {
                let ci = cb.index();
                let spec_cost = cand.costs[ci];
                if spec_cost == u16::MAX {
                    continue;
                }
                if spec_cost > prune_limit && cand.errors[ci] >= cand.min_error {
                    continue;
                }
                let emit = cand.errors[ci] + lambda * f32::from(spec_cost);
                let mut best_in = best_sw_cost;
                let mut best_src = best_sw_src;

                if !new_group {
                    for &(pk, ps, prev_same) in &sc.prev_by_cb[ci] {
                        if prev_same < best_in {
                            let delta = i32::from(sf) - i32::from(ps);
                            if delta.abs() <= sf_delta_max {
                                let c = prev_same + sf_bits_lut[(delta + sf_delta_max) as usize];
                                if c < best_in {
                                    best_in = c;
                                    best_src = (pk, cb);
                                }
                            }
                        }
                    }
                }

                let total_c = best_in + emit;
                cur_back[k][ci] = best_src;
                sc.cur_by_cb[ci].push((k as u16, sf, total_c));
                if total_c < k_best_cost {
                    k_best_cost = total_c;
                    k_best_cb = cb;
                }
            }
            sc.cur_best.push((k_best_cost, k_best_cb));
            if layout.groups == 1 && k_best_cb != Codebook::Zero {
                let term = k_best_cost + sc.suffix_zero[slot + 1] + ms_penalty * (slot + 1) as f32;
                if term < best_term_cost {
                    best_term_cost = term;
                    best_term_slot = Some((slot, k, k_best_cb));
                }
            }
        }

        for &s in &sc.switch_sfs {
            sc.switch_cost[usize::from(s)] = f32::INFINITY;
        }

        // Advance zero_none and zero_dp to `slot`.
        let zerr = band_data.zero_error;
        let z_hdr = if new_group { sec_hdr } else { 0.0 };
        zero_none += zerr + z_hdr;

        sc.next_zero_sfs.clear();
        for &s in &sc.zero_sfs {
            let si = usize::from(s);
            sc.next_zero_dp[si] = sc.zero_dp[si] + zerr + z_hdr;
            sc.zero_back[slot][si] = ZERO_STAY;
            sc.next_zero_sfs.push(s);
            sc.zero_dp[si] = f32::INFINITY;
        }
        for (pk, (&(cost, cb), &s)) in sc.prev_best.iter().zip(&sc.prev_sf).enumerate() {
            if cb == Codebook::Zero {
                continue;
            }
            let si = usize::from(s);
            let cand_z = cost + zerr + sec_hdr;
            if cand_z < sc.next_zero_dp[si] {
                if sc.next_zero_dp[si] == f32::INFINITY {
                    sc.next_zero_sfs.push(s);
                }
                sc.next_zero_dp[si] = cand_z;
                // Pack exact `(prev_k, prev_cb)` so unwinding never guesses the codebook.
                sc.zero_back[slot][si] = ((pk as u16) << 4) | u16::from(cb.id());
            }
        }

        std::mem::swap(&mut sc.zero_dp, &mut sc.next_zero_dp);
        std::mem::swap(&mut sc.zero_sfs, &mut sc.next_zero_sfs);
        std::mem::swap(&mut sc.prev_by_cb, &mut sc.cur_by_cb);
        std::mem::swap(&mut sc.prev_sf, &mut sc.cur_sf);
        std::mem::swap(&mut sc.prev_best, &mut sc.cur_best);
    }

    output.clear();
    output.resize(total, Choice::default());
    for (idx, choice) in output.iter_mut().enumerate() {
        choice.error = band_at(idx).zero_error;
    }

    let unwind_zero = |mut s_slot: usize,
                       sf: usize,
                       zero_back: &[[u16; 256]]|
     -> Option<(usize, usize, Codebook)> {
        loop {
            let zb = zero_back[s_slot][sf];
            if s_slot == 0 {
                return None;
            }
            s_slot -= 1;
            if zb != ZERO_STAY {
                let pk = usize::from(zb >> 4);
                let cb = Codebook::from_id((zb & 0x0f) as u8);
                return Some((s_slot, pk, cb));
            }
        }
    };

    let mut cur_state: Option<(usize, usize, Codebook)> = if layout.groups == 1 {
        best_term_slot
    } else {
        let mut best_c = sc.suffix_zero[0];
        let mut state: Option<(usize, u16, Codebook)> = None;
        for (k, &(cost, cb)) in sc.prev_best.iter().enumerate() {
            if cb != Codebook::Zero && cost < best_c {
                best_c = cost;
                state = Some((total - 1, k as u16, cb));
            }
        }
        for &s in &sc.zero_sfs {
            let c = sc.zero_dp[usize::from(s)];
            if c < best_c {
                best_c = c;
                state = Some((total - 1, FROM_ZERO_FLAG | u16::from(s), Codebook::Zero));
            }
        }
        match state {
            None => None,
            Some((slot, tag, cb)) if tag < FROM_ZERO_FLAG => Some((slot, usize::from(tag), cb)),
            Some((slot, tag, _)) => unwind_zero(slot, usize::from(tag & 0x7fff), &sc.zero_back),
        }
    };

    while let Some((slot, k, cb)) = cur_state {
        let cand = &band_at(slot).cands[k];
        output[slot] = Choice {
            sf: cand.sf,
            book: cb,
            cost: cand.costs[cb.index()],
            error: cand.errors[cb.index()],
        };
        let (prev_tag, prev_cb) = sc.active_back[slot][k][cb.index()];
        if prev_tag == FROM_NONE || slot == 0 {
            break;
        }
        cur_state = if prev_tag < FROM_ZERO_FLAG {
            Some((slot - 1, usize::from(prev_tag), prev_cb))
        } else {
            unwind_zero(slot - 1, usize::from(prev_tag & 0x7fff), &sc.zero_back)
        };
    }
}

#[inline]
fn band_env_weight(raw_zero_err: f32) -> f32 {
    if raw_zero_err > 1.5 { 0.85 } else { 0.0 }
}

struct RdoqCtx<'a> {
    xs: &'a [f32],
    ps: &'a [f32],
    scale: f32,
    factor: f32,
    inv_mask: f32,
    lambda: f32,
    orig_norm: f32,
    env_weight: f32,
    inverse: &'a [f32],
}

impl<'a> RdoqCtx<'a> {
    fn new(
        xs: &'a [f32],
        ps: &'a [f32],
        sf: u8,
        inv_mask: f32,
        lambda: f32,
        inverse: &'a [f32],
    ) -> Self {
        let (scale, factor) = gains(sf);
        let orig_energy: f32 = xs.iter().map(|&x| x * x).sum();
        Self {
            xs,
            ps,
            scale,
            factor,
            inv_mask,
            lambda,
            orig_norm: orig_energy.sqrt(),
            env_weight: band_env_weight(orig_energy * inv_mask),
            inverse,
        }
    }

    #[inline]
    fn env_penalty(&self, hat_energy: f32) -> f32 {
        if self.env_weight > 0.0 {
            let d = self.orig_norm - hat_energy.sqrt();
            self.env_weight * d * d * self.inv_mask
        } else {
            0.0
        }
    }
}

/// Exact tuple-level RDOQ across relevant codebooks for a band at scalefactor `sf`.
/// Tests both the natural codebook tier and 1-2 lower tiers (clamping outlier peaks)
/// plus up to 1 higher tier (for section merging), skipping wildly wasteful books.
fn rdoq_all_books(
    xs: &[f32],
    ps: &[f32],
    sf: u8,
    inv_mask: f32,
    lambda: f32,
    inverse: &[f32],
) -> Option<SfCandidate> {
    let ctx = RdoqCtx::new(xs, ps, sf, inv_mask, lambda, inverse);
    let max_u = ps.iter().copied().fold(0.0_f32, f32::max) * ctx.factor;
    if max_u < 0.35 {
        return None;
    }

    let first_id = Codebook::first_for_peak(((max_u + 0.5) as u16).max(1)).id();
    let min_id: u8 = if max_u <= 3.2 {
        Codebook::QuadSigned1A.id()
    } else if max_u <= 5.5 {
        Codebook::QuadUnsigned2A.id()
    } else if max_u <= 9.0 {
        Codebook::PairSigned4A.id()
    } else if max_u <= 15.0 {
        Codebook::PairUnsigned7A.id()
    } else if max_u <= 24.0 {
        Codebook::PairUnsigned12A.id()
    } else {
        Codebook::PairEscape.id()
    };
    let max_id = (first_id + 3).min(Codebook::PairEscape.id());

    let mut cand = SfCandidate::new(sf);

    for id in (min_id..=max_id.min(Codebook::QuadUnsigned2A.id())).step_by(2) {
        let (e0, b0, e1, b1) = rdoq_quad::<true>(&ctx, Codebook::from_id(id), None);
        cand.record(usize::from(id), e0, b0);
        cand.record(usize::from(id + 1), e1, b1);
    }

    let start_pair = min_id.max(Codebook::PairSigned4A.id());
    let end_pair = max_id.min(Codebook::PairUnsigned12A.id());
    for id in (start_pair..=end_pair).step_by(2) {
        let (e0, b0, e1, b1) = rdoq_duo::<true>(&ctx, Codebook::from_id(id), None);
        cand.record(usize::from(id), e0, b0);
        cand.record(usize::from(id + 1), e1, b1);
    }

    if max_id == Codebook::PairEscape.id() {
        let (e11, b11, _, _) = rdoq_duo::<false>(&ctx, Codebook::PairEscape, None);
        cand.record(Codebook::PairEscape.index(), e11, b11);
    }

    (cand.min_cost != u16::MAX).then_some(cand)
}

fn rdoq_single_book(ctx: &RdoqCtx<'_>, book: Codebook, out: &mut [i16]) -> (f32, u16, bool) {
    let (err, bits, _, _) = if book.spec().width == 4 {
        rdoq_quad::<false>(ctx, book, Some(out))
    } else {
        rdoq_duo::<false>(ctx, book, Some(out))
    };
    (err, bits, bits != u16::MAX)
}

fn rdoq_quad<const PAIRED: bool>(
    ctx: &RdoqCtx<'_>,
    book: Codebook,
    mut out: Option<&mut [i16]>,
) -> (f32, u16, f32, u16) {
    let spec = book.spec();
    let limit = usize::from(spec.limit);
    let radix = if spec.signed {
        2 * limit + 1
    } else {
        limit + 1
    };
    let t0 = SPECTRUM[book.index() - 1];
    let t1 = if PAIRED { SPECTRUM[book.index()] } else { t0 };

    let mut err0 = 0.0_f32;
    let mut hat_e0 = 0.0_f32;
    let mut bits0 = 0_u16;
    let mut nz0 = false;
    let mut err1 = 0.0_f32;
    let mut hat_e1 = 0.0_f32;
    let mut bits1 = 0_u16;
    let mut nz1 = false;

    for (tuple_idx, (xq, pq)) in ctx
        .xs
        .as_chunks::<4>()
        .0
        .iter()
        .zip(ctx.ps.as_chunks::<4>().0)
        .enumerate()
    {
        let mut cands = [[0_usize; 2]; 4];
        let mut counts = [1_usize; 4];
        let mut errs = [[0.0_f32; 2]; 4];
        let mut deq2 = [[0.0_f32; 2]; 4];
        let mut digits = [[0_usize; 2]; 4];

        for j in 0..4 {
            let ax = xq[j].abs();
            let neg = xq[j] < 0.0;
            let u = pq[j] * ctx.factor;
            let (q0, q1) = if u < RDOQ_DEADZONE {
                (0, 0)
            } else {
                let lo = (u as usize).min(limit);
                if lo == limit {
                    (limit - 1, limit)
                } else {
                    (lo, lo + 1)
                }
            };
            cands[j][0] = q0;
            let v0 = ctx.inverse[q0] * ctx.scale;
            let d0 = ax - v0;
            errs[j][0] = d0 * d0 * ctx.inv_mask;
            deq2[j][0] = v0 * v0;
            digits[j][0] = if spec.signed {
                if neg { limit - q0 } else { limit + q0 }
            } else {
                q0
            };
            if q1 != q0 {
                counts[j] = 2;
                cands[j][1] = q1;
                let v1 = ctx.inverse[q1] * ctx.scale;
                let d1 = ax - v1;
                errs[j][1] = d1 * d1 * ctx.inv_mask;
                deq2[j][1] = v1 * v1;
                digits[j][1] = if spec.signed {
                    if neg { limit - q1 } else { limit + q1 }
                } else {
                    q1
                };
            }
        }

        let mut b0_j = f32::INFINITY;
        let mut b0_e = 0.0_f32;
        let mut b0_h = 0.0_f32;
        let mut b0_r = 0_u16;
        let mut b0_nz = false;
        let mut b0_q = [0_usize; 4];
        let mut b1_j = f32::INFINITY;
        let mut b1_e = 0.0_f32;
        let mut b1_h = 0.0_f32;
        let mut b1_r = 0_u16;
        let mut b1_nz = false;

        for i0 in 0..counts[0] {
            let e0 = errs[0][i0];
            let h0 = deq2[0][i0];
            let idx0 = digits[0][i0] * radix;
            let s0 = u16::from(cands[0][i0] != 0);
            for i1 in 0..counts[1] {
                let e01 = e0 + errs[1][i1];
                let h01 = h0 + deq2[1][i1];
                let idx1 = (idx0 + digits[1][i1]) * radix;
                let s01 = s0 + u16::from(cands[1][i1] != 0);
                for i2 in 0..counts[2] {
                    let e012 = e01 + errs[2][i2];
                    let h012 = h01 + deq2[2][i2];
                    let idx2 = (idx1 + digits[2][i2]) * radix;
                    let s012 = s01 + u16::from(cands[2][i2] != 0);
                    for i3 in 0..counts[3] {
                        let e = e012 + errs[3][i3];
                        let h = h012 + deq2[3][i3];
                        let idx = idx2 + digits[3][i3];
                        let nz_cnt = s012 + u16::from(cands[3][i3] != 0);
                        let sign_b = if spec.signed { 0 } else { nz_cnt };
                        let any_nz = nz_cnt != 0;
                        let r0 = code_len(t0[idx]) + sign_b;
                        let j0 = e + ctx.lambda * f32::from(r0);
                        if j0 < b0_j {
                            b0_j = j0;
                            b0_e = e;
                            b0_h = h;
                            b0_r = r0;
                            b0_nz = any_nz;
                            if !PAIRED {
                                b0_q = [cands[0][i0], cands[1][i1], cands[2][i2], cands[3][i3]];
                            }
                        }
                        if PAIRED {
                            let r1 = code_len(t1[idx]) + sign_b;
                            let j1 = e + ctx.lambda * f32::from(r1);
                            if j1 < b1_j {
                                b1_j = j1;
                                b1_e = e;
                                b1_h = h;
                                b1_r = r1;
                                b1_nz = any_nz;
                            }
                        }
                    }
                }
            }
        }
        if let Some(buf) = out.as_deref_mut() {
            let dst = &mut buf[tuple_idx * 4..tuple_idx * 4 + 4];
            for j in 0..4 {
                let q = b0_q[j] as i16;
                dst[j] = if xq[j] < 0.0 { -q } else { q };
            }
        }
        err0 += b0_e;
        hat_e0 += b0_h;
        bits0 += b0_r;
        nz0 |= b0_nz;
        if PAIRED {
            err1 += b1_e;
            hat_e1 += b1_h;
            bits1 += b1_r;
            nz1 |= b1_nz;
        }
    }

    if nz0 {
        err0 += ctx.env_penalty(hat_e0);
    }
    if PAIRED && nz1 {
        err1 += ctx.env_penalty(hat_e1);
    }

    (
        err0,
        if nz0 { bits0 } else { u16::MAX },
        err1,
        if nz1 { bits1 } else { u16::MAX },
    )
}

fn rdoq_duo<const PAIRED: bool>(
    ctx: &RdoqCtx<'_>,
    book: Codebook,
    mut out: Option<&mut [i16]>,
) -> (f32, u16, f32, u16) {
    let spec = book.spec();
    let limit = usize::from(spec.limit);
    let is_escape = book == Codebook::PairEscape;
    let max_q = if is_escape { MAX_QUANTIZED } else { limit };
    let radix = if spec.signed {
        2 * limit + 1
    } else {
        limit + 1
    };
    let t0 = SPECTRUM[book.index() - 1];
    let t1 = if PAIRED { SPECTRUM[book.index()] } else { t0 };

    let mut err0 = 0.0_f32;
    let mut hat_e0 = 0.0_f32;
    let mut bits0 = 0_u16;
    let mut nz0 = false;
    let mut err1 = 0.0_f32;
    let mut hat_e1 = 0.0_f32;
    let mut bits1 = 0_u16;
    let mut nz1 = false;

    for (tuple_idx, (xp, pp)) in ctx
        .xs
        .as_chunks::<2>()
        .0
        .iter()
        .zip(ctx.ps.as_chunks::<2>().0)
        .enumerate()
    {
        let mut cands = [[0_usize; 3]; 2];
        let mut counts = [0_usize; 2];
        let mut errs = [[0.0_f32; 3]; 2];
        let mut deq2 = [[0.0_f32; 3]; 2];
        let mut digits = [[0_usize; 3]; 2];
        let mut extra = [[0_u16; 3]; 2];

        for j in 0..2 {
            let ax = xp[j].abs();
            let neg = xp[j] < 0.0;
            let u = pp[j] * ctx.factor;
            let lo = (u as usize).min(max_q);
            let mut n = 0;
            let mut push = |q: usize| {
                cands[j][n] = q;
                let v = ctx.inverse[q] * ctx.scale;
                let d = ax - v;
                errs[j][n] = d * d * ctx.inv_mask;
                deq2[j][n] = v * v;
                digits[j][n] = if spec.signed {
                    if neg { limit - q } else { limit + q }
                } else {
                    q.min(limit)
                };
                extra[j][n] = if spec.signed {
                    0
                } else {
                    u16::from(q != 0) + if is_escape { escape_bits(q as u16) } else { 0 }
                };
                n += 1;
            };
            if u < RDOQ_DEADZONE {
                push(0);
            } else {
                if lo == 1 || lo == limit {
                    push(lo - 1);
                }
                push(lo);
                if lo < max_q {
                    push(lo + 1);
                }
            }
            counts[j] = n;
        }

        let mut b0_j = f32::INFINITY;
        let mut b0_e = 0.0_f32;
        let mut b0_h = 0.0_f32;
        let mut b0_r = 0_u16;
        let mut b0_nz = false;
        let mut b0_q = [0_usize; 2];
        let mut b1_j = f32::INFINITY;
        let mut b1_e = 0.0_f32;
        let mut b1_h = 0.0_f32;
        let mut b1_r = 0_u16;
        let mut b1_nz = false;

        for i0 in 0..counts[0] {
            let e0 = errs[0][i0];
            let h0 = deq2[0][i0];
            let idx0 = digits[0][i0] * radix;
            let x0 = extra[0][i0];
            let q0 = cands[0][i0];
            for i1 in 0..counts[1] {
                let e = e0 + errs[1][i1];
                let h = h0 + deq2[1][i1];
                let idx = idx0 + digits[1][i1];
                let q1 = cands[1][i1];
                let any_nz = q0 != 0 || q1 != 0;
                let extra_b = x0 + extra[1][i1];
                let r0 = code_len(t0[idx]) + extra_b;
                let j0 = e + ctx.lambda * f32::from(r0);
                if j0 < b0_j {
                    b0_j = j0;
                    b0_e = e;
                    b0_h = h;
                    b0_r = r0;
                    b0_nz = any_nz;
                    if !PAIRED {
                        b0_q = [q0, q1];
                    }
                }
                if PAIRED {
                    let r1 = code_len(t1[idx]) + extra_b;
                    let j1 = e + ctx.lambda * f32::from(r1);
                    if j1 < b1_j {
                        b1_j = j1;
                        b1_e = e;
                        b1_h = h;
                        b1_r = r1;
                        b1_nz = any_nz;
                    }
                }
            }
        }
        if let Some(buf) = out.as_deref_mut() {
            let dst = &mut buf[tuple_idx * 2..tuple_idx * 2 + 2];
            for j in 0..2 {
                let q = b0_q[j] as i16;
                dst[j] = if xp[j] < 0.0 { -q } else { q };
            }
        }
        err0 += b0_e;
        hat_e0 += b0_h;
        bits0 += b0_r;
        nz0 |= b0_nz;
        if PAIRED {
            err1 += b1_e;
            hat_e1 += b1_h;
            bits1 += b1_r;
            nz1 |= b1_nz;
        }
    }

    if nz0 {
        err0 += ctx.env_penalty(hat_e0);
    }
    if PAIRED && nz1 {
        err1 += ctx.env_penalty(hat_e1);
    }

    (
        err0,
        if nz0 { bits0 } else { u16::MAX },
        err1,
        if nz1 { bits1 } else { u16::MAX },
    )
}

fn gains(sf: u8) -> (f32, f32) {
    static GAINS: LazyLock<[(f32, f32); 256]> = LazyLock::new(|| {
        std::array::from_fn(|sf| {
            let exponent = (sf as f32 - f32::from(SF_OFFSET)) * 0.25;
            (exponent.exp2(), (-0.75 * exponent).exp2())
        })
    });
    GAINS[usize::from(sf)]
}

#[inline]
fn quantize_scalar(sample: f32, power: f32, factor: f32, boundaries: &[f32]) -> i16 {
    let scaled = power * factor;
    let floor = (scaled as usize).min(MAX_QUANTIZED);
    let magnitude = (floor + usize::from(scaled > boundaries[floor])) as i16;
    if sample < 0.0 { -magnitude } else { magnitude }
}
