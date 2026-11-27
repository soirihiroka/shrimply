use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use ffmpeg_next as ffmpeg;
use shrimply_asset::Asset;
use shrimply_manim_bridge::{Settings, compile_uncancelled, reflected_parameters};
use shrimply_manim_wgpu::{PreparedAnimation, Renderer};
use shrimply_math_core::fraction_new;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
const FPS: u32 = 30;
const RGBA_CHANNELS: usize = 4;
const FRAME_BYTES: usize = WIDTH as usize * HEIGHT as usize * RGBA_CHANNELS;

const SCENES: &[&str] = &[
    "VisualParity",
    "ThreeDParity",
    "VectorMorphParity",
    "GenericPipelineParity",
    "LatexParity",
];

#[test]
#[ignore = "requires WGPU, Manim, and ffmpeg"]
fn native_manim_matches_rust_renderer() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest
        .ancestors()
        .nth(5)
        .expect("Manim crate must be nested under the repository");
    let source = std::env::var_os("SHRIMPLY_MANIM_VISUAL_SOURCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest.join("tests/fixtures/visual_parity.py"));
    let output = tempfile::tempdir().expect("create Manim visual comparison directory");
    let checker = output.path().join("checker.png");
    write_checker_png(&checker);
    // SAFETY: this ignored test is the only test in its process and sets the fixture path
    // before it starts either Python worker.
    unsafe { std::env::set_var("SHRIMPLY_MANIM_CHECKER", &checker) };
    let mut renderer = Renderer::new().expect("create headless Manim WGPU renderer");
    let configured_scene = std::env::var("SHRIMPLY_MANIM_VISUAL_SCENE").ok();
    if let Some(scene) = configured_scene {
        compare_scene(
            root,
            &manifest,
            &source,
            &scene,
            output.path(),
            &mut renderer,
        );
    } else {
        for scene in SCENES {
            compare_scene(
                root,
                &manifest,
                &source,
                scene,
                output.path(),
                &mut renderer,
            );
        }
    }
}

fn compare_scene(
    root: &Path,
    manifest: &Path,
    source: &Path,
    scene: &str,
    output: &Path,
    renderer: &mut Renderer,
) {
    render_native(root, manifest, source, scene, output);
    let native_video = output.join(format!("{scene}.rgba"));
    assert!(
        native_video.is_file(),
        "native Manim did not create {native_video:?}"
    );
    let mut settings = Settings {
        source: Asset::new(source),
        scene: scene.to_string(),
        width: WIDTH,
        height: HEIGHT,
        fps: fraction_new(FPS.into(), 1),
        parameters: Default::default(),
    };
    let first =
        compile_uncancelled(&settings, |_| {}).expect("compile Manim fixture to immutable IR");
    let animation = if first.scene().render_is_current {
        first
    } else {
        settings.parameters = reflected_parameters(&first)
            .expect("decode discovered Manim parameters")
            .into_iter()
            .map(|parameter| (parameter.key, parameter.value))
            .collect();
        compile_uncancelled(&settings, |_| {})
            .expect("recompile Manim fixture with discovered parameters")
    };
    assert!(
        animation.scene().render_is_current,
        "{scene} parameter reconciliation did not produce a current render"
    );
    let cached =
        compile_uncancelled(&settings, |_| {}).expect("reuse compiled Manim fixture from memory");
    assert!(
        Arc::ptr_eq(&animation, &cached),
        "unchanged Manim animation was compiled twice"
    );
    let native_frame_count = native_frame_count(&native_video);
    assert_eq!(
        animation.frames().len(),
        native_frame_count,
        "{scene} frame count differs"
    );
    assert_eq!(
        animation.scene().duration,
        fraction_new(native_frame_count as i64, FPS.into()),
        "{scene} duration differs"
    );
    for (index, frame) in animation.frames().iter().enumerate() {
        assert_eq!(frame.index, index as u64, "{scene} frame index differs");
        assert_eq!(
            frame.time,
            fraction_new(index as i64, FPS.into()),
            "{scene} frame timestamp differs at {index}"
        );
    }
    let prepared = PreparedAnimation::new(animation).expect("prepare Manim fixture for WGPU");
    let compared = decode_native_frames(&native_video, |frame, native| {
        let rust = renderer
            .render_rgba_for_validation(&prepared, frame)
            .expect("render Manim frame with WGPU");
        compare(scene, frame, native, rust.pixels(), root);
    });
    assert_eq!(
        compared, native_frame_count,
        "{scene} decoded frame count differs"
    );
}

fn render_native(root: &Path, manifest: &Path, source: &Path, scene: &str, output: &Path) {
    let python_path = std::env::join_paths([
        root.join("external/manim"),
        manifest.join("../manim-bridge/python"),
    ])
    .expect("build native Manim Python path");
    let output = output.join(format!("{scene}.rgba"));
    let status = Command::new(std::env::var_os("UV").unwrap_or_else(|| "uv".into()))
        .args(["run", "--python", "3.14", "--project"])
        .arg(manifest.join("../manim-bridge/python"))
        .arg("python")
        .arg(manifest.join("tests/fixtures/native_reference.py"))
        .arg(source)
        .arg(scene)
        .arg(&output)
        .args([&WIDTH.to_string(), &HEIGHT.to_string(), &FPS.to_string()])
        .env("PYTHONPATH", python_path)
        .stdin(Stdio::null())
        .status()
        .expect("run native Manim reference render");
    assert!(
        status.success(),
        "native Manim reference render failed with {status}"
    );
}

