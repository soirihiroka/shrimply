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
