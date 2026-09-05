use super::*;
use shrimply_resource_pipeline::{Event, Subscription, TryNext};
use std::path::PathBuf;

pub(super) struct DropPreview {
    path: PathBuf,
    point: Vec2,
    inspection: Option<Subscription<import::InspectionKey, (), import::MediaInfo>>,
    info: Option<Arc<import::MediaInfo>>,
}

impl Scene {
    pub fn update_drop_preview(&mut self, path: PathBuf, point: Vec2) -> bool {
        if !self.pointer_in_viewport(point)
            || self.scrollbar_at(point).is_some()
            || import::file_kind(&path).is_none()
        {
            self.clear_drop_preview();
            return false;
        }
        let nested_target = {
            let project = self.project.borrow();
            let rows = items::track_rows(&project);
            let y = f64::from(point.y).max(RULER_HEIGHT) + self.view.scroll_y;
            crate::math::track_row_at_y(y)
                .and_then(|index| rows.get(index))
                .is_some_and(|row| row.root_key.is_none())
        };
        if nested_target {
            self.clear_drop_preview();
            return false;
        }
        if self
            .drop_preview
            .as_ref()
            .is_none_or(|drop| drop.path != path)
        {
            let project = self.project.borrow();
            let duration = preferences::snapshot(&self.preferences).default_visual_duration;
            self.drop_preview = Some(DropPreview {
                inspection: Some(import::request_inspection(
                    path.clone(),
                    project.canvas_size,
                    duration,
                )),
                path,
                point,
                info: None,
            });
        } else if let Some(drop) = self.drop_preview.as_mut() {
            drop.point = point;
        }
        self.poll_drop_preview();
        true
    }

    pub fn clear_drop_preview(&mut self) {
        self.drop_preview = None;
        self.import_preview = None;
    }

    pub(super) fn poll_drop_preview(&mut self) {
        let Some(drop) = self.drop_preview.as_mut() else {
            return;
        };
        if let Some(subscription) = drop.inspection.as_mut() {
            loop {
                match subscription.try_next() {
                    TryNext::Empty => break,
                    TryNext::Event(Event::Progress(_)) => continue,
                    TryNext::Event(Event::Finished(info)) => {
                        drop.info = Some(info);
                        drop.inspection = None;
                        break;
                    }
                    result => {
                        if let TryNext::Event(Event::Failed(error)) = result {
                            tracing::warn!(path = %drop.path.display(), %error, "Could not inspect drag preview");
                        }
                        drop.inspection = None;
                        break;
                    }
                }
            }
        }
        let Some(info) = &drop.info else {
            self.import_preview = None;
            return;
        };
        if info.video_streams == 0 && info.audio_streams == 0 {
            self.import_preview = None;
            return;
        }
        let project = self.project.borrow();
        let start = crate::math::time_at_x(self.view, drop.point.x.into());
        let y = f64::from(drop.point.y).max(RULER_HEIGHT) + self.view.scroll_y;
        self.import_preview = Some(TimelineImportPreview {
            source: info.source.clone(),
            duration: info.duration,
            visual_kind: info.visual_kind,
            preview: import::preview(
                &project,
                info.duration,
                info.video_streams,
                info.audio_streams,
                start,
                items::NewItemTarget::AtY(y),
                ToolState::from_preferences(&preferences::snapshot(&self.preferences))
                    .drag_collision,
            ),
            y,
        });
    }
}
