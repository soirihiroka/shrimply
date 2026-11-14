//! Container headers and timestamp conversion, independent of the GPU backend.
use super::ffmpeg_check;
use ffmpeg_next::{
    self as ffmpeg,
    packet::{Mut as _, Ref as _},
    sys,
};
use std::{collections::VecDeque, ptr};

pub(super) struct Muxer {
    output: ffmpeg::format::context::Output,
    video_index: usize,
    video_time_base: Option<ffmpeg::Rational>,
    audio_stream: Option<(usize, ffmpeg::Rational)>,
    audio_time_base: Option<ffmpeg::Rational>,
    /// Audio packets remain in the encoder time base until the header has fixed
    /// the container's stream time bases (Matroska uses millisecond ticks).
    pub audio_packets: VecDeque<ffmpeg::Packet>,
}

impl Muxer {
    pub fn new(
        output: ffmpeg::format::context::Output,
        video_index: usize,
        audio_stream: Option<(usize, ffmpeg::Rational)>,
        wait_for_header: bool,
    ) -> Result<Self, String> {
        let mut muxer = Self {
            output,
            video_index,
            audio_stream,
            video_time_base: None,
            audio_time_base: None,
            audio_packets: VecDeque::new(),
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
        self.audio_time_base = self
            .audio_stream
            .map(|(index, _)| {
                self.output
                    .stream(index)
                    .map(|stream| stream.time_base())
                    .ok_or_else(|| "Audio stream disappeared after writing the header".to_string())
            })
            .transpose()?;
        if let (Some((_, source)), Some(destination)) = (self.audio_stream, self.audio_time_base) {
            for packet in &mut self.audio_packets {
                packet.rescale_ts(source, destination);
            }
        }
        Ok(())
    }

    pub fn queue_audio(&mut self, mut packets: VecDeque<ffmpeg::Packet>) {
        if let (Some((_, source)), Some(destination)) = (self.audio_stream, self.audio_time_base) {
            for packet in &mut packets {
                packet.rescale_ts(source, destination);
            }
        }
        self.audio_packets = packets;
    }

    pub fn write_video(
        &mut self,
        mut packet: ffmpeg::Packet,
        encoder: &ffmpeg::codec::encoder::video::Encoder,
    ) -> Result<(), String> {
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
        packet.set_stream(self.video_index);
        if packet.duration() == 0 {
            packet.set_duration(1);
        }
        packet.rescale_ts(encoder.time_base(), time_base);
        if let Some(audio_time_base) = self.audio_time_base {
            while self.audio_packets.front().is_some_and(|audio| {
                crate::math::compare_timestamps(
                    (
                        audio.dts().or_else(|| audio.pts()).unwrap_or(0),
                        audio_time_base.0,
                        audio_time_base.1,
                    ),
                    (
                        packet.dts().or_else(|| packet.pts()).unwrap_or(0),
                        time_base.0,
                        time_base.1,
                    ),
                )
                .is_le()
            }) {
                let audio = self.audio_packets.pop_front().expect("front audio packet");
                self.write_audio(audio)?;
            }
        }
        packet
            .write_interleaved(&mut self.output)
            .map_err(|error| error.to_string())
    }

    pub fn write_audio(&mut self, mut packet: ffmpeg::Packet) -> Result<(), String> {
        let (index, _) = self
            .audio_stream
            .ok_or("Audio packet requires an audio stream")?;
        if self.audio_time_base.is_none() {
            return Err("Audio packet requires a complete media header".into());
        }
        packet.set_stream(index);
        packet
            .write_interleaved(&mut self.output)
            .map_err(|error| format!("Write audio packet: {error}"))
    }

    pub fn finish(&mut self) -> Result<(), String> {
        if self.video_time_base.is_none() {
            return Err("Video encoder produced no codec header".into());
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
