use shrimply_project_document::project::{Project, Time};
use shrimply_timeline_edit::{ItemKey, TrackKey, TrackKind};

#[derive(Clone)]
pub struct SelectedAudioProject {
    pub project: Project,
    pub start: Time,
    pub end: Time,
    pub chunks: Vec<(Time, Time)>,
    pub audio_cut_points: Vec<Time>,
    pub video_cut_points: Vec<Time>,
}

impl SelectedAudioProject {
    pub fn at_zero(mut self) -> Project {
        for track in &mut self.project.audio_tracks {
            for item in &mut track.items {
                item.start = item.start.saturating_sub(self.start);
                item.end = item.end.saturating_sub(self.start);
            }
        }
        self.project
    }
}

impl crate::scene::Scene {
    pub fn references_asset(&self, path: &std::path::Path) -> bool {
        self.project
            .borrow()
            .assets()
            .iter()
            .any(|asset| asset.path() == path)
    }

    pub fn selected_audio_project(&self) -> Option<SelectedAudioProject> {
        selected_audio_project(
            &self.project.borrow(),
            &crate::scene::selected_timeline_items(&self.selection),
            &crate::scene::selected_timeline_tracks(&self.selection),
        )
    }
}
pub fn selected_audio_project(
    project: &Project,
    selected_items: &[ItemKey],
    selected_tracks: &[TrackKey],
) -> Option<SelectedAudioProject> {
    let mut start = None::<Time>;
    let mut end = None::<Time>;
    let mut intervals = Vec::new();
    let mut audio_tracks = Vec::new();
    let video_cut_points = cut_points(
        project
            .video_tracks
            .iter()
            .flat_map(|track| track.items.iter().map(|item| (item.start, item.end)))
            .collect(),
    );
    let selected_audio_tracks = selected_tracks
        .iter()
        .any(|key| key.kind == TrackKind::Audio);

    for (track_index, track) in project.audio_tracks.iter().enumerate() {
        let mut track = track.clone();
        track.items = track
            .items
            .into_iter()
            .enumerate()
            .filter_map(|(item_index, item)| {
                let selected = if selected_audio_tracks {
                    selected_tracks
                        .iter()
                        .any(|key| key.kind == TrackKind::Audio && key.track_index == track_index)
                } else {
                    selected_items.iter().any(|key| {
                        key.kind == TrackKind::Audio
                            && key.track_index == track_index
                            && key.item_index == item_index
                    })
                };
                if !selected {
                    return None;
                }
                start = Some(start.map_or(item.start, |value| value.min(item.start)));
                end = Some(end.map_or(item.end, |value| value.max(item.end)));
                intervals.push((item.start, item.end));
                Some(item)
            })
            .collect();
        audio_tracks.push(track);
    }

    let mut selection = SelectedAudioProject {
        project: Project {
            format_version: project.format_version,
            name: project.name.clone(),
            tags: project.tags.clone(),
            fps: project.fps,
            canvas_size: project.canvas_size,
            audio_tracks,
            folded_sequences: project.folded_sequences.clone(),
            ..Default::default()
        },
        start: start?,
        end: end?,
        chunks: chunk_intervals(intervals.clone()),
        audio_cut_points: cut_points(intervals),
        video_cut_points,
    };
    selection.project.prune_folded_sequences();
    Some(selection)
}

fn cut_points(intervals: Vec<(Time, Time)>) -> Vec<Time> {
    let mut cuts = intervals
        .into_iter()
        .flat_map(|(start, end)| [start, end])
        .collect::<Vec<_>>();
    cuts.sort();
    cuts.dedup();
    cuts
}

fn chunk_intervals(mut intervals: Vec<(Time, Time)>) -> Vec<(Time, Time)> {
    intervals.retain(|(start, end)| end > start);
    intervals.sort_by_key(|(start, _)| *start);

    let mut merged: Vec<(Time, Time)> = Vec::new();
    for (start, end) in intervals {
        let Some((_, last_end)) = merged.last_mut() else {
            merged.push((start, end));
            continue;
        };
        if start < *last_end {
            *last_end = (*last_end).max(end);
        } else {
            merged.push((start, end));
        }
    }
    merged
}
