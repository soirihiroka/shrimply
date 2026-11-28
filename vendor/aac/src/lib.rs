//! Shrimply AAC, an independent stereo AAC-LC encoder for offline export.

mod bits;
mod mdct;
mod psychoacoustics;
mod quantize;
mod tables;

use std::ops::Range;

use bits::{Bits, Codebook, code_len};
use mdct::{Analysis, TransientDetector, WindowShape, short_grouping};
use psychoacoustics::Psychoacoustics;
use quantize::{Choice, MsMode, SF_DELTA_MAX, SF_OFFSET, Search, StereoFrame};
use tables::*;

pub const FRAME_SAMPLES: usize = 1024;
pub const SAMPLE_RATES: [u32; 12] = [
    96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000,
];
const FRAME_LIMIT: usize = 2 * 6144;
const MIN_FRAME_BITS: i64 = 96;
const STAGE1_LOG_MIN: f32 = -22.0;
const STAGE1_LOG_MAX: f32 = 32.0;
const STAGE2_LOG_MAX: f32 = 34.0;

#[repr(u32)]
#[derive(Clone, Copy)]
enum Window {
    Long,
    Start,
    Short,
    Stop,
}

#[derive(Clone, Copy)]
struct Layout {
    groups: usize,
    windows: usize,
    group_lens: [usize; 8],
    group_starts: [usize; 8],
    grouping_bits: u8,
    bands: &'static [usize],
}

impl Layout {
    fn long(bands: &'static [usize]) -> Self {
        Self {
            groups: 1,
            windows: 1,
            group_lens: [1, 0, 0, 0, 0, 0, 0, 0],
            group_starts: [0; 8],
            grouping_bits: 0,
            bands,
        }
    }

    fn short(
        bands: &'static [usize],
        grouping_bits: u8,
        groups: usize,
        group_lens: [usize; 8],
    ) -> Self {
        let mut group_starts = [0; 8];
        let mut start = 0;
        for g in 0..groups {
            group_starts[g] = start;
            start += group_lens[g];
        }
        Self {
            groups,
            windows: 8,
            group_lens,
            group_starts,
            grouping_bits,
            bands,
        }
    }

    fn band_count(self) -> usize {
        self.bands.len() - 1
    }

    fn section_bits(self) -> usize {
        if self.windows == 8 { 3 } else { 5 }
    }

    fn ranges(self, group: usize, band: usize) -> impl Iterator<Item = Range<usize>> {
        let start_w = self.group_starts[group];
        let end_w = start_w + self.group_lens[group];
        let size = FRAME_SAMPLES / self.windows;
        (start_w..end_w).map(move |window| {
            window * size + self.bands[band]..window * size + self.bands[band + 1]
        })
    }
}

pub struct Encoder {
    rate_index: u8,
    sample_rate: u32,
    bitrate: u32,
    remainder: u64,
    reservoir: i64,
    average_pe: f32,
    last_log_lambda: f32,
    delayed: Option<[[f32; 2]; FRAME_SAMPLES]>,
    overlap: [[f32; 2]; 2 * FRAME_SAMPLES],
    transients: TransientDetector,
    attacks: [bool; 2],
    window: Window,
    prev_shape: WindowShape,
    long_bands: &'static [usize],
    short_bands: &'static [usize],
    psy: Psychoacoustics,
    analysis: Analysis,
    quantizer: Search,
}

