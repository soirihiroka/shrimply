use super::*;
use crate::CacheLimits;
use sanim_core::{Geometry, Viewport};
use sanim_lang::{Engine, MemorySources};

fn scene(renderer: &mut Renderer, source: &str) -> Scene {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", source);
    let animation = Engine::new(Viewport {
        width: 240,
        height: 160,
        dpi: 72,
        ..Default::default()
    })
    .prepare("main.sanim", &sources, &Default::default())
    .unwrap()
    .animation;
    renderer.prepare(animation).unwrap()
}

const MOVING_FOREGROUND: &str = r#"
let background=<Box w-140 h-100 bg-blue3><Label>Background</Label></Box>
add(background)
let marker=<Box w-20 h-20 bg-orange3/>
marker.move_to([120pt, 80pt],duration=4s,ease="linear")
"#;

#[test]
fn held_background_reuses_its_state_and_picture_on_new_foreground_frames() {
    let mut renderer = Renderer::default();
    let scene = scene(&mut renderer, MOVING_FOREGROUND);
    let options = Options::for_animation(scene.animation());
    renderer.record(&scene, 0.into(), &options).unwrap();
    let initial = renderer.frame_state(&scene, 0.into(), &options).unwrap();
    let paths = renderer.geometries_prepared;
    let recorded = renderer.layers_recorded;
    let mut previous = None;
    for frame in 1..20u64 {
        let time = Fraction::new(frame, 10u64);
        let picture = renderer.record(&scene, time, &options).unwrap();
        if let Some(previous) = previous.replace(picture.clone()) {
            assert!(
                !Arc::ptr_eq(&previous, &picture),
                "these are genuinely new frames"
            );
        }
        let current = renderer.frame_state(&scene, time, &options).unwrap();
        assert!(Arc::ptr_eq(
            &initial.elements[0].geometry,
            &current.elements[0].geometry
        ));
        assert_ne!(initial.elements[1].position, current.elements[1].position);
    }
    assert_eq!(renderer.layers_recorded, recorded);
    assert_eq!(renderer.layer_recording_hits, 19);
    assert_eq!(renderer.layer_state_hits, 19);
    assert_eq!(renderer.layers_computed, 21);
    assert_eq!(renderer.geometries_prepared, paths);
}

#[test]
fn retained_writes_viewport_changes_and_hot_edits_do_not_replay_stale_backgrounds() {
    let source = r#"
mut tint=Color("blue3")
let background=<Box w-140 h-100 fill={tint}/>
add(background)
spawn {
wait(1s)
tint=Color("green3")
}
let marker=<Box w-20 h-20 bg-orange3/>
marker.move_to([120pt, 80pt],duration=4s,ease="linear")
"#;
    let mut renderer = Renderer::default();
    let scene = scene(&mut renderer, source);
    let mut options = Options::for_animation(scene.animation());
    options.encode_png = false;
    for time in [
        Fraction::from(0),
        Fraction::new(1u64, 2u64),
        Fraction::new(3u64, 2u64),
    ] {
        let actual = renderer.render(&scene, time, &options).unwrap();
        let mut fresh = Renderer::default();
        assert_eq!(
            actual.rgba,
            fresh.render(&scene, time, &options).unwrap().rgba
        );
    }
    options.logical_width *= Fraction::from(2);
    let actual = renderer.render(&scene, 2.into(), &options).unwrap();
    assert_eq!(
        actual.rgba,
        Renderer::default()
            .render(&scene, 2.into(), &options)
            .unwrap()
            .rgba
    );
    let scene = self::scene(&mut renderer, &source.replace("green3", "purple3"));
    let actual = renderer.render(&scene, 2.into(), &options).unwrap();
    assert_eq!(
        actual.rgba,
        Renderer::default()
            .render(&scene, 2.into(), &options)
            .unwrap()
            .rgba
    );
}

#[test]
fn cached_pictures_preserve_work_limits_and_nested_memory_accounting() {
    let limits = CacheLimits {
        entries: 8,
        state_bytes: 200_000,
        recording_bytes: 50_000,
        raster_bytes: 0,
    };
    let mut renderer = Renderer::with_cache_limits(limits);
    let scene = scene(&mut renderer, MOVING_FOREGROUND);
    let mut options = Options::for_animation(scene.animation());
    let first = renderer.record(&scene, 0.into(), &options).unwrap();
    assert!(
        first.nested_bytes > 0,
        "parent pictures must charge retained child commands"
    );
    for frame in 1..80u64 {
        renderer
            .record(&scene, Fraction::new(frame, 20u64), &options)
            .unwrap();
        assert!(renderer.states.usage().0 <= limits.state_bytes);
        assert!(renderer.recordings.usage().0 <= limits.recording_bytes);
        assert!(renderer.states.usage().1 <= limits.entries);
        assert!(renderer.recordings.usage().1 <= limits.entries);
    }
    options.max_work = 1;
    assert!(
        renderer
            .record(&scene, 1.into(), &options)
            .err()
            .unwrap()
            .contains("budget")
    );
}

#[test]
fn exceeding_path_capacity_keeps_the_admitted_working_set_hot() {
    let source = format!(
        "let items=[{}]\nfor item in items {{ add(item) }}\nwait(1s)",
        (1..=draw::MAX_GEOMETRY_CACHE_ENTRIES + 1)
            .map(|size| format!("Box(width={size}pt,height=1pt)"))
            .collect::<Vec<_>>()
            .join(",")
    );
    let mut renderer = Renderer::default();
    let scene = scene(&mut renderer, &source);
    let mut elements = scene.animation.sample(0.into()).unwrap();
    for element in &mut elements {
        assert!(matches!(element.geometry.as_ref(), Geometry::Box { .. }));
        draw::prepared_path(element, &mut renderer).unwrap();
    }
    let built = renderer.geometries_prepared;
    for element in &elements {
        draw::prepared_path(element, &mut renderer).unwrap();
    }
    assert_eq!(renderer.geometries_prepared - built, 1);
}
