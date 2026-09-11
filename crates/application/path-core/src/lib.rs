use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!("shrimply-path-core supports only Linux and macOS");

#[cfg(target_os = "linux")]
const APPLICATION_DIRECTORY: &str = "shrimply";
#[cfg(all(target_os = "macos", debug_assertions))]
const MACOS_APPLICATION_DIRECTORY: &str = "Shrimply Debug";
#[cfg(all(target_os = "macos", not(debug_assertions)))]
const MACOS_APPLICATION_DIRECTORY: &str = "Shrimply";

#[cached::proc_macro::cached]
pub fn config_directory() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        user_directory("XDG_CONFIG_HOME", ".config")
            .expect("XDG_CONFIG_HOME or HOME must provide a settings directory")
            .join(APPLICATION_DIRECTORY)
    }

    #[cfg(target_os = "macos")]
    {
        macos_directory(objc2_foundation::NSSearchPathDirectory::ApplicationSupportDirectory)
    }
}

#[cached::proc_macro::cached]
pub fn cache_directory() -> Result<PathBuf, String> {
    #[cfg(target_os = "linux")]
    {
        user_directory("XDG_CACHE_HOME", ".cache")
            .map(|directory| directory.join(APPLICATION_DIRECTORY))
            .ok_or_else(|| "neither XDG_CACHE_HOME nor HOME is set".to_string())
    }

    #[cfg(target_os = "macos")]
    {
        Ok(macos_directory(
            objc2_foundation::NSSearchPathDirectory::CachesDirectory,
        ))
    }
}

static ACTIVE_PROJECT_PATH: OnceLock<RwLock<PathBuf>> = OnceLock::new();

pub fn set_active_project_path(path: &Path) {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    *ACTIVE_PROJECT_PATH
        .get_or_init(|| RwLock::new(PathBuf::new()))
        .write()
        .unwrap_or_else(|_| panic!("active project path lock died")) = path;
}

pub fn active_project_path() -> PathBuf {
    ACTIVE_PROJECT_PATH
        .get_or_init(|| RwLock::new(PathBuf::new()))
        .read()
        .unwrap_or_else(|_| panic!("active project path lock died"))
        .clone()
}

pub fn project_directory() -> PathBuf {
    active_project_path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

pub fn project_cache_directory() -> PathBuf {
    let project_directory = project_directory();
    let root = if project_directory
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "shrimp")
    {
        project_directory
            .parent()
            .expect("shrimp project directory must have a parent")
    } else {
        &project_directory
    };
    root.join("media/.cache")
}

#[cfg(target_os = "linux")]
fn user_directory(xdg: &str, home_fallback: &str) -> Option<PathBuf> {
    std::env::var_os(xdg)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(home_fallback)))
}

#[cfg(target_os = "macos")]
fn macos_directory(directory: objc2_foundation::NSSearchPathDirectory) -> PathBuf {
    use objc2_foundation::{NSFileManager, NSSearchPathDomainMask};

    let url = NSFileManager::defaultManager()
        .URLForDirectory_inDomain_appropriateForURL_create_error(
            directory,
            NSSearchPathDomainMask::UserDomainMask,
            None,
            true,
        )
        .expect("macOS application directory should be available");
    let path = url
        .path()
        .expect("macOS application directory URL should be a file path");
    PathBuf::from(path.to_string()).join(MACOS_APPLICATION_DIRECTORY)
}
