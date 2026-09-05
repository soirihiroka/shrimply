use crate::project::{AudioSource, Project, SequenceReference, TrackAddress, VideoItemContent};
use shrimply_math_color::Color;
use shrimply_timeline::{TrackKey, TrackKind};
use uuid::Uuid;

use super::super::{RULER_HEIGHT, TRACK_HEIGHT};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TrackRow {
    pub(crate) address: TrackAddress,
    pub(crate) root_key: Option<TrackKey>,
    pub(crate) depth: usize,
}

pub(crate) fn rows(project: &Project) -> Vec<TrackRow> {
    let mut rows = project
        .caption_tracks
        .iter()
        .enumerate()
        .rev()
        .map(|(track_index, track)| {
            let root_key = TrackKey {
                kind: TrackKind::Caption,
                track_index,
            };
            TrackRow {
                address: TrackAddress::Caption { track_id: track.id },
                root_key: Some(root_key),
                depth: 0,
            }
        })
        .collect::<Vec<_>>();
    for (track_index, track) in project.video_tracks.iter().enumerate().rev() {
        let root_key = TrackKey {
            kind: TrackKind::Video,
            track_index,
        };
        rows.push(TrackRow {
            address: TrackAddress::Video {
                sequence_path: Vec::new(),
                track_id: track.id,
            },
            root_key: Some(root_key),
            depth: 0,
        });
        for item in &track.items {
            let VideoItemContent::FoldedSequence(reference) = item.content else {
                continue;
            };
            let path = vec![item.id];
            if crate::folded_sequence::expanded(project, &path) {
                append_video_rows(project, reference, &path, 1, &mut rows, &mut Vec::new());
            }
        }
    }
    for (track_index, track) in project.audio_tracks.iter().enumerate() {
        let root_key = TrackKey {
            kind: TrackKind::Audio,
            track_index,
        };
        rows.push(TrackRow {
            address: TrackAddress::Audio {
                sequence_path: Vec::new(),
                track_id: track.id,
            },
            root_key: Some(root_key),
            depth: 0,
        });
        for item in &track.items {
            let AudioSource::FoldedSequence(reference) = item.source else {
                continue;
            };
            let path = vec![item.id];
            if crate::folded_sequence::expanded(project, &path) {
                append_audio_rows(project, reference, &path, 1, &mut rows, &mut Vec::new());
            }
        }
    }
    rows
}

fn append_video_rows(
    project: &Project,
    reference: SequenceReference,
    path: &[Uuid],
    depth: usize,
    rows: &mut Vec<TrackRow>,
    stack: &mut Vec<Uuid>,
) {
    if stack.contains(&reference.sequence_id) {
        return;
    }
    let Some(sequence) = project.folded_sequence(reference.sequence_id) else {
        return;
    };
    stack.push(reference.sequence_id);
    for track in sequence.video_tracks.iter().rev() {
        rows.push(TrackRow {
            address: TrackAddress::Video {
                sequence_path: path.to_vec(),
                track_id: track.id,
            },
            root_key: None,
            depth,
        });
        for item in &track.items {
            let VideoItemContent::FoldedSequence(reference) = item.content else {
                continue;
            };
            let mut nested_path = path.to_vec();
            nested_path.push(item.id);
            if crate::folded_sequence::expanded(project, &nested_path) {
                append_video_rows(project, reference, &nested_path, depth + 1, rows, stack);
            }
        }
    }
    stack.pop();
}

fn append_audio_rows(
    project: &Project,
    reference: SequenceReference,
    path: &[Uuid],
    depth: usize,
    rows: &mut Vec<TrackRow>,
    stack: &mut Vec<Uuid>,
) {
    if stack.contains(&reference.sequence_id) {
        return;
    }
    let Some(sequence) = project.folded_sequence(reference.sequence_id) else {
        return;
    };
    stack.push(reference.sequence_id);
    for track in &sequence.audio_tracks {
        rows.push(TrackRow {
            address: TrackAddress::Audio {
                sequence_path: path.to_vec(),
                track_id: track.id,
            },
            root_key: None,
            depth,
        });
        for item in &track.items {
            let AudioSource::FoldedSequence(reference) = item.source else {
                continue;
            };
            let mut nested_path = path.to_vec();
            nested_path.push(item.id);
            if crate::folded_sequence::expanded(project, &nested_path) {
                append_audio_rows(project, reference, &nested_path, depth + 1, rows, stack);
            }
        }
    }
    stack.pop();
}

pub(crate) fn color(kind: TrackKind) -> Color {
    match kind {
        TrackKind::Video => Color::ACCENT_BLUE,
        TrackKind::Caption => Color::ACCENT_YELLOW,
        TrackKind::Audio => Color::ACCENT_GREEN,
    }
}

pub(crate) fn track_at_y(project: &Project, y: f64) -> Option<(TrackKind, usize, usize)> {
    if y < RULER_HEIGHT {
        return None;
    }

    let row = ((y - RULER_HEIGHT) / TRACK_HEIGHT).floor() as usize;
    let key = rows(project).get(row)?.root_key?;
    Some((key.kind, key.track_index, row))
}

pub(crate) fn target_track_at_y(
    project: &Project,
    kind: TrackKind,
    y: f64,
) -> Option<(usize, Option<usize>)> {
    if y < RULER_HEIGHT {
        return None;
    }

    let row = ((y - RULER_HEIGHT) / TRACK_HEIGHT).floor() as usize;
    let count = track_count(project, kind);
    if let Some(track_index) =
        (0..count).find(|track_index| row_for_track(project, kind, *track_index) == Some(row))
    {
        return Some((track_index, None));
    }
    let (start_row, end_row) = track_block_rows(project, kind);
    if row.checked_add(1) == Some(start_row) {
        let index = if reversed(kind) { count } else { 0 };
        return Some((index, Some(index)));
    }
    if row == end_row {
        let index = if reversed(kind) { 0 } else { count };
        return Some((index, Some(index)));
    }
    None
}

