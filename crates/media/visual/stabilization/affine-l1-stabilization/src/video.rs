use opencv::core::{Mat, Vector};
use opencv::prelude::*;
use opencv::videoio;
use shrimply_math_core::{Time, fraction_as_u32_ratio, frame_rate_from_f64};
use std::{ops::Range, path::Path};

/// The same source stream, frame bounds, and overlap convention for both analyzers.
pub struct VideoChunk {
    pub width: u32,
    pub height: u32,
    pub first_frame: u64,
    pub frame_rate_numerator: u32,
    pub frame_rate_denominator: u32,
    capture: videoio::VideoCapture,
    analysis_start: u64,
    analysis_end: u64,
    central_end: u64,
    next_frame: u64,
}

impl VideoChunk {
    pub fn open(
        input: &Path,
        track_id: u32,
        chunk_index: u64,
        chunk_seconds: u32,
        overlap_seconds: u32,
    ) -> Result<Self, String> {
        let input = input
            .to_str()
            .ok_or("video stabilization input path is not UTF-8")?;
        let mut capture = if track_id == 0 {
            videoio::VideoCapture::from_file(input, videoio::CAP_ANY)
        } else {
            videoio::VideoCapture::from_file_with_params(
                input,
                videoio::CAP_FFMPEG,
                &Vector::from_slice(&[
                    videoio::CAP_PROP_VIDEO_STREAM,
                    i32::try_from(track_id)
                        .map_err(|_| "video stabilization stream index is too large")?,
                ]),
            )
        }
        .map_err(|error| error.to_string())?;
        if !capture.is_opened().map_err(|error| error.to_string())? {
            return Err("video stabilization could not open the selected stream".into());
        }
        let dimension = |property| -> Result<u32, String> {
            let value = capture.get(property).map_err(|error| error.to_string())?;
            if !value.is_finite() || value < 1.0 || value > f64::from(i32::MAX) {
                return Err("video stabilization has invalid dimensions".into());
            }
            Ok(value.round() as u32)
        };
        let width = dimension(videoio::CAP_PROP_FRAME_WIDTH)?;
        let height = dimension(videoio::CAP_PROP_FRAME_HEIGHT)?;
        let fps = capture
            .get(videoio::CAP_PROP_FPS)
            .map_err(|error| error.to_string())?;
        if !fps.is_finite() || fps <= 0.0 {
            return Err("video stabilization requires a positive frame rate".into());
        }
        let frame_rate =
            frame_rate_from_f64(fps).ok_or("video stabilization frame rate is out of range")?;
        let (frame_rate_numerator, frame_rate_denominator) = fraction_as_u32_ratio(frame_rate)
            .ok_or("video stabilization frame rate is out of range")?;
        let first_frame =
            Time::from_seconds_u64(chunk_index.saturating_mul(u64::from(chunk_seconds)))
                .as_frame(frame_rate);
        let central_end = Time::from_seconds_u64(
            chunk_index
                .saturating_add(1)
                .saturating_mul(u64::from(chunk_seconds)),
        )
        .as_frame_ceil(frame_rate);
        let overlap = Time::from_seconds_u64(u64::from(overlap_seconds)).as_frame_ceil(frame_rate);
        let analysis_start = first_frame.saturating_sub(overlap);
        let analysis_end = central_end.saturating_add(overlap);
        capture
            .set(videoio::CAP_PROP_POS_FRAMES, analysis_start as f64)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            width,
            height,
            first_frame,
            frame_rate_numerator,
            frame_rate_denominator,
            capture,
            analysis_start,
            analysis_end,
            central_end,
            next_frame: analysis_start,
        })
    }

    pub fn read(&mut self) -> Result<Option<Mat>, String> {
        if self.next_frame >= self.analysis_end {
            return Ok(None);
        }
        let mut frame = Mat::default();
        if !self
            .capture
            .read(&mut frame)
            .map_err(|error| error.to_string())?
            || frame.empty()
        {
            return Ok(None);
        }
        self.next_frame = self.next_frame.saturating_add(1);
        Ok(Some(frame))
    }

    pub fn central_range(&self, frames: usize) -> Result<Range<usize>, String> {
        let first = self.first_frame.saturating_sub(self.analysis_start) as usize;
        let end = (self.central_end.saturating_sub(self.analysis_start) as usize).min(frames);
        if first >= end {
            return Err("video stabilization chunk contains no central frames".into());
        }
        Ok(first..end)
    }
}
