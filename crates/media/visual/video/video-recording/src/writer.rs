use crate::FinishedVideoRecording;
use ffmpeg::{format::Pixel, software::scaling};
use ffmpeg_next as ffmpeg;
use shrimply_math_core::{Fraction, Time};

use std::{fs, path::PathBuf, time::Instant};
use uuid::Uuid;

const DEFAULT_FPS_NUMERATOR: u64 = 30;
const DEFAULT_FPS_DENOMINATOR: u64 = 1;
pub(super) const BYTES_PER_PIXEL: usize = 4;
const NVENC_CONSTANT_QP: &str = "20";
const NVENC_B_FRAMES: usize = 0;
const NVENC_KEYFRAME_INTERVAL_SECONDS: u32 = 1;

pub(super) fn recording_paths() -> Result<(PathBuf, PathBuf), String> {
    let directory = shrimply_path_core::recording_directory()?;
    let name = Uuid::new_v4().to_string();
    let final_path = directory.join(format!("{name}.mp4"));
    let temporary_path = directory.join(format!("{name}.mp4.part"));
    Ok((final_path, temporary_path))
}

fn open_hevc_encoder(
    width: u32,
    height: u32,
    time_base: ffmpeg::Rational,
    frame_rate: ffmpeg::Rational,
    global_header: bool,
) -> Result<ffmpeg::codec::encoder::video::Encoder, String> {
    let codec = ffmpeg::codec::encoder::find_by_name("hevc_nvenc")
        .ok_or_else(|| "FFmpeg encoder hevc_nvenc was not found".to_string())?;
    let mut encoder = ffmpeg::codec::Context::new_with_codec(codec)
        .encoder()
        .video()
        .map_err(|error| error.to_string())?;
    encoder.set_width(width);
    encoder.set_height(height);
    encoder.set_format(Pixel::YUV420P);
    // Match libswscale's default RGB/gray to limited-range YUV conversion at every resolution.
    encoder.set_colorspace(ffmpeg::color::Space::SMPTE170M);
    encoder.set_color_range(ffmpeg::color::Range::MPEG);
    encoder.set_time_base(time_base);
    encoder.set_frame_rate(Some(frame_rate));
    encoder.set_max_b_frames(NVENC_B_FRAMES);
    encoder.set_gop(
        ((u128::from(NVENC_KEYFRAME_INTERVAL_SECONDS) * frame_rate.0 as u128)
            / frame_rate.1 as u128)
            .max(1)
            .min(u128::from(u32::MAX)) as u32,
    );
    if global_header {
        unsafe {
            (*encoder.as_mut_ptr()).flags |= ffmpeg::sys::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        }
    }
    let mut options = ffmpeg::Dictionary::new();
    options.set("preset", "p3");
    options.set("tune", "ll");
    options.set("profile", "main");
    options.set("rc", "constqp");
    options.set("qp", NVENC_CONSTANT_QP);
    options.set("bf", &NVENC_B_FRAMES.to_string());
    options.set("spatial-aq", "1");
    options.set("temporal-aq", "0");
    options.set("zerolatency", "1");
    options.set("delay", "0");
    encoder
        .open_as_with(codec, options)
        .map_err(|error| format!("Could not open hevc_nvenc: {error}"))
}

pub(super) fn valid_fps(fps: Fraction) -> Fraction {
    if shrimply_math_core::fraction_numerator(fps) > 0
        && shrimply_math_core::fraction_denominator(fps) > 0
    {
        fps
    } else {
        Fraction::new_raw(DEFAULT_FPS_NUMERATOR, DEFAULT_FPS_DENOMINATOR)
    }
}

fn fps_parts_i32(fps: Fraction) -> Result<(i32, i32), String> {
    let numerator = shrimply_math_core::fraction_numerator(fps);
    let denominator = shrimply_math_core::fraction_denominator(fps);
    if numerator <= 0
        || denominator <= 0
        || numerator > i64::from(i32::MAX)
        || denominator > i64::from(i32::MAX)
    {
        return Err("Recording frame rate is outside FFmpeg's supported range".to_string());
    }
    Ok((numerator as i32, denominator as i32))
}

#[derive(Clone, Copy, PartialEq)]
pub(super) struct CaptureFormat {
    pub pixel: Pixel,
    pub width: u32,
    pub height: u32,
    /// A separate alpha track can use the fourth byte or be filled with opaque samples.
    pub alpha: Option<bool>,
}

struct Track {
    encoder: ffmpeg::codec::encoder::video::Encoder,
    index: usize,
    scaler: scaling::Context,
    input: ffmpeg::frame::Video,
    frame: ffmpeg::frame::Video,
}

