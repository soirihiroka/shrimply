use shrimply_math_core::Fraction;
use shrimply_project::project;
use shrimply_project_core::CanvasSize;
use shrimply_support::recent_projects::{self, RecentProject};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::UNIX_EPOCH;

pub const PROJECT_FILE_PATTERNS: [&str; 4] = ["*.shrimp", "*.json", "*.otio", "*.kdenlive"];

#[derive(Default)]
pub struct Launcher {
    query: String,
    recents: Vec<RecentProject>,
}

impl Launcher {
    pub fn reload(&mut self) -> Result<(), String> {
        self.recents = load_recent_projects(&self.query)?;
        Ok(())
    }

    pub fn set_search(&mut self, query: String) -> Result<(), String> {
        self.query = query;
        self.reload()
    }

    pub fn clear_history(&mut self) -> Result<(), String> {
        clear_recent_projects()?;
        self.reload()
    }

    pub fn remove_recent(&mut self, index: usize) -> Result<(), String> {
        let Some(path) = self.recents.get(index).map(|recent| recent.path.clone()) else {
            return Ok(());
        };
        remove_recent_project(&path)?;
        self.reload()
    }

    pub fn recent_count(&self) -> usize {
        self.recents.len()
    }

    pub fn recent_name(&self, index: usize) -> Option<&str> {
        self.recents.get(index).map(|recent| recent.name.as_str())
    }

    pub fn recent_path(&self, index: usize) -> Option<&Path> {
        self.recents.get(index).map(|recent| recent.path.as_path())
    }

    pub fn recent_last_edited(&self, index: usize) -> Option<String> {
        self.recent_path(index).and_then(last_edited)
    }

    pub fn desktop_action(&self, index: usize) -> Result<crate::desktop_open::Action, String> {
        let path = self
            .recent_path(index)
            .ok_or_else(|| "Recent project does not exist.".to_string())?;
        crate::desktop_open::prepare(path, None)
    }

    pub fn launch_recent(&self, index: usize) -> Result<Child, String> {
        let path = self
            .recent_path(index)
            .ok_or_else(|| "Recent project does not exist.".to_string())?;
        launch_editor(path)
    }
}

pub fn load_recent_projects(query: &str) -> Result<Vec<RecentProject>, String> {
    let projects = recent_projects::load()?;
    let stored_count = projects.len();
    let query = query.trim().to_lowercase();
    let projects = projects
        .into_iter()
        .filter(|project| {
            query.is_empty()
                || project.name.to_lowercase().contains(&query)
                || project
                    .path
                    .to_string_lossy()
                    .to_lowercase()
                    .contains(&query)
        })
        .collect::<Vec<_>>();
    tracing::info!(
        database = %recent_projects::settings_db_path().display(),
        stored_count,
        visible_count = projects.len(),
        query,
        "launcher history loaded"
    );
    Ok(projects)
}

pub fn clear_recent_projects() -> Result<(), String> {
    recent_projects::clear()
}

pub fn remove_recent_project(path: &Path) -> Result<(), String> {
    recent_projects::remove(path)
}

pub fn create_project(
    mut path: PathBuf,
    name: &str,
    canvas_size: CanvasSize,
    fps: Fraction,
) -> Result<PathBuf, String> {
    if !has_shrimp_extension(&path) {
        path.set_extension("shrimp");
    }
    let project = project::Project {
        format_version: project::PROJECT_FORMAT_VERSION,
        name: name.to_string(),
        fps,
        canvas_size,
        caption_tracks: vec![project::CaptionTrack::default()],
        video_tracks: vec![project::VisualTrack::default()],
        audio_tracks: vec![project::AudioTrack::default()],
        folded_sequences: Vec::new(),
        expanded_sequence_paths: Vec::new(),
        cursor_position: None,
        timeline_zoom: None,
        preview_guides: Default::default(),
    };
    project::create_project_file(&path, &project)?;
    Ok(path)
}

