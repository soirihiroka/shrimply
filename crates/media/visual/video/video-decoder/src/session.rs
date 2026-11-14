use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use crate::backend::DecoderBackend;
use crate::math::{frame_time, source_time_to_stream_timestamp};
use ffmpeg::sys;
use ffmpeg_next as ffmpeg;
use shrimply_asset::AssetSnapshot;
use shrimply_math_core::Time;
use shrimply_visual_frame::VideoFrame;

#[cfg(feature = "cuda")]
use crate::lifecycle::DecoderStartupMeasurement;
use crate::{LOCAL_FORWARD_DECODE_SECONDS, MAX_NONADVANCING_FRAMES};

#[derive(Clone)]
pub struct DecodeControl {
    generation: u64,
    latest_generation: Arc<AtomicU64>,
}

impl DecodeControl {
    pub fn new(generation: u64, latest_generation: Arc<AtomicU64>) -> Self {
        Self {
            generation,
            latest_generation,
        }
    }

    pub fn superseded(&self) -> bool {
        self.latest_generation.load(Ordering::Acquire) != self.generation
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn generation_check(&self) -> (&AtomicU64, u64) {
        (self.latest_generation.as_ref(), self.generation)
    }
}

pub type DecodedVisual = (Time, Arc<VideoFrame>);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodeMode {
    BestEffort,
    Accurate,
    Continuous,
    LocalScrub,
}

impl DecodeMode {
    pub const fn time_accurate(self) -> bool {
        !matches!(self, Self::BestEffort)
    }

    pub const fn continuous(self) -> bool {
        matches!(self, Self::Continuous)
    }

    pub const fn realtime(self) -> bool {
        matches!(self, Self::Continuous | Self::LocalScrub)
    }
}

pub type DecodeControls<'a> = [Option<&'a DecodeControl>; 2];

pub enum DecodeOutcome {
    Frame(Option<DecodedVisual>),
    Superseded(Option<DecodedVisual>),
}

struct DecodeRequestState {
    target: Time,
    best_before: Option<DecodedVisual>,
}

pub struct VideoDecoderSession {
    source: AssetSnapshot,
    media_track_id: u32,
    input: ffmpeg::format::context::Input,
    stream_index: usize,
    stream_time_base: ffmpeg::Rational,
    stream_start_time: i64,
    pub(crate) frame_duration: Time,
    decoder: ffmpeg::decoder::Video,
    #[cfg(target_os = "android")]
    android_pool: Option<Arc<shrimply_visual_frame::android::AndroidFramePool>>,
    decoder_name: String,
    frame_index: usize,
    pub(crate) last_decoded_position: Option<Time>,
    previous_decoded_position: Option<Time>,
    lookahead: Option<DecodedVisual>,
    pending_packet: Option<ffmpeg::Packet>,
    input_eof: bool,
    eof_sent: bool,
    decoder_eof: bool,
    pub(crate) initialized: bool,
    #[cfg(feature = "cuda")]
    pub(crate) startup: Option<DecoderStartupMeasurement>,
    #[cfg(feature = "cuda")]
    pub(crate) startup_bytes: u64,
    opened_at: Option<Instant>,
    seek_started_at: Option<Instant>,
    seek_target: Option<Time>,
    seek_first_frame_received: bool,
    active_request: Option<DecodeRequestState>,
    nonadvancing_frames: usize,
}

impl VideoDecoderSession {
    pub fn next_frame_time(&self) -> Option<Time> {
        self.lookahead.as_ref().map(|frame| frame.0)
    }

