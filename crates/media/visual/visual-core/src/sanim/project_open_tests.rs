//! Saved-project loading must not compile any individual clip's source language.
use super::{tests::Fixture, *};
use shrimply_project_document::project::{
    Project, VisualTrack, create_project_file, load_project_snapshot, prepare_project,
};
use std::time::{Duration, Instant};

fn saved_project(fixture: &Fixture, item: VideoItem) -> Project {
    let path = fixture.0.join("project.shrimp");
    let mut project = Project {
        fps: 30.into(),
        canvas_size: CanvasSize {
            width: 100,
            height: 100,
        },
        ..Project::default()
    };
    project.video_tracks.push(VisualTrack::default());
    project.video_tracks[0].items.push(item);
    let unaffected =
        VideoItem::shape_item(project.canvas_size, Time::ZERO, Time { seconds: 10.into() });
    let unaffected_id = unaffected.id;
    project.video_tracks.push(VisualTrack::default());
    project.video_tracks[1].items.push(unaffected);
    create_project_file(&path, &project).unwrap();
    let loaded =
        load_project_snapshot(&path).expect("a clip source cannot prevent project deserialization");
    assert_eq!(loaded.video_tracks[1].items[0].id, unaffected_id);
    assert!(matches!(
        loaded.video_tracks[1].items[0].content,
        VideoItemContent::Shape(_)
    ));
    loaded
        .watch_assets()
        .expect("editor asset startup tolerates absent sources");
    drop(prepare_project(&path).expect("editor project preparation must not compile clip sources"));
    assert_eq!(loaded.video_tracks[0].items[0].start, Time::ZERO);
    assert_eq!(
        loaded.video_tracks[0].items[0].end,
        Time { seconds: 10.into() }
    );
    assert_eq!(
        loaded.video_tracks[0].items[0].source_duration,
        Time { seconds: 1.into() }
    );
    loaded
}

#[test]
fn native_sanim_syntax_error_is_clip_local_when_saved_project_opens() {
    let fixture = Fixture::new("repeat {");
    let project = saved_project(&fixture, fixture.item());
    let item = &project.video_tracks[0].items[0];
    let mut source = Source::new(item).unwrap();
    let error = source
        .poll(item, Time::ZERO, project.canvas_size, project.fps, true)
        .err()
        .expect("strict render must fail for malformed Sanim");
    assert!(error.contains("scene.sanim"), "{error}");
    assert!(
        source.runtime.snapshot().prepared.is_none(),
        "no fabricated valid scene"
    );
    assert!(
        matches!(
            source
                .poll(item, Time::ZERO, project.canvas_size, project.fps, false)
                .unwrap(),
            Status::Failed(_)
        ),
        "interactive preview exposes clip-local failure"
    );
    assert!(
        source.take_error().is_some(),
        "the diagnostic stays available to the editor"
    );
    let generation = source.runtime.snapshot().generation;
    fixture.write("add(Circle(radius=10pt,fill=Color(\"red\")))\nwait(1s)");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut state = source.runtime.snapshot();
    while (state.prepared.is_none() || state.preparing || state.generation == generation)
        && Instant::now() < deadline
    {
        state = source
            .runtime
            .wait_for_change(state.generation, Duration::from_millis(100));
    }
    assert!(state.diagnostic.is_none(), "{:?}", state.diagnostic);
    assert!(
        state.prepared.is_some(),
        "saving a corrected source recovers without reopening the project"
    );
    assert!(matches!(
        source
            .poll(item, Time::ZERO, project.canvas_size, project.fps, false)
            .unwrap(),
        Status::Ready(_)
    ));
}

#[test]
fn native_sanim_missing_sources_preserve_saved_timeline_on_project_open() {
    let fixture = Fixture::new("wait(1s)");
    std::fs::remove_file(fixture.0.join("scene.sanim")).unwrap();
    let project = saved_project(&fixture, fixture.item());
    let item = &project.video_tracks[0].items[0];
    let mut source = Source::new(item).unwrap();
    let error = source
        .poll(item, Time::ZERO, project.canvas_size, project.fps, true)
        .err()
        .expect("strict render must fail for missing source");
    assert!(error.contains("scene.sanim"), "{error}");
    assert!(matches!(
        source
            .poll(item, Time::ZERO, project.canvas_size, project.fps, false)
            .unwrap(),
        Status::Failed(_)
    ));
}

#[test]
fn native_sanim_missing_source_directory_does_not_prevent_project_open() {
    let fixture = Fixture::new("wait(1s)");
    let mut item = fixture.item();
    item.file = Asset::new(fixture.0.join("missing-directory").join("scene.sanim"));
    let project = saved_project(&fixture, item);
    let item = &project.video_tracks[0].items[0];
    let mut source = Source::new(item)
        .expect("a missing source directory must not prevent clip/runtime creation");
    assert!(
        source
            .poll(item, Time::ZERO, project.canvas_size, project.fps, true)
            .is_err(),
        "strict CLI/export still fails clearly"
    );
    assert!(matches!(
        source
            .poll(item, Time::ZERO, project.canvas_size, project.fps, false)
            .unwrap(),
        Status::Failed(_)
    ));
    std::fs::create_dir_all(item.file.path().parent().unwrap()).unwrap();
    std::fs::write(item.file.path(), "add(Circle(radius=10pt))\nwait(1s)").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut state = source.runtime.snapshot();
    while (state.prepared.is_none() || state.preparing || state.diagnostic.is_some())
        && Instant::now() < deadline
    {
        state = source
            .runtime
            .wait_for_change(state.generation, Duration::from_millis(100));
    }
    assert!(
        state.prepared.is_some() && state.diagnostic.is_none(),
        "source directory creation must recover the clip: {:?}",
        state.diagnostic
    );
    assert!(matches!(
        source
            .poll(item, Time::ZERO, project.canvas_size, project.fps, false)
            .unwrap(),
        Status::Ready(_)
    ));
}
