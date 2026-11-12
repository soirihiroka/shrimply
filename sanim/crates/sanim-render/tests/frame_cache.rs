use sanim_core::{Animation, Fraction, Property, TimelineDiff};
use sanim_lang::{Engine, MemorySources};
use sanim_render::{CacheLimits, Options, Renderer};
use std::{collections::BTreeMap, sync::Arc};

fn animation(source: &str) -> Animation {
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap()
        .animation
}
fn time(n: u64) -> Fraction {
    Fraction::new(n, 30u64)
}

#[test]
fn holds_reuse_layout_and_recording_at_distinct_exact_times() {
    let mut renderer = Renderer::default();
    let scene = renderer
        .prepare(animation(
            "let c=Circle()\nplay(FadeIn(c),duration=1s)\nwait(3s)",
        ))
        .unwrap();
    let options = Options::for_animation(scene.animation());
    let state = renderer
        .frame_state(&scene, Fraction::from(1), &options)
        .unwrap();
    let later = renderer
        .frame_state(&scene, Fraction::from(3), &options)
        .unwrap();
    assert!(Arc::ptr_eq(&state, &later));
    let recording = renderer
        .record(&scene, Fraction::from(1), &options)
        .unwrap();
    let later = renderer
        .record(&scene, Fraction::new(119u64, 30u64), &options)
        .unwrap();
    assert!(Arc::ptr_eq(&recording, &later));
    assert_eq!(renderer.states_computed, 1);
}

#[test]
fn lru_keeps_recently_used_frames_and_evicts_only_oldest() {
    let mut renderer = Renderer::with_cache_limits(CacheLimits {
        entries: 3,
        ..CacheLimits::default()
    });
    let scene = renderer
        .prepare(animation("let c=Circle()\nplay(FadeIn(c),duration=1s)"))
        .unwrap();
    let options = Options::for_animation(scene.animation());
    let first = renderer.frame_state(&scene, time(0), &options).unwrap();
    let old = renderer.frame_state(&scene, time(1), &options).unwrap();
    renderer.frame_state(&scene, time(2), &options).unwrap();
    assert!(Arc::ptr_eq(
        &first,
        &renderer.frame_state(&scene, time(0), &options).unwrap()
    ));
    renderer.frame_state(&scene, time(3), &options).unwrap();
    assert!(Arc::ptr_eq(
        &first,
        &renderer.frame_state(&scene, time(0), &options).unwrap()
    ));
    assert!(!Arc::ptr_eq(
        &old,
        &renderer.frame_state(&scene, time(1), &options).unwrap()
    ));
    assert_eq!(renderer.states_computed, 5);
}

#[test]
fn independently_recording_at_new_pixel_sizes_reuses_calculated_layout() {
    let mut renderer = Renderer::default();
    let scene = renderer
        .prepare(animation("play(FadeIn(Circle()),duration=1s)"))
        .unwrap();
    let mut options = Options::for_animation(scene.animation());
    renderer.record(&scene, time(5), &options).unwrap();
    options.width /= 2;
    options.height /= 2;
    renderer.record(&scene, time(5), &options).unwrap();
    assert_eq!(renderer.states_computed, 1);
    assert_eq!(renderer.state_cache_hits, 1);
    options.logical_width /= Fraction::from(2);
    renderer.record(&scene, time(5), &options).unwrap();
    assert_eq!(renderer.states_computed, 2);
}

#[test]
fn byte_limits_and_clear_release_cache_ownership_without_invalidating_callers() {
    let mut renderer = Renderer::with_cache_limits(CacheLimits {
        state_bytes: 1,
        ..CacheLimits::default()
    });
    let scene = renderer.prepare(animation("add(Circle())")).unwrap();
    let options = Options::for_animation(scene.animation());
    let a = renderer.frame_state(&scene, time(0), &options).unwrap();
    let b = renderer.frame_state(&scene, time(0), &options).unwrap();
    assert!(!Arc::ptr_eq(&a, &b));
    let recording = renderer.record(&scene, time(0), &options).unwrap();
    renderer.clear_cache();
    assert_eq!(a.elements(), b.elements());
    assert!(!Arc::ptr_eq(
        &recording,
        &renderer.record(&scene, time(0), &options).unwrap()
    ));
}