    pub fn open(
        source: &AssetSnapshot,
        media_track_id: u32,
        backend: impl FnOnce(&ffmpeg::codec::Parameters) -> Result<DecoderBackend, String>,
        cancellation: Option<DecodeControl>,
    ) -> Result<Self, String> {
        source.verify_current()?;
        ffmpeg::init().map_err(|error| error.to_string())?;
        let input = ffmpeg::format::input_with_interrupt(&source.path(), move || {
            cancellation.as_ref().is_some_and(DecodeControl::superseded)
        })
        .map_err(|error| error.to_string())?;
        let (stream_index, stream_time_base, stream_start_time, frame_rate, parameters) = {
            let stream = input
                .streams()
                .filter(|stream| stream.parameters().medium() == ffmpeg::media::Type::Video)
                .filter(|stream| {
                    !stream
                        .disposition()
                        .contains(ffmpeg::format::stream::Disposition::ATTACHED_PIC)
                })
                .nth(media_track_id as usize)
                .ok_or_else(|| format!("video stream {} not found", media_track_id))?;
            let mut frame_rate = stream.avg_frame_rate();
            if frame_rate.numerator() <= 0 || frame_rate.denominator() <= 0 {
                frame_rate = stream.rate();
            }
            (
                stream.index(),
                stream.time_base(),
                stream.start_time(),
                frame_rate,
                stream.parameters(),
            )
        };
        let frame_duration = if frame_rate.numerator() > 0 && frame_rate.denominator() > 0 {
            Time::from_fraction(
                i64::from(frame_rate.denominator()),
                i64::from(frame_rate.numerator()),
            )
        } else {
            Time::from_fraction(
                i64::from(stream_time_base.numerator()),
                i64::from(stream_time_base.denominator()),
            )
        };

        let decoder_open_started = Instant::now();
        let backend = backend(&parameters)?;
        #[cfg(target_os = "android")]
        let android_pool = match &backend {
            DecoderBackend::Android { pool, .. } => Some(pool.clone()),
            _ => None,
        };
        let decoder = backend.open(&parameters, stream_time_base)?;
        let decoder_name = decoder
            .codec()
            .ok_or("opened video decoder has no codec")?
            .name()
            .to_owned();
        Ok(Self {
            source: source.clone(),
            media_track_id,
            input,
            stream_index,
            stream_time_base,
            stream_start_time,
            frame_duration,
            decoder,
            #[cfg(target_os = "android")]
            android_pool,
            decoder_name,
            frame_index: 0,
            last_decoded_position: None,
            previous_decoded_position: None,
            lookahead: None,
            pending_packet: None,
            input_eof: false,
            eof_sent: false,
            decoder_eof: false,
            initialized: false,
            #[cfg(feature = "cuda")]
            startup: None,
            #[cfg(feature = "cuda")]
            startup_bytes: 0,
            opened_at: Some(decoder_open_started),
            seek_started_at: None,
            seek_target: None,
            seek_first_frame_received: false,
            active_request: None,
            nonadvancing_frames: 0,
        })
    }

    pub fn frame(
        &mut self,
        position: Time,
        cached: Option<DecodedVisual>,
        controls: DecodeControls<'_>,
        mode: DecodeMode,
    ) -> Result<DecodeOutcome, String> {
        if mode.time_accurate() {
            self.frame_at_exact(position, cached, controls, mode.continuous())
        } else {
            self.frame_at(position, cached, controls)
        }
    }

