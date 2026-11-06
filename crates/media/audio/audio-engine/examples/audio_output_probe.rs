use std::{sync::Arc, thread, time::Duration};
use shrimply_audio_engine::{AudioLevels, AudioPlayer};
use shrimply_audio_engine::project::{AudioItem, AudioSource, AudioTrack, Project, Time, VisualItem, VisualTrack};

fn main() {
    let empty = Project::default();
    let levels = Arc::new(AudioLevels::default());
    let player = AudioPlayer::new(&empty, levels.clone()).unwrap();
    for frame in 0..20 {
        player.preview_from(Time::from_fraction(frame, 30));
        player.set_playing(true);
        player.set_playing(false);
        assert_eq!(player.take_failure(), None);
    }
    println!("empty project: 20 seeks and playback toggles succeeded");

    let end = Time::from_seconds(10);
    let mut visual_only = Project::default();
    visual_only.video_tracks.push(VisualTrack {
        items: vec![VisualItem::background_item(visual_only.canvas_size, Time::ZERO, end)],
        ..Default::default()
    });
    player.set_project(&visual_only);
    for frame in 0..20 {
        player.preview_from(Time::from_fraction(frame, 30));
        player.set_playing(true);
        player.set_playing(false);
        assert_eq!(player.take_failure(), None);
    }
    assert_eq!(levels.take_peaks(), [0.0, 0.0]);
    println!("visual-only project: 20 scrubs and playback toggles succeeded without audio output");
    let item = AudioItem::builder(Time::ZERO, end)
        .source_duration(end)
        .source(AudioSource::Generator(Default::default()))
        .build();
    let mut project = Project {
        audio_tracks: vec![AudioTrack { enabled: false, gain_db: -40.0, items: vec![item], ..Default::default() }],
        ..Default::default()
    };
    player.set_project(&project);
    for frame in 0..20 {
        player.preview_from(Time::from_fraction(frame, 30));
        assert_eq!(player.take_failure(), None);
    }
    assert_eq!(levels.take_peaks(), [0.0, 0.0]);
    println!("muted audio: 20 scrubs succeeded without audio output");

    let host = cpal::default_host();
    assert_eq!(host.id(), cpal::HostId::PipeWire);
    println!("selected audio host: {}", host.id());
    project.audio_tracks[0].enabled = true;
    player.set_project(&project);
    for frame in 0..20 {
        player.seek(Time::from_fraction(frame, 30));
        player.preview_from(Time::from_fraction(frame, 30));
        thread::sleep(Duration::from_millis(60));
        assert_eq!(player.take_failure(), None);
    }
    assert!(levels.take_peaks().iter().all(|peak| *peak > 0.0));
    println!("generated audio: 20 scrubs produced samples without errors");
    for _ in 0..5 {
        player.seek(Time::ZERO);
        player.set_playing(true);
        thread::sleep(Duration::from_millis(160));
        player.set_playing(false);
        assert_eq!(player.take_failure(), None);
        assert!(levels.take_peaks().iter().all(|peak| *peak > 0.0));
    }
    println!("generated audio: 5 playback/pause cycles produced samples without errors");
    player.stop();
}