#[test]
fn hot_edit_reuses_earlier_and_reconverged_states_but_invalidates_changed_motion() {
    let mut renderer = Renderer::default();
    let old = animation(
        "let c=Circle(id=\"c\")\nadd(c)\nwait(1s)\nplay(FadeOut(c),duration=1s)\nplay(FadeIn(c),duration=1s)\nwait(1s)",
    );
    let first = renderer.prepare(old.clone()).unwrap();
    let options = Options::for_animation(first.animation());
    let early = renderer.record(&first, time(10), &options).unwrap();
    let middle = renderer.record(&first, time(45), &options).unwrap();
    let late = renderer.record(&first, time(100), &options).unwrap();
    let mut edited = old.clone();
    edited
        .tracks
        .get_mut("c")
        .unwrap()
        .get_mut(&Property::Opacity)
        .unwrap()[0]
        .easing = sanim_core::Easing::CubicOut;
    let diff = TimelineDiff::between(&old, &edited);
    assert!(!diff.affects(time(10)));
    assert!(diff.affects(time(45)));
    assert!(!diff.affects(time(60)));
    let second = renderer.prepare(edited).unwrap();
    assert!(Arc::ptr_eq(
        &early,
        &renderer.record(&second, time(10), &options).unwrap()
    ));
    assert!(!Arc::ptr_eq(
        &middle,
        &renderer.record(&second, time(45), &options).unwrap()
    ));
    assert!(Arc::ptr_eq(
        &late,
        &renderer.record(&second, time(100), &options).unwrap()
    ));
    let expected = Renderer::default()
        .render(&second, time(45), &options)
        .unwrap();
    assert_eq!(
        renderer.render(&second, time(45), &options).unwrap().rgba,
        expected.rgba
    );
}

#[test]
fn inserting_motion_into_an_old_hold_never_reuses_that_hold_at_changed_times() {
    let mut renderer = Renderer::default();
    let old = animation("let c=Circle(id=\"c\")\nadd(c)\nwait(4s)");
    let first = renderer.prepare(old).unwrap();
    let options = Options::for_animation(first.animation());
    let held = renderer
        .record(&first, Fraction::from(3), &options)
        .unwrap();
    let edited = animation(
        "let c=Circle(id=\"c\")\nadd(c)\nwait(1s)\nplay(FadeOut(c),duration=1s)\nwait(2s)",
    );
    let second = renderer.prepare(edited).unwrap();
    let changed = renderer
        .record(&second, Fraction::from(3), &options)
        .unwrap();
    assert!(!Arc::ptr_eq(&held, &changed));
    assert_ne!(
        held.rasterize(false).unwrap().rgba,
        changed.rasterize(false).unwrap().rgba
    );
    assert!(Arc::ptr_eq(
        &held,
        &renderer.record(&second, time(10), &options).unwrap()
    ));
}

#[test]
fn birth_paint_order_background_and_geometry_changes_invalidate_cached_frames() {
    let base = animation("add(Circle(id=\"a\"))\nwait(1s)\nadd(Rect(id=\"b\"))\nwait(1s)");
    let mut edits = Vec::new();
    let mut birth = base.clone();
    birth.layers[1].birth = Fraction::from(0);
    edits.push(birth);
    let mut order = base.clone();
    order.layers = order.layers.iter().rev().cloned().collect();
    edits.push(order);
    let mut background = base.clone();
    background.background = sanim_core::Color([Fraction::from(1); 4]);
    edits.push(background);
    let mut geometry = base.clone();
    geometry.layers[0].element.geometry = sanim_core::Geometry::Circle {
        radius: Fraction::from(100),
    }
    .into();
    edits.push(geometry);
    for edited in edits {
        let mut renderer = Renderer::default();
        let first = renderer.prepare(base.clone()).unwrap();
        let options = Options::for_animation(first.animation());
        let before = renderer
            .record(&first, Fraction::from(1), &options)
            .unwrap();
        let second = renderer.prepare(edited).unwrap();
        assert!(!Arc::ptr_eq(
            &before,
            &renderer
                .record(&second, Fraction::from(1), &options)
                .unwrap()
        ));
    }
}

