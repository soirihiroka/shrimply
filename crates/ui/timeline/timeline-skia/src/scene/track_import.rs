use super::*;
use std::path::PathBuf;

impl Scene {
    pub fn activate_track_add(
        &mut self,
        key: TrackKey,
        action: TrackAddAction,
    ) -> Result<TrackAddOutcome, String> {
        self.activate_track_add_at(key, action, player_state::current_time(&self.player))
    }

    pub fn activate_track_add_at(
        &mut self,
        key: TrackKey,
        action: TrackAddAction,
        position: Time,
    ) -> Result<TrackAddOutcome, String> {
        crate::activate_track_add_checked_at(
            &self.project,
            &self.player,
            &self.selection,
            key,
            action,
            TrackAddSettings {
                default_visual_duration: self.default_visual_duration,
                default_text_font_family: &self.default_text_font_family,
            },
            position,
        )
    }

    pub fn import_track_file(
        &mut self,
        path: PathBuf,
        targets: &[TrackKey],
    ) -> Result<crate::import_queue::ImportStart, String> {
        self.import_track_file_at(path, targets, player_state::current_time(&self.player))
    }

    pub fn import_track_file_at(
        &mut self,
        path: PathBuf,
        targets: &[TrackKey],
        position: Time,
    ) -> Result<crate::import_queue::ImportStart, String> {
        self.external_imports.enqueue_tracks(
            vec![path],
            &self.project.borrow(),
            targets,
            position,
            self.default_visual_duration,
        )
    }
}
