#[cfg(not(target_os = "macos"))]
fn main() -> Result<(), String> {
    use ffmpeg_next as ffmpeg;
    use shrimply_export_cuda::*;
    use shrimply_math_core::{Time, fraction_new};
    use shrimply_project_document::project::{
        Asset, CanvasSize, Project, VideoItem, VideoItemContent, VideoTrack,
    };
    use std::{
        path::PathBuf,
        sync::{Arc, atomic::AtomicBool},
        time::Instant,
    };

    const OUTPUT_FPS: i64 = 30;
    const DEFAULT_FRAMES: i64 = 120;
    #[cfg(target_os = "linux")]
    shrimply_visual_cuda::gpu::configure_gtk_backend()?;
    let mut arguments = std::env::args().skip(1);
    let source = PathBuf::from(
        arguments
            .next()
            .ok_or("Pass input video, output path, codec, and optional frame count")?,
    )
    .canonicalize()
    .map_err(|e| e.to_string())?;
    let path = PathBuf::from(arguments.next().ok_or("Pass an output path")?);
    let (video_codec, profile) = match arguments.next().as_deref() {
        Some("h264") => (ExportVideoCodec::H264, ExportProfile::Main),
        Some("hevc") => (ExportVideoCodec::H265, ExportProfile::Main),
        Some("hevc10") => (ExportVideoCodec::H265, ExportProfile::Main10),
        Some("gif") => (ExportVideoCodec::Gif, ExportProfile::Main),
        value => {
            return Err(format!(
                "Expected h264, hevc, hevc10, or gif; received {value:?}"
            ));
        }
    };
    let frames = arguments
        .next()
        .map(|value| value.parse::<i64>().map_err(|e| e.to_string()))
        .transpose()?
        .unwrap_or(DEFAULT_FRAMES);
    if frames <= 0 {
        return Err("Frame count must be positive".into());
    }
    ffmpeg::init().map_err(|e| e.to_string())?;
    let project = if source
        .extension()
        .is_some_and(|extension| extension == "shrimp")
    {
        // Native projects retain their timeline duration and linked asset paths.
        shrimply_project_document::project::load_project_snapshot(&source)?
    } else {
        let input = ffmpeg::format::input(&source).map_err(|e| e.to_string())?;
        let stream = input
            .streams()
            .best(ffmpeg::media::Type::Video)
            .ok_or("Input has no video stream")?;
        let parameters = stream.parameters();
        let parameters = unsafe { &*parameters.as_ptr() };
        let size = CanvasSize {
            width: u32::try_from(parameters.width).map_err(|_| "Invalid video width")?,
            height: u32::try_from(parameters.height).map_err(|_| "Invalid video height")?,
        };
        let rate = stream.avg_frame_rate();
        if rate.numerator() <= 0 || rate.denominator() <= 0 {
            return Err("Input has no valid frame rate".into());
        }
        let mut project = Project {
            canvas_size: size,
            fps: fraction_new(OUTPUT_FPS, 1),
            ..Default::default()
        };
        let duration = Time::from_fraction(frames, OUTPUT_FPS);
        let mut item = VideoItem::background_item(size, Time::ZERO, duration);
        item.content = VideoItemContent::Media;
        item.file = Asset::new(source);
        item.source_width = size.width;
        item.source_height = size.height;
        item.source_duration = duration;
        item.playback_fps =
            fraction_new(i64::from(rate.numerator()), i64::from(rate.denominator()));
        project.video_tracks.push(VideoTrack {
            items: vec![item],
            ..Default::default()
        });
        project
    };
    let settings = ExportSettings {
        path,
        overwrite: false,
        video_codec,
        container: if video_codec == ExportVideoCodec::Gif {
            ExportContainer::Gif
        } else {
            ExportContainer::Mp4
        },
        fps: project.fps,
        background_alpha: u8::MAX,
        rate_control: ExportRateControl::ConstantQp,
        constant_qp: 20,
        bitrate_kbps: 10_000,
        max_bitrate_kbps: 10_000,
        target_quality: 20,
        keyframe_interval_seconds: 2,
        preset: ExportPreset::P6,
        tuning: ExportTuning::HighQuality,
        multipass: ExportMultipass::SinglePass,
        profile,
        look_ahead: false,
        adaptive_quantization: false,
        b_frames: 2,
        b_frame_as_reference: false,
        audio_encoder: ExportAudioEncoder::Aac,
        audio_sample_rate: 48_000,
        audio_bitrate_kbps: 192,
        maximum_temporal_decoders: 4,
        gpu_host_memory_gib: fraction_new(2, 1),
    };
    let started = Instant::now();
    let mut exported_frames = 0;
    export_project(
        project,
        settings,
        Arc::new(AtomicBool::new(false)),
        |progress| {
            if let ExportProgress::EncodingVideo {
                current_frame,
                total_frames,
                fps_milli,
            } = progress
            {
                exported_frames = current_frame;
                if current_frame.is_multiple_of(OUTPUT_FPS as u64) {
                    println!(
                        "{current_frame}/{total_frames} frames; {} milli-fps",
                        fps_milli
                    );
                }
            }
        },
    )?;
    println!(
        "Exported {exported_frames} frames in {:?}",
        started.elapsed()
    );
    Ok(())
}

#[cfg(target_os = "macos")]
fn main() {
    panic!("Use the Metal export backend on macOS");
}