#[test]
fn horizontal_ranges_expand_about_their_anchor_without_changing_vertical_extent() {
    let mut renderer = Renderer::default();
    let scene = renderer.prepare(animation("let interval_box=Group(id=\"range\",children=[Rect(width=4pt,height=20pt,position=Vec2(-40pt,0pt)),Rect(width=4pt,height=20pt,position=Vec2(40pt,0pt))])\nplay(ExpandX(interval_box,origin=50%,offset=10pt),duration=1s)")).unwrap();
    let mut options = Options::for_animation(scene.animation());
    options.width = 128;
    options.height = 128;
    options.logical_width = Fraction::from(128);
    options.logical_height = Fraction::from(128);
    let bounds = |frame: &sanim_render::Frame| {
        let pixels: Vec<_> = frame
            .rgba
            .chunks_exact(4)
            .enumerate()
            .filter(|(_, pixel)| pixel[3] > 0)
            .map(|(index, _)| (index % frame.width as usize, index / frame.width as usize))
            .collect();
        (
            pixels.iter().map(|p| p.0).min().unwrap(),
            pixels.iter().map(|p| p.0).max().unwrap(),
            pixels.iter().map(|p| p.1).min().unwrap(),
            pixels.iter().map(|p| p.1).max().unwrap(),
        )
    };
    let empty = renderer
        .render(&scene, Fraction::from(0), &options)
        .unwrap();
    assert!(empty.rgba.chunks_exact(4).all(|pixel| pixel[3] == 0));
    let half = bounds(
        &renderer
            .render(&scene, Fraction::new(1u64, 2u64), &options)
            .unwrap(),
    );
    let full = bounds(
        &renderer
            .render(&scene, Fraction::from(1), &options)
            .unwrap(),
    );
    assert_eq!((half.2, half.3), (full.2, full.3));
    assert_eq!((full.0, full.1), (22, 105));
    assert_eq!((half.0, half.1), (48, 89));
    options.width = 256;
    options.logical_width = Fraction::from(256);
    let resized = bounds(
        &renderer
            .render(&scene, Fraction::new(1u64, 2u64), &options)
            .unwrap(),
    );
    assert_eq!((resized.0, resized.1), (112, 153));
    assert_eq!((resized.2, resized.3), (full.2, full.3));
}

#[test]
fn transparent_and_opaque_exports_cache_distinct_pixels_and_reuse_holds() {
    let mut renderer = Renderer::default();
    let scene = renderer
        .prepare(animation("add(Circle(opacity=0))\nwait(2s)\n"))
        .unwrap();
    let mut options = Options::for_animation(scene.animation());
    options.width = 16;
    options.height = 16;
    options.encode_png = false;
    let transparent = renderer
        .render(&scene, Fraction::from(0), &options)
        .unwrap();
    let opaque = renderer
        .render_opaque(&scene, Fraction::from(0), &options)
        .unwrap();
    assert!(
        transparent
            .rgba
            .chunks_exact(4)
            .all(|pixel| pixel == [0, 0, 0, 0])
    );
    assert!(
        opaque
            .rgba
            .chunks_exact(4)
            .all(|pixel| pixel == [0, 0, 0, 255])
    );
    assert!(Arc::ptr_eq(
        &opaque,
        &renderer
            .render_opaque(&scene, Fraction::from(1), &options)
            .unwrap()
    ));
    assert_eq!(renderer.states_computed, 1);
}

#[test]
fn unused_animated_cells_and_unadded_bindings_do_not_destroy_static_frame_holds() {
    for declarations in [
        "",
        "sig fixed_scale=1\nlet decorated=Circle(id=\"decorated\",scale=fixed_scale)\nadd(decorated)\n",
        "sig ghost_scale=unused+1\nlet ghost=Circle(id=\"ghost\",scale=ghost_scale)\n",
    ] {
        let mut renderer = Renderer::default();
        let scene=renderer.prepare(animation(&format!("mut unused=0\n{declarations}add(Rect(id=\"still\"))\nbackground {{animate(unused,to=1,duration=10s)}}\nwait(10s)"))).unwrap();
        let options = Options::for_animation(scene.animation());
        let first = renderer
            .frame_state(&scene, Fraction::from(0), &options)
            .unwrap();
        let later = renderer
            .frame_state(&scene, Fraction::from(5), &options)
            .unwrap();
        assert!(Arc::ptr_eq(&first, &later), "{declarations}");
        let record = renderer
            .record(&scene, Fraction::from(0), &options)
            .unwrap();
        let later = renderer
            .record(&scene, Fraction::from(5), &options)
            .unwrap();
        assert!(Arc::ptr_eq(&record, &later), "{declarations}");
        assert_eq!(renderer.states_computed, 1);
    }
}
