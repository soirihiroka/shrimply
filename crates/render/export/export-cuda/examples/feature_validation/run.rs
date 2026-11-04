use shrimply_export_cuda::*;
use shrimply_math_core::{Time, fraction_new};
use shrimply_preview_render_vulkan::Compositor;
use shrimply_project_document::project::*;
use shrimply_visual_modifiers::{
    ModifierEffect, ModifierModel, RasterModifierEffect, VectorModifierEffect, scene_3d,
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};

const WIDTH: u32 = 160;
const HEIGHT: u32 = 96;
const FPS: i64 = 30;
const FRAMES: i64 = 6;

pub fn run() -> Result<(), String> {
    let output = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("Pass a new output directory and optional case filter")?,
    );
    std::fs::create_dir(&output).map_err(|e| e.to_string())?;
    let output = output.canonicalize().map_err(|e| e.to_string())?;
    let filter = std::env::args().nth(2).unwrap_or_default();
    create_assets(&output)?;
    let cases = cases(&output)?;
    let mut failed = Vec::new();
    let mut completed = 0;
    for (name, project) in cases {
        if !name.contains(&filter) {
            continue;
        }
        let started = Instant::now();
        println!("START {name}");
        match validate(&name, project, &output) {
            Ok(()) => {
                completed += 1;
                println!("PASS {name} {:?}", started.elapsed());
            }
            Err(error) => {
                eprintln!("FAIL {name}: {error}");
                failed.push(format!("{name}: {error}"));
            }
        }
    }
    println!("Completed {completed} cases; {} failures", failed.len());
    if completed == 0 || !failed.is_empty() {
        return Err(failed.join("\n"));
    }
    Ok(())
}

