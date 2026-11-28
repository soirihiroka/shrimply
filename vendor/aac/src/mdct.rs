use std::{f32::consts::PI, sync::Arc};

use rustfft::{Fft, FftPlanner, num_complex::Complex32};

use crate::{FRAME_SAMPLES, Window};

const SHORT: usize = FRAME_SAMPLES / 8;
const OFFSET: usize = (FRAME_SAMPLES - SHORT) / 2;

pub(crate) struct Analysis {
    long: Transform,
    short: Transform,
    start: Vec<f32>,
    stop: Vec<f32>,
}

impl Analysis {
    pub fn new() -> Self {
        let mut planner = FftPlanner::new();
        let long = Transform::new(FRAME_SAMPLES, &mut planner);
        let short = Transform::new(SHORT, &mut planner);
        let mut start = long.window.clone();
        start[FRAME_SAMPLES..FRAME_SAMPLES + OFFSET].fill(1.0);
        start[FRAME_SAMPLES + OFFSET..FRAME_SAMPLES + OFFSET + SHORT]
            .copy_from_slice(&short.window[SHORT..]);
        start[FRAME_SAMPLES + OFFSET + SHORT..].fill(0.0);
        let stop = start.iter().rev().copied().collect();
        Self {
            long,
            short,
            start,
            stop,
        }
    }

    pub fn transform(
        &mut self,
        window: Window,
        samples: &[[f32; 2]],
        channel: usize,
        spectrum: &mut [f32; FRAME_SAMPLES],
    ) {
        match window {
            Window::Short => {
                for (index, lines) in spectrum.chunks_exact_mut(SHORT).enumerate() {
                    let start = OFFSET + index * SHORT;
                    self.short
                        .run(&samples[start..start + 2 * SHORT], channel, None, lines);
                }
            }
            Window::Long => self.long.run(samples, channel, None, spectrum),
            Window::Start => self.long.run(samples, channel, Some(&self.start), spectrum),
            Window::Stop => self.long.run(samples, channel, Some(&self.stop), spectrum),
        }
    }
}

struct Transform {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    input_rotation: Vec<Complex32>,
    output_rotation: Vec<Complex32>,
    spectrum: Vec<Complex32>,
    scratch: Vec<Complex32>,
}

impl Transform {
    fn new(size: usize, planner: &mut FftPlanner<f32>) -> Self {
        let fft = planner.plan_fft_forward(2 * size);
        let scratch = vec![Complex32::ZERO; fft.get_inplace_scratch_len()];
        let length = size as f32;
        let window = (0..2 * size)
            .map(|n| (PI * (n as f32 + 0.5) / (2.0 * length)).sin())
            .collect();
        // A half-bin FFT shift followed by the MDCT's half-window phase shift.
        let input_rotation = (0..2 * size)
            .map(|n| Complex32::from_polar(1.0, -PI * n as f32 / (2.0 * length)))
            .collect();
        let output_rotation = (0..size)
            .map(|k| {
                // AAC's factor of two and the conversion from normalized PCM.
                Complex32::from_polar(
                    65536.0,
                    -PI * (k as f32 + 0.5) * (0.5 + length / 2.0) / length,
                )
            })
            .collect();
        Self {
            fft,
            window,
            input_rotation,
            output_rotation,
            spectrum: vec![Complex32::ZERO; 2 * size],
            scratch,
        }
    }

    fn run(
        &mut self,
        samples: &[[f32; 2]],
        channel: usize,
        window: Option<&[f32]>,
        lines: &mut [f32],
    ) {
        let window = window.unwrap_or(&self.window);
        for (bin, ((sample, amplitude), rotation)) in self
            .spectrum
            .iter_mut()
            .zip(samples.iter().zip(window).zip(&self.input_rotation))
        {
            *bin = rotation * (sample[channel] * amplitude);
        }
        self.fft
            .process_with_scratch(&mut self.spectrum, &mut self.scratch);
        for (line, (bin, rotation)) in lines
            .iter_mut()
            .zip(self.spectrum.iter().zip(&self.output_rotation))
        {
            *line = (bin * rotation).re;
        }
    }
}

pub(crate) fn transient(
    samples: &[[f32; 2]],
    smooth: &mut [f32; 2],
    previous: &mut [f32; 2],
) -> [bool; 2] {
    let mut attack = [false; 2];
    for (index, block) in samples.chunks_exact(64).enumerate() {
        let mut energy = [0.0; 2];
        for sample in block {
            for ((energy, previous), current) in energy.iter_mut().zip(&mut *previous).zip(sample) {
                let difference = current - *previous;
                *energy += difference * difference;
                *previous = *current;
            }
        }
        for (long, current) in smooth.iter_mut().zip(energy) {
            attack[usize::from(index * 64 >= OFFSET)] |= current > 12.0 * *long + 0.0003;
            *long = *long * 0.8 + current * 0.2;
        }
    }
    attack
}
