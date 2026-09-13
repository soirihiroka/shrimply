use std::ffi::{CStr, CString};
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use ffmpeg::sys;
use ffmpeg_next as ffmpeg;
use shrimply_gpu_cuda::sys as cuda_sys;
use shrimply_math_core::{Fraction, fraction_ratio_i128};
use shrimply_project_document::project::Time;
use shrimply_visual_frame::{GPU_FRAME_ALLOCATION_EXHAUSTED, VisualFrame, ffmpeg_cuda_context};

use crate::startup::DecoderStartupMeasurement;
use crate::track::VideoSource;
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

pub type DecodedVisual = (Time, VisualFrame);
pub(crate) type DecodeControls<'a> = [Option<&'a DecodeControl>; 2];

pub enum DecodeOutcome {
    Frame(Option<DecodedVisual>),
    Superseded(Option<DecodedVisual>),
}

struct DecodeRequestState {
    target: Time,
    best_before: Option<DecodedVisual>,
}

pub(crate) struct VideoDecoderSession {
    source: VideoSource,
    input: ffmpeg::format::context::Input,
    stream_index: usize,
    stream_time_base: ffmpeg::Rational,
    stream_start_time: i64,
    pub(crate) frame_duration: Time,
    decoder: ffmpeg::decoder::Video,
    decoder_name: String,
    frame_index: usize,
    pub(crate) last_decoded_position: Option<Time>,
    lookahead: Option<DecodedVisual>,
    pending_packet: Option<ffmpeg::Packet>,
    input_eof: bool,
    eof_sent: bool,
    decoder_eof: bool,
    pub(crate) initialized: bool,
    pub(crate) startup: Option<DecoderStartupMeasurement>,
    pub(crate) startup_bytes: u64,
    opened_at: Option<Instant>,
    seek_started_at: Option<Instant>,
    seek_target: Option<Time>,
    seek_first_frame_received: bool,
    active_request: Option<DecodeRequestState>,
    nonadvancing_frames: usize,
}

impl VideoDecoderSession {
    pub(crate) fn open(source: &VideoSource) -> Result<Self, String> {
        source.asset.verify_current()?;
        ffmpeg::init().map_err(|error| error.to_string())?;
        let input =
            ffmpeg::format::input(source.asset.path()).map_err(|error| error.to_string())?;
        let (stream_index, stream_time_base, stream_start_time, frame_rate, parameters) = {
            let stream = input
                .streams()
                .filter(|stream| stream.parameters().medium() == ffmpeg::media::Type::Video)
                .filter(|stream| {
                    !stream
                        .disposition()
                        .contains(ffmpeg::format::stream::Disposition::ATTACHED_PIC)
                })
                .nth(source.media_track_id as usize)
                .ok_or_else(|| format!("video stream {} not found", source.media_track_id))?;
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
        let opened = open_nvidia_decoder(&parameters)?;
        Ok(Self {
            source: source.clone(),
            input,
            stream_index,
            stream_time_base,
            stream_start_time,
            frame_duration,
            decoder: opened.decoder,
            decoder_name: opened.decoder_name,
            frame_index: 0,
            last_decoded_position: None,
            lookahead: None,
            pending_packet: None,
            input_eof: false,
            eof_sent: false,
            decoder_eof: false,
            initialized: false,
            startup: None,
            startup_bytes: 0,
            opened_at: Some(decoder_open_started),
            seek_started_at: None,
            seek_target: None,
            seek_first_frame_received: false,
            active_request: None,
            nonadvancing_frames: 0,
        })
    }

    pub(crate) fn frame(
        &mut self,
        position: Time,
        cached: Option<DecodedVisual>,
        controls: DecodeControls<'_>,
        time_accurate: bool,
        continuous: bool,
    ) -> Result<DecodeOutcome, String> {
        if time_accurate {
            self.frame_at_exact(position, cached, controls, continuous)
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
        if !continuing_active_seek
            && !self.can_decode_forward(position, self.frame_duration, maximum_forward_gap)
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
            return Err(format!("seek video demuxer: {}", ffmpeg_error(result)));
        }
        self.decoder.flush();
        shrimply_profiling::increment("Video decode / Decoder flush");
        self.frame_index = 0;
        self.last_decoded_position = None;
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

    fn next_frame(&mut self, controls: DecodeControls<'_>) -> Result<NextFrame, String> {
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
                        return Ok(NextFrame::Superseded);
                    }
                    return Ok(NextFrame::Frame(frame));
                }
                ReceiveState::EndOfStream => return Ok(NextFrame::EndOfStream),
                ReceiveState::NeedInput if send_was_blocked => {
                    shrimply_profiling::increment("Video decode / Fatal send errors");
                    return Err(
                        "NVDEC made no progress: send and receive both returned EAGAIN".to_string(),
                    );
                }
                ReceiveState::NeedInput => {}
            }

