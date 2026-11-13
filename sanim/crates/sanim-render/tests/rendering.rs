use sanim_core::Fraction;
use sanim_lang::{Engine, MemorySources};
use sanim_render::{Options, Renderer, Scene};
use std::collections::BTreeMap;

fn scene(source: &str) -> Scene {
    scene_in(source, 1920, 1080)
}
fn scene_in(source: &str, width: u32, height: u32) -> Scene {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        format!("bg(Color(\"transparent\"))\n{source}"),
    );
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
    assert_eq!(
        sanim_core::Animation::default().background,
        sanim_core::Color::parse("window_bg_dark").unwrap()
    );
    let scene = scene_in("add(Circle(radius= 10pt, fill= Color(\"red\")))\n", 64, 64);
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
        "add(Circle(radius= 5pt, fill= Color(\"blue\"), position= Vec2(-25%, 0%)))\n",
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
    let scene = scene_in("add(Group(children= [Circle()], opacity= 0.5))\n", 64, 64);
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
        "add(Group(children= [Circle(radius= 2pt)], opacity= 0.5))\n",
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
        "let a = Rect(width= 16pt, height= 16pt, fill= Color(\"red\"))\nlet b = Rect(width= 16pt, height= 16pt, fill= Color(\"blue\"))\nadd(Group(children= [a,b], opacity= 0.5))\n",
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
        "let marker = Circle(radius= 4pt, position= Vec2(-25%, 0%))\nMoveTo(marker, Vec2(25%, 0%), duration=1s, ease=\"linear\")\n",
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
        "add(Rect(width=100pt,height=100pt,fill=Color(\"red\")))",
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
        include_bytes!("../../../examples/assets/DejaVuSansCondensed-subset.ttf").as_slice(),
    );
    files.insert("main.sanim","bg(Color(\"transparent\"))\nlet font=read<Font>(\"font.ttf\")\nlet label=Text(\"AV ffi السلام\",font=font,font_size=24pt,fill=Color(\"white\"))\nWrite(label, duration=1s, ease=\"linear\")");
    let prepared = Engine::new(sanim_core::Viewport {
        width: 240,
        height: 96,
        dpi: 72,
        ..sanim_core::Viewport::default()
    })
    .prepare("main.sanim", &files, &BTreeMap::new())
    .unwrap();
    let sanim_core::Geometry::Text { outline, .. } =
        prepared.animation.layers[0].element.geometry.as_ref()
    else {
        panic!("text geometry")
    };
    assert!(!outline.is_empty());
    let scene = Scene::new(prepared.animation).unwrap();
    let mut renderer = Renderer::default();
    let options = Options::for_animation(scene.animation());
    let start = renderer
        .render(&scene, Fraction::from(0), &options)
        .unwrap();
    let trace = renderer
        .render(&scene, Fraction::new(1u64, 4u64), &options)
        .unwrap();
    let frame = renderer
        .render(&scene, Fraction::from(1), &options)
        .unwrap();
    assert!(start.rgba.chunks_exact(4).all(|p| p[3] == 0));
    assert!(trace.rgba.chunks_exact(4).any(|p| p[3] > 0));
    assert!(frame.rgba.chunks_exact(4).any(|p| p[3] > 0));
    assert_ne!(trace.rgba, frame.rgba);
}

#[test]
fn create_draws_a_line_segment_instead_of_fading_the_whole_line() {
    let scene = scene_in(
        "let line=Line(end=Vec2(40pt,0pt),width=4pt,fill=Color(\"red\"))\nCreate(line, duration=1s, ease=\"linear\")",
        64,
        64,
    );
    let mut renderer = Renderer::default();
    let options = Options::for_animation(scene.animation());
    let start = renderer
        .render(&scene, Fraction::from(0), &options)
        .unwrap();
    assert!(start.rgba.chunks_exact(4).all(|p| p[3] == 0));
    let middle = renderer
        .render(&scene, Fraction::new(1u64, 2u64), &options)
        .unwrap();
    assert_eq!(
        &middle.rgba[(32 * 64 + 16) * 4..(32 * 64 + 16) * 4 + 4],
        &[255, 0, 0, 255]
    );
    assert_eq!(middle.rgba[(32 * 64 + 48) * 4 + 3], 0);
    let end = renderer
        .render(&scene, Fraction::from(1), &options)
        .unwrap();
    assert_eq!(end.rgba[(32 * 64 + 48) * 4 + 3], 255);
}

