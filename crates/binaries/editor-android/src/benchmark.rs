//! Explicit renderer benchmark input. Normal launches always start at the launcher.
use shrimply_project_document::{
    AudioItem, AudioSource, AudioTrack, CaptionItem, CaptionTrack, CommentItem, CommentTrack,
    Project, Time, VisualItem, VisualTrack,
};

const NORMAL_TRACKS: usize = 16;
const STRESS_TRACKS: usize = 128;
const NORMAL_CLIPS: usize = 24;
const STRESS_CLIPS: usize = 2048;
const TRACK_KINDS: usize = 4;
const CLIP_STRIDE_SECONDS: i64 = 6;

pub(crate) fn project(stress: bool) -> Project {
    let mut project = Project {
        name: "Timeline benchmark".into(),
        ..Project::default()
    };
    let tracks = if stress { STRESS_TRACKS } else { NORMAL_TRACKS };
    let clips = if stress { STRESS_CLIPS } else { NORMAL_CLIPS };
    for row in 0..tracks {
        let bounds = |index: usize| {
            let start = Time::from_seconds(index as i64 * CLIP_STRIDE_SECONDS + (row % 3) as i64);
            let end = start.saturating_add(Time::from_seconds(if index.is_multiple_of(3) {
                5
            } else {
                3
            }));
            (start, end)
        };
        match row % TRACK_KINDS {
            0 => project.comment_tracks.push(CommentTrack {
                items: (0..clips)
                    .map(|index| {
                        let (start, end) = bounds(index);
                        CommentItem::new(
                            start,
                            end,
                            format!("Comment {} — long timeline label for clipping", index + 1),
                        )
                    })
                    .collect(),
                ..CommentTrack::default()
            }),
            1 => project.caption_tracks.push(CaptionTrack {
                items: (0..clips)
                    .map(|index| {
                        let (start, end) = bounds(index);
                        CaptionItem::new(
                            start,
                            end,
                            format!("Caption {} — long timeline label for clipping", index + 1),
                        )
                    })
                    .collect(),
                ..CaptionTrack::default()
            }),
            2 => project.video_tracks.push(VisualTrack {
                items: (0..clips)
                    .map(|index| {
                        let (start, end) = bounds(index);
                        VisualItem::shape_item(project.canvas_size, start, end)
                    })
                    .collect(),
                ..VisualTrack::default()
            }),
            _ => project.audio_tracks.push(AudioTrack {
                items: (0..clips)
                    .map(|index| {
                        let (start, end) = bounds(index);
                        AudioItem::builder(start, end)
                            .source(AudioSource::Generator(Box::default()))
                            .source_duration(end.saturating_sub(start))
                            .build()
                    })
                    .collect(),
                ..AudioTrack::default()
            }),
        }
    }
    project.ensure_ids();
    project
        .validate()
        .expect("benchmark project must obey the real project invariants");
    project
}
