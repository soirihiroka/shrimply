use sanim_core::Fraction;
use sanim_lang::{Engine, MemorySources};
use sanim_render::{Options, Renderer, Scene};
use std::collections::BTreeMap;

fn scene(source: &str) -> Scene {
    scene_in(source, 1920, 1080)
}
fn scene_in(source: &str, width: u32, height: u32) -> Scene {
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    Scene::new(
        Engine::new(sanim_core::Viewport {
            width,
            height,
            dpi: 72,
            ..sanim_core::Viewport::default()
        })
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap()
        .animation,
    )
    .unwrap()
}
#[test]
fn renders_actual_circle_pixels_and_png() {
    let scene = scene_in("add(Circle(radius: 10pt, fill: Color(\"red\")))\n", 64, 64);
    let frame = Renderer::default()
        .render(
            &scene,
            Fraction::from(0),
            &Options::for_animation(scene.animation()),
        )
        .unwrap();
    assert_eq!(&frame.png[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(
        &frame.rgba[(32 * 64 + 32) * 4..(32 * 64 + 32) * 4 + 4],
        &[255, 0, 0, 255]
    );
    assert_eq!(frame.rgba[3], 0);
}
#[test]
fn output_resizing_preserves_relative_position() {
    let scene = scene_in(
        "add(Circle(radius: 5pt, fill: Color(\"blue\"), position: Vec2(-25%, 0%)))\n",
        64,
        64,
    );
    let mut options = Options::for_animation(scene.animation());
    options.width = 128;
    options.height = 128;
    let frame = Renderer::default()
        .render(&scene, Fraction::from(0), &options)
        .unwrap();
    assert_eq!(
        &frame.rgba[(64 * 128 + 32) * 4..(64 * 128 + 32) * 4 + 4],
        &[0, 0, 255, 255]
    );
}
#[test]
fn same_frame_reuses_cache() {
    let scene = scene("add(Rect())\n");
    let mut renderer = Renderer::default();
    let options = Options::for_animation(scene.animation());
    let first = renderer
        .render(&scene, Fraction::from(0), &options)
        .unwrap();
    let second = renderer
        .render(&scene, Fraction::from(0), &options)
        .unwrap();
    assert!(std::sync::Arc::ptr_eq(&first, &second));
    assert_eq!(renderer.cache_hits, 1);
}

#[test]
fn render_work_limits_apply_to_compositing() {
    let scene = scene_in("add(Group(children: [Circle()], opacity: 0.5))\n", 64, 64);
    let mut options = Options::for_animation(scene.animation());
    options.max_work = 10;
    assert!(
        Renderer::default()
            .render(&scene, Fraction::from(0), &options)
            .unwrap_err()
            .contains("work budget")
    );
}
#[test]
fn cached_frames_respect_changed_work_limits() {
    let scene = scene_in(
        "add(Group(children: [Circle(radius: 2pt)], opacity: 0.5))\n",
        16,
        16,
    );
    let mut options = Options::for_animation(scene.animation());
    let mut renderer = Renderer::default();
    renderer
        .render(&scene, Fraction::from(0), &options)
        .unwrap();
    options.max_work = 1;
    assert!(
        renderer
            .render(&scene, Fraction::from(0), &options)
            .is_err()
    );
}
#[test]
fn group_opacity_applies_after_children_composite() {
    let scene = scene_in(
        "let a = Rect(width: 16pt, height: 16pt, fill: Color(\"red\"))\nlet b = Rect(width: 16pt, height: 16pt, fill: Color(\"blue\"))\nadd(Group(children: [a,b], opacity: 0.5))\n",
        16,
        16,
    );
    let frame = Renderer::default()
        .render(
            &scene,
            Fraction::from(0),
            &Options::for_animation(scene.animation()),
        )
        .unwrap();
    let pixel = &frame.rgba[(8 * 16 + 8) * 4..(8 * 16 + 8) * 4 + 4];
    assert_eq!(pixel[0], 0);
    assert_eq!(pixel[2], 255);
    assert!((127..=128).contains(&pixel[3]));
}
#[test]
fn pixel_budget_rejects_large_surfaces() {
    let scene = scene("add(Circle())\n");
    let mut options = Options::for_animation(scene.animation());
    options.max_pixels = 1;
    assert!(
        Renderer::default()
            .render(&scene, Fraction::from(0), &options)
            .unwrap_err()
            .contains("limit")
    );
}
#[test]
fn later_motion_frame_changes_pixels() {
    let scene = scene_in(
        "let marker = Circle(radius: 4pt, position: Vec2(-25%, 0%))\nplay(MoveTo(marker, Vec2(25%, 0%)), duration: 1s)\n",
        64,
        64,
    );
    let mut renderer = Renderer::default();
    let options = Options::for_animation(scene.animation());
    let first = renderer
        .render(&scene, Fraction::from(0), &options)
        .unwrap();
    let later = renderer
        .render(&scene, Fraction::from(1), &options)
        .unwrap();
    assert_ne!(first.rgba, later.rgba);
}

#[test]
fn skia_recording_clips_overflow_when_replayed_after_a_host_transform() {
    let scene = scene_in(
        "add(Rect(width:100pt,height:100pt,fill:Color(\"red\")))",
        16,
        16,
    );
    let recording = Renderer::default()
        .record(
            &scene,
            Fraction::from(0),
            &Options::for_animation(scene.animation()),
        )
        .unwrap();
    let mut surface = skia_safe::surfaces::raster_n32_premul((48, 48)).unwrap();
    surface.canvas().clear(skia_safe::Color::TRANSPARENT);
    surface.canvas().translate((16., 16.));
    recording.draw(surface.canvas());
    let pixels = surface.peek_pixels().unwrap();
    assert_eq!(pixels.get_color((4, 4)), skia_safe::Color::TRANSPARENT);
    assert_eq!(pixels.get_color((20, 20)), skia_safe::Color::RED);
}
#[test]
fn pango_shaped_outlines_render_without_retaining_font_objects() {
    let mut files = MemorySources::default();
    files.insert(
        "font.ttf",
        include_bytes!("../../../tests/fixtures/DejaVuSansCondensed-subset.ttf").as_slice(),
    );
    files.insert("main.sanim","let font=read<Font>(\"font.ttf\")\nadd(Text(\"AV ffi السلام\",font:font,font_size:24pt,fill:Color(\"white\")))");
    let prepared = Engine::new(sanim_core::Viewport {
        width: 240,
        height: 96,
        dpi: 72,
        ..sanim_core::Viewport::default()
    })
    .prepare("main.sanim", &files, &BTreeMap::new())
    .unwrap();
    let sanim_core::Geometry::Text { outline, .. } = &prepared.animation.layers[0].element.geometry
    else {
        panic!("text geometry")
    };
    assert!(!outline.is_empty());
    let scene = Scene::new(prepared.animation).unwrap();
    let frame = Renderer::default()
        .render(
            &scene,
            Fraction::from(0),
            &Options::for_animation(scene.animation()),
        )
        .unwrap();
    assert!(frame.rgba.chunks_exact(4).any(|p| p[3] > 0));
}
