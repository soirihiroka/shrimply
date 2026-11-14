use super::*;

#[test]
fn shared_image_svg_and_caption_imports_use_canonical_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let png = directory.path().join("pixel.PNG");
    // A complete one-pixel PNG; only metadata inspection is needed for import.
    fs::write(
        &png,
        [
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1,
            8, 4, 0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 252, 255, 31,
            0, 3, 3, 2, 0, 239, 163, 170, 169, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
        ],
    )
    .unwrap();
    let svg = directory.path().join("chart.svg");
    fs::write(&svg, r#"<svg xmlns="http://www.w3.org/2000/svg" width="120" height="80"><rect width="120" height="80"/></svg>"#).unwrap();
    let caption = directory.path().join("captions.vtt");
    fs::write(
        &caption,
        "WEBVTT\n\n00:00.000 --> 00:02.000\nA real caption\n",
    )
    .unwrap();
    let canvas = CanvasSize {
        width: 1920,
        height: 1080,
    };
    for (path, size, kind) in [
        (png, UVec2::new(1, 1), VisualMediaKind::Image),
        (svg, UVec2::new(120, 80), VisualMediaKind::Svg),
    ] {
        let info = inspect(path, canvas, Time::from_seconds(3)).unwrap();
        assert_eq!(info.video_sizes, [size]);
        assert_eq!(info.visual_kind, Some(kind));
        let mut project = Project::default();
        project.video_tracks.push(Default::default());
        apply_media_to_tracks(&mut project, &info, TrackKind::Video, &[0], Time::ZERO).unwrap();
        project.ensure_ids();
        project.validate().unwrap();
        assert_eq!(project.video_tracks[0].items[0].end, Time::from_seconds(3));
    }
    let info = inspect(caption, canvas, Time::from_seconds(3)).unwrap();
    let mut project = Project::default();
    project.caption_tracks.push(Default::default());
    apply_media_to_tracks(&mut project, &info, TrackKind::Caption, &[0], Time::ZERO).unwrap();
    project.ensure_ids();
    project.validate().unwrap();
    assert_eq!(project.caption_tracks[0].items[0].text, "A real caption");
    assert_eq!(project.duration(), Time::from_seconds(2));
}

#[test]
fn overlapping_caption_cues_fail_before_mutating_the_project() {
    let mut project = Project::default();
    project.caption_tracks.push(Default::default());
    let cues = parse_vtt_contents(
        "WEBVTT\n\n00:00.000 --> 00:02.000\nFirst\n\n00:01.000 --> 00:03.000\nSecond\n",
    );
    assert!(
        apply_vtt_cues_to_tracks(&mut project, &cues, &[0], Time::ZERO)
            .err()
            .unwrap()
            .contains("overlap")
    );
    assert!(project.caption_tracks[0].items.is_empty());
}

#[test]
fn data_dependent_sanim_duration_import_does_not_need_a_renderer() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("steps.csv"), "value\n1\n2\n3\n4\n").unwrap();
    let path = directory.path().join("source.sanim");
    fs::write(&path, "type Step={value:Number}\nlet steps=read<List<Step>>(\"steps.csv\")\nlet marker_shape=Circle(radius=20pt)\nadd(marker_shape)\nrepeat len(steps) { wait(250ms) }\n").unwrap();
    let canvas = CanvasSize {
        width: 1920,
        height: 1080,
    };
    let info = inspect(path, canvas, Time::from_seconds(3)).unwrap();
    assert_eq!(info.visual_kind, Some(VisualMediaKind::Sanim));
    assert_eq!(info.duration, Time::from_seconds(1));
    let mut project = Project::default();
    project.video_tracks.push(Default::default());
    apply_media_to_tracks(&mut project, &info, TrackKind::Video, &[0], Time::ZERO).unwrap();
    project.ensure_ids();
    project.validate().unwrap();
    assert!(matches!(
        project.video_tracks[0].items[0].content,
        VideoItemContent::Sanim(_)
    ));
    assert_eq!(project.duration(), Time::from_seconds(1));
}

#[test]
fn shared_container_inspection_preserves_exact_time_and_does_not_invent_a_rate() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.wav");
    // Real 1.5-second, mono, 48 kHz, signed-16 PCM source.
    let samples: u32 = 72_000;
    let data_bytes = samples * 2;
    let mut wav = Vec::new();
    wav.extend(b"RIFF");
    wav.extend((36 + data_bytes).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(48_000u32.to_le_bytes());
    wav.extend(96_000u32.to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(data_bytes.to_le_bytes());
    wav.resize(wav.len() + data_bytes as usize, 0);
    fs::write(&path, wav).unwrap();
    let info = inspect(
        path,
        CanvasSize {
            width: 1920,
            height: 1080,
        },
        Time::from_seconds(3),
    )
    .unwrap();
    assert_eq!(info.duration, Time::from_fraction(3, 2));
    assert_eq!(info.video_fps, None);
    let mut project = Project::default();
    project.audio_tracks.push(Default::default());
    apply_media_to_tracks(&mut project, &info, TrackKind::Audio, &[0], Time::ZERO).unwrap();
    project.ensure_ids();
    project.validate().unwrap();
    assert_eq!(project.duration(), Time::from_fraction(3, 2));
}