impl Encoder {
    pub fn new(sample_rate: u32, bitrate: u32) -> Result<Self, &'static str> {
        let rate_index = SAMPLE_RATES
            .iter()
            .position(|&rate| rate == sample_rate)
            .ok_or("Unsupported Shrimply AAC sample rate")?;
        if bitrate < 16_000
            || u64::from(bitrate) * FRAME_SAMPLES as u64
                > FRAME_LIMIT as u64 * u64::from(sample_rate)
        {
            return Err("Unsupported Shrimply AAC bitrate for this sample rate");
        }
        let (long_bands, short_bands) = match rate_index {
            0 | 1 => (BANDS_56, BANDS_57),
            2 => (BANDS_54, BANDS_55),
            3 | 4 => (BANDS_45, BANDS_46),
            5 => (BANDS_47, BANDS_46),
            6 | 7 => (BANDS_52, BANDS_53),
            8..=10 => (BANDS_50, BANDS_51),
            _ => (BANDS_48, BANDS_49),
        };
        let psy = Psychoacoustics::new(sample_rate, bitrate, long_bands.len() - 1);
        Ok(Self {
            rate_index: rate_index as u8,
            sample_rate,
            bitrate,
            remainder: 0,
            reservoir: 0,
            average_pe: 0.0,
            last_log_lambda: 0.0,
            delayed: None,
            overlap: [[0.0; 2]; 2 * FRAME_SAMPLES],
            transients: TransientDetector::default(),
            attacks: [false; 2],
            window: Window::Long,
            prev_shape: WindowShape::Kbd,
            long_bands,
            short_bands,
            psy,
            analysis: Analysis::new(),
            quantizer: Search::new(),
        })
    }

    /// MPEG-4 AudioSpecificConfig for stereo AAC-LC.
    pub fn configuration(&self) -> [u8; 2] {
        ((2_u16 << 11) | (u16::from(self.rate_index) << 7) | (2 << 3)).to_be_bytes()
    }

    /// Submit normalized stereo PCM. Queue one frame of lookahead before emitting packets.
    /// After the final input frame, submit two silent frames to drain the filterbank.
    /// The decoded first frame is encoder priming and must be skipped by the muxer.
    pub fn encode(&mut self, block: &[[f32; 2]; FRAME_SAMPLES]) -> Option<Vec<u8>> {
        let block = block.map(|sample| {
            sample.map(|value| {
                if value.is_finite() {
                    value.clamp(-1.0, 1.0)
                } else {
                    0.0
                }
            })
        });
        let next = self.transients.detect(&block);
        let attack = self.attacks[1] || next[0];
        self.attacks = next;
        let current = self.delayed.replace(block)?;
        self.window = match (self.window, attack) {
            (Window::Start, _) | (Window::Short, true) => Window::Short,
            (Window::Short, false) => Window::Stop,
            (_, true) => Window::Start,
            (_, false) => Window::Long,
        };
        let curr_shape = match self.window {
            Window::Long | Window::Stop => WindowShape::Kbd,
            Window::Start | Window::Short => WindowShape::Sine,
        };
        self.overlap.copy_within(FRAME_SAMPLES.., 0);
        self.overlap[FRAME_SAMPLES..].copy_from_slice(&current);

        let mut spectra_lr = [[0.0; FRAME_SAMPLES]; 2];
        for (channel, spectrum) in spectra_lr.iter_mut().enumerate() {
            self.analysis.transform(
                self.window,
                self.prev_shape,
                curr_shape,
                &self.overlap,
                channel,
                spectrum,
            );
        }
        self.prev_shape = curr_shape;

        let layout = if matches!(self.window, Window::Short) {
            let (grouping_bits, groups, group_lens) = short_grouping(&spectra_lr);
            Layout::short(self.short_bands, grouping_bits, groups, group_lens)
        } else {
            Layout::long(self.long_bands)
        };

        let mut spectra_ms = [[0.0; FRAME_SAMPLES]; 2];
        for i in 0..FRAME_SAMPLES {
            spectra_ms[0][i] = (spectra_lr[0][i] + spectra_lr[1][i]) * 0.5;
            spectra_ms[1][i] = (spectra_lr[0][i] - spectra_lr[1][i]) * 0.5;
        }

        let (thr_lr, pe) = self.psy.analyze(layout, &self.overlap, &spectra_lr);
        let thr_ms: Vec<f32> = thr_lr[0]
            .iter()
            .zip(&thr_lr[1])
            .map(|(&l, &r)| 0.5 * l.min(r))
            .collect();

        let frame = StereoFrame {
            layout,
            spectra_lr: &spectra_lr,
            spectra_ms: &spectra_ms,
            thr_lr: &thr_lr,
            thr_ms: &thr_ms,
        };
        self.quantizer.analyze_stereo(frame);

        // Full AAC-LC compliant bit reservoir with Perceptual Entropy bit distribution.
        self.remainder += u64::from(self.bitrate) * FRAME_SAMPLES as u64;
        let frame_budget = (self.remainder / u64::from(self.sample_rate)) as i64;
        self.remainder %= u64::from(self.sample_rate);

        let max_reservoir = ((FRAME_LIMIT as i64) - frame_budget)
            .min(frame_budget * 4)
            .max(0);
        if self.average_pe == 0.0 {
            self.average_pe = pe.max(300.0);
        }
        let pe_floor = 300.0_f32;
        let ratio = ((pe + pe_floor) / (self.average_pe + pe_floor)).powf(0.45);
        self.average_pe += (pe - self.average_pe) / 24.0;

        let is_transient = matches!(self.window, Window::Short | Window::Start);
        let desired_scale = if is_transient {
            (ratio * 1.25).clamp(0.85, 1.85)
        } else {
            ratio.clamp(0.75, 1.45)
        };
        let fill = if max_reservoir > 0 {
            (self.reservoir as f32 / max_reservoir as f32).clamp(-0.5, 1.0)
        } else {
            0.0
        };
        let reservoir_pull = 1.0 + 0.45 * (fill - 0.45);
        let target = (frame_budget as f32 * desired_scale * reservoir_pull) as i64;

        let borrow_limit = (frame_budget / 2).min(max_reservoir / 3);
        let max_spend = (frame_budget + self.reservoir + borrow_limit)
            .clamp(MIN_FRAME_BITS, FRAME_LIMIT as i64);
        let min_spend = (frame_budget / 2).max(MIN_FRAME_BITS).min(max_spend);
        let limit = target.clamp(min_spend, max_spend) as usize / 8;

        let packet = self.allocate(frame, curr_shape, limit);
        self.reservoir =
            (self.reservoir + frame_budget - (packet.len() * 8) as i64).min(max_reservoir);
        Some(packet)
    }

    fn allocate(&mut self, frame: StereoFrame<'_>, shape: WindowShape, limit: usize) -> Vec<u8> {
        let layout = frame.layout;
        let mut ms_mask = Vec::with_capacity(layout.groups * layout.band_count());
        let mut best_ms_mask;
        let mut best_ms_mode;
        let mut picks: [Vec<Choice>; 2] = std::array::from_fn(|_| Vec::new());
        let mut candidate: [Vec<Choice>; 2] =
            std::array::from_fn(|_| Vec::with_capacity(layout.groups * layout.band_count()));

        // Stage 1: warm-started bracket + exact 2D Viterbi search over scalar (sf, cb) tables.
        let mut low = (self.last_log_lambda - 5.0).max(STAGE1_LOG_MIN);
        let mut high = (self.last_log_lambda + 5.0).min(STAGE1_LOG_MAX);
        let mut chosen_log_lambda;

        let eval_s1 = |log_lam: f32,
                       q: &mut Search,
                       cand: &mut [Vec<Choice>; 2],
                       mask: &mut Vec<bool>|
         -> (MsMode, usize) {
            let lam = log_lam.exp2();
            let mode = q.choose_ms(layout, lam, mask);
            q.select_stage1(layout, mask, mode, lam, cand);
            (mode, packet_bytes(layout, cand, mode, mask))
        };

        // Expand lower/upper bracket only if the warm-start window doesn't straddle `limit`.
        let (mut mode_lo, mut sz_lo) =
            eval_s1(low, &mut self.quantizer, &mut candidate, &mut ms_mask);
        while sz_lo <= limit && low > STAGE1_LOG_MIN {
            high = low;
            low = (low - 10.0).max(STAGE1_LOG_MIN);
            (mode_lo, sz_lo) = eval_s1(low, &mut self.quantizer, &mut candidate, &mut ms_mask);
        }
        if sz_lo <= limit {
            picks = candidate;
            best_ms_mask = ms_mask;
            best_ms_mode = mode_lo;
            chosen_log_lambda = low;
        } else {
            let (mut mode_hi, mut sz_hi) =
                eval_s1(high, &mut self.quantizer, &mut picks, &mut ms_mask);
            while sz_hi > limit && high < STAGE1_LOG_MAX {
                low = high;
                high = (high + 10.0).min(STAGE1_LOG_MAX);
                (mode_hi, sz_hi) = eval_s1(high, &mut self.quantizer, &mut picks, &mut ms_mask);
            }
            best_ms_mask = ms_mask.clone();
            best_ms_mode = mode_hi;
            chosen_log_lambda = high;

            for _ in 0..6 {
                let mid = (low + high) * 0.5;
                let (mode, sz) = eval_s1(mid, &mut self.quantizer, &mut candidate, &mut ms_mask);
                if sz <= limit {
                    high = mid;
                    chosen_log_lambda = mid;
                    best_ms_mode = mode;
                    std::mem::swap(&mut picks, &mut candidate);
                    std::mem::swap(&mut best_ms_mask, &mut ms_mask);
                } else {
                    low = mid;
                }
            }
        }

        // Stage 2: Exact Tuple-RDOQ around Stage 1 scalefactors + joint Viterbi refinement.
        let mut rdoq_log_lambda = chosen_log_lambda;
        let mut build_lambda = 0.0_f32;
        let mut rdoq_picks = picks;
        let mut rdoq_cand: [Vec<Choice>; 2] =
            std::array::from_fn(|_| Vec::with_capacity(layout.groups * layout.band_count()));

        for (pass, radius) in [(0, 3_i32), (1, 2_i32)] {
            build_lambda = rdoq_log_lambda.exp2();
            self.quantizer
                .build_rdoq(frame, &best_ms_mask, &rdoq_picks, build_lambda, radius);
            let mut r_lo = (rdoq_log_lambda - if pass == 0 { 4.0 } else { 1.8 }).max(-24.0);
            let mut r_hi = (rdoq_log_lambda + 2.0).min(STAGE2_LOG_MAX);
            let mut found = false;
            let mut best_log = rdoq_log_lambda;

            self.quantizer
                .select_stage2(layout, best_ms_mode, r_lo.exp2(), &mut rdoq_cand);
            if packet_bytes(layout, &rdoq_cand, best_ms_mode, &best_ms_mask) <= limit {
                std::mem::swap(&mut rdoq_picks, &mut rdoq_cand);
                best_log = r_lo;
                found = true;
            } else {
                for _ in 0..5 {
                    let mid = (r_lo + r_hi) * 0.5;
                    self.quantizer
                        .select_stage2(layout, best_ms_mode, mid.exp2(), &mut rdoq_cand);
                    if packet_bytes(layout, &rdoq_cand, best_ms_mode, &best_ms_mask) <= limit {
                        r_hi = mid;
                        best_log = mid;
                        std::mem::swap(&mut rdoq_picks, &mut rdoq_cand);
                        found = true;
                    } else {
                        r_lo = mid;
                    }
                }
            }
            if !found {
                self.quantizer
                    .select_stage2(layout, best_ms_mode, r_hi.exp2(), &mut rdoq_picks);
                while packet_bytes(layout, &rdoq_picks, best_ms_mode, &best_ms_mask) > limit
                    && r_hi < STAGE2_LOG_MAX
                {
                    r_hi = (r_hi + 4.0).min(STAGE2_LOG_MAX);
                    self.quantizer.select_stage2(
                        layout,
                        best_ms_mode,
                        r_hi.exp2(),
                        &mut rdoq_picks,
                    );
                }
                best_log = r_hi;
            }
            let shift = (best_log - rdoq_log_lambda).abs();
            rdoq_log_lambda = best_log;
            if shift < 0.65 {
                break;
            }
        }
        self.last_log_lambda = rdoq_log_lambda.clamp(-18.0, 24.0);

        self.quantizer
            .materialize_quantized(frame, &best_ms_mask, &mut rdoq_picks, build_lambda);

        let ms_ref = &best_ms_mask;
        self.quantizer.polish_residual(
            frame,
            ms_ref,
            &mut rdoq_picks,
            rdoq_log_lambda.exp2(),
            limit,
            |c| packet_bytes(layout, c, best_ms_mode, ms_ref),
        );

        let packet = self.packet(layout, shape, best_ms_mode, &best_ms_mask, &rdoq_picks);
        debug_assert_eq!(
            packet.len(),
            packet_bytes(layout, &rdoq_picks, best_ms_mode, &best_ms_mask)
        );
        debug_assert!(packet.len() <= limit);
        packet
    }

    fn packet(
        &self,
        layout: Layout,
        shape: WindowShape,
        ms_mode: MsMode,
        stereo: &[bool],
        choices: &[Vec<Choice>; 2],
    ) -> Vec<u8> {
        let max_sfb = max_band(layout, choices);
        let mut bits = Bits::default();
        bits.put(3, 1); // Channel pair element.
        bits.put(4, 0); // Element instance tag.
        bits.put(1, 1); // Common window.
        bits.put(1, 0);
        bits.put(2, self.window as u32);
        bits.put(1, u32::from(shape as u8));
        if layout.windows == 8 {
            bits.put(4, max_sfb as u32);
            bits.put(7, u32::from(layout.grouping_bits));
        } else {
            bits.put(6, max_sfb as u32);
            bits.put(1, 0); // No prediction.
        }
        let emitted_ms = effective_ms_mode(layout, ms_mode, stereo, max_sfb);
        bits.put(2, emitted_ms as u32);
        if emitted_ms == MsMode::PerBand {
            for group in stereo.chunks_exact(layout.band_count()) {
                for &mid_side in &group[..max_sfb] {
                    bits.put(1, u32::from(mid_side));
                }
            }
        }

        for (channel, choices) in choices.iter().enumerate() {
            let global_gain = choices
                .iter()
                .find(|choice| choice.book != Codebook::Zero)
                .map_or(SF_OFFSET, |choice| choice.sf);
            bits.put(8, u32::from(global_gain));
            for section in sections(layout, choices, max_sfb) {
                bits.put(4, u32::from(section[0].book.id()));
                let width = layout.section_bits();
                let escape = (1 << width) - 1;
                for _ in 0..section.len() / escape {
                    bits.put(width, escape as u32);
                }
                bits.put(width, (section.len() % escape) as u32);
            }
            let mut previous = global_gain;
            for choice in choices
                .iter()
                .filter(|choice| choice.book != Codebook::Zero)
            {
                let delta = i16::from(choice.sf) - i16::from(previous);
                bits.code(SCALEFACTORS[(delta + SF_DELTA_MAX) as usize]);
                previous = choice.sf;
            }
            bits.put(3, 0); // No pulse data, TNS or gain control.
            for (index, choice) in choices
                .iter()
                .enumerate()
                .filter(|(_, choice)| choice.book != Codebook::Zero)
            {
                let group = index / layout.band_count();
                let band = index % layout.band_count();
                for range in layout.ranges(group, band) {
                    choice
                        .book
                        .write(&mut bits, &self.quantizer.quantized[channel][range]);
                }
            }
        }
        bits.put(3, 7); // End element.
        bits.bytes
    }
}

