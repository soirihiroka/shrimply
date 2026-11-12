//! Standalone exact-time Skia frames streamed to an external FFmpeg encoder.
use crate::{Args, render_scene};
use sanim_lang::Prepared;
use sanim_render::Renderer;
use std::{
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
};

const MAX_VIDEO_FRAMES: u64 = 1_000_000;
const MAX_ENCODER_DIAGNOSTIC: u64 = 16_384;

pub(super) fn render(prepared: &Prepared, args: &Args) -> Result<(), String> {
    if args.time.is_some() || args.frame != 0 {
        return Err(
            "video renders the whole timeline; --time and --frame select a single PNG with render"
                .into(),
        );
    }
    let output = args
        .output
        .as_ref()
        .ok_or("video requires --output PATH.mp4")?;
    if output.extension().and_then(|s| s.to_str()) != Some("mp4") {
        return Err("video currently supports H.264 MP4; use an .mp4 output path".into());
    }
    let mut renderer = Renderer::default();
    let (scene, mut options) = render_scene(prepared, args, &mut renderer)?;
    if options.width < 2
        || options.height < 2
        || !options.width.is_multiple_of(2)
        || !options.height.is_multiple_of(2)
    {
        return Err(
            "H.264 yuv420p output requires even width and height, each at least 2 pixels".into(),
        );
    }
    options.encode_png = false;
    let frames = sanim_core::math::frame_count(scene.animation().duration, scene.animation().rate)?;
    if frames == 0 || frames > MAX_VIDEO_FRAMES {
        return Err(format!(
            "video needs 1..={MAX_VIDEO_FRAMES} frames, got {frames}; give the animation a positive bounded duration"
        ));
    }
    // Validate output size/work limits before starting the encoder.
    renderer.record(&scene, scene.animation().frame_time(0)?, &options)?;
    let directory = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = tempfile::Builder::new()
        .prefix(".sanim-video-")
        .suffix(".mp4")
        .tempfile_in(directory)
        .map_err(|e| format!("video output: {e}"))?;
    let mut errors = tempfile::tempfile().map_err(|e| e.to_string())?;
    let mut encoder = Command::new(args.ffmpeg.as_deref().unwrap_or(Path::new("ffmpeg")))
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-y",
            "-f",
            "rawvideo",
            "-pixel_format",
            "rgba",
            "-video_size",
        ])
        .arg(format!("{}x{}", options.width, options.height))
        .arg("-framerate")
        .arg(scene.animation().rate.to_string())
        .args([
            "-i",
            "pipe:0",
            "-an",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "+faststart",
            "-f",
            "mp4",
        ])
        .arg(temporary.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::from(errors.try_clone().map_err(|e| e.to_string())?))
        .spawn()
        .map_err(|e| {
            format!("could not start FFmpeg: {e}; install ffmpeg or pass --ffmpeg PATH")
        })?;
    let started = std::time::Instant::now();
    let result = (|| {
        let mut input = encoder.stdin.take().ok_or("FFmpeg stdin unavailable")?;
        for index in 0..frames {
            let recording =
                renderer.record(&scene, scene.animation().frame_time(index)?, &options)?;
            let frame = recording.rasterize_opaque()?;
            input
                .write_all(&frame.rgba)
                .map_err(|e| format!("FFmpeg frame {index}: {e}"))?;
        }
        Ok::<_, String>(())
    })();
    if result.is_err() {
        let _ = encoder.kill();
    }
    let status = encoder
        .wait()
        .map_err(|e| format!("FFmpeg wait failed: {e}"))?;
    if !status.success() || result.is_err() {
        use std::io::{Seek, SeekFrom};
        errors.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let mut diagnostic = String::new();
        errors
            .take(MAX_ENCODER_DIAGNOSTIC)
            .read_to_string(&mut diagnostic)
            .map_err(|e| e.to_string())?;
        return Err(format!(
            "video encoding failed ({status}): {}\n{}",
            result.err().unwrap_or_default(),
            diagnostic.trim()
        ));
    }
    temporary
        .persist(output)
        .map_err(|e| format!("cannot publish video: {e}"))?;
    println!(
        "{}",
        serde_json::json!({"ok":true,"output":output,"frames":frames,"rate":scene.animation().rate.to_string(),"duration":scene.animation().duration.to_string(),"render_micros":started.elapsed().as_micros(),"width":options.width,"height":options.height})
    );
    Ok(())
}