    fn frame_at(
        &mut self,
        position: Time,
        cached: Option<DecodedVisual>,
        controls: DecodeControls<'_>,
    ) -> Result<DecodeOutcome, String> {
        let _measurement = shrimply_profiling::measure("Video decode / Interactive total");
        if self.cancel_if_superseded(controls) {
            return Ok(DecodeOutcome::Superseded(cached));
        }
        if let Some(frame) = cached_frame(cached.as_ref(), position, self.frame_duration) {
            shrimply_profiling::increment("Video decoder retained state / Hit");
            return Ok(DecodeOutcome::Frame(Some(frame)));
        }
        if self.decoder_eof && self.position_at_or_after_decoded_end(position) {
            self.active_request = None;
            let frame = cached.filter(|frame| frame.0 <= position).ok_or_else(|| {
                self.missing_frame_error(position, "interactive request started at end-of-stream")
            })?;
            return Ok(DecodeOutcome::Frame(Some(frame)));
        }

        // Random interactive seeks deliberately return the first decoded frame. A later settled
        // request takes the exact path and decodes from that keyframe to the requested timestamp.
        let decode_to_target = self.can_decode_forward(
            position,
            self.frame_duration,
            Some(Time::from_seconds(LOCAL_FORWARD_DECODE_SECONDS)),
        );
        let continuing_active_seek = self
            .active_request
            .as_ref()
            .is_some_and(|request| request.target == position);
        if !decode_to_target && !continuing_active_seek {
            self.seek(position)?;
        } else {
            self.active_request = Some(DecodeRequestState {
                target: position,
                best_before: cached.clone().filter(|frame| frame.0 <= position),
            });
        }

        let target = decode_to_target.then_some(position);
        let mut progress = cached.clone().filter(|frame| frame.0 <= position);
        let _packet_loop = shrimply_profiling::measure("Video decode / Interactive packet loop");
        loop {
            let frame = match self.next_frame(controls)? {
                NextFrame::Frame(frame) => frame,
                NextFrame::EndOfStream => {
                    self.active_request = None;
                    let frame = cached.filter(|frame| frame.0 <= position).ok_or_else(|| {
                        self.missing_frame_error(
                            position,
                            "interactive request reached end-of-stream",
                        )
                    })?;
                    return Ok(DecodeOutcome::Frame(Some(frame)));
                }
                NextFrame::Superseded => return Ok(DecodeOutcome::Superseded(progress)),
            };
            if frame.0 <= position {
                progress = Some(frame.clone());
                if let Some(request) = self.active_request.as_mut() {
                    request.best_before = Some(frame.clone());
                }
            }
            if target.is_some_and(|target| frame.0.saturating_add(self.frame_duration) < target) {
                shrimply_profiling::increment(
                    "Video decode / Frames skipped before requested timestamp",
                );
            } else {
                return Ok(DecodeOutcome::Frame(Some(frame)));
            }
        }
    }

    fn frame_at_exact(
        &mut self,
        position: Time,
        cached: Option<DecodedVisual>,
        controls: DecodeControls<'_>,
        continuous: bool,
    ) -> Result<DecodeOutcome, String> {
        let _measurement = shrimply_profiling::measure("Video decode / Exact total");
        if self.cancel_if_superseded(controls) {
            return Ok(DecodeOutcome::Superseded(cached));
        }
        if let Some(frame) = cached.as_ref().filter(|frame| frame.0 == position).cloned() {
            shrimply_profiling::increment("Video decoder retained state / Exact hit");
            self.finish_exact(position);
            return Ok(DecodeOutcome::Frame(Some(frame)));
        }
        let continuing_active_seek = self
            .active_request
            .as_ref()
            .is_some_and(|request| request.target == position);
        let mut candidate = cached.filter(|frame| frame.0 <= position);
        if let Some(best_before) = self
            .active_request
            .as_ref()
            .filter(|request| request.target == position)
            .and_then(|request| request.best_before.clone())
        {
            candidate = Some(best_before);
        }
        if self.decoder_eof && self.position_at_or_after_decoded_end(position) {
            self.finish_exact(position);
            let frame = candidate.ok_or_else(|| {
                self.missing_frame_error(position, "exact request started at end-of-stream")
            })?;
            return Ok(DecodeOutcome::Frame(Some(frame)));
        }

        let maximum_forward_gap =
            (!continuous).then_some(Time::from_seconds(LOCAL_FORWARD_DECODE_SECONDS));
        // Cancellation can discard the candidate preceding a retained lookahead frame. Without
        // that candidate, decoding forward from beyond the target would return lookahead forever.
        let backward_tolerance = if cached_frame(candidate.as_ref(), position, self.frame_duration)
            .is_some()
            || (cfg!(target_os = "windows")
                && candidate_precedes_retained_lookahead(
                    candidate.as_ref().map(|frame| frame.0),
                    self.previous_decoded_position,
                    position,
                    self.lookahead.as_ref().map(|frame| frame.0),
                    self.last_decoded_position,
                )) {
            self.frame_duration
        } else {
            Time::ZERO
        };
        if !continuing_active_seek
            && !self.can_decode_forward(position, backward_tolerance, maximum_forward_gap)
        {
            self.seek(position)?;
            candidate = candidate.filter(|frame| frame.0 <= position);
        } else if !continuing_active_seek {
            self.active_request = Some(DecodeRequestState {
                target: position,
                best_before: candidate.clone(),
            });
        }

        let _fill = shrimply_profiling::measure("Video decode / Exact fill");
        loop {
            let frame = match self.next_frame(controls)? {
                NextFrame::Frame(frame) => frame,
                NextFrame::EndOfStream => {
                    self.finish_exact(position);
                    let frame = candidate.ok_or_else(|| {
                        self.missing_frame_error(position, "exact request reached end-of-stream")
                    })?;
                    return Ok(DecodeOutcome::Frame(Some(frame)));
                }
                NextFrame::Superseded => return Ok(DecodeOutcome::Superseded(candidate)),
            };
            if frame.0 <= position {
                candidate = Some(frame.clone());
                if let Some(request) = self.active_request.as_mut() {
                    request.best_before = Some(frame);
                }
                continue;
            }

            if candidate.is_none() {
                candidate = Some(frame.clone());
            }
            self.lookahead = Some(frame);
            self.finish_exact(position);
            return Ok(DecodeOutcome::Frame(candidate));
        }
    }

