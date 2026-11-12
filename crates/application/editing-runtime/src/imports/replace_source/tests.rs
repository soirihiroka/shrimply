use super::*;
use shrimply_editing_api::{preview, protocol::*};
use shrimply_project_document::project::{
    TransitionSide, VideoItemContent, VisualModifier, VisualTrack, VisualTransition,
};
use shrimply_visual_modifiers::{ModifierEffect, rasterize::RasterizeModifier};

struct Fixture {
    root: PathBuf,
    project: Project,
    address: ClipAddress,
    source: PathBuf,
    project_path: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("shrimply-replace-source-{}", Uuid::new_v4()));
        let sources = root.join("sources");
        let project_dir = root.join("project");
        fs::create_dir_all(&sources).unwrap();
        fs::create_dir_all(&project_dir).unwrap();
        let source = sources.join("new.sanim");
        fs::write(sources.join("duration.sanim"), "let length=3s").unwrap();
        fs::write(
            &source,
            "import \"./duration\" as timing\nwait(timing.length)",
        )
        .unwrap();
        let old = sources.join("old.sanim");
        fs::write(&old, "wait(5s)").unwrap();
        let mut project = Project::default();
        project.fps = 30.into();
        project.canvas_size.width = 1080;
        project.canvas_size.height = 1920;
        project.video_tracks.push(VisualTrack::default());
        let info = native::inspect(old, project.canvas_size, Time { seconds: 5.into() }).unwrap();
        let start = time_from_frame(372, project.fps).unwrap();
        native::apply_media_to_tracks(&mut project, &info, TrackKind::Video, &[0], start).unwrap();
        let item = &mut project.video_tracks[0].items[0];
        item.content = VideoItemContent::Manim(Box::default());
        item.group_id = Some(12);
        item.end = time_from_frame(503, 30.into()).unwrap();
        item.time_offset = Time {
            seconds: shrimply_math_core::fraction_new(1, 10),
        };
        item.animation_time_offset = Time {
            seconds: shrimply_math_core::fraction_new(1, 5),
        };
        item.playback_speed = shrimply_math_core::fraction_new(2, 3);
        item.source_width = 640;
        item.source_height = 360;
        item.modifiers
            .push(VisualModifier::new(ModifierEffect::Rasterize(
                RasterizeModifier::new([1080.0, 1920.0].into()),
            )));
        item.transitions.intro = Some(VisualTransition::new(
            TransitionSide::Intro,
            Time {
                seconds: shrimply_math_core::fraction_new(1, 5),
            },
            project.canvas_size,
        ));
        let mut rotation = serde_json::to_value(&item.transform.rotation_degrees).unwrap();
        rotation["base"]["const"] = serde_json::json!(17.0);
        item.transform.rotation_degrees = serde_json::from_value(rotation).unwrap();
        let mut opacity = serde_json::to_value(&item.compositing.opacity).unwrap();
        opacity["base"]["const"] = serde_json::json!(0.65);
        item.compositing.opacity = serde_json::from_value(opacity).unwrap();
        let address = ClipAddress {
            kind: ClipKind::Video,
            sequence_path: Vec::new(),
            track_id: project.video_tracks[0].id.to_string(),
            item_id: project.video_tracks[0].items[0].id.to_string(),
        };
        Self {
            root,
            project,
            address,
            source,
            project_path: project_dir.join("test.shrimp"),
        }
    }
    fn replacement(&self, link: bool) -> ReplaceClipSourceRequest {
        ReplaceClipSourceRequest {
            address: self.address.clone(),
            source: self.source.to_string_lossy().into_owned(),
            link,
            copy_root: None,
        }
    }
    fn request(&self, operations: Vec<EditOperation>) -> EditRequest {
        EditRequest {
            history_label: "Replace animation".into(),
            frame: None,
            scope: None,
            operations,
        }
    }
    fn prepare(&self, request: &EditRequest) -> Result<PreparedEdit, String> {
        prepare(
            self.project.clone(),
            request,
            0,
            SequenceScopeId::root(),
            Time { seconds: 5.into() },
            &self.project_path,
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).expect("clean source replacement fixture");
    }
}

