use ffmpeg_next::format::Pixel;
use shrimply_asset::Asset;
use shrimply_math_core::{Time, fraction_as_u32_ratio};
use shrimply_video_decoder::{
    DecodeOutcome, DecoderBackend, NextFrame, RgbFrameConverter, VideoDecoderSession,
};
use shrimply_vision::{RgbaImage, math::rotate_rgba};
use std::{ops::Range, path::Path};

/// The same source stream, frame bounds, and overlap convention for both analyzers.
pub struct VideoChunk {
    pub width: u32,
    pub height: u32,
    pub first_frame: u64,
    pub frame_rate_numerator: u32,
    pub frame_rate_denominator: u32,
    decoder: VideoDecoderSession,
    scaler: RgbFrameConverter,
    rotation_degrees: i32,
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
        let source = Asset::new(input).snapshot()?;
        let decoder = VideoDecoderSession::open(
            &source,
            track_id,
            DecoderBackend::hardware_for_readback,
            None,
        )?;
        let rotation_degrees = decoder.rotation_degrees();
        let (width, height) = decoder.dimensions();
        let (width, height) = if rotation_degrees % 180 == 90 {
            (height, width)
        } else {
            (width, height)
        };
        if width == 0 || height == 0 || width > i32::MAX as u32 || height > i32::MAX as u32 {
            return Err("video stabilization has invalid dimensions".into());
        }
        let frame_rate = decoder.frame_rate();
        let (frame_rate_numerator, frame_rate_denominator) = fraction_as_u32_ratio(frame_rate)
            .filter(|(numerator, _)| *numerator > 0)
            .ok_or("video stabilization requires a positive frame rate")?;
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
        Ok(Self {
            width,
            height,
            first_frame,
            frame_rate_numerator,
            frame_rate_denominator,
            decoder,
            scaler: RgbFrameConverter::new(Pixel::BGR24)?,
            rotation_degrees,
            analysis_start,
            analysis_end,
            central_end,
            next_frame: analysis_start,
        })
    }

    pub fn read(&mut self) -> Result<Option<RgbaImage>, String> {
        if self.next_frame >= self.analysis_end {
            return Ok(None);
        }
        let decoded = if self.next_frame == self.analysis_start {
            match self.decoder.frame_by_index(self.next_frame, [None, None])? {
                DecodeOutcome::Frame(frame) => frame,
                DecodeOutcome::Superseded(_) => {
                    return Err("video stabilization decode was superseded".into());
                }
            }
        } else {
            match self.decoder.next_frame([None, None])? {
                NextFrame::Frame(frame) => Some(frame),
                NextFrame::EndOfStream => None,
                NextFrame::Superseded => {
                    return Err("video stabilization decode was superseded".into());
                }
            }
        };
        let Some((_, frame)) = decoded else {
            self.next_frame = self.analysis_end;
            return Ok(None);
        };
        let downloaded = frame.download()?;
        let frame = downloaded.as_ref().unwrap_or_else(|| frame.native());
        let (width, height) = if self.rotation_degrees % 180 == 90 {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        };
        if frame.width() != width || frame.height() != height {
            return Err("video stabilization frame dimensions changed".into());
        }
        // Keep the stabilization reference pipeline’s BGR24 conversion before channel reordering.
        let bgr = self.scaler.convert(frame)?;
        let row_bytes = width as usize * 3;
        let data = bgr
            .data(0)
            .chunks(bgr.stride(0))
            .take(height as usize)
            .flat_map(|row| row[..row_bytes].chunks_exact(3))
            .flat_map(|pixel| [pixel[2], pixel[1], pixel[0], u8::MAX])
            .collect();
        self.next_frame = self.next_frame.saturating_add(1);
        Ok(Some(rotate_rgba(
            RgbaImage {
                width: width as usize,
                height: height as usize,
                data,
            },
            self.rotation_degrees,
        )?))
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
