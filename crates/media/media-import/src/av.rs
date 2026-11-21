//! FFmpeg media inspection and remuxing.
mod vp9;

use super::*;
use ffmpeg::{codec, encoder, format, media};
use ffmpeg_next as ffmpeg;
use shrimply_resource_pipeline::CancelToken;

pub(super) fn inspect(
    source: Asset,
    snapshot: AssetSnapshot,
    file_kind: FileKind,
) -> Result<MediaInfo, String> {
    let path = snapshot.path();
    let input = shrimply_media_info::demux::open(path)
        .map_err(|error| format!("could not open {}: {error}", path.display()))?;
    // Check the demuxer too: a renamed Matroska file is still a remux input.
    if input
        .format()
        .name()
        .split(',')
        .any(|name| name == "matroska")
    {
        return Err(format!(
            "{} uses a Matroska/WebM container and must be remuxed to MP4",
            path.display()
        ));
    }
    let mut video_streams = 0;
    let mut video_sizes = Vec::new();
    let mut video_fps = None;
    let mut audio_streams = 0;
    let mut stream_duration = Time::ZERO;

    for stream in input.streams() {
        // Cover art is reported as video, but has no playback timestamps.
        if stream
            .disposition()
            .contains(format::stream::Disposition::ATTACHED_PIC)
        {
            continue;
        }
        match stream.parameters().medium() {
            media::Type::Video if file_kind != FileKind::Audio => {
                video_streams += 1;
                if video_fps.is_none() {
                    let mut rate = stream.avg_frame_rate();
                    if rate.numerator() <= 0 || rate.denominator() <= 0 {
                        rate = stream.rate();
                    }
                    if rate.numerator() > 0 && rate.denominator() > 0 {
                        video_fps = Some(Fraction::new_raw(
                            u64::try_from(rate.numerator()).expect("positive video frame rate"),
                            u64::try_from(rate.denominator()).expect("positive video frame rate"),
                        ));
                    }
                }
                video_sizes.push(track_dimensions(&snapshot, video_streams - 1, &stream)?);
            }
            media::Type::Audio => audio_streams += 1,
            _ => {}
        }
        let base = stream.time_base();
        if stream.duration() > 0 && base.numerator() > 0 && base.denominator() > 0 {
            let ticks_per_second =
                Fraction::new_raw(base.denominator() as u64, base.numerator() as u64);
            let duration =
                shrimply_math_core::time_from_signed_frame(stream.duration(), ticks_per_second)
                    .ok_or_else(|| {
                        format!(
                            "{} has a stream duration outside the exact timeline range",
                            path.display()
                        )
                    })?;
            stream_duration = stream_duration.max(duration);
        }
    }

    if video_streams == 0 && audio_streams == 0 {
        return Err(format!("{} has no audio or video streams", path.display()));
    }

    let duration = Time::from_fraction(
        input.duration().max(0),
        i64::from(ffmpeg::sys::AV_TIME_BASE),
    )
    .max(stream_duration);
    let visual_kind = (video_streams > 0).then_some(VisualMediaKind::Video);
    snapshot.verify_current()?;
    Ok(MediaInfo {
        source,
        snapshot,
        duration,
        visual_kind,
        video_streams,
        video_sizes,
        video_fps,
        audio_streams,
        caption_cues: Vec::new(),
        layered_image_layers: Vec::new(),
    })
}

pub fn video_dimensions(path: &Path, track_id: u32) -> Option<UVec2> {
    ffmpeg::init().ok()?;
    let source = Asset::new(path).snapshot().ok()?;
    let input = shrimply_media_info::demux::open(path).ok()?;
    let stream = input
        .streams()
        .filter(|stream| {
            stream.parameters().medium() == media::Type::Video
                && !stream
                    .disposition()
                    .contains(format::stream::Disposition::ATTACHED_PIC)
        })
        .nth(track_id as usize)?;
    track_dimensions(&source, track_id as usize, &stream).ok()
}

fn track_dimensions(
    source: &AssetSnapshot,
    index: usize,
    stream: &format::stream::Stream<'_>,
) -> Result<UVec2, String> {
    let parameters = stream.parameters();
    let raw = unsafe { &*parameters.as_ptr() };
    if raw.width > 0 && raw.height > 0 {
        return Ok(UVec2::new(raw.width as u32, raw.height as u32));
    }
    use shrimply_video_decoder::{DecoderBackend, NextFrame, VideoDecoderSession};
    let mut decoder = VideoDecoderSession::open(
        source,
        index.try_into().map_err(|_| "Video track index overflow")?,
        DecoderBackend::hardware_for_readback,
        None,
    )?;
    match decoder.next_frame([None, None])? {
        NextFrame::Frame((_, frame)) => {
            Ok(UVec2::new(frame.native().width(), frame.native().height()))
        }
        _ => Err(format!(
            "Could not determine video dimensions for {}",
            source.path().display()
        )),
    }
}