impl Track {
    fn new(
        output: &mut ffmpeg::format::context::Output,
        input: CaptureFormat,
        pixel: Pixel,
        size: [u32; 2],
        rate: ffmpeg::Rational,
        title: Option<&str>,
    ) -> Result<Self, String> {
        let time_base = rate.invert();
        let global_header = output
            .format()
            .flags()
            .contains(ffmpeg::format::Flags::GLOBAL_HEADER);
        let encoder = open_hevc_encoder(size[0], size[1], time_base, rate, global_header)?;
        let mut stream = output
            .add_stream_with(encoder.as_ref())
            .map_err(|e| e.to_string())?;
        stream.set_time_base(time_base);
        stream.set_rate(rate);
        stream.set_avg_frame_rate(rate);
        if let Some(title) = title {
            let mut metadata = ffmpeg::Dictionary::new();
            metadata.set("title", title);
            stream.set_metadata(metadata);
        }
        let (scaler, source) = Self::scaler(input, pixel, size)?;
        Ok(Self {
            index: stream.index(),
            encoder,
            scaler,
            input: source,
            frame: ffmpeg::frame::Video::new(Pixel::YUV420P, size[0], size[1]),
        })
    }

    fn scaler(
        input: CaptureFormat,
        pixel: Pixel,
        [w, h]: [u32; 2],
    ) -> Result<(scaling::Context, ffmpeg::frame::Video), String> {
        let scaler = scaling::Context::get(
            pixel,
            input.width,
            input.height,
            Pixel::YUV420P,
            w,
            h,
            scaling::Flags::BILINEAR,
        )
        .map_err(|e| e.to_string())?;
        Ok((
            scaler,
            ffmpeg::frame::Video::new(pixel, input.width, input.height),
        ))
    }

    fn submit(
        &mut self,
        pts: i64,
        output: &mut ffmpeg::format::context::Output,
    ) -> Result<(), String> {
        self.frame.set_pts(Some(pts));
        self.encoder
            .send_frame(&self.frame)
            .map_err(|e| e.to_string())?;
        self.drain(output)
    }

    fn drain(&mut self, output: &mut ffmpeg::format::context::Output) -> Result<(), String> {
        // The muxer chooses the final stream time base when it writes the header.
        let time_base = output
            .stream(self.index)
            .ok_or("Missing screen recording stream")?
            .time_base();
        loop {
            let mut packet = ffmpeg::Packet::empty();
            match self.encoder.receive_packet(&mut packet) {
                Ok(()) => {
                    packet.set_stream(self.index);
                    packet.rescale_ts(self.encoder.time_base(), time_base);
                    packet
                        .write_interleaved(output)
                        .map_err(|error| error.to_string())?;
                }
                Err(ffmpeg::Error::Other { errno }) if errno == ffmpeg::error::EAGAIN => {
                    return Ok(());
                }
                Err(ffmpeg::Error::Eof) => return Ok(()),
                Err(error) => return Err(error.to_string()),
            }
        }
    }
}

pub(super) struct VideoWriter {
    output: ffmpeg::format::context::Output,
    color: Track,
    alpha: Option<Track>,
    input: CaptureFormat,
    pub width: u32,
    pub height: u32,
    fps: Fraction,
    first_timestamp: Option<(i64, u64)>,
    started: Option<Instant>,
    last_pts: Option<i64>,
    final_path: PathBuf,
    temporary_path: PathBuf,
}

impl VideoWriter {
    pub fn new(
        input: CaptureFormat,
        fps: Fraction,
        final_path: PathBuf,
        temporary_path: PathBuf,
    ) -> Result<Self, String> {
        let size = [input.width & !1, input.height & !1];
        if size.contains(&0) {
            return Err("Screen capture frame is too small".into());
        }
        let mut output =
            ffmpeg::format::output_as(&temporary_path, "mp4").map_err(|e| e.to_string())?;
        let (num, den) = fps_parts_i32(fps)?;
        let rate = ffmpeg::Rational(num, den);
        let color = Track::new(
            &mut output,
            input,
            input.pixel,
            size,
            rate,
            input.alpha.map(|_| "RGB"),
        )?;
        let alpha = input
            .alpha
            .map(|_| Track::new(&mut output, input, Pixel::GRAY8, size, rate, Some("Alpha")))
            .transpose()?;
        output.write_header().map_err(|e| e.to_string())?;
        Ok(Self {
            output,
            color,
            alpha,
            input,
            width: size[0],
            height: size[1],
            fps,
            first_timestamp: None,
            started: None,
            last_pts: None,
            final_path,
            temporary_path,
        })
    }