#[test]
fn replace_clip_source_preserves_every_placement_and_effect_field() {
    let fixture = Fixture::new();
    let before = &fixture.project.video_tracks[0].items[0];
    let prepared = fixture
        .prepare(&fixture.request(vec![EditOperation::ReplaceClipSource(
            fixture.replacement(true),
        )]))
        .unwrap();
    let after = &prepared.project.video_tracks[0].items[0];
    assert!(matches!(after.content, VideoItemContent::Sanim(_)));
    assert_eq!(after.file.path(), fixture.source.canonicalize().unwrap());
    assert_eq!(
        after.source_duration.seconds,
        shrimply_math_core::Fraction::from(3)
    );
    assert_eq!((after.source_width, after.source_height), (1080, 1920));
    let mut before = serde_json::to_value(before).unwrap();
    let mut after = serde_json::to_value(after).unwrap();
    for field in [
        "file",
        "content",
        "source_duration",
        "source_width",
        "source_height",
        "track_id",
        "svg_color_overrides",
        "video_generation",
    ] {
        before.as_object_mut().unwrap().remove(field);
        after.as_object_mut().unwrap().remove(field);
    }
    assert_eq!(before, after);
    let results = prepared.results().unwrap();
    assert_eq!(
        results[0].changed_addresses[0].item_id,
        fixture.address.item_id
    );
    assert!(results[0].deleted_addresses.is_empty());
}

#[test]
fn replace_clip_source_copy_root_promotes_dependencies_together() {
    let fixture = Fixture::new();
    let mut replacement = fixture.replacement(false);
    replacement.copy_root = Some(
        fixture
            .source
            .parent()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
    );
    let mut prepared = fixture
        .prepare(&fixture.request(vec![EditOperation::ReplaceClipSource(replacement)]))
        .unwrap();
    let path = prepared.project.video_tracks[0].items[0]
        .file
        .path()
        .to_path_buf();
    assert!(!path.exists());
    prepared.promote().unwrap();
    assert!(path.exists());
    assert!(path.parent().unwrap().join("duration.sanim").exists());
    assert_eq!(
        native::inspect(
            path,
            prepared.project.canvas_size,
            Time { seconds: 5.into() }
        )
        .unwrap()
        .duration
        .seconds,
        shrimply_math_core::Fraction::from(3)
    );
}

#[test]
fn replace_clip_source_failure_discards_all_staged_operations() {
    let fixture = Fixture::new();
    let before = serde_json::to_value(&fixture.project).unwrap();
    let bad = fixture.root.join("broken.sanim");
    fs::write(&bad, "repeat {").unwrap();
    let mut second = fixture.replacement(false);
    second.source = bad.to_string_lossy().into_owned();
    let request = fixture.request(vec![
        EditOperation::ReplaceClipSource(fixture.replacement(true)),
        EditOperation::ReplaceClipSource(second),
    ]);
    let error = match fixture.prepare(&request) {
        Ok(_) => panic!("bad source accepted"),
        Err(error) => error,
    };
    assert!(
        error.contains("operation 1 (replace_clip_source) failed"),
        "{error}"
    );
    assert_eq!(before, serde_json::to_value(&fixture.project).unwrap());
    assert_eq!(
        fs::read_dir(
            fixture
                .project_path
                .parent()
                .unwrap()
                .join("media/imported")
        )
        .unwrap()
        .count(),
        0
    );
}

#[test]
fn replace_clip_source_dry_run_validates_without_executing_or_copying() {
    let fixture = Fixture::new();
    // Invalid source code still permits request-only preview: no code is run.
    fs::write(&fixture.source, "repeat {").unwrap();
    let snapshot = preview::saved_snapshot(
        fixture.project.clone(),
        fixture.project_path.to_string_lossy().into_owned(),
    );
    let call = ApiCall {
        name: "replace_clip_source".into(),
        arguments: serde_json::to_value(fixture.replacement(false)).unwrap(),
        dry_run: true,
        if_revision: Some(preview::revision(&fixture.project).unwrap()),
    };
    let result = preview::dry_run(&call, &snapshot).unwrap();
    assert_eq!(result["validation"], "partial");
    assert_eq!(
        result["deferred"][0]["operation"]["type"],
        "replace_clip_source"
    );
    assert!(
        !fixture
            .project_path
            .parent()
            .unwrap()
            .join("media")
            .exists()
    );
    let mut conflict = call.clone();
    conflict.if_revision = Some("stale".into());
    assert!(
        preview::dry_run(&conflict, &snapshot)
            .unwrap_err()
            .contains("revision conflict")
    );
    let mut missing = call;
    missing.arguments["address"]["item_id"] = serde_json::json!(Uuid::new_v4().to_string());
    assert!(
        preview::dry_run(&missing, &snapshot)
            .unwrap_err()
            .contains("target video clip was not found")
    );
}

#[test]
fn replace_clip_source_detects_link_changes_before_commit() {
    let fixture = Fixture::new();
    let prepared = fixture
        .prepare(&fixture.request(vec![EditOperation::ReplaceClipSource(
            fixture.replacement(true),
        )]))
        .unwrap();
    fs::write(&fixture.source, "wait(9s)").unwrap();
    assert!(prepared.ensure_linked_sources_current().is_err());
}

