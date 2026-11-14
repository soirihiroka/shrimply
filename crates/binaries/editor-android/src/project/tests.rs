use super::*;

#[test]
fn canonical_io_receipts_do_not_adopt_racing_external_writes_as_our_baseline() {
    let Some(root) = isolated_root(
        "project::tests::canonical_io_receipts_do_not_adopt_racing_external_writes_as_our_baseline",
        false,
    ) else {
        return;
    };
    let path = root.join("project/receipt.shrimp");
    let mut backend = ProjectBackend::new(|| {}).unwrap();
    backend
        .dispatch(&json!({"op":"create","path":path,"name":"Original"}))
        .unwrap();
    let mut committed = backend.required().unwrap().clone();
    committed.name = "Our durable edit".into();
    document::commit_edit_saved(&committed, "receipt regression").unwrap();
    let own_receipt = document::saved_project_fingerprint().unwrap();
    backend.project = Some(Rc::new(RefCell::new(committed.clone())));
    let mut external = committed;
    external.name = "External racing edit".into();
    // Deterministically place an external write after persistence, before
    // the frontend observes its baseline; no timing or thread race needed.
    document::create_project_file(&path, &external).unwrap();
    backend.observe_saved().unwrap();
    assert_eq!(backend.fingerprint.as_ref(), Some(&own_receipt));
    assert_ne!(own_receipt, fingerprint(&path).unwrap());
    assert!(backend.reload().unwrap());
    assert_eq!(backend.required().unwrap().name, "External racing edit");
    assert_eq!(
        document::saved_project_fingerprint().unwrap(),
        fingerprint(&path).unwrap()
    );
    // An edit/undo may be dequeued before the native watch wake. Preserve
    // that external content, and resolve stable targets after reloading it.
    external
        .video_tracks
        .extend([Default::default(), Default::default()]);
    external.ensure_ids();
    let target = external.video_tracks[1].id;
    document::create_project_file(&path, &external).unwrap();
    backend
        .dispatch(&json!({"op":"toggle_track","kind":"video","track":0,"track_id":target}))
        .unwrap();
    assert!(backend.required().unwrap().video_tracks[0].enabled);
    assert!(!backend.required().unwrap().video_tracks[1].enabled);
    external = backend.required().unwrap().clone();
    external.tags = vec!["External unsignalled change".into()];
    document::create_project_file(&path, &external).unwrap();
    backend
        .dispatch(&json!({"op":"settings","name":"Combined edit"}))
        .unwrap();
    assert_eq!(backend.required().unwrap().tags, external.tags);
    external = backend.required().unwrap().clone();
    external.name = "External before undo".into();
    document::create_project_file(&path, &external).unwrap();
    backend.dispatch(&json!({"op":"undo"})).unwrap();
    assert_eq!(backend.required().unwrap().name, "Combined edit");
    backend.dispatch(&json!({"op":"redo"})).unwrap();
    assert_eq!(backend.required().unwrap().name, "External before undo");
    backend.close().unwrap();

    // Activation keeps the receipt of what was read, not a later pathname
    // lookup. The watcher-install recheck therefore sees this newer save.
    let prepared = document::prepare_project(&path).unwrap();
    external.name = "Changed before activation".into();
    document::create_project_file(&path, &external).unwrap();
    let project = document::activate_project(prepared);
    assert_eq!(project.name, "External before undo");
    assert_ne!(
        document::saved_project_fingerprint().unwrap(),
        fingerprint(&path).unwrap()
    );
    backend.project = Some(Rc::new(RefCell::new(project)));
    backend.path = Some(path.clone());
    backend.observe_saved().unwrap();
    backend.watch().unwrap();
    assert!(backend.reload().unwrap());
    assert_eq!(
        backend.required().unwrap().name,
        "Changed before activation"
    );
    backend.close().unwrap();
    assert!(
        document::saved_project_fingerprint().is_none(),
        "closing a document releases its bounded receipt"
    );
    // Legacy JSON uses the same conversion/validation and exact-byte receipt
    // as its existing public file API, without a second read or serialization.
    let json_path = path.with_file_name("legacy.json");
    fs::write(
        &json_path,
        document::serialize_project_json(&json_path, &external).unwrap(),
    )
    .unwrap();
    let public = document::from_json_file(&json_path).unwrap();
    let (loaded, receipt) = document::load_project_snapshot_with_fingerprint(&json_path).unwrap();
    assert_eq!(loaded.name, public.name);
    assert_eq!(receipt, fingerprint(&json_path).unwrap());
    backend
        .dispatch(&json!({"op":"open","path":json_path}))
        .unwrap();
    backend
        .dispatch(&json!({"op":"settings","name":"JSON edit"}))
        .unwrap();
    assert_eq!(
        document::from_json_file(&json_path).unwrap().name,
        "JSON edit"
    );
    assert_eq!(
        document::saved_project_fingerprint().unwrap(),
        fingerprint(&json_path).unwrap()
    );
    backend.close().unwrap();
}

