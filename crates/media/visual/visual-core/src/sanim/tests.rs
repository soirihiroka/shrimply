use super::*;
use std::{path::PathBuf, time::Duration};
struct Fixture(PathBuf);
impl Fixture {
    fn new(text: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "shrimply-sanim-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        let fixture = Self(path);
        fixture.write(text);
        fixture
    }
    fn write(&self, text: &str) {
        std::fs::write(self.0.join("scene.sanim"), text).unwrap();
    }
    fn item(&self) -> VideoItem {
        let canvas = CanvasSize {
            width: 100,
            height: 100,
        };
        let mut item = VideoItem::shape_item(canvas, Time::ZERO, Time { seconds: 10.into() });
        item.content = VideoItemContent::Sanim(Box::default());
        item.file = Asset::new(self.0.join("scene.sanim"));
        item.modifiers.clear();
        item.source_duration = Time { seconds: 1.into() };
        item
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn ready(
    source: &mut Source,
    item: &VideoItem,
    canvas: CanvasSize,
    strict: bool,
) -> Arc<Recording> {
    match source
        .poll(
            item,
            Time::ZERO,
            canvas,
            sanim_core::Fraction::from(30),
            strict,
        )
        .unwrap()
    {
        Status::Ready(frame) => frame,
        _ => panic!("Sanim frame not ready"),
    }
}
fn center(recording: &Recording) -> [u8; 4] {
    let frame = recording.rasterize(false).unwrap();
    frame.rgba[((frame.height / 2 * frame.width + frame.width / 2) * 4) as usize..][..4]
        .try_into()
        .unwrap()
}
fn changed(source: &Source, old: u64) -> sanim_runtime::Snapshot {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut state = source.runtime.snapshot();
    while state.revision <= old
        && state.diagnostic.is_none()
        && std::time::Instant::now() < deadline
    {
        state = source
            .runtime
            .wait_for_change(state.generation, Duration::from_millis(100));
    }
    state
}
#[test]
fn native_sanim_frames_reflow_with_editor_viewport() {
    let f = Fixture::new(
        "bg(Color(\"transparent\"))\nadd(Circle(radius= 2pt,position= Vec2(25%,0%),fill= Color(\"red\")))\nwait(1s)\n",
    );
    let item = f.item();
    let mut source = Source::new(&item).unwrap();
    let small = ready(
        &mut source,
        &item,
        CanvasSize {
            width: 100,
            height: 100,
        },
        true,
    );
    let wide = ready(
        &mut source,
        &item,
        CanvasSize {
            width: 200,
            height: 100,
        },
        true,
    );
    let small = small.rasterize(false).unwrap();
    let wide = wide.rasterize(false).unwrap();
    assert!(small.png.is_empty() && wide.png.is_empty());
    assert_eq!(wide.width, 200);
    assert_eq!(small.rgba[(50 * 100 + 75) * 4 + 3], 255);
    assert_eq!(wide.rgba[(50 * 200 + 150) * 4 + 3], 255);
    assert_eq!(wide.rgba[(50 * 200 + 75) * 4 + 3], 0);
}
#[test]
fn native_sanim_reflected_inputs_are_applied_before_export() {
    let f = Fixture::new(
        "let tint=reflect(Color(\"red\"))\nadd(Circle(radius= 10pt,fill= tint))\nwait(1s)\n",
    );
    let mut item = f.item();
    let VideoItemContent::Sanim(settings) = &mut item.content else {
        unreachable!()
    };
    settings
        .parameters
        .insert("scene.sanim::tint".into(), serde_json::json!("blue"));
    let mut source = Source::new(&item).unwrap();
    let frame = ready(
        &mut source,
        &item,
        CanvasSize {
            width: 100,
            height: 100,
        },
        true,
    );
    assert_eq!(center(&frame), [0, 0, 255, 255]);
}
#[test]
fn native_sanim_export_pins_one_revision_and_preview_can_resume_live() {
    let f = Fixture::new("add(Circle(radius= 10pt,fill= Color(\"red\")))\nwait(1s)\n");
    let item = f.item();
    let mut source = Source::new(&item).unwrap();
    let canvas = CanvasSize {
        width: 100,
        height: 100,
    };
    let first = ready(&mut source, &item, canvas, true);
    let revision = source.runtime.snapshot().revision;
    f.write("add(Circle(radius= 10pt,fill= Color(\"blue\")))\nwait(1s)\n");
    let next = changed(&source, revision);
    assert!(next.revision > revision);
    let pinned = ready(&mut source, &item, canvas, true);
    assert!(Arc::ptr_eq(&first, &pinned));
    let live = ready(&mut source, &item, canvas, false);
    assert_eq!(center(&live), [0, 0, 255, 255]);
}
#[test]
fn native_sanim_failed_preview_keeps_frame_but_new_export_fails() {
    let f = Fixture::new("add(Circle(radius= 10pt,fill= Color(\"red\")))\nwait(1s)\n");
    let item = f.item();
    let mut source = Source::new(&item).unwrap();
    let canvas = CanvasSize {
        width: 100,
        height: 100,
    };
    let first = ready(&mut source, &item, canvas, true);
    let revision = source.runtime.snapshot().revision;
    f.write("let broken=(\n");
    let failed = changed(&source, revision);
    assert!(failed.diagnostic.is_some());
    let preview = ready(&mut source, &item, canvas, false);
    assert!(
        !Arc::ptr_eq(&first, &preview),
        "preview includes a reload diagnostic overlay"
    );
    assert_eq!(center(&first), center(&preview));
    assert!(source.take_error().is_some());
    assert!(
        source
            .poll(
                &item,
                Time::ZERO,
                canvas,
                sanim_core::Fraction::from(30),
                true
            )
            .is_err()
    );
}
#[test]
fn native_sanim_reuses_same_frame_without_encoding_png() {
    let f = Fixture::new("add(Circle())\nwait(1s)\n");
    let item = f.item();
    let mut source = Source::new(&item).unwrap();
    let canvas = CanvasSize {
        width: 100,
        height: 100,
    };
    let a = ready(&mut source, &item, canvas, true);
    let b = ready(&mut source, &item, canvas, true);
    assert!(Arc::ptr_eq(&a, &b));
    assert!(b.rasterize(false).unwrap().png.is_empty());
}

#[test]
fn native_sanim_reflection_observations_do_not_publish_strict_export_sessions() {
    let f = Fixture::new(
        "let color1=reflect(Color(\"red\"),label=\"Display Color\",key=\"theme.accent\")\nadd(Circle(fill=color1))\nwait(1s)",
    );
    let item = f.item();
    let canvas = CanvasSize {
        width: 100,
        height: 100,
    };
    let mut source = Source::with_viewport(
        &item,
        Viewport {
            width: 100,
            height: 100,
            ..Default::default()
        },
    )
    .unwrap();
    ready(&mut source, &item, canvas, true);
    assert!(reflection::observation(item.id, item.file.path()).is_none());
    ready(&mut source, &item, canvas, false);
    let state = reflection::observation(item.id, item.file.path())
        .unwrap()
        .snapshot();
    let prepared = state.prepared.unwrap();
    assert_eq!(prepared.reflected[0].key, "theme.accent");
    assert_eq!(prepared.reflected[0].label, "Display Color");
    assert_eq!(prepared.reflected[0].name, "color1");
    assert_eq!(prepared.reflected[0].source, "scene.sanim");
}

#[test]
fn native_sanim_reflection_patches_validate_aliases_and_distinguish_null_from_reset() {
    let f = Fixture::new(
        "let optional:Option<Number>=reflect(Some(2))\nlet count=reflect(1)\nwait(1s)",
    );
    let mut item = f.item();
    let VideoItemContent::Sanim(settings) = &mut item.content else {
        unreachable!()
    };
    settings
        .parameters
        .insert("scene.sanim::count".into(), serde_json::json!(7));
    let state = reflection::inspect(
        &item,
        Viewport::default(),
        &std::sync::atomic::AtomicBool::new(false),
        reflection::Inspection::Values,
    )
    .unwrap();
    let prepared = state.prepared.unwrap();
    assert!(
        reflection::patch(
            &mut item,
            &prepared,
            &BTreeMap::from([
                ("scene.sanim::optional".into(), serde_json::Value::Null),
                ("scene.sanim::count".into(), serde_json::json!(3))
            ]),
            &[]
        )
        .unwrap()
    );
    let VideoItemContent::Sanim(settings) = &item.content else {
        panic!()
    };
    assert_eq!(
        settings.parameters["scene.sanim::optional"],
        serde_json::Value::Null
    );
    assert!(!settings.parameters.contains_key("optional"));
    let before = settings.parameters.clone();
    assert!(
        !reflection::patch(&mut item, &prepared, &before, &[]).unwrap(),
        "identical overrides must not create undo entries or reload jobs"
    );
    assert!(
        reflection::patch(
            &mut item,
            &prepared,
            &BTreeMap::from([
                ("scene.sanim::count".into(), serde_json::json!(4)),
                ("scene.sanim::optional".into(), serde_json::json!("bad"))
            ]),
            &[]
        )
        .is_err()
    );
    let VideoItemContent::Sanim(settings) = &item.content else {
        panic!()
    };
    assert_eq!(
        before, settings.parameters,
        "invalid batch must not partially update controls"
    );
    let effective = reflection::effective_inputs(&prepared, &settings.parameters).unwrap();
    assert!(matches!(
        effective
            .iter()
            .find(|v| v.name == "optional")
            .unwrap()
            .value,
        reflection::Value::None
    ));
    assert!(
        reflection::patch(
            &mut item,
            &prepared,
            &BTreeMap::new(),
            &["scene.sanim::optional".into()]
        )
        .unwrap()
    );
    let VideoItemContent::Sanim(settings) = &item.content else {
        panic!()
    };
    assert!(!settings.parameters.contains_key("scene.sanim::optional"));
    assert!(
        reflection::patch(
            &mut item,
            &prepared,
            &BTreeMap::new(),
            &["scene.sanim::count".into()]
        )
        .unwrap()
    );
    let VideoItemContent::Sanim(settings) = &item.content else {
        unreachable!()
    };
    let effective = reflection::effective_inputs(&prepared, &settings.parameters).unwrap();
    let count = effective
        .iter()
        .find(|input| input.name == "count")
        .unwrap();
    assert!(
        matches!(count.value,reflection::Value::Number(value) if value.value==1.into()),
        "reset must show the declaration default before runtime re-preparation, not the last applied override"
    );
}

#[test]
fn native_sanim_reflection_discovery_prepares_off_playhead_and_stops_without_ui_join() {
    let f = Fixture::new(
        "let color1=reflect(Color(\"red\"))\nrepeat 100000 {let calculation=1+2}\nwait(1s)",
    );
    let item = f.item();
    let discovery = reflection::Discovery::new(&item, Viewport::default()).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    let observation = loop {
        if let Some(observation) = discovery.observation().unwrap() {
            break observation;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    };
    assert!(observation.snapshot().preparing);
    let started = std::time::Instant::now();
    drop(discovery);
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "inspector teardown joined native preparation"
    );
}

#[test]
#[ignore = "microbenchmark: make sanim-reflection-bench"]
fn native_sanim_reflection_benchmark() {
    use std::hint::black_box;
    let mut script = (0..64)
        .map(|index| format!("let field_{index}=reflect({index})\n"))
        .collect::<String>();
    script.push_str("add(Circle())\nwait(1s)\n");
    let fixture = Fixture::new(&script);
    let mut item = fixture.item();
    let mut source = Source::new(&item).unwrap();
    let canvas = CanvasSize {
        width: 100,
        height: 100,
    };
    ready(&mut source, &item, canvas, true);
    ready(&mut source, &item, canvas, false);
    let observation = reflection::observation(item.id, item.file.path()).unwrap();
    let prepared = observation.snapshot().prepared.unwrap();
    let samples = 100_000;
    let started = std::time::Instant::now();
    for _ in 0..samples {
        black_box(observation.snapshot());
    }
    println!(
        "reflection observation snapshot: {:.1} ns/op ({samples} iterations)",
        started.elapsed().as_nanos() as f64 / f64::from(samples)
    );
    let started = std::time::Instant::now();
    for _ in 0..samples {
        black_box(reflection::observation(item.id, item.file.path()).unwrap());
    }
    println!(
        "reflection published-source lookup: {:.1} ns/op ({samples} iterations)",
        started.elapsed().as_nanos() as f64 / f64::from(samples)
    );
    let samples = 1_000;
    let started = std::time::Instant::now();
    for index in 0..samples {
        black_box(
            reflection::set(
                &mut item,
                &prepared,
                "field_32",
                Some(serde_json::json!(index)),
            )
            .unwrap(),
        );
    }
    println!(
        "reflection typed single-input patch / 64 inputs: {:.1} us/op ({samples} iterations)",
        started.elapsed().as_micros() as f64 / f64::from(samples)
    );
    let VideoItemContent::Sanim(settings) = &item.content else {
        unreachable!()
    };
    let started = std::time::Instant::now();
    for _ in 0..samples {
        black_box(reflection::effective_inputs(&prepared, &settings.parameters).unwrap());
    }
    println!(
        "reflection effective 64-input schema: {:.1} us/op ({samples} iterations)",
        started.elapsed().as_micros() as f64 / f64::from(samples)
    );
}

#[test]
fn native_sanim_reflection_inspection_rejects_stale_published_schema() {
    let fixture = Fixture::new("let first=reflect(1)\nwait(1s)");
    let item = fixture.item();
    let mut source = Source::new(&item).unwrap();
    let canvas = CanvasSize {
        width: 100,
        height: 100,
    };
    ready(&mut source, &item, canvas, true);
    ready(&mut source, &item, canvas, false);
    fixture.write("let second=reflect(2)\nwait(1s)");
    let state = reflection::inspect(
        &item,
        Viewport::default(),
        &std::sync::atomic::AtomicBool::new(false),
        reflection::Inspection::Values,
    )
    .unwrap();
    let prepared = state.prepared.expect("fresh reflected metadata");
    assert_eq!(prepared.reflected[0].name, "second");
    assert!(prepared.reflected.iter().all(|input| input.name != "first"));
}

#[test]
fn native_sanim_reflection_cold_invalid_overrides_can_be_reset() {
    let fixture = Fixture::new("let count=reflect(1)\nwait(1s)");
    let mut item = fixture.item();
    let VideoItemContent::Sanim(settings) = &mut item.content else {
        unreachable!()
    };
    settings.parameters.insert(
        "scene.sanim::count".into(),
        serde_json::json!("invalid count"),
    );
    assert!(reflection::observation(item.id, item.file.path()).is_none());
    let state = reflection::inspect(
        &item,
        Viewport::default(),
        &std::sync::atomic::AtomicBool::new(false),
        reflection::Inspection::Schema,
    )
    .unwrap();
    assert_eq!(
        state
            .diagnostic
            .as_ref()
            .expect("retain invalid override diagnostic")
            .code,
        "S302"
    );
    let prepared = state
        .prepared
        .expect("default schema must remain available for typed repair");
    assert_eq!(prepared.reflected[0].name, "count");
    assert!(
        reflection::patch(
            &mut item,
            &prepared,
            &BTreeMap::new(),
            &["scene.sanim::count".into()]
        )
        .unwrap()
    );
    let VideoItemContent::Sanim(settings) = &item.content else {
        unreachable!()
    };
    assert!(settings.parameters.is_empty());
    let state = reflection::inspect(
        &item,
        Viewport::default(),
        &std::sync::atomic::AtomicBool::new(false),
        reflection::Inspection::Values,
    )
    .unwrap();
    assert!(state.diagnostic.is_none());
}

#[test]
fn native_sanim_reflection_sets_reuse_source_schema_but_gets_refresh_dependent_defaults() {
    let fixture = Fixture::new("let count=reflect(1)\nlet derived=reflect(count+1)\nwait(1s)");
    let mut item = fixture.item();
    let mut source = Source::new(&item).unwrap();
    let canvas = CanvasSize {
        width: 100,
        height: 100,
    };
    ready(&mut source, &item, canvas, true);
    ready(&mut source, &item, canvas, false);
    let previous = reflection::observation(item.id, item.file.path())
        .unwrap()
        .snapshot()
        .prepared
        .unwrap();
    let VideoItemContent::Sanim(settings) = &mut item.content else {
        unreachable!()
    };
    settings
        .parameters
        .insert("scene.sanim::count".into(), serde_json::json!(9));
    let viewport = Viewport {
        width: 100,
        height: 100,
        ..Default::default()
    };
    let canceled = std::sync::atomic::AtomicBool::new(false);
    let schema = reflection::inspect(
        &item,
        viewport.clone(),
        &canceled,
        reflection::Inspection::Schema,
    )
    .unwrap()
    .prepared
    .unwrap();
    assert!(
        Arc::ptr_eq(&previous, &schema),
        "rapid sets must reuse verified source schema without cold preparation"
    );
    let values = reflection::inspect(&item, viewport, &canceled, reflection::Inspection::Values)
        .unwrap()
        .prepared
        .unwrap();
    let derived = values
        .reflected
        .iter()
        .find(|input| input.name == "derived")
        .unwrap();
    assert!(
        matches!(derived.default,reflection::Value::Number(value) if value.value==10.into()),
        "get must compute dependent defaults for current input values"
    );
}
