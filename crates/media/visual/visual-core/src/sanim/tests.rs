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
        "add(Circle(radius: 2pt,position: Vec2(25%,0%),fill: Color(\"red\")))\nwait(1s)\n",
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
        "reflect tint=Color(\"red\")\nadd(Circle(radius: 10pt,fill: tint))\nwait(1s)\n",
    );
    let mut item = f.item();
    let VideoItemContent::Sanim(settings) = &mut item.content else {
        unreachable!()
    };
    settings
        .parameters
        .insert("tint".into(), serde_json::json!("blue"));
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
    let f = Fixture::new("add(Circle(radius: 10pt,fill: Color(\"red\")))\nwait(1s)\n");
    let item = f.item();
    let mut source = Source::new(&item).unwrap();
    let canvas = CanvasSize {
        width: 100,
        height: 100,
    };
    let first = ready(&mut source, &item, canvas, true);
    let revision = source.runtime.snapshot().revision;
    f.write("add(Circle(radius: 10pt,fill: Color(\"blue\")))\nwait(1s)\n");
    let next = changed(&source, revision);
    assert!(next.revision > revision);
    let pinned = ready(&mut source, &item, canvas, true);
    assert!(Arc::ptr_eq(&first, &pinned));
    let live = ready(&mut source, &item, canvas, false);
    assert_eq!(center(&live), [0, 0, 255, 255]);
}
#[test]
fn native_sanim_failed_preview_keeps_frame_but_new_export_fails() {
    let f = Fixture::new("add(Circle(radius: 10pt,fill: Color(\"red\")))\nwait(1s)\n");
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
    assert!(Arc::ptr_eq(&first, &preview));
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