fn project(item: VideoItem) -> Project {
    Project {
        canvas_size: CanvasSize {
            width: WIDTH,
            height: HEIGHT,
        },
        fps: fraction_new(FPS, 1),
        video_tracks: vec![VideoTrack {
            items: vec![item],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn modifier(effect: ModifierEffect) -> VisualModifier {
    VisualModifier {
        id: uuid::Uuid::new_v4(),
        enabled: true,
        alpha_mask: None,
        effect,
    }
}

fn cases(output: &Path) -> Result<Vec<(String, Project)>, String> {
    let size = CanvasSize {
        width: WIDTH,
        height: HEIGHT,
    };
    let duration = Time::from_fraction(FRAMES, FPS);
    let mut image = VideoItem::background_item(size, Time::ZERO, duration);
    image.content = VideoItemContent::Image;
    image.file = Asset::new(output.join("source.png"));
    image.source_width = WIDTH;
    image.source_height = HEIGHT;
    let mut result = vec![("source-image".into(), project(image.clone()))];
    let shape = VideoItem::shape_item(size, Time::ZERO, duration);
    result.push(("source-shape".into(), project(shape.clone())));
    let mut text = VideoItem::text_item(size, Time::ZERO, duration);
    if let VideoItemContent::Text(content) = &mut text.content {
        content.text = TimelineValue::new_const("Intel GTK".into());
    }
    result.push(("source-text".into(), project(text.clone())));
    result.push((
        "source-background".into(),
        project(VideoItem::background_item(size, Time::ZERO, duration)),
    ));
    let mut svg = image.clone();
    svg.content = VideoItemContent::Svg;
    svg.file = Asset::new(output.join("source.svg"));
    result.push(("source-svg".into(), project(svg)));
    let mut gif = image.clone();
    gif.content = VideoItemContent::Gif;
    gif.file = Asset::new("assets/loading-shrimp.gif");
    result.push(("source-gif".into(), project(gif)));
    let mut manim = image.clone();
    manim.content = VideoItemContent::Manim(Box::new(ManimItem {
        scene: "IntelValidation".into(),
        ..Default::default()
    }));
    manim.file = Asset::new(output.join("manim.py"));
    result.push(("source-manim".into(), project(manim)));
    let mut gaussian = image.clone();
    gaussian.content = VideoItemContent::Gaussian(Box::default());
    gaussian.file = Asset::new(output.join("source.ply"));
    result.push(("source-gaussian".into(), project(gaussian)));
    let mut obj = VideoItem::obj_scene_item(size, Time::ZERO, duration);
    obj.modifiers = vec![
        modifier(ModifierEffect::scene_3d(
            scene_3d::Scene3dModifierEffect::Object(Box::new(
                scene_3d::Object3dModifier::with_file(Asset::new(output.join("source.obj"))),
            )),
        )),
        modifier(ModifierEffect::scene_3d(
            scene_3d::Scene3dModifierEffect::SunLight(Box::default()),
        )),
    ];
    result.push(("source-obj".into(), project(obj.clone())));
    for factory in scene_3d::Scene3dModifierEffect::CATALOG {
        let effect = factory();
        let name = effect.display_name().replace(' ', "-");
        let mut item = obj.clone();
        item.modifiers
            .push(modifier(ModifierEffect::scene_3d(effect)));
        result.push((format!("scene3d-{name}"), project(item)));
    }
    for projection in [
        shrimply_scene_3d::CameraProjection::Perspective,
        shrimply_scene_3d::CameraProjection::Orthographic,
        shrimply_scene_3d::CameraProjection::Equirectangular,
        shrimply_scene_3d::CameraProjection::Cylindrical,
        shrimply_scene_3d::CameraProjection::Fisheye,
    ] {
        let mut item = obj.clone();
        if let VideoItemContent::Obj(scene) = &mut item.content {
            scene.camera.projection = projection;
        }
        result.push((format!("obj-projection-{projection:?}"), project(item)));
    }
    for shading in [
        shrimply_scene_3d::ShadingModel::Pbr,
        shrimply_scene_3d::ShadingModel::Toon,
        shrimply_scene_3d::ShadingModel::Depth,
    ] {
        let mut item = obj.clone();
        if let VideoItemContent::Obj(scene) = &mut item.content {
            scene.material.shading_model = shading;
        }
        result.push((format!("obj-shading-{shading:?}"), project(item)));
    }
    for factory in RasterModifierEffect::CATALOG {
        let effect = factory();
        let name = effect.display_name().replace(' ', "-").replace('/', "-");
        let mut item = image.clone();
        item.modifiers
            .push(modifier(ModifierEffect::raster(effect)));
        result.push((format!("raster-{name}"), project(item)));
    }
    for factory in VectorModifierEffect::CATALOG {
        let effect = factory();
        let name = effect.display_name().replace(' ', "-").replace('/', "-");
        let mut item = text.clone();
        item.modifiers
            .push(modifier(ModifierEffect::vector(effect)));
        result.push((format!("vector-{name}"), project(item)));
    }
    for kind in [
        VisualTransitionKind::Fade,
        VisualTransitionKind::Slide,
        VisualTransitionKind::SlideFade,
        VisualTransitionKind::Write,
        VisualTransitionKind::Drawing,
        VisualTransitionKind::Create,
        VisualTransitionKind::FacetAssembly,
        VisualTransitionKind::Coalesce,
        VisualTransitionKind::ContourCurrent,
        VisualTransitionKind::SoftRefraction,
        VisualTransitionKind::MorphologicalResolve,
        VisualTransitionKind::LivingFill,
        VisualTransitionKind::Diffusion,
        VisualTransitionKind::ReverseDiffusion,
        VisualTransitionKind::Wipe,
        VisualTransitionKind::Iris,
        VisualTransitionKind::ClockWipe,
        VisualTransitionKind::Zoom,
        VisualTransitionKind::Spin,
        VisualTransitionKind::Blur,
        VisualTransitionKind::Pixelate,
        VisualTransitionKind::Dissolve,
        VisualTransitionKind::TriangularFold,
        VisualTransitionKind::Origami,
        VisualTransitionKind::StreakWipe,
    ] {
        let mut item = text.clone();
        let mut transition = VisualTransition::new(TransitionSide::Intro, duration, size);
        transition.set_kind(TransitionSide::Intro, kind);
        item.transitions.intro = Some(transition);
        result.push((format!("intro-{kind:?}"), project(item)));
    }
    for kind in [
        VisualClipTransitionKind::CrossFade,
        VisualClipTransitionKind::FadeThroughColor,
        VisualClipTransitionKind::Wipe,
        VisualClipTransitionKind::Morph,
        VisualClipTransitionKind::Iris,
        VisualClipTransitionKind::ClockWipe,
        VisualClipTransitionKind::Dissolve,
        VisualClipTransitionKind::Slide,
        VisualClipTransitionKind::Push,
        VisualClipTransitionKind::Zoom,
    ] {
        let mut outgoing = image.clone();
        outgoing.end = Time::from_fraction(FRAMES / 2, FPS);
        let mut incoming = image.clone();
        incoming.id = uuid::Uuid::new_v4();
        incoming.file = Asset::new(output.join("target.png"));
        incoming.start = outgoing.end;
        let mut transition =
            VisualClipTransition::new(incoming.id, duration / Time::from_fraction(2, 1));
        transition.set_kind(kind);
        outgoing.transitions.to_next = Some(transition);
        let mut project = project(outgoing);
        project.video_tracks[0].items.push(incoming);
        result.push((format!("clip-{kind:?}"), project));
    }
    Ok(result)
}

fn validate(name: &str, project: Project, output: &Path) -> Result<(), String> {
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut compositor = Compositor::new()?;
    let mut captures = Vec::new();
    for frame in [0, FRAMES / 2, FRAMES - 1] {
        let rendered =
            compositor.render_accurate(&project, Time::from_fraction(frame, FPS), 0, &cancelled)?;
        rendered.wait()?;
        let pixels = compositor.renderer.readback(&rendered.buffer)?;
        if pixels.len() != WIDTH as usize * HEIGHT as usize * 4 {
            return Err("Preview returned wrong pixel dimensions".into());
        }
        save_png(&output.join(format!("{name}-{frame}.png")), &pixels)?;
        captures.push(pixels);
    }
    if name == "source-manim" && captures[0] == captures[2] {
        return Err("Animated Manim frames are identical".into());
    }
    if [
        "source-image",
        "source-manim",
        "source-gaussian",
        "source-obj",
    ]
    .contains(&name)
        && captures[1].chunks_exact(4).all(|pixel| pixel[3] == 0)
    {
        return Err("Source preview is fully transparent".into());
    }
    drop(compositor);
    let settings = ExportSettings {
        path: output.join(format!("{name}.mp4")),
        overwrite: false,
        video_codec: ExportVideoCodec::H264,
        container: ExportContainer::Mp4,
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
        profile: ExportProfile::Main,
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
    let path = settings.path.clone();
    let mut exported = 0;
    export_project(project, settings, cancelled, |progress| {
        if let ExportProgress::EncodingVideo { current_frame, .. } = progress {
            exported = current_frame;
        }
    })?;
    if exported != FRAMES as u64 {
        return Err(format!("Exported {exported} frames, expected {FRAMES}"));
    }
    let mut input = ffmpeg_next::format::input(&path).map_err(|e| e.to_string())?;
    let video = input
        .streams()
        .best(ffmpeg_next::media::Type::Video)
        .ok_or("Export has no video stream")?;
    if video.parameters().id() != ffmpeg_next::codec::Id::H264 {
        return Err("Export did not use H.264".into());
    }
    let index = video.index();
    let packets = input
        .packets()
        .filter(|(stream, _)| stream.index() == index)
        .count();
    if packets != FRAMES as usize {
        return Err(format!("Export has {packets} packets, expected {FRAMES}"));
    }
    Ok(())
}

fn save_png(path: &Path, pixels: &[u8]) -> Result<(), String> {
    let info = skia_safe::ImageInfo::new(
        (WIDTH as i32, HEIGHT as i32),
        skia_safe::ColorType::RGBA8888,
        skia_safe::AlphaType::Unpremul,
        None,
    );
    let image = skia_safe::images::raster_from_data(
        &info,
        skia_safe::Data::new_copy(pixels),
        WIDTH as usize * 4,
    )
    .ok_or("Create validation PNG")?;
    let data = skia_safe::png_encoder::encode_image(None, &image, &Default::default())
        .ok_or("Encode validation PNG")?;
    std::fs::write(path, data.as_bytes()).map_err(|e| e.to_string())
}

fn create_assets(output: &Path) -> Result<(), String> {
    let mut source = vec![0_u8; WIDTH as usize * HEIGHT as usize * 4];
    let mut target = source.clone();
    for y in 8..HEIGHT - 8 {
        for x in 8..WIDTH - 32 {
            let color = if (x / 8 + y / 8) % 2 == 0 {
                [230, 80, 30, 255]
            } else {
                [30, 150, 220, 255]
            };
            let index = ((y * WIDTH + x) * 4) as usize;
            source[index..index + 4].copy_from_slice(&color);
            let index = ((y * WIDTH + x + 16) * 4) as usize;
            target[index..index + 4].copy_from_slice(&color);
        }
    }
    save_png(&output.join("source.png"), &source)?;
    save_png(&output.join("target.png"), &target)?;
    for (name, text) in [
        (
            "source.svg",
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"160\" height=\"96\"><rect x=\"20\" y=\"20\" width=\"120\" height=\"56\" fill=\"#e35732\"/></svg>",
        ),
        (
            "manim.py",
            "from manim import *\nclass IntelValidation(Scene):\n def construct(self):\n  circle = Circle(radius=1.0, color=BLUE, fill_opacity=1)\n  self.add(circle)\n  self.play(circle.animate.shift(RIGHT*2), run_time=0.2, rate_func=linear)\n",
        ),
        (
            "source.obj",
            "v -1 -0.8 0\nv 1 -0.8 0\nv 0 0.8 0\nv 0 0 1\nf 1 2 3\nf 1 4 2\nf 2 4 3\nf 3 4 1\n",
        ),
        (
            "source.ply",
            "ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nproperty float f_dc_0\nproperty float f_dc_1\nproperty float f_dc_2\nproperty float opacity\nproperty float scale_0\nproperty float scale_1\nproperty float scale_2\nproperty float rot_0\nproperty float rot_1\nproperty float rot_2\nproperty float rot_3\nend_header\n-0.4 0 0 1 -1 -1 4 -1.7 -1.7 -1.7 1 0 0 0\n0.4 0 0 -1 1 -1 4 -1.7 -1.7 -1.7 1 0 0 0\n0 0.4 0 -1 -1 1 4 -1.7 -1.7 -1.7 1 0 0 0\n",
        ),
    ] {
        std::fs::write(output.join(name), text).map_err(|e| e.to_string())?;
    }
    Ok(())
}
