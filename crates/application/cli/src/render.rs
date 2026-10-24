use crate::args::Options;
use shrimply_export_core::video::{ExportProgress, ExportVideoCodec};
use shrimply_preferences_core as preferences;
use shrimply_project_document::project;
use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

const PROGRESS_INTERVAL: Duration = Duration::from_secs(1);

pub fn run(options: Options) -> Result<(), String> {
    let path = options
        .project
        .canonicalize()
        .map_err(|error| format!("Could not open {}: {error}", options.project.display()))?;
    if !options.overwrite
        && options
            .output
            .try_exists()
            .map_err(|error| error.to_string())?
    {
        return Err(format!(
            "Output already exists: {}. Use --overwrite to replace it.",
            options.output.display()
        ));
    }
    eprintln!("Loading {}", path.display());
    let project = project::load_project_snapshot(&path)?;
    for source in project.executable_sources() {
        shrimply_trust_core::require(&source)?;
    }
    // The exporter also writes a benchmark sidecar; neither path may replace a source.
    let mut benchmark_path = options.output.as_os_str().to_os_string();
    benchmark_path.push(".benchmark.json");
    for output in [&options.output, &PathBuf::from(benchmark_path)] {
        let resolved = output
            .canonicalize()
            .or_else(|_| std::path::absolute(output))
            .map_err(|error| format!("Could not resolve {}: {error}", output.display()))?;
        if resolved == path {
            return Err("Export destination would replace the input project".into());
        }
        shrimply_export_core::ensure_output_is_not_an_asset(&project, output)?;
    }
    if shrimply_export_core::video::export_duration(&project) <= project::Time::ZERO {
        return Err("Project has no video or audio duration to render".into());
    }
    project::set_active_project_path(&path);
    let preferences = preferences::snapshot(&preferences::open_with_defaults());
    shrimply_blender_core::set_binary(preferences.blender_binary.clone());
    shrimply_audio_engine::pneuma::set_server_url(&preferences.compute_server_url);
    let settings = settings(&options, options.fps.unwrap_or(project.fps), &preferences);
    let mut last_progress = Instant::now();
    #[cfg(not(target_os = "macos"))]
    use shrimply_export_cuda::export_project;
    #[cfg(target_os = "macos")]
    use shrimply_export_metal::export_project;
    export_project(
        project,
        settings,
        Arc::new(AtomicBool::new(false)),
        |progress| {
            let immediate = matches!(
                progress,
                ExportProgress::SettingUp(_) | ExportProgress::Finalizing
            );
            if !immediate && last_progress.elapsed() < PROGRESS_INTERVAL {
                return;
            }
            last_progress = Instant::now();
            match progress {
                ExportProgress::SettingUp(stage) => eprintln!("{stage}"),
                ExportProgress::MixingAudio {
                    current_frame,
                    total_frames,
                } => {
                    eprintln!("Mixing audio: {current_frame}/{total_frames}");
                }
                ExportProgress::EncodingAudio {
                    current_frame,
                    total_frames,
                } => {
                    eprintln!("Encoding audio: {current_frame}/{total_frames}");
                }
                ExportProgress::EncodingVideo {
                    current_frame,
                    total_frames,
                    ..
                } => {
                    eprintln!("Rendering video: {current_frame}/{total_frames}");
                }
                ExportProgress::Finalizing => eprintln!("Finalizing"),
            }
        },
    )?;
    eprintln!("Rendered {}", options.output.display());
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn settings(
    options: &Options,
    fps: shrimply_math_core::Fraction,
    preferences: &preferences::PreferencesSnapshot,
) -> shrimply_export_cuda::ExportSettings {
    use shrimply_export_cuda::{
        ExportMultipass, ExportPreset, ExportProfile, ExportRateControl, ExportSettings,
        ExportTuning,
    };
    ExportSettings {
        path: options.output.clone(),
        overwrite: options.overwrite,
        video_codec: options.codec,
        container: options.container,
        fps,
        background_alpha: if options.codec == ExportVideoCodec::Gif {
            0
        } else {
            u8::MAX
        },
        rate_control: ExportRateControl::VariableBitrate,
        constant_qp: 20,
        bitrate_kbps: options.video_bitrate,
        max_bitrate_kbps: options.video_bitrate,
        target_quality: 20,
        keyframe_interval_seconds: 0,
        preset: ExportPreset::P6,
        tuning: ExportTuning::HighQuality,
        multipass: ExportMultipass::QuarterResolution,
        profile: ExportProfile::Main,
        look_ahead: true,
        adaptive_quantization: true,
        b_frames: 2,
        b_frame_as_reference: false,
        audio_encoder: options.audio_encoder,
        audio_sample_rate: options.audio_sample_rate,
        audio_bitrate_kbps: options.audio_bitrate,
        maximum_temporal_decoders: preferences.temporal_decoder_pool_size as usize,
        gpu_host_memory_gib: preferences.gpu_host_memory_gib,
    }
}

#[cfg(target_os = "macos")]
fn settings(
    options: &Options,
    fps: shrimply_math_core::Fraction,
    preferences: &preferences::PreferencesSnapshot,
) -> shrimply_export_metal::ExportSettings {
    use shrimply_export_metal::{
        ExportSettings, H264Entropy, VideoToolboxProfile, VideoToolboxRateControl,
    };
    ExportSettings {
        path: options.output.clone(),
        overwrite: options.overwrite,
        video_codec: options.codec,
        container: options.container,
        fps,
        background_alpha: if options.codec == ExportVideoCodec::Gif {
            0
        } else {
            u8::MAX
        },
        rate_control: VideoToolboxRateControl::AverageBitrate,
        profile: if options.codec == ExportVideoCodec::H264 {
            VideoToolboxProfile::H264Baseline
        } else {
            VideoToolboxProfile::HevcMain
        },
        bitrate_kbps: options.video_bitrate,
        keyframe_interval_seconds: 2,
        b_frames: 0,
        h264_entropy: H264Entropy::Cabac,
        prioritize_speed: false,
        power_efficient: false,
        spatial_aq: false,
        max_reference_frames: 0,
        audio_encoder: options.audio_encoder,
        audio_sample_rate: options.audio_sample_rate,
        audio_bitrate_kbps: options.audio_bitrate,
        maximum_temporal_decoders: preferences.temporal_decoder_pool_size as usize,
    }
}