pub(super) fn check_cancellation(cancellation: &CancelToken) -> Result<(), String> {
    if cancellation.is_cancelled() {
        return Err("Media import was cancelled".into());
    }
    Ok(())
}

pub fn remux_to_mp4(input: &Path, cancellation: CancelToken) -> Result<OwnedFile, String> {
    remux_to(input, input.with_extension("mp4"), cancellation)
}

pub(super) fn remux_to(
    input: &Path,
    destination: PathBuf,
    cancellation: CancelToken,
) -> Result<OwnedFile, String> {
    let (path, _) = super::staging::create_unique(destination)?;
    let output = OwnedFile::new(path);
    remux(input, output.path(), cancellation)?;
    Ok(output)
}

fn remux(input: &Path, output: &Path, cancellation: CancelToken) -> Result<(), String> {
    let interrupt = cancellation.clone();
    let mut input_context =
        shrimply_media_info::demux::open_with_interrupt(input, move || interrupt.is_cancelled())
            .map_err(|error| format!("could not open {}: {error}", input.display()))?;
    let mut output_context = format::output(output)
        .map_err(|error| format!("could not create {}: {error}", output.display()))?;
    let mut input_time_bases = Vec::with_capacity(input_context.nb_streams() as usize);

    for (input_index, input_stream) in input_context.streams().enumerate() {
        let medium = input_stream.parameters().medium();
        if !matches!(
            medium,
            media::Type::Audio | media::Type::Video | media::Type::Subtitle
        ) {
            return Err(format!(
                "MP4 cannot preserve the {medium:?} stream in {}",
                input.display()
            ));
        }
        if input_stream.metadata().get("alpha_mode") == Some("1") {
            return Err(format!(
                "MP4 stream copying would discard the alpha channel in {}. Relink a compatible source to import this clip.",
                input.display()
            ));
        }
        input_time_bases.push(input_stream.time_base());

        let mut output_stream = output_context
            .add_stream(encoder::find(codec::Id::None))
            .map_err(|error| format!("could not add output stream: {error}"))?;
        let mut parameters = input_stream.parameters();
        if parameters.id() == codec::Id::VP9 && unsafe { (*parameters.as_ptr()).format } < 0 {
            // FFmpeg's packet-only VP9 parser leaves the MP4 configuration incomplete.
            parameters = parameters.clone();
            vp9::complete(input, input_index, &mut parameters, &cancellation)?;
        }
        output_stream.set_parameters(parameters);
        output_stream.set_metadata(input_stream.metadata().to_owned());
        output_stream.set_avg_frame_rate(input_stream.avg_frame_rate());
        unsafe {
            (*output_stream.parameters().as_mut_ptr()).codec_tag = 0;
            (*output_stream.as_mut_ptr()).disposition = (*input_stream.as_ptr()).disposition;
        }
    }

    for chapter in input_context.chapters() {
        let metadata = chapter.metadata();
        output_context
            .add_chapter(
                chapter.id(),
                chapter.time_base(),
                chapter.start(),
                chapter.end(),
                metadata.get("title").unwrap_or(""),
            )
            .map_err(|e| e.to_string())?;
    }
    output_context.set_metadata(input_context.metadata().to_owned());
    output_context
        .write_header()
        .map_err(|error| format!("could not write {} header: {error}", output.display()))?;

    loop {
        check_cancellation(&cancellation)?;
        let mut packet = ffmpeg::Packet::empty();
        match packet.read(&mut input_context) {
            Ok(()) => {}
            Err(ffmpeg::Error::Eof) => break,
            Err(error) => {
                return Err(format!(
                    "could not read {} packet: {error}",
                    input.display()
                ));
            }
        }
        let input_index = packet.stream();
        if packet
            .side_data()
            .any(|data| data.kind() == ffmpeg::packet::side_data::Type::MatroskaBlockAdditional)
        {
            return Err(format!(
                "MP4 cannot preserve the supplemental packet data in {}, which can include an alpha channel.",
                input.display()
            ));
        }
        let output_stream = output_context
            .stream(input_index)
            .ok_or_else(|| "remux output stream disappeared".to_string())?;
        packet.rescale_ts(input_time_bases[input_index], output_stream.time_base());
        packet.set_position(-1);
        packet
            .write_interleaved(&mut output_context)
            .map_err(|error| format!("could not write {} packet: {error}", output.display()))?;
    }

    check_cancellation(&cancellation)?;
    output_context
        .write_trailer()
        .map_err(|error| format!("could not write {} trailer: {error}", output.display()))
}