pub fn create_project_from_values(
    path: PathBuf,
    name: &str,
    width: i32,
    height: i32,
    frame_rate_index: i32,
) -> Result<PathBuf, String> {
    let width = u32::try_from(width).map_err(|_| "Invalid canvas width.".to_string())?;
    let height = u32::try_from(height).map_err(|_| "Invalid canvas height.".to_string())?;
    let frame_rate_index =
        usize::try_from(frame_rate_index).map_err(|_| "Invalid frame rate.".to_string())?;
    let fps = shrimply_project_core::COMMON_FRAME_RATES
        .get(frame_rate_index)
        .ok_or_else(|| "Invalid frame rate.".to_string())?
        .value;
    create_project(path, name, CanvasSize { width, height }, fps)
}

pub fn default_project_filename(name: &str) -> String {
    let safe_name = name
        .chars()
        .map(|character| match character {
            '/' | '\\' | '\0' => '_',
            character => character,
        })
        .collect::<String>();
    let timestamp = glib::DateTime::now_local()
        .and_then(|time| time.format("%Y-%m-%d_%H-%M-%S"))
        .map(|value| value.to_string())
        .unwrap_or_else(|_| "unknown-time".to_string());
    format!("{safe_name}_{timestamp}.shrimp")
}

pub fn default_project_path(directory: &Path, name: &str) -> PathBuf {
    directory.join(default_project_filename(name))
}

pub fn last_edited(path: &Path) -> Option<String> {
    path.metadata()
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .and_then(|modified| i64::try_from(modified.as_secs()).ok())
        .and_then(|seconds| glib::DateTime::from_unix_local(seconds).ok())
        .and_then(|date| date.format("%x %X").ok())
        .map(|date| date.to_string())
}

pub fn launch_editor(path: &Path) -> Result<Child, String> {
    launch_sibling_editor(path, "shrimply-editor")
}

#[cfg(target_os = "macos")]
pub fn launch_qt_editor(_path: &Path) -> Result<Child, String> {
    panic!("the Qt editor is not available on macOS");
}

#[cfg(not(target_os = "macos"))]
pub fn launch_qt_editor(path: &Path) -> Result<Child, String> {
    launch_sibling_editor(path, "shrimply-editor-qt")
}

fn launch_sibling_editor(path: &Path, binary_name: &str) -> Result<Child, String> {
    let sibling = std::env::current_exe()
        .map(|path| path.with_file_name(binary_name))
        .ok()
        .filter(|path| path.is_file());
    let editor = sibling.unwrap_or_else(|| PathBuf::from(binary_name));
    Command::new(&editor)
        .arg(path)
        .spawn()
        .map_err(|error| format!("could not launch {}: {error}", editor.display()))
}

pub fn project_file_name_filter(label: &str) -> String {
    format!("{label} ({})", PROJECT_FILE_PATTERNS.join(" "))
}

pub fn preset_labels() -> Vec<&'static str> {
    shrimply_project_core::PROJECT_PRESETS
        .iter()
        .map(|preset| preset.label)
        .chain(["Custom"])
        .collect()
}

pub fn frame_rate_labels() -> Vec<&'static str> {
    shrimply_project_core::COMMON_FRAME_RATES
        .iter()
        .map(|rate| rate.label)
        .collect()
}

pub fn preset_width(index: usize) -> u32 {
    shrimply_project_core::PROJECT_PRESETS
        .get(index)
        .map(|preset| preset.canvas_size.width)
        .unwrap_or(shrimply_project_core::DEFAULT_CANVAS_SIZE.width)
}

pub fn preset_height(index: usize) -> u32 {
    shrimply_project_core::PROJECT_PRESETS
        .get(index)
        .map(|preset| preset.canvas_size.height)
        .unwrap_or(shrimply_project_core::DEFAULT_CANVAS_SIZE.height)
}

pub fn preset_frame_rate(index: usize) -> usize {
    shrimply_project_core::PROJECT_PRESETS
        .get(index)
        .and_then(|preset| {
            shrimply_project_core::COMMON_FRAME_RATES
                .iter()
                .position(|rate| rate.value == preset.fps)
        })
        .unwrap_or_else(|| {
            shrimply_project_core::COMMON_FRAME_RATES
                .iter()
                .position(|rate| rate.value == shrimply_project_core::DEFAULT_PROJECT_FPS)
                .unwrap_or(0)
        })
}

fn has_shrimp_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("shrimp"))
}
