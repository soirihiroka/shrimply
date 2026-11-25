use shrimply_math_core::Fraction;
use shrimply_project_document::project;
use shrimply_project_types::CanvasSize;
use shrimply_recent_projects::{self as recent_projects, RecentProject};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

pub const EDITOR_OPEN_CANCELED_EXIT_CODE: u8 = 2;

pub const PROJECT_FILE_PATTERNS: [&str; 5] =
    ["*.shrimp", "*.sjson", "*.json", "*.otio", "*.kdenlive"];

pub const UNSUPPORTED_GPU_HEADING: &str = "GPU Not Supported";
pub const UNSUPPORTED_GPU_MESSAGE: &str = "Your GPU is not supported or the NVIDIA driver is missing or unavailable. Shrimply requires an NVIDIA GPU with a working NVIDIA driver. Install or update the NVIDIA driver if you have an NVIDIA GPU. Shrimply will now exit.";

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub fn check_nvidia_gpu() -> Result<(), String> {
    #[cfg(target_os = "linux")]
    const DRIVER_LIBRARY: &str = "libcuda.so.1";
    #[cfg(target_os = "windows")]
    const DRIVER_LIBRARY: &str = "nvcuda.dll";
    const CUDA_SUCCESS: i32 = 0;

    // Load the driver at runtime so a missing driver can be reported in the UI.
    // SAFETY: These signatures match the CUDA driver API. The library stays
    // loaded while its symbols are used, and the device count pointer is valid.
    unsafe {
        let driver = libloading::Library::new(DRIVER_LIBRARY)
            .map_err(|error| format!("could not load NVIDIA driver: {error}"))?;
        let init = driver
            .get::<unsafe extern "system" fn(u32) -> i32>(b"cuInit\0")
            .map_err(|error| format!("could not load cuInit: {error}"))?;
        let get_count = driver
            .get::<unsafe extern "system" fn(*mut i32) -> i32>(b"cuDeviceGetCount\0")
            .map_err(|error| format!("could not load cuDeviceGetCount: {error}"))?;
        // CUDA requires the initialization flags to be zero.
        let status = init(0);
        if status != CUDA_SUCCESS {
            return Err(format!(
                "NVIDIA driver initialization failed: CUDA error {status}"
            ));
        }
        let mut count = 0;
        let status = get_count(&mut count);
        if status != CUDA_SUCCESS {
            return Err(format!("NVIDIA GPU detection failed: CUDA error {status}"));
        }
        if count <= 0 {
            return Err("No CUDA-capable NVIDIA GPU was found.".to_string());
        }
    }
    Ok(())
}

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
        self.recents.get(index).and_then(recent_last_edited)
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
                    .tags
                    .iter()
                    .any(|tag| tag.to_lowercase().contains(&query))
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
    path: PathBuf,
    name: &str,
    canvas_size: CanvasSize,
    fps: Fraction,
    tags: &[String],
) -> Result<PathBuf, String> {
    project::creation::NewProject::from_settings(name, canvas_size, fps, tags)?.write(path, true)
}

pub fn create_project_from_values(
    path: PathBuf,
    name: &str,
    width: i32,
    height: i32,
    frame_rate_index: i32,
) -> Result<PathBuf, String> {
    let (canvas, fps) = project_settings_from_values(width, height, frame_rate_index)?;
    create_project(path, name, canvas, fps, &[])
}

pub fn project_settings_from_values(
    width: i32,
    height: i32,
    frame_rate_index: i32,
) -> Result<(CanvasSize, Fraction), String> {
    let width = u32::try_from(width).map_err(|_| "Invalid canvas width.".to_string())?;
    let height = u32::try_from(height).map_err(|_| "Invalid canvas height.".to_string())?;
    let frame_rate_index =
        usize::try_from(frame_rate_index).map_err(|_| "Invalid frame rate.".to_string())?;
    let fps = shrimply_project_types::COMMON_FRAME_RATES
        .get(frame_rate_index)
        .ok_or_else(|| "Invalid frame rate.".to_string())?
        .value;
    Ok((CanvasSize { width, height }, fps))
}

pub fn project_servers() -> Vec<String> {
    let store = shrimply_editor_state::preferences::open_with_defaults();
    shrimply_editor_state::preferences::snapshot(&store).project_server_urls
}

pub fn update_project_servers(previous: Option<&str>, value: Option<&str>) -> Result<(), String> {
    let store = shrimply_editor_state::preferences::open_with_defaults();
    shrimply_editor_state::preferences::update_project_server(&store, previous, value)
}

pub fn create_shared_project(
    server: &str,
    name: &str,
    canvas_size: CanvasSize,
    fps: Fraction,
    tags: &[String],
) -> Result<PathBuf, String> {
    let project =
        project::creation::NewProject::from_settings(name, canvas_size, fps, tags)?.into_document();
    let path =
        shrimply_project_service::SharedWorkspace::managed_path(&default_project_filename(name))?;
    shrimply_project_service::SharedWorkspace::create(&path, server, &project)?;
    Ok(path)
}

