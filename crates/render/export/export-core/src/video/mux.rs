//! Container headers and timestamp conversion, independent of the GPU backend.
use super::{check_cancelled, ffmpeg_check};
use crate::audio::AudioPackets;
use ffmpeg_next::{
    self as ffmpeg,
    packet::{Mut as _, Ref as _},
    sys,
};
use std::{
    ptr,
    sync::atomic::{AtomicBool, Ordering},
};

pub(crate) struct Muxer<'a> {
    output: ffmpeg::format::context::Output,
    video_index: usize,
    video_time_base: Option<ffmpeg::Rational>,
    audio: Option<AudioTrack<'a>>,
    decode_order: Option<crate::math::decode_order::DecodeOrder>,
}

pub(crate) struct AudioTrack<'a> {
    index: usize,
    time_base: Option<ffmpeg::Rational>,
    packets: AudioPackets<'a>,
    pending: Option<ffmpeg::Packet>,
    cancelled: &'a AtomicBool,
}

impl<'a> AudioTrack<'a> {
    pub fn new(index: usize, packets: AudioPackets<'a>, cancelled: &'a AtomicBool) -> Self {
        Self {
            index,
            time_base: None,
            packets,
            pending: None,
            cancelled,
        }
    }

    fn write_through(
        &mut self,
        output: &mut ffmpeg::format::context::Output,
        limit: Option<(i64, i32, i32)>,
    ) -> Result<(), String> {
        let source = self.packets.time_base();
        let destination = self
            .time_base
            .ok_or("Audio packet requires a complete media header")?;
        loop {
            check_cancelled(self.cancelled)?;
            let packet = match self.pending.take() {
                Some(packet) => Some(packet),
                None => self
                    .packets
                    .next(|_| !self.cancelled.load(Ordering::Relaxed))?,
            };
            let Some(packet) = packet else {
                return Ok(());
            };
            let timestamp = packet.dts().or_else(|| packet.pts()).unwrap_or(0);
            if limit.is_some_and(|limit| {
                crate::math::compare_timestamps((timestamp, source.0, source.1), limit).is_gt()
            }) {
                self.pending = Some(packet);
                return Ok(());
            }
            crate::write_packet(output, packet, self.index, source, destination)
                .map_err(|error| format!("Write audio packet: {error}"))?;
        }
    }
}

impl<'a> Muxer<'a> {
    pub fn new(
        output: ffmpeg::format::context::Output,
        video_index: usize,
        audio: Option<AudioTrack<'a>>,
        wait_for_header: bool,
        decode_order: Option<crate::math::decode_order::DecodeOrder>,
    ) -> Result<Self, String> {
        let mut muxer = Self {
            output,
            video_index,
            video_time_base: None,
            audio,
            decode_order,
        };
        if !wait_for_header {
            muxer.write_header()?;
        }
        Ok(muxer)
    }

    fn write_header(&mut self) -> Result<(), String> {
        self.output
            .write_header()
            .map_err(|error| format!("Write media header: {error}"))?;
        self.video_time_base = Some(
            self.output
                .stream(self.video_index)
                .ok_or("Video stream disappeared after writing the header")?
                .time_base(),
        );
        if let Some(audio) = &mut self.audio {
            audio.time_base = Some(
                self.output
                    .stream(audio.index)
                    .ok_or("Audio stream disappeared after writing the header")?
                    .time_base(),
            );
        }
        Ok(())
    }

