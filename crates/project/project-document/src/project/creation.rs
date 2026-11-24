use std::path::{Path, PathBuf};

use serde_json::Value;
use shrimply_math_core::Fraction;

use super::{AudioTrack, CanvasSize, CaptionTrack, Project, VisualTrack, history, normalize_tags};

pub struct NewProject(Project);

impl NewProject {
    pub fn from_settings(
        name: &str,
        size: CanvasSize,
        fps: Fraction,
        tags: &[String],
    ) -> Result<Self, String> {
        let mut project = Project {
            name: project_name(name)?,
            tags: normalize_tags(tags),
            ..Project::default()
        };
        project.set_video_settings(size, fps)?;
        Ok(Self::with_initial_tracks(project))
    }

    pub fn from_fields(fields: &Value) -> Result<Self, String> {
        let mut project = Project::default();
        if !fields["name"].is_string() {
            return Err("project name must be a string".into());
        }
        apply_project_settings(&mut project, fields)?;
        Ok(Self::with_initial_tracks(project))
    }

    fn with_initial_tracks(mut project: Project) -> Self {
        project.caption_tracks.push(CaptionTrack::default());
        project.video_tracks.push(VisualTrack::default());
        project.audio_tracks.push(AudioTrack::default());
        Self(project)
    }

    pub fn write(self, mut path: PathBuf, replace: bool) -> Result<PathBuf, String> {
        if !path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("shrimp"))
        {
            path.set_extension("shrimp");
        }
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "could not create project directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        if replace {
            history::create_project_file(&path, &self.0)?;
        } else {
            history::create_new_project_file(&path, &self.0)?;
        }
        Ok(path)
    }

    pub fn write_in(self, directory: &Path) -> Result<PathBuf, String> {
        let stem = filename_stem(&self.0.name);
        let path = directory
            .join(uuid::Uuid::new_v4().to_string())
            .join(format!("{stem}.shrimp"));
        self.write(path, false)
    }
}

pub fn filename_stem(name: &str) -> String {
    name.trim()
        .chars()
        .map(|character| match character {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            character => character,
        })
        .collect()
}

fn project_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("project name cannot be empty".into());
    }
    Ok(name.into())
}

pub fn apply_project_settings(project: &mut Project, fields: &Value) -> Result<(), String> {
    let name = fields
        .get("name")
        .map(|name| project_name(name.as_str().ok_or("project name must be a string")?))
        .transpose()?;
    let frame_rate = fields
        .get("fps")
        .map(|value| {
            value
                .as_str()
                .ok_or("fps must be an exact string, for example 30 or 30000/1001")?
                .trim()
                .parse()
                .map_err(|_| "invalid exact project frame rate")
        })
        .transpose()?
        .unwrap_or(project.fps);

    let mut size = project.canvas_size;
    for (key, dimension) in [
        ("canvas_width", &mut size.width),
        ("canvas_height", &mut size.height),
    ] {
        if let Some(value) = fields.get(key) {
            *dimension = value
                .as_u64()
                .and_then(|number| u32::try_from(number).ok())
                .or_else(|| value.as_str()?.trim().parse().ok())
                .ok_or_else(|| format!("{key} must be a positive integer"))?;
        }
    }
    let tags = fields
        .get("tags")
        .map(|value| {
            let tags: Vec<String> = if let Some(tags) = value.as_str() {
                tags.split(',').map(str::to_owned).collect()
            } else {
                serde_json::from_value(value.clone())
                    .map_err(|_| "tags must be a list of strings")?
            };
            Ok::<_, &str>(normalize_tags(&tags))
        })
        .transpose()?;
    project.set_video_settings(size, frame_rate)?;
    if let Some(name) = name {
        project.name = name;
    }
    if let Some(tags) = tags {
        project.tags = tags;
    }
    Ok(())
}