    pub(crate) fn seek(&mut self, position: Time) -> Result<(), String> {
        shrimply_profiling::increment("Video decode / Seek");
        let _measurement = shrimply_profiling::measure("Video decode / Seek time");
        let target_timestamp = source_time_to_stream_timestamp(
            position,
            self.stream_time_base,
            self.stream_start_time,
        );
        let result = unsafe {
            sys::avformat_seek_file(
                self.input.as_mut_ptr(),
                self.stream_index as i32,
                i64::MIN,
                target_timestamp,
                target_timestamp,
                sys::AVSEEK_FLAG_BACKWARD,
            )
        };
        if result < 0 {
            return Err(format!(
                "seek video demuxer: {}",
                ffmpeg::Error::from(result)
            ));
        }
        self.decoder.flush();
        shrimply_profiling::increment("Video decode / Decoder flush");
        self.frame_index = 0;
        self.last_decoded_position = None;
        self.previous_decoded_position = None;
        self.lookahead = None;
        self.pending_packet = None;
        self.input_eof = false;
        self.eof_sent = false;
        self.decoder_eof = false;
        self.seek_started_at = Some(Instant::now());
        self.seek_target = Some(position);
        self.seek_first_frame_received = false;
        self.nonadvancing_frames = 0;
        self.active_request = Some(DecodeRequestState {
            target: position,
            best_before: None,
        });
        Ok(())
    }

    fn can_decode_forward(
        &self,
        position: Time,
        backward_tolerance: Time,
        maximum_forward_gap: Option<Time>,
    ) -> bool {
        let Some(last_decoded_position) = self.last_decoded_position else {
            return false;
        };
        if position.saturating_add(backward_tolerance) < last_decoded_position {
            return false;
        }
        maximum_forward_gap
            .is_none_or(|maximum| position.saturating_sub(last_decoded_position) <= maximum)
    }

    fn position_at_or_after_decoded_end(&self, position: Time) -> bool {
        let Some(last_decoded_position) = self.last_decoded_position else {
            return true;
        };
        position >= last_decoded_position
    }

    pub fn next_frame(&mut self, controls: DecodeControls<'_>) -> Result<NextFrame, String> {
        let result = self.next_frame_inner(controls);
        if result.is_err() {
            self.active_request = None;
            self.seek_target = None;
            self.seek_started_at = None;
        }
        result
    }

