use shrimply_math_core::Fraction;
use shrimply_project_document::project;
pub use shrimply_project_service::catalog::{ProjectEntry, list as load_projects, local_projects};
use shrimply_project_types::CanvasSize;
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
    projects: Vec<ProjectEntry>,
    visible: Vec<usize>,
    project_load: u64,
}

impl Launcher {
    pub fn set_projects(&mut self, projects: Vec<ProjectEntry>) {
        self.projects = projects;
        self.set_search(&self.query.clone());
    }

    pub fn begin_project_load(&mut self) -> u64 {
        self.project_load += 1;
        self.project_load
    }

    pub fn finish_project_load(
        &mut self,
        load: u64,
        projects: Result<Vec<ProjectEntry>, String>,
    ) -> Result<bool, String> {
        if load != self.project_load {
            return Ok(false);
        }
        self.set_projects(projects?);
        Ok(true)
    }

    pub fn set_search(&mut self, query: &str) {
        self.query = query.trim().to_lowercase();
        self.visible = self
            .projects
            .iter()
            .enumerate()
            .filter_map(|(index, project)| project.matches(&self.query).then_some(index))
            .collect();
    }

    pub fn project_count(&self) -> usize {
        self.visible.len()
    }

    pub fn project(&self, index: usize) -> Option<&ProjectEntry> {
        self.visible.get(index).map(|index| &self.projects[*index])
    }
}

pub enum ProjectLocation {
    Local(PathBuf),
    Remote(String),
}

pub fn create_project(
    location: ProjectLocation,
    name: &str,
    (canvas_size, fps): (CanvasSize, Fraction),
    tags: &[String],
) -> Result<PathBuf, String> {
    let project = project::creation::NewProject::from_settings(name, canvas_size, fps, tags)?;
    match location {
        ProjectLocation::Local(path) => project.write(path, true),
        ProjectLocation::Remote(server) => {
            use shrimply_project_service::SharedWorkspace;
            let path = SharedWorkspace::managed_path(&default_project_filename(name))?;
            SharedWorkspace::create(&path, &server, &project.into_document())?;
            Ok(path)
        }
    }
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
