//! VP9 uncompressed keyframe metadata required by the MP4 vpcC box.
use ffmpeg_next::{self as ffmpeg, codec::Parameters, format::Pixel, sys};
use shrimply_resource_pipeline::CancelToken;
use std::path::Path;

pub(super) fn complete(
    input: &Path,
    stream_index: usize,
    parameters: &mut Parameters,
    cancel: &CancelToken,
) -> Result<(), String> {
    let interrupt = cancel.clone();
    let mut probe =
        shrimply_media_info::demux::open_with_interrupt(input, move || interrupt.is_cancelled())
            .map_err(|error| error.to_string())?;
    loop {
        super::check_cancellation(cancel)?;
        let mut packet = ffmpeg::Packet::empty();
        packet.read(&mut probe).map_err(|error| {
            format!("Could not read VP9 stream {stream_index} keyframe: {error}")
        })?;
        if packet.stream() == stream_index && packet.is_key() {
            return read_color_header(packet.data().unwrap_or_default(), parameters);
        }
    }
}

fn read_color_header(packet: &[u8], parameters: &mut Parameters) -> Result<(), String> {
    // VP9 sections 6.2.1–6.2.2. All fields through color_config fit within the
    // first six bytes. Frame reconstruction and GPU output formats are irrelevant to stream copy.
    let mut header = u64::from_be_bytes(*packet.first_chunk().ok_or("Truncated VP9 keyframe")?);
    let mut bits = |width| {
        let value = (header >> (64u32 - width)) as u32;
        header <<= width;
        value
    };
    if bits(2) != 2 {
        return Err("Invalid VP9 frame marker".into());
    }
    let profile = bits(1) | (bits(1) << 1);
    if (profile == 3 && bits(1) != 0) || bits(2) != 0 {
        return Err("Invalid VP9 keyframe header".into());
    }
    bits(2); // show_frame and error_resilient_mode
    if bits(24) != 0x498342 {
        return Err("Invalid VP9 keyframe sync code".into());
    }
    let depth = if profile < 2 { 0 } else { 1 + bits(1) as usize };
    use sys::AVColorSpace::*;
    let space = [
        AVCOL_SPC_UNSPECIFIED,
        AVCOL_SPC_BT470BG,
        AVCOL_SPC_BT709,
        AVCOL_SPC_SMPTE170M,
        AVCOL_SPC_SMPTE240M,
        AVCOL_SPC_BT2020_NCL,
        AVCOL_SPC_RESERVED,
        AVCOL_SPC_RGB,
    ][bits(3) as usize];
    let rgb = space == AVCOL_SPC_RGB;
    let has_chroma_flags = profile & 1 != 0;
    let full_range = rgb || bits(1) != 0;
    let pixel = if rgb && has_chroma_flags {
        [Pixel::GBRP, Pixel::GBRP10, Pixel::GBRP12][depth]
    } else if rgb || space == AVCOL_SPC_RESERVED {
        return Err("Invalid VP9 color space for profile".into());
    } else if !has_chroma_flags {
        [Pixel::YUV420P, Pixel::YUV420P10, Pixel::YUV420P12][depth]
    } else {
        match bits(2) {
            // horizontal and vertical chroma subsampling flags
            0 => [Pixel::YUV444P, Pixel::YUV444P10, Pixel::YUV444P12][depth],
            2 => [Pixel::YUV422P, Pixel::YUV422P10, Pixel::YUV422P12][depth],
            _ => return Err("VP9 chroma subsampling cannot be represented in MP4".into()),
        }
    };
    if has_chroma_flags && bits(1) != 0 {
        return Err("Invalid VP9 color configuration".into());
    }
    let params = unsafe { &mut *parameters.as_mut_ptr() };
    params.format = sys::AVPixelFormat::from(pixel) as i32;
    params.profile = profile as i32;
    if params.color_space == AVCOL_SPC_UNSPECIFIED {
        params.color_space = space;
    }
    if params.color_range == sys::AVColorRange::AVCOL_RANGE_UNSPECIFIED {
        params.color_range = if full_range {
            sys::AVColorRange::AVCOL_RANGE_JPEG
        } else {
            sys::AVColorRange::AVCOL_RANGE_MPEG
        };
    }
    Ok(())
}
