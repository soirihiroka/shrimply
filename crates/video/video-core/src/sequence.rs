//! Shared visual-track traversal and folded-sequence time resolution.
use shrimply_project::project::{
    FoldedSequence, Project, SequenceReference, Time, VideoItem, VisualTrack, video_source_time_at,
};
use uuid::Uuid;

pub struct ActiveVideoItem<'a> {
    pub track_index: usize,
    pub track_id: Uuid,
    pub item: &'a VideoItem,
    pub clip_transition: Option<ActiveClipTransition>,
    pub previous: Option<&'a VideoItem>,
}

use crate::clip_transition::ActiveClipTransition;

pub fn active_video_items<'a>(
    track_index: usize,
    track_id: Uuid,
    items: &'a [VideoItem],
    position: Time,
    item_ids: Option<&[Uuid]>,
) -> Vec<ActiveVideoItem<'a>> {
    crate::clip_transition::active_items(items, position, item_ids)
        .into_iter()
        .map(|active| ActiveVideoItem {
            track_index,
            track_id,
            item: active.item,
            clip_transition: active.clip_transition,
            previous: active.previous,
        })
        .collect()
}

pub fn active_tracks<'a>(
    tracks: &'a [VisualTrack],
    position: Time,
    item_ids: Option<&[Uuid]>,
) -> Vec<ActiveVideoItem<'a>> {
    tracks
        .iter()
        .enumerate()
        .filter(|(_, track)| track.enabled)
        .flat_map(|(index, track)| {
            active_video_items(index, track.id, &track.items, position, item_ids)
        })
        .collect()
}

pub fn resolve<'a>(
    project: &'a Project,
    item: &VideoItem,
    reference: SequenceReference,
    position: Time,
    ancestors: &[Uuid],
) -> Result<Option<(&'a FoldedSequence, Time)>, String> {
    let Some(position) = video_source_time_at(item, position) else {
        return Ok(None);
    };
    if ancestors.contains(&reference.sequence_id) {
        return Err(format!(
            "cyclic folded sequence reference involving {}",
            reference.sequence_id
        ));
    }
    let sequence = project
        .folded_sequence(reference.sequence_id)
        .ok_or_else(|| format!("missing folded sequence {}", reference.sequence_id))?;
    Ok(Some((sequence, position)))
}
