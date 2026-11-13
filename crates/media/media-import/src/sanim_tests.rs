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