    pub fn configure_input(&mut self, input: CaptureFormat) -> Result<(), String> {
        if self.input == input {
            return Ok(());
        }
        let size = [self.width, self.height];
        (self.color.scaler, self.color.input) = Track::scaler(input, input.pixel, size)?;
        if let Some(track) = &mut self.alpha {
            (track.scaler, track.input) = Track::scaler(input, Pixel::GRAY8, size)?;
        }
        self.input = input;
        Ok(())
    }

    pub fn write_frame(
        &mut self,
        bytes: &[u8],
        stride: i32,
        timestamp: Option<i64>,
    ) -> Result<(), String> {
        let bottom_up = stride < 0;
        let stride = stride.unsigned_abs() as usize;
        let width = self.input.width as usize;
        let height = self.input.height as usize;
        let row_bytes = width * BYTES_PER_PIXEL;
        let required = stride.saturating_mul(height - 1).saturating_add(row_bytes);
        if stride < row_bytes || bytes.len() < required {
            return Err("Screen capture returned an incomplete frame".into());
        }
        let wall_time = self.started.get_or_insert_with(Instant::now).elapsed();
        let elapsed_ns = u64::try_from(wall_time.as_nanos()).unwrap_or(u64::MAX);
        let elapsed = match timestamp {
            Some(timestamp) => {
                let (first, offset) = *self.first_timestamp.get_or_insert((timestamp, elapsed_ns));
                offset.saturating_add(timestamp.saturating_sub(first).max(0) as u64)
            }
            None => elapsed_ns,
        };
        let pts = shrimply_math_core::frame_index(Time::from_nanos(elapsed), self.fps)
            .ok_or("Invalid recording frame rate")?;
        if self.last_pts.is_some_and(|last| pts <= last) {
            return Ok(());
        }
        let color_stride = self.color.input.stride(0);
        let color = self.color.input.data_mut(0);
        let mut alpha = self.alpha.as_mut().map(|track| {
            let stride = track.input.stride(0);
            (stride, track.input.data_mut(0))
        });
        for row in 0..height {
            let start = (if bottom_up { height - row - 1 } else { row }) * stride;
            let src = &bytes[start..start + row_bytes];
            color[row * color_stride..row * color_stride + row_bytes].copy_from_slice(src);
            if let Some((stride, plane)) = &mut alpha {
                let offset = row * *stride;
                let dst = &mut plane[offset..offset + width];
                if self.input.alpha == Some(true) {
                    for (alpha, pixel) in dst.iter_mut().zip(src.chunks_exact(BYTES_PER_PIXEL)) {
                        *alpha = pixel[3];
                    }
                } else {
                    dst.fill(u8::MAX);
                }
            }
        }
        for track in std::iter::once(&mut self.color).chain(self.alpha.iter_mut()) {
            track
                .scaler
                .run(&track.input, &mut track.frame)
                .map_err(|e| e.to_string())?;
            track.submit(pts, &mut self.output)?;
        }
        self.last_pts = Some(pts);
        Ok(())
    }

    pub fn finish(mut self) -> Result<FinishedVideoRecording, String> {
        let last = self
            .last_pts
            .ok_or("Screen recording captured no video frames")?;
        // A damage-driven capture can stop producing frames while the screen is static.
        // Both timelines start at the first delivered sample.
        let elapsed = self
            .started
            .expect("encoded frame has a start time")
            .elapsed();
        let end = shrimply_math_core::frame_index(
            Time::from_nanos(u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX)),
            self.fps,
        )
        .ok_or("Invalid recording frame rate")?
        .max(last);
        for track in std::iter::once(&mut self.color).chain(self.alpha.iter_mut()) {
            if end > last {
                track.submit(end, &mut self.output)?;
            }
            track.encoder.send_eof().map_err(|e| e.to_string())?;
            track.drain(&mut self.output)?;
        }
        self.output.write_trailer().map_err(|e| e.to_string())?;
        drop(self.output);
        fs::rename(&self.temporary_path, &self.final_path).map_err(|e| e.to_string())?;
        let duration = Time::from_fraction(
            end.saturating_add(1)
                .saturating_mul(shrimply_math_core::fraction_denominator(self.fps)),
            shrimply_math_core::fraction_numerator(self.fps),
        );
        Ok(FinishedVideoRecording::new(
            self.final_path,
            duration,
            self.width,
            self.height,
            self.alpha.map(|track| track.index as u32),
        ))
    }
}
