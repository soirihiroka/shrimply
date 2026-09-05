use super::*;

impl Scene {
    pub(super) fn build_snap_repository(&self) -> shrimply_timeline_snap::SnapRepo {
        let project = self.project.borrow();
        let preferences = preferences::snapshot(&self.preferences);
        crate::snapping::repository(
            &project,
            crate::snapping::Request {
                folded_drag: self.folded_drag.as_ref(),
                dragged_group: self.dragged_group.as_ref(),
                resize_drag: self.resize_drag.as_ref(),
                beats: if ToolState::from_preferences(&preferences).beat_grid {
                    beat_grid::snap_targets(&project, &self.beats, self.view)
                } else {
                    Vec::new()
                },
                playhead: player_state::current_time(&self.player),
                distance: (preferences.timeline_magnet == "true").then(|| {
                    crate::math::snap_distance(
                        self.view,
                        preferences.timeline_snap_radius_px.into(),
                    )
                }),
            },
        )
    }
}

pub(super) fn project_change(group: &DraggedGroup, duration: Time) -> player_state::ProjectChange {
    let mut change = player_state::ProjectChange {
        duration: Some(duration),
        ..player_state::ProjectChange::default()
    };
    for item in &group.items {
        match item.key.kind {
            TrackKind::Caption => change.captions = true,
            TrackKind::Video => change.video = true,
            TrackKind::Audio => {
                change.audio = true;
                change.audio_waveforms = true;
            }
        }
    }
    change
}
