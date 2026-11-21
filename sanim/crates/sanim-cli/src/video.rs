//! Standalone exact-time Skia frames streamed to an external FFmpeg encoder.
use crate::{Args, render_scene};
use sanim_lang::Prepared;
use sanim_render::Renderer;
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
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

struct HardwareEncoder {
    name: &'static str,
    device: &'static [&'static str],
    output: &'static [&'static str],
}

fn ffmpeg(
    override_path: Option<&Path>,
    size: &str,
    rate: &str,
) -> Result<(PathBuf, HardwareEncoder), String> {
    let executable = override_path.map(Path::to_owned).unwrap_or_else(|| {
        let bundled = std::env::current_exe().ok().map(|app| {
            app.with_file_name(format!("shrimply-ffmpeg{}", std::env::consts::EXE_SUFFIX))
        });
        let development = std::env::var_os("FFMPEG_DIR").map(|prefix| {
            PathBuf::from(prefix).join(if cfg!(windows) {
                "tools/ffmpeg/ffmpeg.exe"
            } else {
                "bin/ffmpeg"
            })
        });
        bundled
            .into_iter()
            .chain(development)
            .find(|path| path.is_file())
            .unwrap_or_else(|| PathBuf::from("ffmpeg"))
    });
    // The encoder listing also prints the linked release in FFmpeg's banner.
    let probe = Command::new(&executable)
        .arg("-encoders")
        .output()
        .map_err(|error| {
            format!("could not start FFmpeg: {error}; install ffmpeg or pass --ffmpeg PATH")
        })?;
    let banner = String::from_utf8_lossy(&probe.stderr);
    if !probe.status.success() {
        return Err(format!(
            "could not query FFmpeg encoders at {} ({}): {}",
            executable.display(),
            probe.status,
            banner.trim()
        ));
    }
    let expected = include_str!("../../../../external/ffmpeg/RELEASE").trim();
    let version = banner
        .lines()
        .find_map(|line| line.strip_prefix("ffmpeg version "))
        .and_then(|line| line.split_ascii_whitespace().next())
        .unwrap_or("unknown");
    // Releases built in a Git checkout may retain the upstream tag's n prefix.
    if version.strip_prefix('n').unwrap_or(version) != expected {
        return Err(format!(
            "expected FFmpeg {expected}, found {version} at {}; pass --ffmpeg PATH",
            executable.display()
        ));
    }
    let listing = String::from_utf8_lossy(&probe.stdout);
    // A compiled-in encoder can still be unusable on this device. Check the actual
    // frame size, rate and pixel path before rendering any animation frames.
    let mut failures = Vec::new();
    for encoder in [
        HardwareEncoder {
            name: "h264_nvenc",
            device: &[],
            output: &["-pix_fmt", "yuv420p"],
        },
        HardwareEncoder {
            name: "h264_videotoolbox",
            device: &[],
            output: &["-allow_sw", "0", "-pix_fmt", "yuv420p"],
        },
        HardwareEncoder {
            name: "h264_vaapi",
            device: &["-init_hw_device", "vaapi=va", "-filter_hw_device", "va"],
            output: &["-vf", "format=nv12,hwupload"],
        },
    ] {
        if !listing
            .lines()
            .any(|line| line.split_ascii_whitespace().nth(1) == Some(encoder.name))
        {
            continue;
        }
        let probe = Command::new(&executable)
            .args(["-hide_banner", "-loglevel", "error", "-nostdin"])
            .args(encoder.device)
            .args(["-f", "lavfi", "-i"])
            .arg(format!("color=size={size}:rate={rate},format=rgba"))
            .args(["-frames:v", "1", "-c:v", encoder.name])
            .args(encoder.output)
            .args(["-f", "null", "-"])
            .output()
            .map_err(|error| format!("could not probe {}: {error}", encoder.name))?;
        if probe.status.success() {
            return Ok((executable, encoder));
        }
        failures.push(format!(
            "{}: {}",
            encoder.name,
            String::from_utf8_lossy(&probe.stderr).trim()
        ));
    }
    Err(format!(
        "No usable hardware H.264 encoder for {size} at {rate} fps. {}",
        failures.join("\n")
    ))
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
    let size = format!("{width}x{height}");
    let fps = rate.to_string();
    let (executable, codec) = ffmpeg(args.ffmpeg.as_deref(), &size, &fps)?;
    let mut encoder = Command::new(executable)
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"])
        .args(codec.device)
        .args([
            "-f",
            "rawvideo",
            "-pixel_format",
            "rgba",
            "-video_size",
            &size,
            "-framerate",
            &fps,
            "-i",
            "pipe:0",
            "-an",
            "-c:v",
            codec.name,
        ])
        .args(codec.output)
        .args(["-movflags", "+faststart", "-f", "mp4"])
        .arg(temporary.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::from(errors.try_clone().map_err(|e| e.to_string())?))
        .spawn()
        .map_err(|e| format!("could not start FFmpeg encoder: {e}"))?;
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
        serde_json::json!({"ok":true,"output":output,"encoder":codec.name,"frames":frames,"rate":rate.to_string(),"duration":duration.to_string(),"render_micros":started.elapsed().as_micros(),"width":width,"height":height}),
    )
}
