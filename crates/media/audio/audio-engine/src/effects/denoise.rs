use std::cell::RefCell;
use std::collections::VecDeque;

use df::tract::{DfParams, DfTract, RuntimeParams};
use ndarray::Array2;
use shrimply_project_document::project::Time;
use shrimply_property_model::timeline_value::TimelineBase;

use super::{CHANNELS, local_time};

thread_local! {
    static DEEP_FILTER_TEMPLATE: RefCell<Option<Result<DfTract, String>>> = const { RefCell::new(None) };
}

pub(super) struct RnnoiseState {
    channels: [Box<nnnoiseless::DenoiseState<'static>>; CHANNELS],
    input: [Vec<f32>; CHANNELS],
    output: [[f32; nnnoiseless::DenoiseState::FRAME_SIZE]; CHANNELS],
    ready: [VecDeque<f32>; CHANNELS],
    dry: [VecDeque<f32>; CHANNELS],
    amounts: VecDeque<f32>,
}

pub(super) struct DeepFilterNetState {
    pub(super) model: DfTract,
    input: Array2<f32>,
    output: Array2<f32>,
    pending: [Vec<f32>; CHANNELS],
    ready: [VecDeque<f32>; CHANNELS],
    dry: [VecDeque<f32>; CHANNELS],
    amounts: VecDeque<f32>,
    pending_reduction_db: f32,
    model_delay_remaining: usize,
}

impl RnnoiseState {
    pub(super) fn new() -> Self {
        let frame_size = nnnoiseless::DenoiseState::FRAME_SIZE;
        Self {
            channels: std::array::from_fn(|_| nnnoiseless::DenoiseState::new()),
            input: std::array::from_fn(|_| Vec::with_capacity(frame_size)),
            output: [[0.0; nnnoiseless::DenoiseState::FRAME_SIZE]; CHANNELS],
            ready: std::array::from_fn(|_| VecDeque::with_capacity(frame_size * 2)),
            dry: std::array::from_fn(|_| VecDeque::with_capacity(frame_size * 2)),
            amounts: VecDeque::with_capacity(frame_size * 2),
        }
    }

    pub(super) fn process(
        &mut self,
        samples: &mut [f32],
        value: &shrimply_audio_modifiers::DenoiseModifier,
        local_start: Time,
        sample_rate: u32,
    ) {
        let frame_size = nnnoiseless::DenoiseState::FRAME_SIZE;
        let constant_amount = match &value.amount.base {
            TimelineBase::Const(amount) => Some(amount.clamp(0.0, 1.0)),
            TimelineBase::Keyframes(_) => None,
        };
        for (frame, channels) in samples.as_chunks_mut::<CHANNELS>().0.iter_mut().enumerate() {
            let amount = constant_amount.unwrap_or_else(|| {
                value
                    .amount
                    .value_at(local_time(local_start, frame, sample_rate))
                    .clamp(0.0, 1.0)
            });
            self.amounts.push_back(amount);
            for (channel, sample) in channels.iter().copied().enumerate() {
                self.dry[channel].push_back(sample);
                self.input[channel].push(sample * i16::MAX as f32);
            }
            if self.input[0].len() == frame_size {
                for channel in 0..CHANNELS {
                    self.channels[channel]
                        .process_frame(&mut self.output[channel], &self.input[channel]);
                    self.ready[channel].extend(
                        self.output[channel]
                            .iter()
                            .map(|sample| *sample / i16::MAX as f32),
                    );
                    self.input[channel].clear();
                }
            }
            if self.ready[0].is_empty() {
                channels.fill(0.0);
                continue;
            }
            let amount = self.amounts.pop_front().unwrap_or(0.0);
            for (channel, sample) in channels.iter_mut().enumerate() {
                let dry = self.dry[channel].pop_front().unwrap_or(0.0);
                let wet = self.ready[channel].pop_front().unwrap_or(dry);
                *sample = dry + (wet - dry) * amount;
            }
        }
    }
}

impl DeepFilterNetState {
    pub(super) fn new(sample_rate: u32) -> Result<Self, String> {
        let model = DEEP_FILTER_TEMPLATE.with(|template| {
            let mut template = template.borrow_mut();
            template
                .get_or_insert_with(|| {
                    DfTract::new(
                        DfParams::default(),
                        &RuntimeParams::default_with_ch(CHANNELS),
                    )
                    .map_err(|error| error.to_string())
                })
                .clone()
        })?;
        if model.sr != sample_rate as usize {
            return Err(format!(
                "DeepFilterNet requires {} Hz audio, got {sample_rate} Hz",
                model.sr
            ));
        }
        let input = Array2::zeros((CHANNELS, model.hop_size));
        let output = Array2::zeros((CHANNELS, model.hop_size));
        let hop = model.hop_size;
        let model_delay_remaining = model
            .fft_size
            .saturating_sub(model.hop_size)
            .saturating_add(model.lookahead * model.hop_size);
        Ok(Self {
            model,
            input,
            output,
            pending: std::array::from_fn(|_| Vec::with_capacity(hop)),
            ready: std::array::from_fn(|_| VecDeque::with_capacity(hop * 2)),
            dry: std::array::from_fn(|_| VecDeque::with_capacity(hop * 2)),
            amounts: VecDeque::with_capacity(hop * 2),
            pending_reduction_db: 0.0,
            model_delay_remaining,
        })
    }

