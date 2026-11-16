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

pub(super) fn render(
    prepared: &Prepared,
    args: &Args,
    lint: &sanim_lang::LintReport,
) -> Result<(), String> {
    if args.time.is_some() || args.frame != 0 {
        return Err(
            "video renders the whole timeline; --time and --frame select a single PNG with render"
                .into(),
        );
    }
    let mut renderer = Renderer::default();
    let (scene, mut options) = render_scene(prepared, args, &mut renderer)?;
    options.encode_png = false;
    let duration = args
        .duration
        .or_else(|| (!scene.animation().indefinite).then_some(scene.animation().duration))
        .ok_or("this timeline waits forever; provide --duration 10s to render a finite video")?;
    // Validate output size/work limits before starting the encoder.
    renderer.record(&scene, scene.animation().frame_time(0)?, &options)?;
    let mut result = encode(
        args,
        scene.animation().rate,
        duration,
        options.width,
        options.height,
        |index, output| {
            let frame =
                renderer.render_opaque(&scene, scene.animation().frame_time(index)?, &options)?;
            output
                .write_all(&frame.rgba)
                .map_err(|error| error.to_string())?;
            Ok(frame.rgba.len())
        },
    )?;
    result["statistics"] = serde_json::to_value(&prepared.statistics).map_err(|e| e.to_string())?;
    result["performance"] =
        serde_json::to_value(crate::performance_diagnostics(&prepared.performance))
            .map_err(|e| e.to_string())?;
    result["diagnostics"] = serde_json::to_value(&lint.diagnostics).map_err(|e| e.to_string())?;
    println!("{result}");
    Ok(())
}

pub(super) fn encode(
    args: &Args,
    rate: sanim_core::Fraction,
    duration: sanim_core::Fraction,
    width: u32,
    height: u32,
    mut frame: impl FnMut(u64, &mut dyn Write) -> Result<usize, String>,
) -> Result<serde_json::Value, String> {
    let output = args
        .output
        .as_ref()
        .ok_or("video requires --output PATH.mp4")?;
    if output.extension().and_then(|s| s.to_str()) != Some("mp4") {
        return Err("video supports H.264 MP4; use an .mp4 output path".into());
    }
    if width < 2 || height < 2 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
        return Err("H.264 yuv420p requires even width and height, each at least 2 pixels".into());
    }
    let frames = sanim_core::math::frame_count(duration, rate)?;
    if frames == 0 || frames > MAX_VIDEO_FRAMES {
        return Err(format!(
            "video needs 1..={MAX_VIDEO_FRAMES} frames, got {frames}"
        ));
    }
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
        .arg(format!("{}x{}", width, height))
        .arg("-framerate")
        .arg(rate.to_string())
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
            let bytes = frame(index, &mut input)
                .map_err(|error| format!("FFmpeg frame {index}: {error}"))?;
            let expected = width as usize * height as usize * 4;
            if bytes != expected {
                return Err(format!(
                    "video frame {index} has {bytes} bytes; expected {expected}"
                ));
            }
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
    Ok(
        serde_json::json!({"ok":true,"output":output,"frames":frames,"rate":rate.to_string(),"duration":duration.to_string(),"render_micros":started.elapsed().as_micros(),"width":width,"height":height}),
    )
}
