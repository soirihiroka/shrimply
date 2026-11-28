//! Shrimply AAC, an independent stereo AAC-LC encoder for offline export.

mod bits;
mod mdct;
mod psychoacoustics;
mod quantize;
mod tables;

use std::ops::Range;

use bits::{Bits, Book};
use mdct::Analysis;
use psychoacoustics::Masking;
use quantize::{Choice, Search, gains};
use tables::*;

pub const FRAME_SAMPLES: usize = 1024;
pub const SAMPLE_RATES: [u32; 12] = [
    96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000,
];
const FRAME_LIMIT: usize = 2 * 6144;

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
    bands: &'static [usize],
}

impl Layout {
    fn new(bands: &'static [usize], short: bool) -> Self {
        Self {
            groups: if short { 4 } else { 1 },
            windows: if short { 8 } else { 1 },
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
        let per_group = self.windows / self.groups;
        let size = FRAME_SAMPLES / self.windows;
        (group * per_group..(group + 1) * per_group).map(move |window| {
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
    average_demand: f32,
    delayed: Option<[[f32; 2]; FRAME_SAMPLES]>,
    overlap: [[f32; 2]; 2 * FRAME_SAMPLES],
    envelope: [f32; 2],
    previous: [f32; 2],
    attacks: [bool; 2],
    window: Window,
    layouts: [Layout; 2],
    masking: [Masking; 2],
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
        let (long, short) = match rate_index {
            0 | 1 => (BANDS_56, BANDS_57),
            2 => (BANDS_54, BANDS_55),
            3 | 4 => (BANDS_45, BANDS_46),
            5 => (BANDS_47, BANDS_46),
            6 | 7 => (BANDS_52, BANDS_53),
            8..=10 => (BANDS_50, BANDS_51),
            _ => (BANDS_48, BANDS_49),
        };
        let layouts = [Layout::new(long, false), Layout::new(short, true)];
        Ok(Self {
            rate_index: rate_index as u8,
            sample_rate,
            bitrate,
            remainder: 0,
            reservoir: 0,
            average_demand: 0.0,
            delayed: None,
            overlap: [[0.0; 2]; 2 * FRAME_SAMPLES],
            envelope: [0.0; 2],
            previous: [0.0; 2],
            attacks: [false; 2],
            window: Window::Long,
            layouts,
            masking: layouts.map(|layout| Masking::new(sample_rate, layout)),
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
        let next = mdct::transient(&block, &mut self.envelope, &mut self.previous);
        let attack = self.attacks[1] || next[0];
        self.attacks = next;
        let current = self.delayed.replace(block)?;
        self.window = match (self.window, attack) {
            (Window::Start, _) | (Window::Short, true) => Window::Short,
            (Window::Short, false) => Window::Stop,
            (_, true) => Window::Start,
            (_, false) => Window::Long,
        };
        self.overlap.copy_within(FRAME_SAMPLES.., 0);
        self.overlap[FRAME_SAMPLES..].copy_from_slice(&current);
        let index = usize::from(matches!(self.window, Window::Short));
        let layout = self.layouts[index];
        let mut spectra = [[0.0; FRAME_SAMPLES]; 2];
        for (channel, spectrum) in spectra.iter_mut().enumerate() {
            self.analysis
                .transform(self.window, &self.overlap, channel, spectrum);
        }
        let stereo = stereo(layout, &mut spectra);
        let masking = spectra
            .each_ref()
            .map(|lines| self.masking[index].thresholds(layout, lines));
        let demand = self.quantizer.analyze(layout, &spectra, &masking);

        self.remainder += u64::from(self.bitrate) * FRAME_SAMPLES as u64;
        let frame_budget = (self.remainder / u64::from(self.sample_rate)) as i64;
        self.remainder %= u64::from(self.sample_rate);
        // Budget more bits for difficult frames while keeping the long-run rate bounded.
        let demand_ratio = ((demand + 256.0) / (self.average_demand + 256.0)).sqrt();
        self.average_demand += (demand - self.average_demand) / 16.0;
        let target = (frame_budget as f32 * demand_ratio.clamp(0.5, 1.75)) as i64;
        let available = (target + self.reservoir / 8).clamp(
            frame_budget / 2,
            frame_budget + self.reservoir + frame_budget / 2,
        ) as usize;
        let limit = available.min(FRAME_LIMIT) / 8;
        let packet = self.allocate(layout, &spectra, &stereo, limit);
        self.reservoir =
            (self.reservoir + frame_budget - (packet.len() * 8) as i64).min(frame_budget * 2);
        Some(packet)
    }

    fn allocate(
        &self,
        layout: Layout,
        spectra: &[[f32; FRAME_SAMPLES]; 2],
        stereo: &[bool],
        limit: usize,
    ) -> Vec<u8> {
        let size = |choices: &[Vec<Choice>; 2]| {
            let max_sfb = max_band(layout, choices);
            let stereo_bits = if layout.groups == 1 { 24 } else { 28 }
                + if stereo_used(layout, stereo, max_sfb) {
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
        };
        let mut low = -20.0_f32;
        let mut high = 40.0_f32;
        let mut picks = std::array::from_fn(|_| Vec::new());
        let mut candidate =
            std::array::from_fn(|_| Vec::with_capacity(layout.groups * layout.band_count()));
        self.quantizer.select(layout, low.exp2(), &mut candidate);
        if size(&candidate) <= limit {
            picks = candidate;
        } else {
            for _ in 0..11 {
                let lambda = (low + high) / 2.0;
                self.quantizer.select(layout, lambda.exp2(), &mut candidate);
                if size(&candidate) <= limit {
                    high = lambda;
                    std::mem::swap(&mut picks, &mut candidate);
                } else {
                    low = lambda;
                }
            }
        }
        // The discrete choices can leave room below the packet limit. Spend it on
        // the largest remaining masking violations using exact section and offset bits.
        if size(&picks) < limit {
            let mut worst: Vec<_> = picks
                .iter()
                .enumerate()
                .flat_map(|(channel, choices)| {
                    choices
                        .iter()
                        .enumerate()
                        .map(move |(band, choice)| (channel, band, choice.error))
                })
                .collect();
            worst.sort_unstable_by(|left, right| right.2.total_cmp(&left.2));
            for (channel, band, _) in worst.into_iter().take(6) {
                let previous = picks[channel][..band]
                    .iter()
                    .rfind(|choice| choice.book != 0)
                    .map(|choice| choice.sf);
                let next = picks[channel][band + 1..]
                    .iter()
                    .find(|choice| choice.book != 0)
                    .map(|choice| choice.sf);
                let mut best = picks[channel][band];
                for &choice in &self.quantizer.bands[channel][band] {
                    if choice.error >= best.error
                        || choice.book == 0
                        || previous.is_some_and(|sf| sf.abs_diff(choice.sf) > 60)
                        || next.is_some_and(|sf| sf.abs_diff(choice.sf) > 60)
                    {
                        continue;
                    }
                    picks[channel][band] = choice;
                    if size(&picks) <= limit {
                        best = choice;
                    }
                }
                picks[channel][band] = best;
            }
        }
        let packet = self.packet(layout, spectra, stereo, &picks);
        debug_assert_eq!(packet.len(), size(&picks));
        debug_assert!(packet.len() <= limit);
        packet
    }

    fn packet(
        &self,
        layout: Layout,
        spectra: &[[f32; FRAME_SAMPLES]; 2],
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
        bits.put(1, 1); // Kaiser–Bessel derived window.
        if layout.windows == 8 {
            bits.put(4, max_sfb as u32);
            bits.put(7, 0b1010101); // Four groups of two short windows.
        } else {
            bits.put(6, max_sfb as u32);
            bits.put(1, 0); // No prediction.
        }
        let ms = stereo_used(layout, stereo, max_sfb);
        bits.put(2, u32::from(ms));
        if ms {
            for group in stereo.chunks_exact(layout.band_count()) {
                for &mid_side in &group[..max_sfb] {
                    bits.put(1, u32::from(mid_side));
                }
            }
        }

        let mut values = [0_i16; FRAME_SAMPLES];
        for (channel, (choices, spectrum)) in choices.iter().zip(spectra).enumerate() {
            let global_gain = choices
                .iter()
                .find(|choice| choice.book != 0)
                .map_or(100, |choice| choice.sf);
            bits.put(8, u32::from(global_gain));
            for section in sections(layout, choices, max_sfb) {
                bits.put(4, u32::from(section[0].book));
                let width = layout.section_bits();
                let escape = (1 << width) - 1;
                for _ in 0..section.len() / escape {
                    bits.put(width, escape as u32);
                }
                bits.put(width, (section.len() % escape) as u32);
            }
            let mut previous = global_gain;
            for choice in choices.iter().filter(|choice| choice.book != 0) {
                let delta = i16::from(choice.sf) - i16::from(previous);
                bits.code(SCALEFACTORS[(delta + 60) as usize]);
                previous = choice.sf;
            }
            bits.put(3, 0); // No pulse data, TNS or gain control.
            for (index, choice) in choices
                .iter()
                .enumerate()
                .filter(|(_, choice)| choice.book != 0)
            {
                let group = index / layout.band_count();
                let band = index % layout.band_count();
                let (_, factor) = gains(choice.sf);
                let book = Book::new(choice.book);
                for range in layout.ranges(group, band) {
                    for (value, i) in values.iter_mut().zip(range.clone()) {
                        *value = self.quantizer.coefficient(channel, i, spectrum[i], factor);
                    }
                    book.write(&mut bits, choice.book, &values[..range.len()]);
                }
            }
        }
        bits.put(3, 7); // End element.
        bits.bytes
    }
}

fn max_band(layout: Layout, choices: &[Vec<Choice>; 2]) -> usize {
    choices
        .iter()
        .flat_map(|channel| channel.chunks_exact(layout.band_count()))
        .filter_map(|group| group.iter().rposition(|choice| choice.book != 0))
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
    for choice in choices.iter().filter(|choice| choice.book != 0) {
        let delta = i16::from(choice.sf) - i16::from(previous.unwrap_or(choice.sf));
        count += usize::from(choice.cost) + (SCALEFACTORS[(delta + 60) as usize] & 31) as usize;
        previous = Some(choice.sf);
    }
    count
}

fn stereo_used(layout: Layout, stereo: &[bool], max_sfb: usize) -> bool {
    stereo
        .chunks_exact(layout.band_count())
        .any(|group| group[..max_sfb].contains(&true))
}

fn stereo(layout: Layout, spectra: &mut [[f32; FRAME_SAMPLES]; 2]) -> Vec<bool> {
    let [left, right] = spectra;
    let mut mask = Vec::with_capacity(layout.groups * layout.band_count());
    for group in 0..layout.groups {
        for band in 0..layout.band_count() {
            let mut cross = 0.0;
            let mut energy = 0.0;
            for index in layout.ranges(group, band).flatten() {
                cross += left[index] * right[index];
                energy += left[index].powi(2) + right[index].powi(2);
            }
            let mid_side = 2.0 * cross.abs() > 0.6 * energy;
            mask.push(mid_side);
            if mid_side {
                for index in layout.ranges(group, band).flatten() {
                    (left[index], right[index]) = (
                        (left[index] + right[index]) * 0.5,
                        (left[index] - right[index]) * 0.5,
                    );
                }
            }
        }
    }
    mask
}