fn decode_native_frames(video: &Path, mut consume: impl FnMut(usize, &[u8])) -> usize {
    let mut input = BufReader::new(File::open(video).expect("open native Manim RGBA stream"));
    let mut pixels = vec![0; FRAME_BYTES];
    let mut frame = 0;
    loop {
        let mut filled = 0;
        while filled < pixels.len() {
            let read = input
                .read(&mut pixels[filled..])
                .expect("read native Manim RGBA stream");
            if read == 0 {
                assert_eq!(filled, 0, "native Manim RGBA stream ends mid-frame");
                return frame;
            }
            filled += read;
        }
        consume(frame, &pixels);
        frame += 1;
    }
}

fn native_frame_count(video: &Path) -> usize {
    let bytes = usize::try_from(
        std::fs::metadata(video)
            .expect("inspect native Manim RGBA stream")
            .len(),
    )
    .expect("native Manim RGBA stream is too large");
    assert_eq!(
        bytes % FRAME_BYTES,
        0,
        "native Manim RGBA stream ends mid-frame"
    );
    bytes / FRAME_BYTES
}

fn compare(scene: &str, frame: usize, native: &[u8], rust: &[u8], root: &Path) {
    assert_eq!(native.len(), rust.len());
    if native == rust {
        return;
    }
    let mismatches = native
        .iter()
        .zip(rust)
        .filter(|(native, rust)| native != rust)
        .count();
    let first = native
        .iter()
        .zip(rust)
        .position(|(native, rust)| native != rust)
        .expect("different Manim frames must contain a mismatched byte");
    let mut difference = Vec::with_capacity(rust.len());
    for (native, rust) in native
        .as_chunks::<4>()
        .0
        .iter()
        .zip(rust.as_chunks::<4>().0.iter())
    {
        let alpha = native[3].abs_diff(rust[3]);
        difference.extend(
            native[..3]
                .iter()
                .zip(rust)
                .map(|(native, rust)| native.abs_diff(*rust).max(alpha)),
        );
        difference.push(255);
    }
    let artifacts = root.join("target/manim-visual-diff");
    std::fs::create_dir_all(&artifacts).expect("create Manim visual diff artifact directory");
    write_png(
        &artifacts.join(format!("{scene}-frame-{frame}-native.png")),
        native,
    );
    write_png(
        &artifacts.join(format!("{scene}-frame-{frame}-rust.png")),
        rust,
    );
    write_png(
        &artifacts.join(format!("{scene}-frame-{frame}-difference.png")),
        &difference,
    );
    panic!(
        "Manim {scene} frame {frame} differs in {mismatches} channels; first mismatch at pixel {} channel {}: native={:?} rust={:?}; artifacts: {}",
        first / RGBA_CHANNELS,
        first % RGBA_CHANNELS,
        &native[first / RGBA_CHANNELS * RGBA_CHANNELS..][..RGBA_CHANNELS],
        &rust[first / RGBA_CHANNELS * RGBA_CHANNELS..][..RGBA_CHANNELS],
        artifacts.display()
    );
}

fn write_png(path: &Path, rgba: &[u8]) {
    write_rgba_png(path, rgba, WIDTH, HEIGHT);
}

fn write_checker_png(path: &Path) {
    const SIZE: u32 = 4;
    let mut pixels = Vec::with_capacity((SIZE * SIZE * RGBA_CHANNELS as u32) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let value = if (x + y) % 2 == 0 { 255 } else { 32 };
            pixels.extend_from_slice(&[value, 96, 255 - value, 255]);
        }
    }
    write_rgba_png(path, &pixels, SIZE, SIZE);
}

fn write_rgba_png(path: &Path, rgba: &[u8], width: u32, height: u32) {
    ffmpeg::init().expect("initialize FFmpeg");
    let codec = ffmpeg::codec::encoder::find(ffmpeg::codec::Id::PNG)
        .expect("FFmpeg PNG encoder must be installed");
    let mut encoder = ffmpeg::codec::Context::new_with_codec(codec)
        .encoder()
        .video()
        .expect("create FFmpeg PNG encoder");
    encoder.set_width(width);
    encoder.set_height(height);
    encoder.set_format(ffmpeg::format::Pixel::RGBA);
    encoder.set_time_base((1, 1));
    let mut encoder = encoder.open_as(codec).expect("open FFmpeg PNG encoder");
    let mut frame = ffmpeg::frame::Video::new(ffmpeg::format::Pixel::RGBA, width, height);
    let row_bytes = width as usize * RGBA_CHANNELS;
    let stride = frame.stride(0);
    for (source, destination) in rgba
        .chunks_exact(row_bytes)
        .zip(frame.data_mut(0).chunks_exact_mut(stride))
    {
        destination[..row_bytes].copy_from_slice(source);
    }
    frame.set_pts(Some(0));
    encoder.send_frame(&frame).expect("encode Manim diff PNG");
    encoder.send_eof().expect("finish Manim diff PNG encoder");
    let mut packet = ffmpeg::Packet::empty();
    encoder
        .receive_packet(&mut packet)
        .expect("receive Manim diff PNG packet");
    std::fs::write(
        path,
        packet
            .data()
            .expect("FFmpeg PNG encoder returned an empty packet"),
    )
    .expect("write Manim visual diff PNG");
}
