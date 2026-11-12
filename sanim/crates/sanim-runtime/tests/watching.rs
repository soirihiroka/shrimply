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
    f.write("main.sanim","type Sample = {value: Number}\nlet samples=read<List<Sample>>(\"data.csv\")\nwait(len(samples)*1s)\n");
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
    f.write("main.sanim", "let count=reflect(1)\nwait(count*1s)\n");
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
    f.write("main.sanim", "let count=reflect(1)\nwait(count*1s)\n");
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
    f.write("main.sanim", "let amount=reflect(1)\nwait(1s)\n");
    let runtime = f.open();
    until(&runtime, |s| s.prepared.is_some());
    runtime
        .set_inputs(BTreeMap::from([("amount".into(), serde_json::json!(2))]))
        .unwrap();
    until(&runtime, |s| s.applied_inputs == 1 && !s.preparing);
    f.write("main.sanim", "let amount=reflect(\"new\")\nwait(1s)\n");
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

#[test]
fn progress_distinguishes_ready_failed_reload_and_recovery() {
    use sanim_runtime::State;
    let f = Fixture::new();
    f.write("main.sanim", "wait(1s)");
    let runtime = f.open();
    let initial = runtime.snapshot();
    if initial.preparing {
        assert_eq!(initial.progress.state, State::Running);
        assert!(!initial.reloading);
    }
    let first = until(&runtime, |s| !s.preparing && s.prepared.is_some());
    assert_eq!(first.progress.state, State::Ready);
    assert!(!first.reloading);
    f.write("main.sanim", "let broken = (");
    let failed = until(&runtime, |s| !s.preparing && s.diagnostic.is_some());
    assert_eq!(failed.progress.state, State::Failed);
    assert!(failed.reloading);
    assert!(std::sync::Arc::ptr_eq(
        first.prepared.as_ref().unwrap(),
        failed.prepared.as_ref().unwrap()
    ));
    runtime.reload().unwrap();
    let unchanged = runtime.wait_for_change(failed.generation, Duration::from_millis(200));
    assert_eq!(unchanged.progress.state, State::Failed);
    assert_eq!(
        unchanged.progress.elapsed_micros,
        failed.progress.elapsed_micros
    );
    f.write("main.sanim", "wait(2s)");
    let ready = until(&runtime, |s| {
        !s.preparing && s.revision > first.revision && s.diagnostic.is_none()
    });
    assert_eq!(ready.progress.state, State::Ready);
    assert!(!ready.reloading);
}

#[test]
fn wake_observer_can_read_and_request_another_revision() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let f = Fixture::new();
    f.write("main.sanim", "let count=reflect(1)\nwait(count*1s)");
    let runtime = Arc::new(f.open());
    until(&runtime, |s| !s.preparing && s.prepared.is_some());
    let weak = Arc::downgrade(&runtime);
    let requested = Arc::new(AtomicBool::new(false));
    runtime.set_waker(Some(Arc::new(move || {
        if let Some(runtime) = weak.upgrade() {
            let state = runtime.snapshot();
            if state.applied_inputs == 1
                && state.progress.state == sanim_runtime::State::Ready
                && !requested.swap(true, Ordering::Relaxed)
            {
                runtime
                    .set_inputs(BTreeMap::from([("count".into(), serde_json::json!(3))]))
                    .unwrap();
            }
        }
    })));
    runtime
        .set_inputs(BTreeMap::from([("count".into(), serde_json::json!(2))]))
        .unwrap();
    let ready = until(&runtime, |s| !s.preparing && s.applied_inputs == 2);
    assert_eq!(ready.progress.state, sanim_runtime::State::Ready);
    assert_eq!(
        ready.prepared.unwrap().animation.duration,
        sanim_core::Fraction::from(3)
    );
    runtime.set_waker(None);
}

#[test]
fn last_runtime_owner_can_be_dropped_inside_worker_observer() {
    use std::sync::{Arc, Barrier, mpsc};
    let f = Fixture::new();
    f.write("main.sanim", "let count=reflect(1)\nwait(count*1s)");
    let runtime = Arc::new(f.open());
    until(&runtime, |s| !s.preparing && s.prepared.is_some());
    let weak = Arc::downgrade(&runtime);
    let barrier = Arc::new(Barrier::new(2));
    let observer_barrier = barrier.clone();
    let (send, receive) = mpsc::channel();
    runtime.set_waker(Some(Arc::new(move || {
        if let Some(runtime) = weak.upgrade() {
            let state = runtime.snapshot();
            if state.applied_inputs == 1
                && state.progress.state == sanim_runtime::State::Ready
                && std::thread::current().name() == Some("sanim-runtime")
            {
                send.send(false).unwrap();
                observer_barrier.wait();
                drop(runtime);
                send.send(true).unwrap();
            }
        }
    })));
    runtime
        .set_inputs(BTreeMap::from([("count".into(), serde_json::json!(2))]))
        .unwrap();
    assert!(!receive.recv_timeout(Duration::from_secs(8)).unwrap());
    drop(runtime);
    barrier.wait();
    assert!(
        receive
            .recv_timeout(Duration::from_secs(8))
            .expect("observer deadlocked dropping the last runtime owner")
    );
}