pub fn shared_project_link(path: &Path) -> Result<Option<String>, String> {
    Ok(
        shrimply_project_service::SharedWorkspace::open(path)?.map(|workspace| {
            let link = workspace.link();
            format!(
                "{}/v1/projects/{}",
                link.server.trim_end_matches('/'),
                link.project_id
            )
        }),
    )
}

pub fn open_shared_project(link: &str) -> Result<PathBuf, String> {
    let (server, id) = link
        .trim()
        .rsplit_once("/v1/projects/")
        .ok_or("Enter a Shrimply project link")?;
    let server = shrimply_editor_state::preferences::normalize_project_server_url(server)?;
    if !project_servers().contains(&server) {
        return Err(format!(
            "Add {server} in Project Servers before opening its project link"
        ));
    }
    let id = uuid::Uuid::parse_str(id).map_err(|_| "The project link has an invalid ID")?;
    let path = shrimply_project_service::SharedWorkspace::managed_path(&format!("{id}.shrimp"))?;
    let (_, document) = shrimply_project_service::SharedWorkspace::join(&path, &server, id)?;
    recent_projects::touch(&path, &document.name, &document.tags)?;
    Ok(path)
}

pub fn default_project_filename(name: &str) -> String {
    let safe_name = project::creation::filename_stem(name);
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
    recent_projects::modified_at(path).and_then(format_modified)
}

pub fn recent_last_edited(recent: &RecentProject) -> Option<String> {
    recent.modified_at.and_then(format_modified)
}

pub fn is_shared_project(path: &Path) -> bool {
    shrimply_project_service::SharedWorkspace::database(path).is_file()
}

fn format_modified(seconds: i64) -> Option<String> {
    glib::DateTime::from_unix_local(seconds)
        .ok()
        .and_then(|date| date.format("%x %X").ok())
        .map(|date| date.to_string())
}

#[cfg(target_os = "macos")]
pub fn launch_appkit_editor(path: &Path) -> Result<Child, String> {
    if !path.is_file() {
        return Err(format!("Project does not exist: {}", path.display()));
    }
    let editor = std::env::current_exe().map_err(|error| format!("locate Shrimply: {error}"))?;
    Command::new(&editor)
        .arg(path)
        .spawn()
        .map_err(|error| format!("could not launch {}: {error}", editor.display()))
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
    launch_sibling_editor(path, "shrimply-editor")
}

fn launch_sibling_editor(path: &Path, binary_name: &str) -> Result<Child, String> {
    if !path.is_file() {
        return Err(format!("Project does not exist: {}", path.display()));
    }
    let editor = std::env::current_exe()
        .map(|path| path.with_file_name(binary_name))
        .unwrap_or_else(|_| PathBuf::from(binary_name));
    Command::new(&editor)
        .arg(path)
        .spawn()
        .map_err(|error| format!("could not launch {}: {error}", editor.display()))
}

pub fn project_file_name_filter(label: &str) -> String {
    format!("{label} ({})", PROJECT_FILE_PATTERNS.join(" "))
}

pub fn preset_labels() -> Vec<&'static str> {
    shrimply_project_types::PROJECT_PRESETS
        .iter()
        .map(|preset| preset.label)
        .chain(["Custom"])
        .collect()
}

pub fn frame_rate_labels() -> Vec<&'static str> {
    shrimply_project_types::COMMON_FRAME_RATES
        .iter()
        .map(|rate| rate.label)
        .collect()
}

pub fn preset_width(index: usize) -> u32 {
    shrimply_project_types::PROJECT_PRESETS
        .get(index)
        .map(|preset| preset.canvas_size.width)
        .unwrap_or(shrimply_project_types::DEFAULT_CANVAS_SIZE.width)
}

pub fn preset_height(index: usize) -> u32 {
    shrimply_project_types::PROJECT_PRESETS
        .get(index)
        .map(|preset| preset.canvas_size.height)
        .unwrap_or(shrimply_project_types::DEFAULT_CANVAS_SIZE.height)
}

pub fn preset_frame_rate(index: usize) -> usize {
    shrimply_project_types::PROJECT_PRESETS
        .get(index)
        .and_then(|preset| {
            shrimply_project_types::COMMON_FRAME_RATES
                .iter()
                .position(|rate| rate.value == preset.fps)
        })
        .unwrap_or_else(|| {
            shrimply_project_types::COMMON_FRAME_RATES
                .iter()
                .position(|rate| rate.value == shrimply_project_types::DEFAULT_PROJECT_FPS)
                .unwrap_or(0)
        })
}