pub(crate) fn track_count(project: &Project, kind: TrackKind) -> usize {
    match kind {
        TrackKind::Caption => project.caption_tracks.len(),
        TrackKind::Video => project.video_tracks.len(),
        TrackKind::Audio => project.audio_tracks.len(),
    }
}

pub(super) fn active_new_track_at_y(
    project: &Project,
    kind: TrackKind,
    new_tracks: &[(TrackKind, usize)],
    y: f64,
) -> Option<(usize, Option<usize>)> {
    if y < RULER_HEIGHT {
        return None;
    }

    let row = ((y - RULER_HEIGHT) / TRACK_HEIGHT).floor() as usize;
    new_tracks
        .iter()
        .copied()
        .filter(|(track_kind, index)| *track_kind == kind && *index <= track_count(project, kind))
        .find_map(|(_, index)| {
            (row == projected_row_for_virtual_track(project, new_tracks, (kind, index))?)
                .then_some((index, Some(index)))
        })
}

pub(crate) fn row_for_track(
    project: &Project,
    kind: TrackKind,
    track_index: usize,
) -> Option<usize> {
    rows(project)
        .iter()
        .position(|row| row.root_key == Some(TrackKey { kind, track_index }))
}

pub(crate) fn row_for_address(project: &Project, address: &TrackAddress) -> Option<usize> {
    rows(project).iter().position(|row| &row.address == address)
}

pub(crate) fn projected_row_for_track(
    project: &Project,
    kind: TrackKind,
    track_index: usize,
    virtual_tracks: &[(TrackKind, usize)],
) -> Option<usize> {
    let row = row_for_track(project, kind, track_index)?;
    let new_indices = virtual_indices(virtual_tracks, kind);
    let final_index = final_track_index(track_index, &new_indices);
    let prior_kinds = virtual_tracks
        .iter()
        .filter(|(virtual_kind, _)| kind_order(*virtual_kind) < kind_order(kind))
        .count();
    let same_kind = new_indices
        .iter()
        .filter(|index| displayed_before(kind, **index, final_index))
        .count();
    Some(row + prior_kinds + same_kind)
}

pub(crate) fn projected_row_for_virtual_track(
    project: &Project,
    virtual_tracks: &[(TrackKind, usize)],
    virtual_track: (TrackKind, usize),
) -> Option<usize> {
    if !virtual_tracks.contains(&virtual_track) {
        return None;
    }
    let (kind, virtual_index) = virtual_track;
    let new_indices = virtual_indices(virtual_tracks, kind);
    let prior_kinds = virtual_tracks
        .iter()
        .filter(|(virtual_kind, _)| kind_order(*virtual_kind) < kind_order(kind))
        .count();
    let prior_virtual = new_indices
        .iter()
        .filter(|index| displayed_before(kind, **index, virtual_index))
        .count();
    let prior_real = (0..track_count(project, kind))
        .filter(|track_index| {
            displayed_before(
                kind,
                final_track_index(*track_index, &new_indices),
                virtual_index,
            )
        })
        .map(|track_index| root_track_row_span(project, kind, track_index))
        .sum::<usize>();
    Some(track_block_rows(project, kind).0 + prior_kinds + prior_virtual + prior_real)
}

fn virtual_indices(virtual_tracks: &[(TrackKind, usize)], kind: TrackKind) -> Vec<usize> {
    let mut indices = virtual_tracks
        .iter()
        .filter_map(|(track_kind, index)| (*track_kind == kind).then_some(*index))
        .collect::<Vec<_>>();
    indices.sort_unstable();
    indices.dedup();
    indices
}

fn final_track_index(track_index: usize, new_indices: &[usize]) -> usize {
    new_indices.iter().fold(track_index, |index, insertion| {
        index + usize::from(*insertion <= index)
    })
}

fn displayed_before(kind: TrackKind, candidate: usize, target: usize) -> bool {
    if reversed(kind) {
        candidate > target
    } else {
        candidate < target
    }
}

fn reversed(kind: TrackKind) -> bool {
    matches!(kind, TrackKind::Caption | TrackKind::Video)
}

fn kind_order(kind: TrackKind) -> usize {
    match kind {
        TrackKind::Caption => 0,
        TrackKind::Video => 1,
        TrackKind::Audio => 2,
    }
}

fn root_track_row_span(project: &Project, kind: TrackKind, track_index: usize) -> usize {
    let Some(row) = row_for_track(project, kind, track_index) else {
        return 0;
    };
    let next_index = if reversed(kind) {
        track_index.checked_sub(1)
    } else {
        track_index.checked_add(1)
    };
    next_index
        .and_then(|index| row_for_track(project, kind, index))
        .unwrap_or_else(|| track_block_rows(project, kind).1)
        .saturating_sub(row)
}

fn track_block_rows(project: &Project, kind: TrackKind) -> (usize, usize) {
    let caption_end = project.caption_tracks.len();
    let video_end = caption_end
        + project.video_tracks.len()
        + expanded_rows_before(project, TrackKind::Video, project.video_tracks.len());
    let audio_end = video_end
        + project.audio_tracks.len()
        + expanded_rows_before(project, TrackKind::Audio, project.audio_tracks.len());
    match kind {
        TrackKind::Caption => (0, caption_end),
        TrackKind::Video => (caption_end, video_end),
        TrackKind::Audio => (video_end, audio_end),
    }
}

pub(crate) fn expanded_rows_before(project: &Project, kind: TrackKind, end: usize) -> usize {
    crate::folded_sequence::child_tracks_before(project, kind, end)
}