#[test]
fn write_traces_then_fills_and_inverse_returns_to_empty() {
    let mut renderer = Renderer::default();
    for (forward, reverse) in [("Create", "Uncreate"), ("Write", "Unwrite")] {
        let scene = scene_in(
            &format!(
                "let shape=Rect(width=32pt,height=32pt,fill=Color(\"red\"))\n{forward}(shape,duration=1s,ease=\"linear\")\n{reverse}(shape,duration=1s,ease=\"linear\")"
            ),
            64,
            64,
        );
        let options = Options::for_animation(scene.animation());
        let frame =
            |renderer: &mut Renderer, time| renderer.render(&scene, time, &options).unwrap();
        let first = frame(&mut renderer, Fraction::from(0));
        let quarter = frame(&mut renderer, Fraction::new(1u64, 4u64));
        assert!(quarter.rgba.chunks_exact(4).any(|p| p[3] > 0));
        assert!(
            quarter.rgba.chunks_exact(4).any(|p| p[3] >= 127),
            "a reveal must expose solid pixels, not fade everything"
        );
        if forward == "Write" {
            assert_eq!(
                quarter.rgba[(32 * 64 + 32) * 4 + 3],
                0,
                "Write leaves the interior empty during its tracing phase"
            );
            let filling = frame(&mut renderer, Fraction::new(3u64, 4u64));
            assert!((127..=128).contains(&filling.rgba[(32 * 64 + 32) * 4 + 3]));
        }
        let whole = frame(&mut renderer, Fraction::from(1));
        assert_eq!(
            &whole.rgba[(32 * 64 + 32) * 4..(32 * 64 + 32) * 4 + 4],
            &[255, 0, 0, 255]
        );
        assert_eq!(
            quarter.rgba,
            frame(&mut renderer, Fraction::new(7u64, 4u64)).rgba
        );
        assert_eq!(first.rgba, frame(&mut renderer, Fraction::from(2)).rgba);
    }
}

#[test]
fn group_write_reveals_children_without_changing_their_layout() {
    let scene = scene_in(
        "let group=Row(children=[Rect(width=16pt,height=16pt),Circle(radius=8pt)],gap=8pt)\nWrite(group, duration=1s, ease=\"linear\")",
        64,
        64,
    );
    let options = Options::for_animation(scene.animation());
    let mut renderer = Renderer::default();
    let start = renderer
        .render(&scene, Fraction::from(0), &options)
        .unwrap();
    let middle = renderer
        .render(&scene, Fraction::new(1u64, 4u64), &options)
        .unwrap();
    let end = renderer
        .render(&scene, Fraction::from(1), &options)
        .unwrap();
    assert!(start.rgba.chunks_exact(4).all(|p| p[3] == 0));
    assert!(middle.rgba.chunks_exact(4).any(|p| p[3] > 0));
    assert!(
        end.rgba.chunks_exact(4).filter(|p| p[3] > 0).count()
            > middle.rgba.chunks_exact(4).filter(|p| p[3] > 0).count()
    );
}

#[test]
fn grow_and_shrink_render_centered_without_zero_scale_artifacts() {
    let scene = scene_in(
        "let shape=Rect(width=32pt,height=32pt)\nGrowFromCenter(shape, duration=1s, ease=\"linear\")\nShrinkToCenter(shape, duration=1s, ease=\"linear\")",
        64,
        64,
    );
    let options = Options::for_animation(scene.animation());
    let mut renderer = Renderer::default();
    let start = renderer
        .render(&scene, Fraction::from(0), &options)
        .unwrap();
    let middle = renderer
        .render(&scene, Fraction::new(1u64, 2u64), &options)
        .unwrap();
    let end = renderer
        .render(&scene, Fraction::from(2), &options)
        .unwrap();
    assert!(start.rgba.chunks_exact(4).all(|p| p[3] == 0));
    assert_eq!(middle.rgba[(32 * 64 + 32) * 4 + 3], 255);
    assert_eq!(middle.rgba[(32 * 64 + 20) * 4 + 3], 0);
    assert_eq!(start.rgba, end.rgba);
}

#[test]
fn style_timing_and_viewport_hot_edits_reuse_paths_but_geometry_edits_do_not() {
    let original = scene_in("add(Rect(width=32pt,height=24pt))", 64, 64);
    let mut renderer = Renderer::default();
    let animation = original.animation().clone();
    let first = renderer.prepare(animation.clone()).unwrap();
    assert_eq!(renderer.geometry_cache_hits, 0);
    let mut edited = animation.clone();
    edited.width = 128;
    edited.layers[0].element.fill = sanim_core::Color([
        Fraction::from(1),
        Fraction::from(0),
        Fraction::from(0),
        Fraction::from(1),
    ])
    .into();
    edited.duration = Fraction::from(2);
    let second = renderer.prepare(edited).unwrap();
    assert_eq!(renderer.geometry_cache_hits, 0);
    assert_eq!(renderer.geometries_prepared, 0);
    let a = renderer
        .render(
            &first,
            Fraction::from(0),
            &Options::for_animation(first.animation()),
        )
        .unwrap();
    let b = renderer
        .render(
            &second,
            Fraction::from(0),
            &Options::for_animation(second.animation()),
        )
        .unwrap();
    assert_ne!(a.rgba, b.rgba);
    assert_eq!(renderer.geometries_prepared, 1);
    assert_eq!(renderer.geometry_cache_hits, 1);
    let mut geometry_edit = animation;
    geometry_edit.layers[0]
        .element
        .set(
            sanim_core::Property::Width,
            sanim_core::Value::Number(sanim_core::Number {
                value: Fraction::from(16),
                unit: sanim_core::Unit::Length,
            }),
        )
        .unwrap();
    let changed = renderer.prepare(geometry_edit).unwrap();
    assert_eq!(renderer.geometries_prepared, 1);
    renderer
        .record(
            &changed,
            0.into(),
            &Options::for_animation(changed.animation()),
        )
        .unwrap();
    assert_eq!(renderer.geometries_prepared, 2);
    assert_eq!(renderer.geometry_cache_hits, 1);
    renderer.clear_cache();
    renderer.prepare(first.animation().clone()).unwrap();
    assert_eq!(renderer.geometry_cache_hits, 1);
}

