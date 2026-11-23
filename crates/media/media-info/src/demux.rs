//! Open media without letting FFmpeg's stream probe decode video frames on the CPU.
use ffmpeg_next::{self as ffmpeg, format::context::Input, media::Type, sys};
use std::{path::Path, ptr, sync::OnceLock};

pub fn open(path: impl AsRef<Path>) -> Result<Input, ffmpeg::Error> {
    open_with_interrupt(path.as_ref(), || false)
}

pub fn open_with_interrupt(
    path: &Path,
    cancelled: impl FnMut() -> bool + Send + 'static,
) -> Result<Input, ffmpeg::Error> {
    open_input(path, cancelled, true)
}

/// Open a file that will be copied into another container.
///
/// Playback opens keep video codecs off the probe list so FFmpeg does not decode
/// pictures on the CPU. A stream copy still needs the sequence header: Matroska
/// stores presentation timestamps, and an H.265 stream with B-frames only gets
/// monotonic decode timestamps once the probe can see that header. The probe
/// skips picture reconstruction. The packets themselves are copied.
pub fn open_for_remux(
    path: &Path,
    cancelled: impl FnMut() -> bool + Send + 'static,
) -> Result<Input, ffmpeg::Error> {
    open_input(path, cancelled, false)
}

fn open_input(
    path: &Path,
    cancelled: impl FnMut() -> bool + Send + 'static,
    restrict_video_probe: bool,
) -> Result<Input, ffmpeg::Error> {
    // ffmpeg-next converts filenames to C strings internally. Reject unrepresentable
    // names before its unchecked conversion can panic on a user-selected file.
    let path = path
        .to_str()
        .filter(|name| !name.as_bytes().contains(&0))
        .ok_or(ffmpeg::Error::InvalidData)?;
    static PROBE_CODECS: OnceLock<String> = OnceLock::new();
    let codecs = PROBE_CODECS.get_or_init(|| {
        let mut names = Vec::new();
        let mut iterator = ptr::null_mut();
        loop {
            let entry = unsafe { sys::av_codec_iterate(&mut iterator) };
            if entry.is_null() {
                break;
            }
            let codec = unsafe { ffmpeg::Codec::wrap(entry) };
            if codec.is_decoder() && codec.medium() != Type::Video {
                names.push(codec.name().to_owned());
            }
        }
        names.join(",")
    });
    let mut options = ffmpeg::Dictionary::new();
    if restrict_video_probe {
        options.set("codec_whitelist", codecs);
    }
    let input = ffmpeg::format::input_with_interrupt_and_dictionary(path, cancelled, options)?;
    for stream in input.streams() {
        // Parsers can supply the source dimensions and pixel format without reconstructing
        // a frame. FFmpeg normally leaves the latter unset when its probe decoder is disabled.
        unsafe {
            let stream = stream.as_ptr();
            let parameters = &mut *(*stream).codecpar;
            if parameters.codec_type == sys::AVMediaType::AVMEDIA_TYPE_VIDEO
                && let Some(parser) = sys::av_stream_get_parser(stream).as_ref()
            {
                if parameters.format < 0 {
                    parameters.format = parser.format;
                }
                if parameters.width <= 0 || parameters.height <= 0 {
                    parameters.width = parser.width;
                    parameters.height = parser.height;
                }
            }
        }
    }
    Ok(input)
}

/// Stream order in the container, including cover pictures which some interchange formats
/// count as video streams and the editorial playback backend excludes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    Video,
    Audio,
    Artwork,
    Other,
}

pub fn stream_kinds(path: &Path) -> Result<Vec<StreamKind>, ffmpeg::Error> {
    ffmpeg::init()?;
    let input = open(path)?;
    Ok(input
        .streams()
        .map(|s| match s.parameters().medium() {
            Type::Audio => StreamKind::Audio,
            Type::Video
                if s.disposition()
                    .contains(ffmpeg::format::stream::Disposition::ATTACHED_PIC) =>
            {
                StreamKind::Artwork
            }
            Type::Video => StreamKind::Video,
            _ => StreamKind::Other,
        })
        .collect())
}
