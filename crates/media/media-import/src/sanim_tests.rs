use super::*;
#[test]
fn sanim_import_uses_native_clip_metadata() {
    let root = std::env::temp_dir().join(format!("sanim-import-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("scene.sanim");
    std::fs::write(&path, "wait(2s)\n").unwrap();
    assert_eq!(file_kind(&path), Some(FileKind::Sanim));
    assert!(direct_media_kind(FileKind::Sanim));
    let info = inspect(
        path,
        CanvasSize {
            width: 1920,
            height: 1080,
        },
        Time { seconds: 10.into() },
    )
    .unwrap();
    assert_eq!(info.visual_kind, Some(VisualMediaKind::Sanim));
    assert_eq!(info.duration.seconds, Fraction::from(2));
    assert_eq!(info.video_fps, Some(Fraction::from(30)));
    assert_eq!(info.video_sizes, vec![UVec2::new(1920, 1080)]);
    assert!(matches!(
        video_content_for_import(&info),
        VideoItemContent::Sanim(_)
    ));
    let canvas = CanvasSize {
        width: 1920,
        height: 1080,
    };
    let mut item =
        shrimply_project_document::VideoItem::shape_item(canvas, Time::ZERO, info.duration);
    item.file = info.source.clone();
    item.content = video_content_for_import(&info);
    let events = shrimply_project_document::source_events::for_item(&item, canvas)
        .expect("inactive imported Sanim clips expose their compiled timeline immediately");
    assert_eq!(
        events.index.finite().as_ref(),
        &[Fraction::from(0), Fraction::from(2)]
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn sanim_import_rejects_invalid_or_missing_source_instead_of_inventing_duration() {
    let root = std::env::temp_dir().join(format!("sanim-invalid-import-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("scene.sanim");
    std::fs::write(&path, "repeat {").unwrap();
    let canvas = CanvasSize {
        width: 1920,
        height: 1080,
    };
    let error = inspect(path.clone(), canvas, Time { seconds: 10.into() })
        .err()
        .expect("malformed Sanim has no valid import duration");
    assert!(error.contains("scene.sanim"), "{error}");
    std::fs::remove_file(&path).unwrap();
    assert!(
        inspect(path, canvas, Time { seconds: 10.into() }).is_err(),
        "missing source must fail strict import inspection"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn parent_sized_imports_use_the_host_default_without_stretching_auto_sources() {
    let root = std::env::temp_dir().join(format!("sanim-parent-import-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let canvas = CanvasSize {
        width: 1920,
        height: 1080,
    };
    for (name, source, duration) in [
        (
            "parent",
            "return Animation(children=[wait(2s)],duration=\"match_parent\")",
            10,
        ),
        ("auto", "wait(2s)", 2),
    ] {
        let path = root.join(format!("{name}.sanim"));
        std::fs::write(&path, source).unwrap();
        let info = inspect(path, canvas, Time { seconds: 10.into() }).unwrap();
        assert_eq!(info.duration.seconds, Fraction::from(duration));
    }
    let path = root.join("invalid.sanim");
    std::fs::write(
        &path,
        "return Animation(children=[Animation(duration=\"match_parent\")])",
    )
    .unwrap();
    let error = inspect(path, canvas, Time { seconds: 10.into() })
        .err()
        .unwrap();
    assert!(error.contains("S139"), "{error}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn infinite_imports_publish_clock_markers_for_the_actual_initial_clip_span() {
    let root = std::env::temp_dir().join(format!("sanim-infinite-import-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("scene.sanim");
    std::fs::write(
        &path,
        "return Animation(children=[wait(250ms),wait(750ms)],flow=\"sequence\").repeat(forever)",
    )
    .unwrap();
    let canvas = CanvasSize {
        width: 1920,
        height: 1080,
    };
    let info = inspect(path, canvas, Time::from_seconds(10)).unwrap();
    assert_eq!(info.duration, Time::from_seconds(10));
    let mut item =
        shrimply_project_document::VideoItem::shape_item(canvas, Time::ZERO, info.duration);
    item.file = info.source.clone();
    item.content = video_content_for_import(&info);
    let events = shrimply_project_document::source_events::for_item(&item, canvas)
        .expect("default import span must match its marker context");
    assert_eq!(
        events.index.neighbors(Fraction::new(91u64, 10u64)),
        [Some(9.into()), Some(Fraction::new(37u64, 4u64))]
    );
    std::fs::remove_dir_all(root).unwrap();
}