#[test]
fn full_geometry_validation_rejects_nonfinite_path_measurements() {
    let mut animation = scene_in("add(Line(end=Vec2(1pt,1pt)))", 64, 64)
        .animation()
        .clone();
    animation.layers[0].element.geometry = sanim_core::Geometry::Line {
        end: sanim_core::Vec2 {
            values: [3e38, 3e38],
            unit: sanim_core::Unit::Length,
        },
        width: Fraction::from(1),
    }
    .into();
    let scene = Scene::new(animation).unwrap();
    let error = Renderer::default().validate_geometry(&scene).unwrap_err();
    assert!(error.contains("path contour length"), "{error}");
}

#[test]
fn overlapping_trace_and_fill_preserve_element_opacity() {
    for kind in ["Create", "Write"] {
        let scene = scene_in(
            &format!(
                "let shape=Rect(width=32pt,height=32pt,opacity=0.5)\n{kind}(shape,duration=1s,ease=\"linear\")"
            ),
            64,
            64,
        );
        let frame = Renderer::default()
            .render(
                &scene,
                Fraction::new(3u64, 4u64),
                &Options::for_animation(scene.animation()),
            )
            .unwrap();
        let max = frame.rgba.chunks_exact(4).map(|p| p[3]).max().unwrap();
        assert!(
            max <= 128,
            "{kind}: trace and fill accumulated opacity to {max}"
        );
        assert!(max > 0);
    }
}

#[test]
fn cached_multicontour_reveals_preserve_gaps_and_reverse_order() {
    let mut path = skia_safe::PathBuilder::new();
    path.move_to((0., 0.))
        .line_to((10., 0.))
        .move_to((100., 0.))
        .line_to((130., 0.));
    let path = shrimply_drawing_skia::path::PreparedPath::new(path.detach()).unwrap();
    let forward = path.partial(0.5, false);
    assert_eq!(forward.bounds().right, 110.0);
    let reverse = path.partial(0.5, true);
    assert_eq!(reverse.bounds().left, 110.0);
    assert_eq!(reverse.bounds().right, 130.0);
    let mut measure = skia_safe::PathMeasure::new(&forward, false, None);
    assert_eq!(measure.length(), 10.0);
    assert!(measure.next_contour());
    assert_eq!(measure.length(), 10.0);
    assert!(!measure.next_contour());
}

#[test]
fn retained_dimension_samples_draw_their_current_paths_and_allow_reverse_seeks() {
    let scene = scene_in(
        "mut extent=10pt\nsig breadth=extent\nadd(Rect(id=\"box\",width=breadth,height=10pt,fill=Color(\"red\")))\nbackground {animate(extent,to=20pt,duration=2s)}\nwait(2s)",
        64,
        64,
    );
    let mut renderer = Renderer::default();
    let options = Options::for_animation(scene.animation());
    let expanded = renderer
        .render(&scene, Fraction::from(2), &options)
        .unwrap();
    let original = renderer
        .render(&scene, Fraction::from(0), &options)
        .unwrap();
    let pixel = (32 * 64 + 40) * 4;
    assert_eq!(&expanded.rgba[pixel..pixel + 4], &[255, 0, 0, 255]);
    assert_eq!(original.rgba[pixel + 3], 0);
    let repeated = renderer
        .render(&scene, Fraction::from(2), &options)
        .unwrap();
    assert!(std::sync::Arc::ptr_eq(&expanded, &repeated));
}

#[test]
fn animated_rectangle_corner_radius_changes_real_corner_pixels_and_reverse_seeks() {
    let scene = scene_in(
        "mut corner=0pt\nsig rounding=corner\nadd(Rect(id=\"box\",width=20pt,height=20pt,radius=rounding,fill=Color(\"red\")))\nbackground {animate(corner,to=10pt,duration=2s)}\nwait(2s)",
        64,
        64,
    );
    let mut renderer = Renderer::default();
    let options = Options::for_animation(scene.animation());
    let rounded = renderer
        .render(&scene, Fraction::from(2), &options)
        .unwrap();
    let sharp = renderer
        .render(&scene, Fraction::from(0), &options)
        .unwrap();
    let corner = (23 * 64 + 23) * 4;
    let center = (32 * 64 + 32) * 4;
    assert_eq!(&sharp.rgba[corner..corner + 4], &[255, 0, 0, 255]);
    assert_eq!(rounded.rgba[corner + 3], 0);
    assert_eq!(&rounded.rgba[center..center + 4], &[255, 0, 0, 255]);
    assert!(std::sync::Arc::ptr_eq(
        &rounded,
        &renderer
            .render(&scene, Fraction::from(2), &options)
            .unwrap()
    ));
}