    pub fn write_video(
        &mut self,
        mut packet: ffmpeg::Packet,
        encoder: &ffmpeg::codec::encoder::video::Encoder,
    ) -> Result<(), String> {
        if let Some(clock) = &mut self.decode_order {
            let pts = packet
                .pts()
                .ok_or("Encoder returned a video packet without a presentation timestamp")?;
            let dts = clock
                .next(pts, packet.dts())
                .map_err(|error| format!("{error} (PTS {pts})"))?;
            packet.set_dts(Some(dts));
        }
        if self.video_time_base.is_none() {
            let header = extract_header(encoder, &packet)?;
            let mut parameters = self
                .output
                .stream_mut(self.video_index)
                .ok_or("Video stream disappeared before writing the header")?
                .parameters();
            unsafe {
                let parameters = &mut *parameters.as_mut_ptr();
                let size =
                    i32::try_from(header.len()).map_err(|_| "Video codec header is too large")?;
                let data =
                    sys::av_mallocz(header.len() + sys::AV_INPUT_BUFFER_PADDING_SIZE as usize)
                        .cast::<u8>();
                if data.is_null() {
                    return Err("Allocate video codec header".into());
                }
                ptr::copy_nonoverlapping(header.as_ptr(), data, header.len());
                sys::av_free(parameters.extradata.cast());
                parameters.extradata = data;
                parameters.extradata_size = size;
            }
            self.write_header()?;
        }
        let time_base = self.video_time_base.expect("written media header");
        if packet.duration() == 0 {
            packet.set_duration(1);
        }
        let source = encoder.time_base();
        if let Some(audio) = &mut self.audio {
            let timestamp = packet.dts().or_else(|| packet.pts()).unwrap_or(0);
            audio.write_through(&mut self.output, Some((timestamp, source.0, source.1)))?;
        }
        crate::write_packet(
            &mut self.output,
            packet,
            self.video_index,
            source,
            time_base,
        )
        .map_err(|error| format!("Write video packet: {error}"))
    }

    pub fn audio_timings(&self) -> (u128, u128) {
        self.audio.as_ref().map_or((0, 0), |audio| {
            (audio.packets.mix_ns, audio.packets.encode_ns)
        })
    }

    pub fn finish(&mut self) -> Result<(), String> {
        if let Some(clock) = &self.decode_order {
            clock.finish().map_err(str::to_owned)?;
        }
        if self.video_time_base.is_none() {
            return Err("Video encoder produced no codec header".into());
        }
        if let Some(audio) = &mut self.audio {
            audio.write_through(&mut self.output, None)?;
        }
        self.output
            .write_trailer()
            .map_err(|error| format!("Could not write export trailer: {error}"))
    }
}

struct Filter(*mut sys::AVBSFContext);
impl Drop for Filter {
    fn drop(&mut self) {
        unsafe { sys::av_bsf_free(&mut self.0) };
    }
}

/// Surface encoders can deliver SPS/PPS/VPS with the first compressed packet,
/// after the encoder was opened. Let FFmpeg parse those bytes for all backends.
fn extract_header(
    encoder: &ffmpeg::codec::encoder::video::Encoder,
    packet: &ffmpeg::Packet,
) -> Result<Vec<u8>, String> {
    let mut filter = Filter(ptr::null_mut());
    unsafe {
        let kind = sys::av_bsf_get_by_name(c"extract_extradata".as_ptr());
        if kind.is_null() {
            return Err("FFmpeg codec-header extraction is unavailable".into());
        }
        ffmpeg_check(
            sys::av_bsf_alloc(kind, &mut filter.0),
            "Allocate codec-header extractor",
        )?;
        ffmpeg_check(
            sys::avcodec_parameters_from_context((*filter.0).par_in, encoder.as_ptr()),
            "Read encoded video parameters",
        )?;
        (*filter.0).time_base_in = encoder.time_base().into();
        ffmpeg_check(
            sys::av_bsf_init(filter.0),
            "Initialize codec-header extractor",
        )?;
        let mut input = packet.clone();
        ffmpeg_check(
            sys::av_bsf_send_packet(filter.0, input.as_mut_ptr()),
            "Read first encoded video packet",
        )?;
        let mut output = ffmpeg::Packet::empty();
        ffmpeg_check(
            sys::av_bsf_receive_packet(filter.0, output.as_mut_ptr()),
            "Extract video codec header",
        )?;
        let mut length = 0;
        let data = sys::av_packet_get_side_data(
            output.as_ptr(),
            sys::AVPacketSideDataType::AV_PKT_DATA_NEW_EXTRADATA,
            &mut length,
        );
        if data.is_null() || length == 0 {
            return Err("First encoded packet did not contain a video codec header".into());
        }
        Ok(std::slice::from_raw_parts(data, length).to_vec())
    }
}
