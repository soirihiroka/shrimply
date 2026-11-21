use std::path::PathBuf;

use shrimply_project_document::project::{CanvasSize, Project, Time};

pub use shrimply_project_document::project::{MAX_CANVAS_DIMENSION, MIN_CANVAS_DIMENSION};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectPresentation {
    pub name: String,
    pub tags: Vec<String>,
    pub canvas_size: CanvasSize,
    pub frame_rate: shrimply_math_core::Fraction,
    pub video_track_count: usize,
    pub audio_track_count: usize,
    pub caption_track_count: usize,
    pub duration: Time,
    pub file: PathBuf,
}

impl ProjectPresentation {
    pub fn details(&self) -> Vec<crate::InspectorDetail> {
        vec![
            crate::InspectorDetail {
                label: "Tracks",
                value: [
                    (
                        self.video_track_count,
                        "1 video track",
                        "%{count} video tracks",
                    ),
                    (
                        self.audio_track_count,
                        "1 audio track",
                        "%{count} audio tracks",
                    ),
                    (
                        self.caption_track_count,
                        "1 caption track",
                        "%{count} caption tracks",
                    ),
                ]
                .map(|(count, singular, plural)| {
                    if count == 1 {
                        shrimply_i18n::text(singular).into_owned()
                    } else {
                        shrimply_i18n::text_args(plural, &[("count", count.to_string())])
                    }
                })
                .join(", "),
            },
            crate::InspectorDetail {
                label: "Duration",
                value: shrimply_project_document::time_format::project_duration(self.duration),
            },
            crate::InspectorDetail {
                label: "Project File",
                value: self.file.to_string_lossy().into_owned(),
            },
        ]
    }
}

pub fn presentation(project: &Project) -> ProjectPresentation {
    ProjectPresentation {
        name: project.name.clone(),
        tags: project.tags.clone(),
        canvas_size: project.canvas_size,
        frame_rate: project.fps,
        video_track_count: project.video_tracks.len(),
        audio_track_count: project.audio_tracks.len(),
        caption_track_count: project.caption_tracks.len(),
        duration: project.duration(),
        file: shrimply_project_document::project::active_project_path(),
    }
}

pub use shrimply_components_core::project_settings::{
    SETTINGS_APPLY, SETTINGS_CANCEL, SETTINGS_DISCARD, SETTINGS_TITLE, SETTINGS_WARNING,
};

#[derive(Clone, Debug, PartialEq)]
pub struct SettingsPresentation {
    pub frame_rate: crate::InspectorControl,
    pub resolution: crate::InspectorControl,
    pub dirty: bool,
}

impl crate::InspectorController {
    pub fn project_settings_presentation(&self) -> SettingsPresentation {
        let project = self.project.borrow();
        let original = (project.canvas_size, project.fps);
        let mut state = self.project_settings.borrow_mut();
        let draft = state.get_or_insert_with(|| {
            shrimply_components_core::project_settings::ProjectSettings::from_values(
                original.0, original.1,
            )
        });
        if draft.original != Some(original) {
            *draft = shrimply_components_core::project_settings::ProjectSettings::from_values(
                original.0, original.1,
            );
        }
        let (canvas, fps) = draft
            .settings()
            .expect("project settings include frame rate");
        settings_presentation(canvas, fps, draft.changed())
    }

    pub fn stage_project_frame_rate(
        &self,
        value: shrimply_math_core::Fraction,
    ) -> Result<(), String> {
        self.project_settings_presentation();
        self.project_settings
            .borrow_mut()
            .as_mut()
            .expect("initialized settings draft")
            .stage_frame_rate(value)
    }

    pub fn stage_project_dimensions(&self, width: u32, height: u32) -> Result<(), String> {
        self.project_settings_presentation();
        self.project_settings
            .borrow_mut()
            .as_mut()
            .expect("initialized settings draft")
            .stage_dimensions(width, height)
    }

    pub fn discard_project_settings(&self) {
        self.project_settings.borrow_mut().take();
    }

    pub fn apply_staged_project_settings(&self) -> Result<(), String> {
        self.project_settings_presentation();
        let (canvas, fps) = self
            .project_settings
            .borrow()
            .as_ref()
            .expect("initialized settings draft")
            .settings()
            .expect("project settings include frame rate");
        self.apply_project_settings(canvas, fps)?;
        self.discard_project_settings();
        Ok(())
    }
}

pub fn settings_presentation(
    canvas: CanvasSize,
    fps: shrimply_math_core::Fraction,
    dirty: bool,
) -> SettingsPresentation {
    let choices = shrimply_project_document::project::frame_rate_choices(fps);
    SettingsPresentation {
        frame_rate: crate::InspectorControl::new(crate::ControlKind::Selector, "frame_rate", "FPS")
            .value(fps.to_string())
            .choices(
                choices.iter().map(|(rate, _)| rate.to_string()).collect(),
                choices.into_iter().map(|(_, label)| label).collect(),
            ),
        resolution: crate::InspectorControl::new(
            crate::ControlKind::Vector2,
            "resolution",
            "Resolution",
        )
        .components(vec![canvas.width.to_string(), canvas.height.to_string()])
        .prefixes(["W", "H"])
        .width_characters(7)
        .number(crate::NumberSpec {
            minimum: f64::from(MIN_CANVAS_DIMENSION),
            maximum: f64::from(MAX_CANVAS_DIMENSION),
            drag_step: 1.0,
            digits: 0,
            ..Default::default()
        })
        .lock(),
        dirty,
    }
}