fn packet_bytes(
    layout: Layout,
    choices: &[Vec<Choice>; 2],
    ms_mode: MsMode,
    stereo: &[bool],
) -> usize {
    let max_sfb = max_band(layout, choices);
    let emitted_ms = effective_ms_mode(layout, ms_mode, stereo, max_sfb);
    let stereo_bits = if layout.windows == 1 { 24 } else { 28 }
        + if emitted_ms == MsMode::PerBand {
            max_sfb * layout.groups
        } else {
            0
        };
    (stereo_bits
        + choices
            .iter()
            .map(|channel| channel_bits(layout, channel, max_sfb))
            .sum::<usize>())
    .div_ceil(8)
}

fn effective_ms_mode(layout: Layout, ms_mode: MsMode, stereo: &[bool], max_sfb: usize) -> MsMode {
    if max_sfb == 0 || ms_mode == MsMode::None {
        return MsMode::None;
    }
    if ms_mode == MsMode::All {
        return MsMode::All;
    }
    let mut any_true = false;
    let mut all_true = true;
    for group in stereo.chunks_exact(layout.band_count()) {
        for &flag in &group[..max_sfb] {
            any_true |= flag;
            all_true &= flag;
        }
    }
    if !any_true {
        MsMode::None
    } else if all_true {
        MsMode::All
    } else {
        MsMode::PerBand
    }
}