    fn next_frame_inner(&mut self, controls: DecodeControls<'_>) -> Result<NextFrame, String> {
        if self.cancel_if_superseded(controls) {
            return Ok(NextFrame::Superseded);
        }
        if let Some(frame) = self.lookahead.take() {
            return Ok(NextFrame::Frame(frame));
        }
        let mut send_was_blocked = false;
        loop {
            match self.receive_frame()? {
                ReceiveState::Frame(frame) => {
                    if self.cancel_if_superseded(controls) {
                        self.lookahead = Some(frame);
                        return Ok(NextFrame::Superseded);
                    }
                    return Ok(NextFrame::Frame(frame));
                }
                ReceiveState::EndOfStream => return Ok(NextFrame::EndOfStream),
                ReceiveState::NeedInput if send_was_blocked => {
                    shrimply_profiling::increment("Video decode / Fatal send errors");
                    return Err(
                        "video decoder made no progress: send and receive both returned EAGAIN"
                            .to_string(),
                    );
                }
                ReceiveState::NeedInput => {}
            }

            if self.pending_packet.is_none() && !self.input_eof {
                if self.cancel_if_superseded(controls) {
                    return Ok(NextFrame::Superseded);
                }
                self.pending_packet = self.read_video_packet()?;
                self.input_eof = self.pending_packet.is_none();
            }

            if let Some(packet) = self.pending_packet.as_ref() {
                shrimply_profiling::increment("Video decode / Video packets offered");
                match self.decoder.send_packet(packet) {
                    Ok(()) => {
                        self.pending_packet = None;
                        shrimply_profiling::increment("Video decode / Video packets accepted");
                        if self.cancel_if_superseded(controls) {
                            return Ok(NextFrame::Superseded);
                        }
                    }
                    Err(ffmpeg::Error::Other { errno }) if errno == libc::EAGAIN => {
                        shrimply_profiling::increment("Video decode / Send packet EAGAIN");
                        send_was_blocked = true;
                    }
                    Err(error) => {
                        shrimply_profiling::increment("Video decode / Fatal send errors");
                        return Err(format!("{} send packet: {error}", self.decoder_name));
                    }
                }
                continue;
            }

            if self.eof_sent {
                shrimply_profiling::increment("Video decode / Fatal receive errors");
                return Err(
                    "video decoder requested input after end-of-stream was sent".to_string()
                );
            }
            match self.decoder.send_eof() {
                Ok(()) => self.eof_sent = true,
                Err(ffmpeg::Error::Other { errno }) if errno == libc::EAGAIN => {
                    shrimply_profiling::increment("Video decode / Send EOF EAGAIN");
                    send_was_blocked = true;
                }
                Err(ffmpeg::Error::Eof) => {
                    self.eof_sent = true;
                    self.decoder_eof = true;
                    shrimply_profiling::increment("Video decode / Decoder EOF");
                    return Ok(NextFrame::EndOfStream);
                }
                Err(error) => {
                    shrimply_profiling::increment("Video decode / Fatal send errors");
                    return Err(format!("video decoder send EOF: {error}"));
                }
            }
        }
    }

    fn receive_frame(&mut self) -> Result<ReceiveState, String> {
        if self.decoder_eof {
            return Ok(ReceiveState::EndOfStream);
        }

        let _measurement = shrimply_profiling::measure("Video decode / Receive frame");
        let mut decoded = ffmpeg::frame::Video::empty();
        match self.decoder.receive_frame(&mut decoded) {
            Ok(()) => {
                shrimply_profiling::increment("Video decode / Decoded frames received");
                let position = frame_time(
                    &decoded,
                    self.stream_time_base,
                    self.stream_start_time,
                    self.frame_index,
                    self.last_decoded_position,
                    self.frame_duration,
                    self.seek_target.is_some(),
                )?;
                if self
                    .last_decoded_position
                    .is_some_and(|previous| position <= previous)
                {
                    self.nonadvancing_frames = self.nonadvancing_frames.saturating_add(1);
                    if self.nonadvancing_frames >= MAX_NONADVANCING_FRAMES {
                        return Err(format!(
                            "video decoder produced {MAX_NONADVANCING_FRAMES} non-advancing timestamps at {}",
                            position.as_label(),
                        ));
                    }
                } else {
                    self.nonadvancing_frames = 0;
                }
                self.previous_decoded_position = self.last_decoded_position;
                self.last_decoded_position = Some(position);
                self.frame_index += 1;
                self.record_first_frame(&decoded);
                #[cfg(target_os = "android")]
                let frame = if let Some(pool) = &self.android_pool {
                    VideoFrame::from_android(decoded, pool.clone())?
                } else {
                    VideoFrame::new(decoded)?
                };
                #[cfg(not(target_os = "android"))]
                let frame = VideoFrame::new(decoded)?;
                let frame = Arc::new(frame);
                Ok(ReceiveState::Frame((position, frame)))
            }
            Err(ffmpeg::Error::Other { errno }) if errno == libc::EAGAIN => {
                shrimply_profiling::increment("Video decode / Receive frame EAGAIN");
                Ok(ReceiveState::NeedInput)
            }
            Err(ffmpeg::Error::Eof) => {
                self.decoder_eof = true;
                shrimply_profiling::increment("Video decode / Decoder EOF");
                Ok(ReceiveState::EndOfStream)
            }
            Err(error) => {
                shrimply_profiling::increment("Video decode / Fatal receive errors");
                Err(format!("video decoder receive frame: {error}"))
            }
        }
    }

