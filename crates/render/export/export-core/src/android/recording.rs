//! Screen projection produces frames directly on FFmpeg's MediaCodec input window.
use super::encoder_window;
use crate::video::mux::Muxer;
use ffmpeg::{Rational, codec::encoder::video::Encoder, format::Pixel};
use ffmpeg_next as ffmpeg;
use ndk::native_window::NativeWindow;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize)]
pub struct Settings {
    path: PathBuf,
    width: u32,
    height: u32,
    bitrate: usize,
    codec_name: String,
    fps_numerator: i32,
    fps_denominator: i32,
    keyframe_seconds: u32,
}

pub struct Recording {
    encoder: Encoder,
    muxer: Muxer<'static>,
    first_pts: Option<i64>,
    pending: Option<ffmpeg::Packet>,
    frame_duration: i64,
}

impl Recording {
    pub fn open(settings: Settings) -> Result<(Self, NativeWindow), String> {
        let map = |error: ffmpeg::Error| error.to_string();
        ffmpeg::init().map_err(map)?;
        if settings.fps_numerator <= 0 || settings.fps_denominator <= 0 {
            return Err("Screen recording frame rate must be positive".into());
        }
        let fps = Rational(settings.fps_numerator, settings.fps_denominator);
        let time_base = Rational(1, ffmpeg::sys::AV_TIME_BASE);
        use ffmpeg::Rescale;
        let frame_duration = 1i64.rescale(fps.invert(), time_base).max(1);
        let codec = ffmpeg::encoder::find_by_name("h264_mediacodec")
            .ok_or("FFmpeg hardware AVC encoder is unavailable")?;
        let mut encoder = ffmpeg::codec::Context::new_with_codec(codec)
            .encoder()
            .video()
            .map_err(map)?;
        encoder.set_width(settings.width);
        encoder.set_height(settings.height);
        encoder.set_bit_rate(settings.bitrate);
        encoder.set_time_base(time_base);
        encoder.set_frame_rate(Some(fps));
        encoder.set_gop(
            i64::from(settings.keyframe_seconds)
                .rescale((1, 1), fps.invert())
                .max(1)
                .try_into()
                .map_err(|_| "Invalid recording keyframe interval")?,
        );
        encoder.set_max_b_frames(0);
        encoder.set_format(Pixel::MEDIACODEC);
        let window = encoder_window(&mut encoder)?;
        let mut options = ffmpeg::Dictionary::new();
        options.set("ndk_codec", "1");
        options.set("ndk_async", "0");
        options.set("screen_capture", "1");
        options.set("codec_name", &settings.codec_name);
        let encoder = encoder.open_as_with(codec, options).map_err(map)?;
        let mut output = ffmpeg::format::output_as(&settings.path, "mp4").map_err(map)?;
        let mut stream = output.add_stream_with(encoder.as_ref()).map_err(map)?;
        stream.set_time_base(time_base);
        stream.set_avg_frame_rate(fps);
        stream.set_rate(fps);
        let index = stream.index();
        let muxer = Muxer::new(output, index, None, true, None)?;
        Ok((
            Self {
                encoder,
                muxer,
                first_pts: None,
                pending: None,
                frame_duration,
            },
            window,
        ))
    }

    /// Drain frames already submitted by MediaProjection. The first surface
    /// timestamp is monotonic device time; the project recording starts at zero.
    pub fn poll(&mut self) -> Result<Option<bool>, String> {
        let mut packet = ffmpeg::Packet::empty();
        match self.encoder.receive_packet(&mut packet) {
            Ok(()) => {
                let pts = packet
                    .pts()
                    .ok_or("Screen recording frame has no timestamp")?;
                let time = pts
                    .checked_sub(*self.first_pts.get_or_insert(pts))
                    .ok_or("Invalid screen timestamp")?;
                packet.set_pts(Some(time));
                packet.set_dts(Some(time));
                if let Some(mut previous) = self.pending.replace(packet) {
                    let interval =
                        time - previous.pts().expect("normalized packet has a timestamp");
                    if interval <= 0 {
                        return Err("Screen recording timestamps must advance".into());
                    }
                    previous.set_duration(interval);
                    self.muxer.write_video(previous, &self.encoder)?;
                }
                Ok(Some(true))
            }
            Err(ffmpeg::Error::Other { errno }) if errno == libc::EAGAIN => Ok(Some(false)),
            Err(ffmpeg::Error::Eof) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    pub fn finish(mut self, elapsed_us: i64) -> Result<i64, String> {
        self.encoder.send_eof().map_err(|error| error.to_string())?;
        while self.poll()?.is_some() {}
        // Some encoders stop repeating frames on a static screen. Hold the
        // last sample until projection stops, including the idle tail in MP4.
        let mut last = self
            .pending
            .take()
            .ok_or("Screen recording captured no video frames")?;
        let last_pts = last.pts().expect("normalized packet has a timestamp");
        let duration = elapsed_us.max(
            last_pts
                .checked_add(self.frame_duration)
                .ok_or("Screen recording duration overflow")?,
        );
        last.set_duration(duration - last_pts);
        self.muxer.write_video(last, &self.encoder)?;
        self.muxer.finish()?;
        Ok(duration)
    }
}
