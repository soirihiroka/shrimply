//! FFmpeg media inspection and remuxing.
use super::*;
use ffmpeg::{Rational, codec, encoder, format, media};
use ffmpeg_next as ffmpeg;

pub(super) fn inspect(
    source: Asset,
    snapshot: AssetSnapshot,
    file_kind: FileKind,
    default_visual_duration: Time,
) -> Result<MediaInfo, String> {
    let path = snapshot.path();
    let input = format::input(path)
        .map_err(|error| format!("could not open {}: {error}", path.display()))?;
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
                let (width, height) = unsafe {
                    let parameters = stream.parameters();
                    let parameters = &*parameters.as_ptr();
                    (parameters.width, parameters.height)
                };
                if width <= 0 || height <= 0 {
                    return Err(format!(
                        "{} has a video stream without positive dimensions",
                        path.display()
                    ));
                }
                video_sizes.push(UVec2::new(width as u32, height as u32));
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

    let source_duration = Time::from_fraction(
        input.duration().max(0),
        i64::from(ffmpeg::sys::AV_TIME_BASE),
    )
    .max(stream_duration);
    let visual_kind = match file_kind {
        FileKind::Image => Some(VisualMediaKind::Image),
        FileKind::Gif => Some(VisualMediaKind::Gif),
        FileKind::Mp4 | FileKind::Mov | FileKind::Mkv | FileKind::WebM if video_streams > 0 => {
            Some(VisualMediaKind::Video)
        }
        _ => None,
    };
    if matches!(file_kind, FileKind::Image | FileKind::Gif) {
        audio_streams = 0;
        video_streams = video_streams.min(1);
        video_sizes.truncate(video_streams);
    }
    let duration = if file_kind == FileKind::Image
        || file_kind == FileKind::Gif && source_duration <= Time::ZERO
    {
        default_visual_duration
    } else {
        source_duration
    };

    if file_kind == FileKind::Image {
        video_fps = None;
    }
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

pub fn remux_mkv_to_mp4(input: &Path) -> Result<PathBuf, String> {
    let file = remux_to_mp4(input, shrimply_resource_pipeline::CancelToken::default())?;
    let path = file.path().to_path_buf();
    file.keep();
    Ok(path)
}

pub fn remux_to_mp4(
    input: &Path,
    cancellation: shrimply_resource_pipeline::CancelToken,
) -> Result<OwnedFile, String> {
    let mut path = input.with_extension("mp4");
    loop {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let mut name = input
                    .file_stem()
                    .ok_or("source has no filename")?
                    .to_os_string();
                name.push(format!("-{}.mp4", uuid::Uuid::new_v4()));
                path = input.with_file_name(name);
            }
            Err(error) => return Err(format!("could not create {}: {error}", path.display())),
        }
    }
    let output = OwnedFile::new(path);
    remux(input, output.path(), cancellation)?;
    Ok(output)
}

fn remux(
    input: &Path,
    output: &Path,
    cancellation: shrimply_resource_pipeline::CancelToken,
) -> Result<(), String> {
    let interrupt = cancellation.clone();
    let mut input_context =
        format::input_with_interrupt(input, move || interrupt.is_cancelled())
            .map_err(|error| format!("could not open {}: {error}", input.display()))?;
    let mut output_context = format::output(output)
        .map_err(|error| format!("could not create {}: {error}", output.display()))?;
    let stream_count = input_context.nb_streams() as usize;
    let mut stream_mapping = vec![-1; stream_count];
    let mut input_time_bases = vec![Rational(0, 1); stream_count];
    let mut output_index = 0;

    for (input_index, input_stream) in input_context.streams().enumerate() {
        let medium = input_stream.parameters().medium();
        if !matches!(
            medium,
            media::Type::Audio | media::Type::Video | media::Type::Subtitle
        ) {
            continue;
        }

        stream_mapping[input_index] = output_index;
        input_time_bases[input_index] = input_stream.time_base();
        output_index += 1;

        let mut output_stream = output_context
            .add_stream(encoder::find(codec::Id::None))
            .map_err(|error| format!("could not add output stream: {error}"))?;
        output_stream.set_parameters(input_stream.parameters());
        unsafe {
            (*output_stream.parameters().as_mut_ptr()).codec_tag = 0;
        }
    }

    output_context.set_metadata(input_context.metadata().to_owned());
    output_context
        .write_header()
        .map_err(|error| format!("could not write {} header: {error}", output.display()))?;

    loop {
        if cancellation.is_cancelled() {
            return Err("Media import was cancelled".into());
        }
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
        let output_index = stream_mapping[input_index];
        if output_index < 0 {
            continue;
        }
        let output_stream = output_context
            .stream(output_index as usize)
            .ok_or_else(|| "remux output stream disappeared".to_string())?;
        packet.rescale_ts(input_time_bases[input_index], output_stream.time_base());
        packet.set_position(-1);
        packet.set_stream(output_index as usize);
        packet
            .write_interleaved(&mut output_context)
            .map_err(|error| format!("could not write {} packet: {error}", output.display()))?;
    }

    if cancellation.is_cancelled() {
        return Err("Media import was cancelled".into());
    }
    output_context
        .write_trailer()
        .map_err(|error| format!("could not write {} trailer: {error}", output.display()))
}
