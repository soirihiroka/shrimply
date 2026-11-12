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
