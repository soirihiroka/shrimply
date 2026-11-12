use sanim_runtime::{Runtime, Snapshot};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{Duration, Instant},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "sanim-runtime-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn write(&self, name: &str, text: &str) {
        std::fs::write(self.0.join(name), text).unwrap();
    }
    fn open(&self) -> Runtime {
        Runtime::open(self.0.join("main.sanim")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn until(runtime: &Runtime, predicate: impl Fn(&Snapshot) -> bool) -> Snapshot {
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut state = runtime.snapshot();
    while !predicate(&state) && Instant::now() < deadline {
        state = runtime.wait_for_change(state.generation, Duration::from_millis(100));
    }
    assert!(
        predicate(&state),
        "generation={}, error={:?}, preparing={}",
        state.generation,
        state.diagnostic,
        state.preparing
    );
    state
}
#[test]
fn csv_edits_trigger_reload_without_source_edits() {
    let f = Fixture::new();
    f.write("main.sanim","type Sample = {value: Number}\nlet samples=read<Table<Sample>>(\"data.csv\")\nwait(len(samples)*1s)\n");
    f.write("data.csv", "value\n1\n");
    let runtime = f.open();
    let first = until(&runtime, |s| s.prepared.is_some());
    f.write("data.csv", "value\n1\n2\n3\n");
    let next = until(&runtime, |s| s.revision > first.revision);
    assert_eq!(
        next.prepared.unwrap().animation.duration,
        sanim_core::Fraction::from(3)
    );
}
#[test]
fn invalid_reload_preserves_last_good_and_recovers() {
    let f = Fixture::new();
    f.write("main.sanim", "wait(1s)\n");
    let runtime = f.open();
    let first = until(&runtime, |s| s.prepared.is_some());
    f.write("main.sanim", "let x = (\n");
    let failed = until(&runtime, |s| s.diagnostic.is_some());
    assert_eq!(failed.revision, first.revision);
    assert_eq!(
        failed.prepared.unwrap().animation.duration,
        sanim_core::Fraction::from(1)
    );
    f.write("main.sanim", "wait(2s)\n");
    let next = until(&runtime, |s| {
        s.revision > first.revision && s.diagnostic.is_none()
    });
    assert_eq!(
        next.prepared.unwrap().animation.duration,
        sanim_core::Fraction::from(2)
    );
}
#[test]
fn imported_module_changes_trigger_reload() {
    let f = Fixture::new();
    f.write(
        "main.sanim",
        "import \"theme\" as theme\nwait(theme.duration)\n",
    );
    f.write("theme.sanim", "let duration=1s\n");
    let runtime = f.open();
    let first = until(&runtime, |s| s.prepared.is_some());
    f.write("theme.sanim", "let duration=2s\n");
    let next = until(&runtime, |s| s.revision > first.revision);
    assert_eq!(
        next.prepared.unwrap().animation.duration,
        sanim_core::Fraction::from(2)
    );
}
#[test]
fn yaml_and_toml_changes_trigger_reload() {
    for (path, first, next) in [
        ("data.yaml", "count: 1\n", "count: 2\n"),
        ("data.toml", "count=1\n", "count=2\n"),
    ] {
        let f = Fixture::new();
        f.write("main.sanim",&format!("let d=read<Dict>({path:?})\nlet count=decode<Number>(d[\"count\"])\nwait(count*1s)\n"));
        f.write(path, first);
        let runtime = f.open();
        let before = until(&runtime, |s| s.prepared.is_some());
        f.write(path, next);
        let after = until(&runtime, |s| s.revision > before.revision);
        assert_eq!(
            after.prepared.unwrap().animation.duration,
            sanim_core::Fraction::from(2)
        );
    }
}
#[test]
fn initial_inputs_and_resets_are_acknowledged_atomically() {
    let f = Fixture::new();
    f.write("main.sanim", "reflect count = 1\nwait(count*1s)\n");
    let runtime = f.open();
    runtime
        .set_inputs(BTreeMap::from([("count".into(), serde_json::json!(3))]))
        .unwrap();
    let changed = until(&runtime, |s| {
        s.requested_inputs == s.applied_inputs && !s.preparing && s.prepared.is_some()
    });
    assert_eq!(
        changed.prepared.unwrap().animation.duration,
        sanim_core::Fraction::from(3)
    );
    runtime.set_inputs(BTreeMap::new()).unwrap();
    let reset = until(&runtime, |s| s.applied_inputs == 2 && !s.preparing);
    assert_eq!(
        reset.prepared.unwrap().animation.duration,
        sanim_core::Fraction::from(1)
    );
}
#[test]
fn failed_input_keeps_last_good_until_corrected() {
    let f = Fixture::new();
    f.write("main.sanim", "reflect count = 1\nwait(count*1s)\n");
    let runtime = f.open();
    let first = until(&runtime, |s| s.prepared.is_some());
    runtime
        .set_inputs(BTreeMap::from([("count".into(), serde_json::json!("bad"))]))
        .unwrap();
    let failed = until(&runtime, |s| s.diagnostic.is_some());
    assert_eq!(failed.revision, first.revision);
    runtime
        .set_inputs(BTreeMap::from([("count".into(), serde_json::json!(2))]))
        .unwrap();
    until(&runtime, |s| {
        s.revision > first.revision && s.diagnostic.is_none()
    });
}
#[test]
fn missing_data_creation_recovers() {
    let f = Fixture::new();
    f.write(
        "main.sanim",
        "let d=read<Dict>(\"missing.json\")\nwait(1s)\n",
    );
    let runtime = f.open();
    until(&runtime, |s| s.diagnostic.is_some());
    f.write("missing.json", "{}\n");
    until(&runtime, |s| s.prepared.is_some() && s.diagnostic.is_none());
}

#[test]
fn failed_revision_error_persists_on_unchanged_polls() {
    let f = Fixture::new();
    f.write("main.sanim", "wait(1s)\n");
    let runtime = f.open();
    until(&runtime, |s| s.prepared.is_some());
    f.write("main.sanim", "let broken = (\n");
    let failed = until(&runtime, |s| s.diagnostic.is_some());
    runtime.reload().unwrap();
    let later = runtime.wait_for_change(failed.generation, Duration::from_millis(200));
    assert!(later.diagnostic.is_some());
    assert_eq!(later.revision, failed.revision);
}
#[test]
fn reflected_schema_changes_can_recover_with_matching_inputs() {
    let f = Fixture::new();
    f.write("main.sanim", "reflect amount = 1\nwait(1s)\n");
    let runtime = f.open();
    until(&runtime, |s| s.prepared.is_some());
    runtime
        .set_inputs(BTreeMap::from([("amount".into(), serde_json::json!(2))]))
        .unwrap();
    until(&runtime, |s| s.applied_inputs == 1 && !s.preparing);
    f.write("main.sanim", "reflect amount = \"new\"\nwait(1s)\n");
    until(&runtime, |s| s.diagnostic.is_some());
    runtime
        .set_inputs(BTreeMap::from([(
            "amount".into(),
            serde_json::json!("accepted"),
        )]))
        .unwrap();
    let fixed = until(&runtime, |s| {
        s.applied_inputs == 2 && !s.preparing && s.diagnostic.is_none()
    });
    assert_eq!(
        fixed.prepared.unwrap().bindings["amount"],
        sanim_core::Value::String("accepted".into())
    );
}

#[test]
fn host_waker_reports_data_changes_and_can_read_snapshots() {
    let f = Fixture::new();
    f.write(
        "main.sanim",
        "let d=read<Dict>(\"data.json\")\nwait(decode<Number>(d[\"count\"])*1s)\n",
    );
    f.write("data.json", "{\"count\":1}");
    let runtime = std::sync::Arc::new(f.open());
    let first = until(&runtime, |s| s.prepared.is_some());
    let weak = std::sync::Arc::downgrade(&runtime);
    let (send, receive) = std::sync::mpsc::channel();
    runtime.set_waker(Some(std::sync::Arc::new(move || {
        if let Some(runtime) = weak.upgrade() {
            let _ = send.send(runtime.snapshot().revision);
        }
    })));
    assert_eq!(
        receive.recv_timeout(Duration::from_secs(2)).unwrap(),
        first.revision
    );
    f.write("data.json", "{\"count\":2}");
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let revision = receive
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap();
        if revision > first.revision {
            break;
        }
    }
    assert_eq!(
        runtime
            .snapshot()
            .prepared
            .as_ref()
            .unwrap()
            .animation
            .duration,
        sanim_core::Fraction::from(2)
    );
    runtime.set_waker(None);
}

#[test]
fn background_runtime_exposes_planned_duration_and_rejects_overflow() {
    let f = Fixture::new();
    f.write("main.sanim", "wait(1s)");
    let runtime = f.open();
    let first = until(&runtime, |s| s.prepared.is_some());
    assert_eq!(first.planned_duration, Some(sanim_core::Fraction::from(1)));
    f.write("main.sanim", "wait(7201s)");
    let failed = until(&runtime, |s| s.diagnostic.is_some());
    assert_eq!(failed.revision, first.revision);
    assert!(failed.diagnostic.unwrap().message.contains("two-hour"));
}
#[test]
fn external_viewport_configuration_is_acknowledged_and_reloaded() {
    let f = Fixture::new();
    f.write("main.sanim", "wait(1s)");
    let runtime = f.open();
    until(&runtime, |s| s.prepared.is_some());
    runtime
        .set_viewport(sanim_core::Viewport {
            width: 800,
            height: 600,
            rate: sanim_core::Fraction::from(24),
            dpi: 96,
        })
        .unwrap();
    let state = until(&runtime, |s| {
        !s.preparing
            && s.applied_inputs == s.requested_inputs
            && s.prepared
                .as_ref()
                .is_some_and(|p| p.animation.width == 800)
    });
    assert_eq!(
        state.prepared.unwrap().animation.rate,
        sanim_core::Fraction::from(24)
    );
}