    fn read_video_packet(&mut self) -> Result<Option<ffmpeg::Packet>, String> {
        loop {
            let mut packet = ffmpeg::Packet::empty();
            match packet.read(&mut self.input) {
                Ok(()) => {}
                Err(ffmpeg::Error::Eof) => return Ok(None),
                Err(error) => return Err(format!("read video packet: {error}")),
            }
            let stream_index = packet.stream();
            shrimply_profiling::increment("Video decode / Demux packets read");
            if stream_index == self.stream_index {
                if packet.duration() > 0 {
                    self.frame_duration = Time::from_fraction(
                        packet
                            .duration()
                            .saturating_mul(i64::from(self.stream_time_base.numerator())),
                        i64::from(self.stream_time_base.denominator()),
                    );
                }
                return Ok(Some(packet));
            }
        }
    }

    fn missing_frame_error(&self, position: Time, stage: &str) -> String {
        format!(
            "{stage} without a usable frame: file={}, media_track_id={}, requested={}, last_decoded={}, seek_target={}, decoder_eof={}",
            self.source.path().display(),
            self.media_track_id,
            position.as_label(),
            self.last_decoded_position
                .map_or_else(|| "none".to_string(), |time| time.as_label()),
            self.seek_target
                .map_or_else(|| "none".to_string(), |time| time.as_label()),
            self.decoder_eof,
        )
    }

    fn record_first_frame(&mut self, frame: &ffmpeg::frame::Video) {
        if let Some(opened_at) = self.opened_at.take() {
            shrimply_profiling::record(
                "Video decode / Decoder open to first frame",
                opened_at.elapsed(),
            );
        }
        if !self.seek_first_frame_received {
            if let Some(seek_started_at) = self.seek_started_at {
                shrimply_profiling::record(
                    "Video decode / Seek to first decoded frame",
                    seek_started_at.elapsed(),
                );
            }
            self.seek_first_frame_received = true;
        }
        if self.initialized {
            return;
        }
        #[cfg(feature = "cuda")]
        if let Some(startup) = self.startup.take() {
            startup.record(None, &mut self.startup_bytes);
        }
        self.initialized = true;

        unsafe {
            let context = self.decoder.as_ptr();
            let raw_frame = frame.as_ptr();
            let frames_context = (*raw_frame)
                .hw_frames_ctx
                .as_ref()
                .map(|reference| reference.data.cast::<sys::AVHWFramesContext>())
                .unwrap_or(ptr::null_mut());
            let (software_format, initial_pool_size) = if frames_context.is_null() {
                (sys::AVPixelFormat::AV_PIX_FMT_NONE, 0)
            } else {
                (
                    (*frames_context).sw_format,
                    (*frames_context).initial_pool_size,
                )
            };
            tracing::info!(
                codec = self.decoder_name,
                codec_id = ?self.decoder.id(),
                flags = (*context).flags,
                low_delay = ((*context).flags & sys::AV_CODEC_FLAG_LOW_DELAY as i32) != 0,
                hardware_format = (*raw_frame).format,
                software_format = ?software_format,
                coded_width = (*context).coded_width,
                coded_height = (*context).coded_height,
                display_width = (*context).width,
                display_height = (*context).height,
                initial_pool_size,
                color_space = ?(*raw_frame).colorspace,
                color_range = ?(*raw_frame).color_range,
                color_primaries = ?(*raw_frame).color_primaries,
                color_transfer = ?(*raw_frame).color_trc,
                "received first decoded video frame",
            );
        }
    }