#[test]
fn replace_clip_source_rejects_audio_only_sources_without_changing_the_clip() {
    let fixture = Fixture::new();
    let source = fixture.root.join("sound.wav");
    let samples = 800u32;
    let mut wave = Vec::new();
    wave.extend_from_slice(b"RIFF");
    wave.extend_from_slice(&(36 + samples).to_le_bytes());
    wave.extend_from_slice(b"WAVEfmt ");
    wave.extend_from_slice(&16u32.to_le_bytes());
    wave.extend_from_slice(&1u16.to_le_bytes());
    wave.extend_from_slice(&1u16.to_le_bytes());
    wave.extend_from_slice(&8000u32.to_le_bytes());
    wave.extend_from_slice(&8000u32.to_le_bytes());
    wave.extend_from_slice(&1u16.to_le_bytes());
    wave.extend_from_slice(&8u16.to_le_bytes());
    wave.extend_from_slice(b"data");
    wave.extend_from_slice(&samples.to_le_bytes());
    wave.resize(wave.len() + samples as usize, 128);
    fs::write(&source, wave).unwrap();
    let mut replacement = fixture.replacement(true);
    replacement.source = source.to_string_lossy().into_owned();
    let before = serde_json::to_value(&fixture.project).unwrap();
    let error = match fixture
        .prepare(&fixture.request(vec![EditOperation::ReplaceClipSource(replacement)]))
    {
        Ok(_) => panic!("audio accepted as visual source"),
        Err(error) => error,
    };
    assert!(error.contains("exactly one video/visual stream"), "{error}");
    assert_eq!(before, serde_json::to_value(&fixture.project).unwrap());
}

#[test]
fn replace_clip_source_preview_defers_addresses_changed_by_earlier_operations() {
    let mut fixture = Fixture::new();
    fixture.project.video_tracks.push(VisualTrack::default());
    let destination = TrackAddress {
        kind: ClipKind::Video,
        sequence_path: Vec::new(),
        track_id: fixture.project.video_tracks[1].id.to_string(),
    };
    let mut moved = fixture.address.clone();
    moved.track_id = destination.track_id.clone();
    let mut second = fixture.replacement(true);
    second.address = moved;
    let operations = vec![
        EditOperation::ReplaceClipSource(fixture.replacement(true)),
        EditOperation::MoveClip(MoveClipRequest {
            address: fixture.address.clone(),
            start_frame: Some(372),
            offset_frames: None,
            destination: Some(destination),
            collision: CollisionBehavior::Reject,
        }),
        EditOperation::ReplaceClipSource(second),
    ];
    let snapshot = preview::saved_snapshot(
        fixture.project.clone(),
        fixture.project_path.to_string_lossy().into_owned(),
    );
    let call = ApiCall {
        name: "run_edit_script".into(),
        arguments: serde_json::json!({"operations":operations}),
        dry_run: true,
        if_revision: None,
    };
    let result = preview::dry_run(&call, &snapshot).unwrap();
    assert_eq!(result["deferred"].as_array().unwrap().len(), 3);
    assert_eq!(
        result["deferred"][2]["reason"],
        "depends on deferred operations"
    );
    let prepared = fixture.prepare(&fixture.request(operations)).unwrap();
    assert!(prepared.project.video_tracks[0].items.is_empty());
    assert!(matches!(
        prepared.project.video_tracks[1].items[0].content,
        VideoItemContent::Sanim(_)
    ));
}

#[test]
fn replace_clip_source_sanim_retains_vector_capabilities_and_rejects_unavailable_path_effects() {
    let fixture = Fixture::new();
    let mut prepared = fixture
        .prepare(&fixture.request(vec![EditOperation::ReplaceClipSource(
            fixture.replacement(true),
        )]))
        .unwrap();
    let item = &mut prepared.project.video_tracks[0].items[0];
    assert_eq!(
        item.source_visual_kind(),
        shrimply_visual_modifiers::VisualKind::Vector
    );
    assert_eq!(
        item.modifier_output_kind().unwrap(),
        shrimply_visual_modifiers::VisualKind::Raster
    );
    item.modifiers = vec![VisualModifier::new(ModifierEffect::Vector(Box::new(
        shrimply_visual_modifiers::VectorModifierEffect::ShakyPath(Box::default()),
    )))];
    assert!(
        item.modifier_output_kind()
            .unwrap_err()
            .contains("recorded Sanim drawing commands")
    );
}
