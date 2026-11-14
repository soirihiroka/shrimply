mod track_add_actions;
pub use track_add_actions::{
    TrackAddAction, TrackAddMenuEntry, TrackAddSettings, prepare_track_add, track_add_menu,
};
pub mod edit;
pub mod selection_state;

use shrimply_project_document::project::Project;
use shrimply_project_document::timeline_search::TimeSlice;

pub use shrimply_project_document::{ItemKey, TrackGap, TrackKey, TrackKind};

pub fn next_group_id(project: &Project) -> u64 {
    project
        .video_tracks
        .iter()
        .flat_map(|track| track.items.iter().filter_map(|item| item.group_id))
        .chain(
            project
                .audio_tracks
                .iter()
                .flat_map(|track| track.items.iter().filter_map(|item| item.group_id)),
        )
        .chain(
            project
                .comment_tracks
                .iter()
                .flat_map(|track| track.items.iter().filter_map(|item| item.group_id)),
        )
        .chain(
            project
                .caption_tracks
                .iter()
                .flat_map(|track| track.items.iter().filter_map(|item| item.group_id)),
        )
        .chain(project.folded_sequences.iter().flat_map(|sequence| {
            sequence
                .video_tracks
                .iter()
                .flat_map(|track| track.items.iter().filter_map(|item| item.group_id))
                .chain(
                    sequence
                        .audio_tracks
                        .iter()
                        .flat_map(|track| track.items.iter().filter_map(|item| item.group_id)),
                )
        }))
        .max()
        .unwrap_or_default()
        .saturating_add(1)
}

pub fn insert_sorted<T: TimeSlice>(items: &mut Vec<T>, item: T) -> usize {
    let start = item.start();
    let end = item.end();
    assert!(
        start < end,
        "cannot insert a timeline item with a non-positive duration"
    );
    let index =
        items.partition_point(|existing| (existing.start(), existing.end()) <= (start, end));
    assert!(
        index == 0 || items[index - 1].end() <= start,
        "cannot insert overlapping timeline items"
    );
    assert!(
        index == items.len() || end <= items[index].start(),
        "cannot insert overlapping timeline items"
    );
    items.insert(index, item);
    index
}