    pub(super) fn process(
        &mut self,
        samples: &mut [f32],
        value: &shrimply_audio_modifiers::DenoiseModifier,
        local_start: Time,
        sample_rate: u32,
    ) -> Result<(), String> {
        let hop = self.model.hop_size;
        let constant_amount = match &value.amount.base {
            TimelineBase::Const(amount) => Some(amount.clamp(0.0, 1.0)),
            TimelineBase::Keyframes(_) => None,
        };
        let constant_reduction = match &value.reduction_db.base {
            TimelineBase::Const(reduction) => Some(reduction.clamp(0.0, 100.0)),
            TimelineBase::Keyframes(_) => None,
        };
        for (frame, channels) in samples.as_chunks_mut::<CHANNELS>().0.iter_mut().enumerate() {
            if self.pending[0].is_empty() {
                self.pending_reduction_db = constant_reduction.unwrap_or_else(|| {
                    value
                        .reduction_db
                        .value_at(local_time(local_start, frame, sample_rate))
                        .clamp(0.0, 100.0)
                });
            }
            let amount = constant_amount.unwrap_or_else(|| {
                value
                    .amount
                    .value_at(local_time(local_start, frame, sample_rate))
                    .clamp(0.0, 1.0)
            });
            self.amounts.push_back(amount);
            for (channel, sample) in channels.iter().copied().enumerate() {
                self.dry[channel].push_back(sample);
                self.pending[channel].push(sample);
            }
            if self.pending[0].len() == hop {
                for channel in 0..CHANNELS {
                    for index in 0..hop {
                        self.input[[channel, index]] = self.pending[channel][index];
                    }
                }
                self.model.set_atten_lim(self.pending_reduction_db);
                self.model
                    .process(self.input.view(), self.output.view_mut())
                    .map_err(|error| error.to_string())?;
                for channel in 0..CHANNELS {
                    self.ready[channel].extend(self.output.row(channel).iter().copied());
                    self.pending[channel].clear();
                }
            }
            if self.ready[0].is_empty() {
                channels.fill(0.0);
                continue;
            }
            if self.model_delay_remaining > 0 {
                self.model_delay_remaining -= 1;
                for ready in &mut self.ready {
                    ready.pop_front();
                }
                channels.fill(0.0);
                continue;
            }
            let amount = self.amounts.pop_front().unwrap_or(0.0);
            for (channel, sample) in channels.iter_mut().enumerate() {
                let dry = self.dry[channel].pop_front().unwrap_or(0.0);
                let wet = self.ready[channel].pop_front().unwrap_or(dry);
                *sample = dry + (wet - dry) * amount;
            }
        }
        Ok(())
    }

    pub(super) fn latency_frames(&self) -> usize {
        self.model
            .fft_size
            .saturating_sub(self.model.hop_size)
            .saturating_add(self.model.lookahead * self.model.hop_size)
            .saturating_add(self.model.hop_size.saturating_sub(1))
    }
}