fn max_band(layout: Layout, choices: &[Vec<Choice>; 2]) -> usize {
    choices
        .iter()
        .flat_map(|channel| channel.chunks_exact(layout.band_count()))
        .filter_map(|group| {
            group
                .iter()
                .rposition(|choice| choice.book != Codebook::Zero)
        })
        .max()
        .map_or(0, |band| band + 1)
}

fn sections(layout: Layout, choices: &[Choice], max_sfb: usize) -> impl Iterator<Item = &[Choice]> {
    choices
        .chunks_exact(layout.band_count())
        .flat_map(move |group| group[..max_sfb].chunk_by(|a, b| a.book == b.book))
}

fn channel_bits(layout: Layout, choices: &[Choice], max_sfb: usize) -> usize {
    let width = layout.section_bits();
    let mut count = 11;
    for section in sections(layout, choices, max_sfb) {
        count += 4 + width * (section.len() / ((1 << width) - 1) + 1);
    }
    let mut previous = None;
    for choice in choices
        .iter()
        .filter(|choice| choice.book != Codebook::Zero)
    {
        let delta = i16::from(choice.sf) - i16::from(previous.unwrap_or(choice.sf));
        count += usize::from(choice.cost)
            + usize::from(code_len(SCALEFACTORS[(delta + SF_DELTA_MAX) as usize]));
        previous = Some(choice.sf);
    }
    count
}
