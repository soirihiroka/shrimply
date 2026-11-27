// SPDX-License-Identifier: GPL-2.0-or-later
// Port of Rubber Band 4.0.0 BinClassifier, BinSegmenter, and Guide,
// copyright 2007-2024 Particular Programs Ltd.

use std::ops::RangeInclusive;

const MODAL_WINDOW: usize = 18;

#[derive(Clone, Copy, Default)]
struct Segmentation {
    percussive: f64,
    residual: f64,
}

pub(crate) struct Classifier {
    horizontal: Vec<Median<7>>,
    vertical: Vec<f64>,
    classes: Vec<usize>,
    modes: Vec<usize>,
    cached_mode: Option<usize>,
    previous: Segmentation,
    current: Segmentation,
}

impl Classifier {
    pub(crate) fn new(bins: usize) -> Self {
        Self {
            horizontal: vec![Median::default(); bins],
            vertical: vec![0.0; bins],
            classes: vec![0; bins],
            modes: vec![0; bins],
            cached_mode: None,
            previous: Segmentation::default(),
            current: Segmentation::default(),
        }
    }

    fn classify(&mut self, mag: &[f64], bin_hz: f64, nyquist: f64) -> Segmentation {
        let bins = self.classes.len();
        let mut vertical = Median::<10>::default();
        for &value in &mag[..5] {
            vertical.push(value);
        }
        for i in 0..bins {
            let horizontal = &mut self.horizontal[i];
            horizontal.push(mag[i]);
            let h = horizontal.get();
            let v = self.vertical[i];
            self.classes[i] = if h > (v + 1.0e-7) * 2.0 {
                0
            } else if v > (h + 1.0e-7) * 2.0 {
                1
            } else {
                2
            };

            if i + 5 < bins {
                vertical.push(mag[i + 5]);
            } else {
                vertical.pop();
            }
            self.vertical[i] = vertical.get();
        }

        // Keep the selected mode across hops, as in upstream HistogramFilter.
        // A push can change the mode; draining the tail can invalidate it.
        let mut counts = [0; 3];
        let mut mode = self.cached_mode;
        let half = MODAL_WINDOW / 2;
        for i in 0..bins + half {
            if i >= MODAL_WINDOW {
                let dropped = self.classes[i - MODAL_WINDOW];
                counts[dropped] -= 1;
                if i >= bins && mode == Some(dropped) {
                    mode = None;
                }
            }
            if i < bins {
                let added = self.classes[i];
                counts[added] += 1;
                if let Some(selected) = mode
                    && (counts[added] > counts[selected]
                        || (counts[added] == counts[selected] && added < selected))
                {
                    mode = Some(added);
                }
            }
            if i >= half {
                self.modes[i - half] = *mode.get_or_insert_with(|| {
                    (0..counts.len())
                        .max_by_key(|&class| (counts[class], std::cmp::Reverse(class)))
                        .expect("three classification bins")
                });
            }
        }
        self.cached_mode = mode;

        let mut percussive = nyquist;
        let mut residual = nyquist;
        for (i, &class) in self.modes.iter().enumerate().skip(1).rev() {
            if residual == nyquist {
                if class == 1 {
                    residual = i as f64 * bin_hz;
                } else if class == 0 {
                    let hz = i as f64 * bin_hz;
                    return Segmentation {
                        percussive: hz,
                        residual: hz,
                    };
                }
            } else if class != 1 {
                percussive = i as f64 * bin_hz;
                break;
            }
        }
        if percussive == nyquist && residual < nyquist {
            percussive = 0.0;
        }
        Segmentation {
            percussive,
            residual,
        }
    }

    pub(crate) fn update(
        &mut self,
        guide: &mut Guidance,
        mag: &[f64],
        mean: f64,
        params: GuideParams,
    ) {
        let nyquist = params.bin_hz * (mag.len() - 1) as f64;
        let next = self.classify(mag, params.bin_hz, nyquist);
        let current = self.current;
        let previous = self.previous;
        self.previous = current;
        self.current = next;

        let previous_reset = guide.reset.take();
        guide.unlocked = None;
        guide.channel_lock = None;

        if params.reset_on_silence && mean < 1.0e-6 {
            guide.reset = Some(0.0..=nyquist);
        } else if (params.pitch - 1.0).abs() < 1.0e-7 {
            guide.reset = Some(match previous_reset {
                None => 16_000.0..=nyquist,
                Some(range) => {
                    let mut low = range.start() * 0.9;
                    let mut high = range.end() * 1.1;
                    if low < current.residual {
                        low = low.min(current.percussive);
                    }
                    if high > 16_000.0 {
                        high = nyquist;
                    }
                    if low < 100.0 {
                        low = 0.0;
                    }
                    low..=high
                }
            });
        } else {
            let mut channel_limit = if params.link_channels { nyquist } else { 600.0 };
            if current.residual > current.percussive {
                guide.unlocked = Some(current.percussive..=current.residual);
            }
            if current.residual > current.percussive + 4000.0
                && previous.residual < previous.percussive + 4000.0
            {
                let low = current.percussive.min(next.percussive);
                guide.reset = Some(
                    (if low < 200.0 { 0.0 } else { low })..=current.residual.max(next.residual),
                );
            }
            if params.pitch > 2.0 {
                channel_limit = (channel_limit - (params.pitch - 2.0) * 150.0).max(100.0);
                let above = (12_000.0 - (params.pitch - 2.0) * 400.0).max(channel_limit);
                let low = guide
                    .unlocked
                    .as_ref()
                    .map_or(above, |range| above.min(*range.start()));
                guide.unlocked = Some(low..=nyquist);
            }
            guide.channel_lock = Some(channel_limit);
        }
    }
}

pub(crate) struct GuideParams {
    pub(crate) pitch: f64,
    pub(crate) bin_hz: f64,
    pub(crate) link_channels: bool,
    pub(crate) reset_on_silence: bool,
}

pub(crate) struct Guidance {
    pub(crate) reset: Option<RangeInclusive<f64>>,
    pub(crate) unlocked: Option<RangeInclusive<f64>>,
    pub(crate) channel_lock: Option<f64>,
}

#[derive(Clone)]
struct Median<const N: usize> {
    history: [f64; N],
    sorted: [f64; N],
    next: usize,
    count: usize,
}

impl<const N: usize> Default for Median<N> {
    fn default() -> Self {
        Self {
            history: [0.0; N],
            sorted: [0.0; N],
            next: 0,
            count: 0,
        }
    }
}

impl<const N: usize> Median<N> {
    fn push(&mut self, sample: f64) {
        if self.count == N {
            self.pop();
        }
        let sample = if sample.is_finite() { sample } else { 0.0 };
        let at = self.sorted[..self.count].partition_point(|&x| x < sample);
        self.sorted.copy_within(at..self.count, at + 1);
        self.sorted[at] = sample;
        self.history[self.next] = sample;
        self.next = (self.next + 1) % N;
        self.count += 1;
    }

    fn pop(&mut self) {
        let oldest = self.history[(self.next + N - self.count) % N];
        let at = self.sorted[..self.count].partition_point(|&x| x < oldest);
        self.sorted.copy_within(at + 1..self.count, at);
        self.count -= 1;
    }

    fn get(&self) -> f64 {
        self.sorted[(self.count - 1) / 2]
    }
}