    fn record_seek_to_exact(&mut self, position: Time) {
        if self.seek_target != Some(position) {
            return;
        }
        if let Some(seek_started_at) = self.seek_started_at.take() {
            shrimply_profiling::record(
                "Video decode / Seek to requested exact frame",
                seek_started_at.elapsed(),
            );
        }
        self.seek_target = None;
    }

    fn finish_exact(&mut self, position: Time) {
        self.record_seek_to_exact(position);
        if self
            .active_request
            .as_ref()
            .is_some_and(|request| request.target == position)
        {
            self.active_request = None;
        }
    }

    fn cancel_if_superseded(&mut self, controls: DecodeControls<'_>) -> bool {
        let superseded = controls
            .into_iter()
            .flatten()
            .any(DecodeControl::superseded);
        if superseded {
            shrimply_profiling::increment("Video decode / Superseded requests");
            self.active_request = None;
        }
        superseded
    }
}

pub enum NextFrame {
    Frame(DecodedVisual),
    EndOfStream,
    Superseded,
}

enum ReceiveState {
    Frame(DecodedVisual),
    NeedInput,
    EndOfStream,
}

fn cached_frame(
    cached: Option<&DecodedVisual>,
    position: Time,
    frame_duration: Time,
) -> Option<DecodedVisual> {
    cached
        .filter(|frame| frame.0 <= position && position.saturating_sub(frame.0) < frame_duration)
        .cloned()
}

fn candidate_precedes_retained_lookahead(
    candidate: Option<Time>,
    previous_decoded: Option<Time>,
    requested: Time,
    lookahead: Option<Time>,
    last_decoded: Option<Time>,
) -> bool {
    candidate.is_some_and(|candidate| {
        previous_decoded == Some(candidate)
            && candidate <= requested
            && lookahead
                .is_some_and(|lookahead| requested < lookahead && last_decoded == Some(lookahead))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantized_candidate_precedes_retained_lookahead() {
        let candidate = Time::from_fraction(33, 1_000);
        let requested = Time::from_fraction(1, 15);
        let lookahead = Time::from_fraction(67, 1_000);
        let frame_duration = Time::from_fraction(33, 1_000);

        assert!(requested.saturating_sub(candidate) >= frame_duration);
        assert!(candidate_precedes_retained_lookahead(
            Some(candidate),
            Some(candidate),
            requested,
            Some(lookahead),
            Some(lookahead),
        ));
    }

    #[test]
    fn stale_candidate_does_not_precede_retained_lookahead() {
        let candidate = Time::ZERO;
        let previous_decoded = Time::from_fraction(33, 1_000);
        let requested = Time::from_fraction(1, 15);
        let lookahead = Time::from_fraction(67, 1_000);

        assert!(!candidate_precedes_retained_lookahead(
            Some(candidate),
            Some(previous_decoded),
            requested,
            Some(lookahead),
            Some(lookahead),
        ));
    }

    #[test]
    fn incomplete_or_mismatched_predecessor_state_is_rejected() {
        let candidate = Time::from_fraction(33, 1_000);
        let requested = Time::from_fraction(1, 15);
        let lookahead = Time::from_fraction(67, 1_000);

        assert!(!candidate_precedes_retained_lookahead(
            None,
            Some(candidate),
            requested,
            Some(lookahead),
            Some(lookahead),
        ));
        assert!(!candidate_precedes_retained_lookahead(
            Some(candidate),
            Some(Time::ZERO),
            requested,
            Some(lookahead),
            Some(lookahead),
        ));
        assert!(!candidate_precedes_retained_lookahead(
            Some(candidate),
            Some(candidate),
            requested,
            None,
            Some(lookahead),
        ));
        assert!(!candidate_precedes_retained_lookahead(
            Some(candidate),
            Some(candidate),
            requested,
            Some(lookahead),
            Some(Time::from_fraction(68, 1_000)),
        ));
    }
}
