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
    pub fn text_preview(
        &self,
        text: String,
        point: Vec2,
    ) -> Option<crate::external_content::TextPreview> {
        let project = self.project.borrow();
        let x = f64::from(point.x);
        let y = f64::from(point.y);
        if text.is_empty() || x < timeline_x() {
            return None;
        }
        let (kind, track_index, _) = items::track_at_y(&project, y + self.view.scroll_y)?;
        let start = Time::from_seconds_f64(x_to_time(
            x,
            self.view.scroll_seconds,
            self.view.seconds_per_pixel,
        ));
        let start = self.snap_repository.snap(start).unwrap_or(start);
        let mut end = start
            .saturating_add(self.default_visual_duration)
            .snapped(project.frame_step());
        match kind {
            TrackKind::Caption => {
                for item in &project.caption_tracks.get(track_index)?.items {
                    if item.start <= start && start < item.end {
                        return None;
                    }
                    if item.start > start {
                        end = end.min(item.start);
                        break;
                    }
                }
            }
            TrackKind::Video => {
                for item in &project.video_tracks.get(track_index)?.items {
                    if item.start <= start && start < item.end {
                        return None;
                    }
                    if item.start > start {
                        end = end.min(item.start);
                        break;
                    }
                }
            }
            TrackKind::Audio => return None,
        }
        (end > start).then_some(crate::external_content::TextPreview {
            text,
            kind,
            track_index,
            start,
            end,
        })
    }

    pub fn update_text_drop_preview(&mut self, text: String, point: Vec2) {
        self.clear_drop_preview();
        self.text_drop_preview = self.text_preview(text, point);
    }

    pub fn update_drop_preview(&mut self, path: PathBuf, point: Vec2) -> bool {
        self.text_drop_preview = None;
        if !self.pointer_in_viewport(point)
            || self.scrollbar_at(point).is_some()
            || matches!(import::file_kind(&path), None | Some(import::FileKind::Vtt))
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
                inspection: (import::file_kind(&path) != Some(import::FileKind::Python)).then(
                    || import::request_inspection(path.clone(), project.canvas_size, duration),
                ),
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
        self.text_drop_preview = None;
    }

    pub(super) fn poll_drop_preview(&mut self) -> bool {
        let mut changed = false;
        let Some(drop) = self.drop_preview.as_mut() else {
            return false;
        };
        if let Some(subscription) = drop.inspection.as_mut() {
            loop {
                match subscription.try_next() {
                    TryNext::Empty => break,
                    TryNext::Event(Event::Progress(_)) => continue,
                    TryNext::Event(Event::Finished(info)) => {
                        changed = true;
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
        let (source, duration, visual_kind, video_streams, audio_streams) =
            if let Some(info) = &drop.info {
                (
                    info.source.clone(),
                    info.duration,
                    info.visual_kind,
                    info.video_streams,
                    info.audio_streams,
                )
            } else {
                let Some(file_kind) = import::file_kind(&drop.path) else {
                    return false;
                };
                let (visual_kind, video_streams, audio_streams) = match file_kind {
                    import::FileKind::Mp4
                    | import::FileKind::Mov
                    | import::FileKind::Mkv
                    | import::FileKind::WebM => (Some(import::VisualMediaKind::Video), 1, 1),
                    import::FileKind::Image => (Some(import::VisualMediaKind::Image), 1, 0),
                    import::FileKind::Gif => (Some(import::VisualMediaKind::Gif), 1, 0),
                    import::FileKind::Svg => (Some(import::VisualMediaKind::Svg), 1, 0),
                    import::FileKind::Pdf => (Some(import::VisualMediaKind::Pdf), 1, 0),
                    import::FileKind::Python => (Some(import::VisualMediaKind::Manim), 1, 0),
                    import::FileKind::Blender => (Some(import::VisualMediaKind::Blender), 1, 0),
                    import::FileKind::LayeredImage => {
                        (Some(import::VisualMediaKind::LayeredImage), 1, 0)
                    }
                    import::FileKind::Obj => (Some(import::VisualMediaKind::Obj), 1, 0),
                    import::FileKind::Ply => (Some(import::VisualMediaKind::Gaussian), 1, 0),
                    import::FileKind::Audio => (None, 0, 1),
                    import::FileKind::Vtt => return false,
                };

                (
                    project::Asset::new(drop.path.clone()),
                    self.default_visual_duration,
                    visual_kind,
                    video_streams,
                    audio_streams,
                )
            };
        if video_streams == 0 && audio_streams == 0 {
            self.import_preview = None;
            return changed;
        }

        let project = self.project.borrow();
        let start = crate::math::time_at_x(self.view, drop.point.x.into());
        let start = self.snap_repository.snap(start).unwrap_or(start);
        let y = f64::from(drop.point.y).max(RULER_HEIGHT) + self.view.scroll_y;
        self.import_preview = Some(TimelineImportPreview {
            source,
            duration,
            visual_kind,
            preview: import::preview(
                &project,
                duration,
                video_streams,
                audio_streams,
                start,
                items::NewItemTarget::AtY(y),
                ToolState::from_preferences(&preferences::snapshot(&self.preferences))
                    .drag_collision,
            ),
            y,
        });
        changed
    }
}