            if self.pending_packet.is_none() && !self.input_eof {
                if self.cancel_if_superseded(controls) {
                    return Ok(NextFrame::Superseded);
                }
                self.pending_packet = self.read_video_packet();
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
                        let memory = cuda_memory_info()
                            .map(|(free, total)| {
                                format!(
                                    "; CUDA memory: {} MiB free of {} MiB",
                                    free / (1024 * 1024),
                                    total / (1024 * 1024),
                                )
                            })
                            .unwrap_or_default();
                        return Err(format!("NVDEC send packet: {error}{memory}"));
                    }
                }
                continue;
            }

            if self.eof_sent {
                shrimply_profiling::increment("Video decode / Fatal receive errors");
                return Err("NVDEC requested input after end-of-stream was sent".to_string());
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
                    return Err(format!("NVDEC send EOF: {error}"));
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
                self.last_decoded_position = Some(position);
                self.frame_index += 1;
                let frame = {
                    let _measurement =
                        shrimply_profiling::measure("Video decode / Retain CUDA frame");
                    VisualFrame::try_from(&decoded).map(|frame| (position, frame))
                };
                match frame {
                    Ok(frame) => {
                        self.record_first_frame(&decoded);
                        Ok(ReceiveState::Frame(frame))
                    }
                    Err(error) => {
                        if error.starts_with(GPU_FRAME_ALLOCATION_EXHAUSTED) {
                            self.last_decoded_position = None;
                            shrimply_profiling::increment(
                                "Video decode / Recoverable GPU allocation errors",
                            );
                        } else {
                            shrimply_profiling::increment("Video decode / Fatal receive errors");
                        }
                        Err(error)
                    }
                }
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
                Err(format!("NVDEC receive frame: {error}"))
            }
        }
    }

    fn read_video_packet(&mut self) -> Option<ffmpeg::Packet> {
        loop {
            let next = {
                let mut packets = self.input.packets();
                packets
                    .next()
                    .map(|(stream, packet)| (stream.index(), packet))
            };
            let (stream_index, packet) = next?;
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
                return Some(packet);
            }
        }
    }

    fn missing_frame_error(&self, position: Time, stage: &str) -> String {
        format!(
            "{stage} without a usable frame: file={}, media_track_id={}, requested={}, last_decoded={}, seek_target={}, decoder_eof={}",
            self.source.asset.path().display(),
            self.source.media_track_id,
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
        self.startup
            .take()
            .expect("first CUDA frame arrived without a startup reservation")
            .finish(None, &mut self.startup_bytes);
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
            let cuda_context_identity = ffmpeg_cuda_context(frame).unwrap_or(ptr::null_mut());
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
                cuda_device = 0,
                cuda_context = ?cuda_context_identity,
                "received first NVIDIA CUDA video frame",
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

enum NextFrame {
    Frame(DecodedVisual),
    EndOfStream,
    Superseded,
}

enum ReceiveState {
    Frame(DecodedVisual),
    NeedInput,
    EndOfStream,
}

struct OpenedNvidiaDecoder {
    decoder: ffmpeg::decoder::Video,
    decoder_name: String,
}

fn open_nvidia_decoder(
    parameters: &ffmpeg::codec::Parameters,
) -> Result<OpenedNvidiaDecoder, String> {
    let codec_id = parameters.id();
    let mut opaque = ptr::null_mut();
    let codec = loop {
        let codec = unsafe { sys::av_codec_iterate(&mut opaque) };
        if codec.is_null() {
            return Err(format!(
                "no NVIDIA CUDA video decoder available for {codec_id:?}"
            ));
        }
        if unsafe { sys::av_codec_is_decoder(codec) } != 0
            && unsafe { (*codec).id } == codec_id.into()
            && exposes_cuda_frames(codec)
        {
            break unsafe { ffmpeg::Codec::wrap(codec) };
        }
    };
    let decoder_name = codec.name().to_owned();

    let hw_device_ctx = create_cuda_device_context()?;
    let mut context = ffmpeg::codec::context::Context::from_parameters(parameters.clone())
        .map_err(|error| error.to_string())?;
    let flags_before_open = unsafe {
        let raw_context = context.as_mut_ptr();
        (*raw_context).hw_device_ctx = hw_device_ctx;
        (*raw_context).get_format = Some(cuda_get_format);
        (*raw_context).flags
    };
    tracing::debug!(
        flags = flags_before_open,
        "configured NVIDIA video decoder before open"
    );

    let decoder = context
        .decoder()
        .open_as(codec)
        .and_then(|opened| opened.video())
        .map_err(|error| format!("could not open {decoder_name} with CUDA: {error}"))?;
    let live_flags = unsafe { (*decoder.as_ptr()).flags };
    tracing::info!(
        flags = live_flags,
        low_delay = live_flags & sys::AV_CODEC_FLAG_LOW_DELAY as i32 != 0,
        "Using NVIDIA CUDA video decoder {decoder_name}"
    );
    Ok(OpenedNvidiaDecoder {
        decoder,
        decoder_name,
    })
}

fn exposes_cuda_frames(codec: *const sys::AVCodec) -> bool {
    let mut index = 0;
    loop {
        let config = unsafe { sys::avcodec_get_hw_config(codec, index) };
        if config.is_null() {
            return false;
        }
        let config = unsafe { &*config };
        if config.device_type == sys::AVHWDeviceType::AV_HWDEVICE_TYPE_CUDA
            && config.pix_fmt == sys::AVPixelFormat::AV_PIX_FMT_CUDA
            && config.methods & sys::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX as i32 != 0
        {
            return true;
        }
        index += 1;
    }
}

unsafe extern "C" fn cuda_get_format(
    _context: *mut sys::AVCodecContext,
    formats: *const sys::AVPixelFormat,
) -> sys::AVPixelFormat {
    let mut cursor = formats;
    while !cursor.is_null() {
        let format = unsafe { *cursor };
        if format == sys::AVPixelFormat::AV_PIX_FMT_NONE {
            break;
        }
        if format == sys::AVPixelFormat::AV_PIX_FMT_CUDA {
            return format;
        }
        cursor = unsafe { cursor.add(1) };
    }
    sys::AVPixelFormat::AV_PIX_FMT_NONE
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

fn frame_time(
    frame: &ffmpeg::frame::Video,
    time_base: ffmpeg::Rational,
    stream_start_time: i64,
    frame_index: usize,
    last_position: Option<Time>,
    frame_duration: Time,
    timestamp_required: bool,
) -> Result<Time, String> {
    let Some(timestamp) = frame.timestamp() else {
        if let Some(last_position) = last_position {
            return Ok(last_position.saturating_add(frame_duration));
        }
        if timestamp_required {
            return Err(
                "NVDEC returned a frame without a timestamp after random access".to_string(),
            );
        }
        return Ok(Time {
            seconds: frame_duration.seconds
                * Fraction::from(
                    u64::try_from(frame_index).expect("decoded frame index exceeds u64"),
                ),
        });
    };
    let start_time = if stream_start_time == sys::AV_NOPTS_VALUE {
        0
    } else {
        stream_start_time
    };
    Ok(Time::from_fraction(
        timestamp
            .saturating_sub(start_time)
            .saturating_mul(i64::from(time_base.numerator())),
        i64::from(time_base.denominator()),
    ))
}

fn source_time_to_stream_timestamp(
    position: Time,
    time_base: ffmpeg::Rational,
    stream_start_time: i64,
) -> i64 {
    let (position_numerator, position_denominator) =
        fraction_ratio_i128(position.seconds).expect("decoder position must be finite");
    let numerator = position_numerator.saturating_mul(i128::from(time_base.denominator()));
    let denominator = position_denominator.saturating_mul(i128::from(time_base.numerator()));
    let timestamp = if denominator == 0 {
        0
    } else {
        numerator / denominator
    };
    let timestamp = if stream_start_time == sys::AV_NOPTS_VALUE {
        timestamp
    } else {
        timestamp.saturating_add(i128::from(stream_start_time))
    };
    timestamp.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

fn ffmpeg_error(result: i32) -> String {
    unsafe {
        let mut buffer = [0i8; 256];
        if sys::av_strerror(result, buffer.as_mut_ptr(), buffer.len()) == 0 {
            CStr::from_ptr(buffer.as_ptr())
                .to_string_lossy()
                .into_owned()
        } else {
            format!("FFmpeg error {result}")
        }
    }
}

pub(crate) fn create_cuda_device_context() -> Result<*mut sys::AVBufferRef, String> {
    unsafe {
        let mut hw_device_ctx = ptr::null_mut();
        let device = CString::new("0").map_err(|error| error.to_string())?;
        let mut options = ptr::null_mut();
        let option_key = CString::new("primary_ctx").map_err(|error| error.to_string())?;
        let option_value = CString::new("1").map_err(|error| error.to_string())?;
        let option_result =
            sys::av_dict_set(&mut options, option_key.as_ptr(), option_value.as_ptr(), 0);
        if option_result < 0 {
            return Err(format!(
                "could not configure FFmpeg CUDA primary context: {}",
                ffmpeg_error(option_result),
            ));
        }
        let result = sys::av_hwdevice_ctx_create(
            &mut hw_device_ctx,
            sys::AVHWDeviceType::AV_HWDEVICE_TYPE_CUDA,
            device.as_ptr(),
            options,
            0,
        );
        sys::av_dict_free(&mut options);
        if result < 0 {
            return Err(format!(
                "could not create FFmpeg CUDA device context: {}",
                ffmpeg_error(result),
            ));
        }
        Ok(hw_device_ctx)
    }
}

pub(crate) fn cuda_memory_info() -> Result<(usize, usize), String> {
    unsafe {
        if cuda_sys::cuInit(0) != cuda_sys::cudaError_enum_CUDA_SUCCESS {
            return Err("could not initialize CUDA while checking decoder memory".to_string());
        }
        let mut device = 0;
        if cuda_sys::cuDeviceGet(&mut device, 0) != cuda_sys::cudaError_enum_CUDA_SUCCESS {
            return Err("could not select CUDA device 0 while checking decoder memory".to_string());
        }
        let mut context = ptr::null_mut();
        if cuda_sys::cuDevicePrimaryCtxRetain(&mut context, device)
            != cuda_sys::cudaError_enum_CUDA_SUCCESS
        {
            return Err("could not retain CUDA primary context while checking memory".to_string());
        }
        if cuda_sys::cuCtxPushCurrent_v2(context) != cuda_sys::cudaError_enum_CUDA_SUCCESS {
            let _ = cuda_sys::cuDevicePrimaryCtxRelease_v2(device);
            return Err(
                "could not activate CUDA primary context while checking memory".to_string(),
            );
        }
        let mut free = 0;
        let mut total = 0;
        let result = cuda_sys::cuMemGetInfo_v2(&mut free, &mut total);
        let mut popped = ptr::null_mut();
        let pop = cuda_sys::cuCtxPopCurrent_v2(&mut popped);
        let release = cuda_sys::cuDevicePrimaryCtxRelease_v2(device);
        if result != cuda_sys::cudaError_enum_CUDA_SUCCESS
            || pop != cuda_sys::cudaError_enum_CUDA_SUCCESS
            || release != cuda_sys::cudaError_enum_CUDA_SUCCESS
        {
            return Err("could not query or restore CUDA memory context".to_string());
        }
        Ok((free, total))
    }
}