#[test]
fn idle_and_unrelated_file_events_never_publish_loading_or_new_generations() {
    let f = Fixture::new();
    f.write("main.sanim", "wait(1s)\n");
    let runtime = f.open();
    let ready = until(&runtime, |s| s.prepared.is_some() && !s.preparing);
    for index in 0..20 {
        f.write("unrelated.log", &index.to_string());
    }
    // Cross the former five-second recovery-poll interval.
    let idle = runtime.wait_for_change(ready.generation, Duration::from_millis(5500));
    assert_eq!(idle.generation, ready.generation);
    assert!(!idle.preparing);
    assert_eq!(idle.progress, ready.progress);
    f.write("main.sanim", "wait(1s)\n");
    let same = runtime.wait_for_change(idle.generation, Duration::from_millis(250));
    assert_eq!(
        same.generation, idle.generation,
        "same-content writes must not flash loading"
    );
}

#[test]
fn creating_missing_nested_import_recovers_without_polling() {
    let f = Fixture::new();
    f.write(
        "main.sanim",
        "import \"nested/theme\" as theme\nwait(theme.duration)\n",
    );
    let runtime = f.open();
    until(&runtime, |s| s.diagnostic.is_some() && !s.preparing);
    std::fs::create_dir(f.0.join("nested")).unwrap();
    f.write("nested/theme.sanim", "let duration=2s\n");
    let ready = until(&runtime, |s| {
        s.prepared.is_some() && !s.preparing && s.diagnostic.is_none()
    });
    assert_eq!(
        ready.prepared.unwrap().animation.duration,
        sanim_core::Fraction::from(2)
    );
    f.write("nested/theme.next", "let duration=3s\n");
    std::fs::rename(
        f.0.join("nested/theme.next"),
        f.0.join("nested/theme.sanim"),
    )
    .unwrap();
    let next = until(&runtime, |s| s.revision > ready.revision && !s.preparing);
    assert_eq!(
        next.prepared.unwrap().animation.duration,
        sanim_core::Fraction::from(3)
    );
}

#[test]
fn replaced_dependency_directory_reinstalls_watches_for_later_edits() {
    let f = Fixture::new();
    std::fs::create_dir(f.0.join("sub")).unwrap();
    f.write(
        "main.sanim",
        "import \"sub/theme\" as theme\nwait(theme.duration)\n",
    );
    f.write("sub/theme.sanim", "let duration=1s\n");
    let runtime = f.open();
    let first = until(&runtime, |s| s.prepared.is_some() && !s.preparing);
    std::fs::remove_dir_all(f.0.join("sub")).unwrap();
    std::fs::create_dir(f.0.join("sub")).unwrap();
    f.write("sub/theme.sanim", "let duration=2s\n");
    let replaced = until(&runtime, |s| s.revision > first.revision && !s.preparing);
    f.write("sub/theme.sanim", "let duration=3s\n");
    let edited = until(&runtime, |s| s.revision > replaced.revision && !s.preparing);
    assert_eq!(
        edited.prepared.unwrap().animation.duration,
        sanim_core::Fraction::from(3)
    );
}

#[test]
fn observation_retains_published_state_without_keeping_runtime_workers_alive() {
    let f = Fixture::new();
    f.write(
        "main.sanim",
        "let tint=reflect(Color(), label=\"Display Color\")\nwait(1s)\n",
    );
    let runtime = std::sync::Arc::new(f.open());
    let ready = until(&runtime, |s| s.prepared.is_some() && !s.preparing);
    let observed = runtime.observe();
    let weak = std::sync::Arc::downgrade(&runtime);
    drop(runtime);
    assert!(
        weak.upgrade().is_none(),
        "UI observations must never own a worker or join its thread when dropped"
    );
    let last = observed.snapshot();
    assert_eq!(last.generation, ready.generation);
    assert_eq!(last.prepared.unwrap().reflected[0].label, "Display Color");
}