#[test]
fn stable_destination_ids_survive_track_reordering_and_reject_removal() {
    let mut project = Project::default();
    project
        .video_tracks
        .extend([Default::default(), Default::default()]);
    project.ensure_ids();
    let id = project.video_tracks[1].id;
    let request = json!({"track":0,"track_id":id});
    let kind = shrimply_timeline_edit::TrackKind::Video;
    assert_eq!(target_index(&project, kind, &request).unwrap(), 1);
    project.video_tracks.reverse();
    assert_eq!(target_index(&project, kind, &request).unwrap(), 0);
    project.video_tracks.remove(0);
    assert!(
        target_index(&project, kind, &request)
            .unwrap_err()
            .contains("no longer exists")
    );
}
#[test]
fn canonical_project_lifecycle_history_and_external_reload() {
    let Some(root) = isolated_root(
        "project::tests::canonical_project_lifecycle_history_and_external_reload",
        false,
    ) else {
        return;
    };
    let path = root.join("project").join("actual.shrimp");
    let (send, events) = std::sync::mpsc::sync_channel(1);
    let mut backend = ProjectBackend::new(move || {
        let _ = send.try_send(());
    })
    .unwrap();
    backend
        .dispatch(&json!({"op":"create","path":path,"name":"Original"}))
        .unwrap();
    backend
        .dispatch(&json!({"op":"track_add","kind":"video","action":"Shape","duration_frames":90}))
        .unwrap();
    backend.dispatch(&json!({"op":"settings","canvas_width":1280,"canvas_height":720,"tags":["alpha"," alpha ",""]})).unwrap();
    assert_eq!(backend.required().unwrap().canvas_size.width, 1280);
    assert_eq!(backend.required().unwrap().tags, ["alpha"]);
    assert!(
        backend
            .dispatch(&json!({"op":"settings","canvas_width":0}))
            .is_err()
    );
    assert_eq!(backend.required().unwrap().canvas_size.width, 1280);
    for fps in ["0", "NaN", "-1"] {
        assert!(
            backend
                .dispatch(&json!({"op":"settings","fps":fps,"repair_frame_grid":true}))
                .is_err()
        );
        assert_eq!(
            backend.required().unwrap().fps,
            document::DEFAULT_PROJECT_FPS
        );
    }
    backend.dispatch(&json!({"op":"undo"})).unwrap();
    assert_eq!(backend.required().unwrap().canvas_size.width, 1920);
    backend
        .dispatch(&json!({"op":"settings","name":"Edited"}))
        .unwrap();
    // Content edits are automatically durable before any explicit Save.
    assert_eq!(
        document::load_project_snapshot(&path).unwrap().name,
        "Edited"
    );
    assert_eq!(
        backend.snapshot().unwrap()["project"]["duration_frames"],
        90
    );
    backend.dispatch(&json!({"op":"undo"})).unwrap();
    assert_eq!(backend.required().unwrap().name, "Original");
    backend.dispatch(&json!({"op":"redo"})).unwrap();
    assert_eq!(backend.required().unwrap().name, "Edited");
    let shared = backend.shared_project().unwrap();
    let fps = shared.borrow().fps;
    for frame in 0..1000 {
        backend.cursor(shrimply_math_core::time_from_frame(frame % 90, fps).unwrap());
        shared.borrow_mut().timeline_zoom = Some(Time::from_fraction(1, 50));
    }
    assert!(
        Rc::ptr_eq(&shared, &backend.shared_project().unwrap()),
        "playhead and viewport updates must not copy an canonical project root"
    );
    assert_eq!(shared.borrow().cursor_position.unwrap().as_frame(fps), 9);
    backend
        .dispatch(&json!({"op":"set_cursor","frame":45}))
        .unwrap();
    backend.active(false).unwrap();
    assert_eq!(
        document::load_project_snapshot(&path)
            .unwrap()
            .cursor_position
            .unwrap()
            .as_frame(backend.required().unwrap().fps),
        45
    );
    backend.active(true).unwrap();
    assert_eq!(
        document::load_project_snapshot(&path)
            .unwrap()
            .timeline_zoom,
        Some(Time::from_fraction(1, 50))
    );

    let revision = backend.revision;
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    backend.save().unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().modified().unwrap(),
        modified,
        "saving unchanged state must not rewrite the project"
    );
    assert!(backend.dispatch(&json!({"op":"reload"})).is_ok());
    assert_eq!(
        backend.revision, revision,
        "unchanged saves must not reparse/adopt a project"
    );

    // Status and playback notifications use cached recent-project metadata,
    // rather than opening SQLite repeatedly on the native render thread.
    let settings_directory = shrimply_path_core::config_directory();
    let parked_settings = settings_directory.with_extension("parked");
    fs::rename(&settings_directory, &parked_settings).unwrap();
    fs::write(&settings_directory, b"not a directory").unwrap();
    for _ in 0..100 {
        backend.snapshot().unwrap();
    }
    fs::remove_file(&settings_directory).unwrap();
    fs::rename(&parked_settings, &settings_directory).unwrap();

    let mut external = backend.required().unwrap().clone();
    external.name = "External".into();
    while events.try_recv().is_ok() {}
    document::create_project_file(&path, &external).unwrap();
    events
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    let bytes = fs::read(&path).unwrap();
    backend.dispatch(&json!({"op":"reload"})).unwrap();
    assert_eq!(backend.required().unwrap().name, "External");
    assert_eq!(
        fs::read(&path).unwrap(),
        bytes,
        "reload must not write over an external save"
    );
    assert!(
        document::can_undo(),
        "external saves enter the same undo history"
    );
    backend.dispatch(&json!({"op":"undo"})).unwrap();
    assert_eq!(backend.required().unwrap().name, "Edited");
    backend.dispatch(&json!({"op":"redo"})).unwrap();

    // Deliberate corruption of a test-only file verifies last-good recovery.
    fs::write(&path, b"incomplete external write").unwrap();
    assert!(backend.dispatch(&json!({"op":"reload"})).is_err());
    assert_eq!(backend.required().unwrap().name, "External");
    document::create_project_file(&path, &backend.required().unwrap()).unwrap();
    backend.dispatch(&json!({"op":"reload"})).unwrap();

    // The Android provider descriptor has no extension or stable pathname;
    // import uses its supplied name, persists the bytes once, then invokes
    // the same metadata/parser and timeline insertion as the desktop.
    use std::os::fd::AsRawFd;
    let svg = root.join("source.svg");
    fs::write(&svg, r#"<svg xmlns="http://www.w3.org/2000/svg" width="120" height="80"><rect width="120" height="80"/></svg>"#).unwrap();
    let fd = fs::File::open(svg).unwrap();
    backend.dispatch(&json!({"op":"import_file","path":format!("/proc/self/fd/{}",fd.as_raw_fd()),"name":"source.svg","start_frame":90})).unwrap();
    let image = backend.required().unwrap().video_tracks[0].items[1].clone();
    assert!(matches!(image.content, document::VisualSource::Svg));
    assert_eq!(image.source_width, 120);
    assert!(
        image
            .file
            .path()
            .starts_with(path.parent().unwrap().join("media"))
    );
    let vtt = root.join("source.vtt");
    fs::write(&vtt, "WEBVTT\n\n00:00.000 --> 00:02.000\nReal caption\n").unwrap();
    backend
        .dispatch(&json!({"op":"import_file","path":vtt,"name":"source.vtt","start_frame":180}))
        .unwrap();
    assert_eq!(
        backend.required().unwrap().caption_tracks[0].items[0].text,
        "Real caption"
    );
    let bad = root.join("unsupported.pdf");
    fs::write(&bad, b"not a PDF").unwrap();
    assert!(
        backend
            .dispatch(&json!({"op":"import_file","path":bad,"name":"unsupported.pdf"}))
            .is_err()
    );
    assert!(
        !path
            .parent()
            .unwrap()
            .join("media/unsupported.pdf")
            .exists(),
        "failed imports must not leave assets behind"
    );
    assert!(
        backend
            .dispatch(&json!({"op":"export","path":root.join("broken-reference.shrimp")}))
            .is_err()
    );
    let copy = path.with_file_name("copy.shrimp");
    backend
        .dispatch(&json!({"op":"export","path":copy}))
        .unwrap();
    assert_eq!(
        document::load_project_snapshot(&copy).unwrap().name,
        "External"
    );
    backend.close().unwrap();
    assert!(document::project_lock_owner(&path).unwrap().is_none());
    assert!(!document::can_undo());
    assert!(!document::can_redo());
    backend.dispatch(&json!({"op":"open","path":path})).unwrap();
    assert_eq!(backend.required().unwrap().name, "External");
    assert_eq!(backend.required().unwrap().video_tracks[0].items.len(), 2);
    assert_eq!(backend.required().unwrap().caption_tracks[0].items.len(), 1);
    assert_eq!(
        backend.snapshot().unwrap()["recents"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    backend
        .dispatch(&json!({"op":"settings","name":"Retry shutdown"}))
        .unwrap();
    document::save_view_state(&backend.required().unwrap());
    let directory = path.parent().unwrap();
    let parked = directory.with_extension("parked");
    fs::rename(directory, &parked).unwrap();
    fs::write(directory, b"storage unavailable").unwrap();
    assert!(document::shutdown_history().is_err());
    assert!(
        document::can_undo(),
        "a failed shutdown must retain undo history and its live worker"
    );
    fs::remove_file(directory).unwrap();
    fs::rename(&parked, directory).unwrap();
    backend.close().unwrap();
    assert!(!document::can_undo());
    assert!(!document::can_redo());
}

pub(crate) fn isolated_root(test: &str, ignored: bool) -> Option<PathBuf> {
    // Configure the canonical settings database in a fresh process before
    // platform paths or native workers have been initialized.
    if let Ok(root) = std::env::var("SHRIMPLY_ANDROID_PROJECT_TEST_ROOT") {
        let root = PathBuf::from(root);
        assert!(shrimply_recent_projects::settings_db_path().starts_with(&root));
        return Some(root);
    }
    let directory = tempfile::tempdir().unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap());
    child.args(["--exact", test, "--nocapture"]);
    if ignored {
        child.arg("--ignored");
    }
    let status = child
        .env("SHRIMPLY_ANDROID_PROJECT_TEST_ROOT", directory.path())
        .env("XDG_CONFIG_HOME", directory.path().join("config"))
        .status()
        .unwrap();
    assert!(status.success());
    None
}

#[test]
#[ignore]
fn project_edit_memory_benchmark() {
    use std::time::Instant;
    let Some(root) = isolated_root("project::tests::project_edit_memory_benchmark", true) else {
        return;
    };
    let mut backend = ProjectBackend::new(|| {}).unwrap();
    backend
        .dispatch(
            &json!({"op":"create","path":root.join("project/benchmark.shrimp"),"name":"Benchmark"}),
        )
        .unwrap();
    backend
        .dispatch(&json!({"op":"track_add","kind":"video","action":"Shape","duration_frames":90}))
        .unwrap();
    const WARMUP_EDITS: usize = 1200;
    const MEASURED_EDITS: usize = 600;
    const MAX_RSS_GROWTH_KIB: u64 = 8 * 1024;
    let mut samples = Vec::with_capacity(MEASURED_EDITS);
    let (mut warmed_rss, mut peak_rss) = (0, 0);
    for index in 0..WARMUP_EDITS + MEASURED_EDITS {
        let start = Instant::now();
        backend
            .dispatch(&json!({"op":"settings","name":format!("Edited {index}")}))
            .unwrap();
        if index >= WARMUP_EDITS {
            samples.push(start.elapsed());
        }
        if index + 1 >= WARMUP_EDITS {
            let rss = rss_kib();
            if index + 1 == WARMUP_EDITS {
                warmed_rss = rss;
            }
            peak_rss = peak_rss.max(rss);
        }
    }
    samples.sort_unstable();
    eprintln!(
        "Android canonical project: {MEASURED_EDITS} durable edits after {WARMUP_EDITS} warmup edits; median {:?}, p95 {:?}; RSS warmup={warmed_rss}KiB peak={peak_rss}KiB",
        samples[samples.len() / 2],
        samples[samples.len() * 95 / 100]
    );
    assert!(
        peak_rss.saturating_sub(warmed_rss) <= MAX_RSS_GROWTH_KIB,
        "repeated scalar edits must plateau within the shared bounded undo window"
    );
    let start = Instant::now();
    for _ in 0..10_000 {
        std::hint::black_box(backend.snapshot().unwrap());
    }
    eprintln!(
        "10,000 small-project status snapshots: {:?}",
        start.elapsed()
    );
    backend.close().unwrap();
    backend
        .dispatch(&json!({"op":"create","path":root.join("clock/benchmark.shrimp"),"name":"Clock"}))
        .unwrap();
    backend
        .dispatch(&json!({"op":"track_add","kind":"video","action":"Shape","duration_frames":90}))
        .unwrap();
    const CLOCK_UPDATES: usize = 100_000;
    const LARGE_ITEM_COUNT: u64 = 100_000;
    let positions = [Time::ZERO, Time::from_fraction(1, 30)];
    let clock_sample = |backend: &mut ProjectBackend| {
        (0..3)
            .map(|_| {
                let started = Instant::now();
                for index in 0..CLOCK_UPDATES {
                    backend.cursor(std::hint::black_box(positions[index % positions.len()]));
                }
                started.elapsed()
            })
            .min()
            .unwrap()
    };
    let small = clock_sample(&mut backend);
    backend
        .edit("Create actual large timeline workload", |project| {
            project.caption_tracks.push(Default::default());
            project.caption_tracks[0].items = (0..LARGE_ITEM_COUNT)
                .map(|frame| {
                    document::CaptionItem::new(
                        shrimply_math_core::time_from_frame(frame, project.fps).unwrap(),
                        shrimply_math_core::time_from_frame(frame + 1, project.fps).unwrap(),
                        "Caption".into(),
                    )
                })
                .collect();
            Ok(())
        })
        .unwrap();
    let shared = backend.shared_project().unwrap();
    let memory_before = rss_kib();
    let large = clock_sample(&mut backend);
    let memory_after = rss_kib();
    assert!(
        Rc::ptr_eq(&shared, &backend.shared_project().unwrap()),
        "clock updates must retain the same canonical document root"
    );
    assert!(
        memory_after.saturating_sub(memory_before) <= 1024,
        "clock updates must not allocate document state"
    );
    assert!(
        large <= small * 8 + std::time::Duration::from_millis(1),
        "clock cost must be independent of item count: small={small:?}, large={large:?}"
    );
    eprintln!(
        "{CLOCK_UPDATES} clock updates: 1 item {small:?}; {LARGE_ITEM_COUNT} captions + 1 shape {large:?}; RSS {memory_before}KiB -> {memory_after}KiB; canonical root retained"
    );
    drop(shared);
    backend.close().unwrap();
    // Exercise the complete shared native Scene, including track labels,
    // folded-row lookup and ruler. Setup/validation/row caching stay outside
    // timing; PictureRecorder measures draw-call construction without a GPU.
    const SMALL_VIDEO_ITEMS: u64 = 1_000;
    const LARGE_VIDEO_ITEMS: u64 = 10_000;
    let mut paint_project = Project::default();
    paint_project.video_tracks.push(Default::default());
    paint_project.video_tracks[0].items = (0..LARGE_VIDEO_ITEMS)
        .map(|frame| {
            document::VideoItem::shape_item(
                paint_project.canvas_size,
                shrimply_math_core::time_from_frame(frame, paint_project.fps).unwrap(),
                shrimply_math_core::time_from_frame(frame + 1, paint_project.fps).unwrap(),
            )
        })
        .collect();
    paint_project.ensure_ids();
    paint_project.validate().unwrap();
    let mut small_project = paint_project.clone();
    small_project.video_tracks[0]
        .items
        .truncate(SMALL_VIDEO_ITEMS as usize);
    let small_paint = paint_sample(&small_project);
    let large_paint = paint_sample(&paint_project);
    eprintln!(
        "Shared native Scene PictureRecorder: {SMALL_VIDEO_ITEMS} shape clips {small_paint:?}/frame; {LARGE_VIDEO_ITEMS} shape clips {large_paint:?}/frame (model setup and cached rows excluded)"
    );
    assert!(
        large_paint <= small_paint * 8 + std::time::Duration::from_millis(2),
        "visible-frame painting must scale with visible clips, not total source items"
    );
}

fn paint_sample(project: &Project) -> std::time::Duration {
    const WIDTH: u32 = 1280;
    const HEIGHT: u32 = 720;
    const WARMUP: usize = 3;
    const FRAMES: u32 = 30;
    let mut project = project.clone();
    project.timeline_zoom = Some(Time::from_fraction(1, 50));
    project.cursor_position = Some(project.duration().saturating_sub(Time::from_seconds(2)));
    let player = shrimply_player_core::new(project.duration(), project.fps);
    shrimply_player_core::seek_time(&player, project.cursor_position.unwrap());
    let performance = shrimply_playback_performance::open(Arc::new(project.clone()));
    let mut scene = shrimply_timeline_skia::scene::Scene::new(
        Rc::new(RefCell::new(project)),
        player,
        shrimply_timeline_edit::selection_state::new(),
        shrimply_preferences_core::open_with_defaults(),
        shrimply_property_transfer::new_clipboard(),
        performance,
    );
    let mut samples = Vec::with_capacity(FRAMES as usize);
    for index in 0..WARMUP + FRAMES as usize {
        let start = std::time::Instant::now();
        let mut recorder = skia_safe::PictureRecorder::new();
        let canvas =
            recorder.begin_recording(skia_safe::Rect::from_wh(WIDTH as f32, HEIGHT as f32), false);
        scene.draw(
            canvas,
            shrimply_components_skia::canvas::vec2(WIDTH as f32, HEIGHT as f32),
        );
        drop(std::hint::black_box(
            recorder.finish_recording_as_picture(None).unwrap(),
        ));
        if index >= WARMUP {
            samples.push(start.elapsed());
        }
    }
    samples.sort_unstable();
    samples[samples.len() / 2]
}

fn rss_kib() -> u64 {
    fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap()
}
