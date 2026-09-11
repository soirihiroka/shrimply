use super::*;
use crate::import::{self, TrackImportStart};
use shrimply_resource_pipeline::{Event, TryNext};
use std::path::PathBuf;

impl Scene {
    pub fn activate_track_add(
        &mut self,
        key: TrackKey,
        action: TrackAddAction,
    ) -> Result<TrackAddOutcome, String> {
        crate::activate_track_add_checked(
            &self.project,
            &self.player,
            &self.selection,
            key,
            action,
            TrackAddSettings {
                default_visual_duration: self.default_visual_duration,
                default_text_font_family: &self.default_text_font_family,
            },
        )
    }

    pub fn import_track_file(&mut self, path: PathBuf, targets: &[TrackKey]) -> Result<(), String> {
        let kind = targets
            .first()
            .ok_or("No import tracks were selected")?
            .kind;
        if targets.iter().any(|target| target.kind != kind) {
            return Err("Selected tracks must have the same type".into());
        }
        let started = import::start_track_import(
            &mut self.project.borrow_mut(),
            path,
            kind,
            targets.iter().map(|target| target.track_index).collect(),
            player_state::current_time(&self.player),
            self.default_visual_duration,
        )?;
        match started {
            TrackImportStart::Inspect(inspection) => self.track_imports.push(inspection),
            TrackImportStart::Complete(result) => {
                import::finish_track_import(&self.player, &self.selection, Ok(result))?;
            }
        }
        Ok(())
    }

    pub(super) fn poll_track_imports(&mut self) -> bool {
        let mut changed = false;
        let mut index = 0;
        while index < self.track_imports.len() {
            let event = self.track_imports[index].subscription.try_next();
            let result = match event {
                TryNext::Event(Event::Finished(info)) => {
                    let pending = self.track_imports.remove(index);
                    import::finish_track_import_inspection(
                        &mut self.project.borrow_mut(),
                        pending.context,
                        &info,
                    )
                }
                TryNext::Event(Event::Failed(error)) => {
                    self.track_imports.remove(index);
                    Err(error.to_string())
                }
                TryNext::Event(Event::Cancelled) => {
                    self.track_imports.remove(index);
                    continue;
                }
                TryNext::Closed => {
                    self.track_imports.remove(index);
                    Err("Track import stopped before completion".into())
                }
                TryNext::Event(Event::Progress(_)) | TryNext::Empty => {
                    index += 1;
                    continue;
                }
            };
            changed = true;
            if let Err(error) = import::finish_track_import(&self.player, &self.selection, result) {
                self.pending_errors.push_back(error);
            }
        }
        changed
    }
}
