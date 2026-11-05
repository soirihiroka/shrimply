use shrimply_gpu_cuda::sys;
use std::os::unix::process::CommandExt;
use std::sync::OnceLock;

static INTEL: OnceLock<bool> = OnceLock::new();
const VULKAN_PREFERENCE_BOOTSTRAP: &str = "SHRIMPLY_VULKAN_PREFERENCE_BOOTSTRAP";

/// GTK selects once, before any render worker starts. Existing CUDA hosts keep
/// their current backend; SHRIMPLY_GPU=intel explicitly selects the Intel GPU.
pub fn configure_gtk_backend() -> Result<(), String> {
    let preference = if std::env::var_os("SHRIMPLY_GPU").is_none() {
        shrimply_preferences_core::snapshot(&shrimply_preferences_core::open_with_defaults())
            .acceleration_device
    } else {
        String::new()
    };
    let intel_preference = preference
        .starts_with("intel:")
        .then_some(preference.as_str());
    // Resolving the saved UUID also enumerates Vulkan. Filter before that work,
    // but remember the attempt through CUDA fallback if this device disappeared.
    if INTEL.get().is_none()
        && intel_preference.is_some()
        && std::env::var_os(VULKAN_PREFERENCE_BOOTSTRAP).as_deref()
            != intel_preference.map(std::ffi::OsStr::new)
    {
        configure_vulkan_loader(true, intel_preference)?;
    }
    let selected = if preference.is_empty() {
        None
    } else {
        let selected = super::devices::acceleration_devices()?
            .into_iter()
            .find(|device| device.id == preference);
        if selected.is_none() {
            tracing::warn!(device = %preference, "Saved acceleration device is unavailable; selecting automatically");
        }
        selected
    };
    let preferred_intel = match selected.map(|device| device.selection) {
        Some(super::devices::Selection::Cuda(ordinal)) => {
            shrimply_gpu_cuda::select_device(ordinal)?;
            Some(false)
        }
        Some(super::devices::Selection::Intel(path)) => {
            shrimply_render_vulkan::device::Device::select_render_device(path)?;
            Some(true)
        }
        None => None,
    };
    let intel = if let Some(intel) = preferred_intel {
        intel
    } else {
        match std::env::var("SHRIMPLY_GPU").as_deref() {
            Ok("intel") => true,
            Ok("cuda") => false,
            Ok("auto") | Err(std::env::VarError::NotPresent) => {
                let mut device = 0;
                unsafe {
                    sys::cuInit(0) != sys::CUDA_SUCCESS
                        || sys::cuDeviceGet(&mut device, 0) != sys::CUDA_SUCCESS
                }
            }
            value => {
                return Err(format!(
                    "Invalid SHRIMPLY_GPU value {value:?}; expected auto, cuda or intel"
                ));
            }
        }
    };
    if INTEL.set(intel).is_err() && INTEL.get() != Some(&intel) {
        return Err("GTK GPU backend cannot change after initialization".into());
    }
    configure_vulkan_loader(intel, intel_preference)?;
    tracing::info!(
        backend = if intel { "intel" } else { "cuda" },
        "GTK GPU backend selected"
    );
    Ok(())
}

pub fn intel_backend_selected() -> bool {
    INTEL.get().copied().unwrap_or(false)
}

/// Intel rendering does not use NVIDIA's Vulkan ICD. Avoid loading it unless
/// the user selected their own loader configuration, and undo only our filter
/// when restarting with CUDA. Re-exec keeps driver thread environments intact.
fn configure_vulkan_loader(intel: bool, intel_preference: Option<&str>) -> Result<(), String> {
    const DISABLE: &str = "VK_LOADER_DRIVERS_DISABLE";
    const MANAGED: &str = "SHRIMPLY_MANAGED_VK_DRIVERS_DISABLE";
    const NVIDIA: &str = "*nvidia*";
    let disabled = std::env::var_os(DISABLE);
    let marker = std::env::var_os(MANAGED);
    let managed = marker.as_deref() == Some(std::ffi::OsStr::new(NVIDIA)) && disabled == marker;
    let explicit = [
        "VK_LOADER_DRIVERS_SELECT",
        "VK_DRIVER_FILES",
        "VK_ICD_FILENAMES",
        "VK_ADD_DRIVER_FILES",
    ]
    .iter()
    .any(|key| std::env::var_os(key).is_some())
        || disabled.is_some() && !managed;
    let filter = intel && !explicit;
    let environment = [
        (
            DISABLE,
            if filter {
                Some(std::ffi::OsStr::new(NVIDIA))
            } else if managed {
                None
            } else {
                disabled.as_deref()
            },
        ),
        (MANAGED, filter.then(|| std::ffi::OsStr::new(NVIDIA))),
        (
            VULKAN_PREFERENCE_BOOTSTRAP,
            if explicit {
                None
            } else {
                intel_preference.map(std::ffi::OsStr::new)
            },
        ),
    ];
    if environment
        .iter()
        .all(|(key, value)| std::env::var_os(key).as_deref() == *value)
    {
        return Ok(());
    }
    let mut command =
        std::process::Command::new(std::env::current_exe().map_err(|error| error.to_string())?);
    command.args(std::env::args_os().skip(1));
    for (key, value) in environment {
        if let Some(value) = value {
            command.env(key, value);
        } else {
            command.env_remove(key);
        }
    }
    Err(format!(
        "Restart with selected Vulkan drivers: {}",
        command.exec()
    ))
}

/// Apply native PRIME selection before GTK loads its OpenGL driver. Re-exec
/// instead of changing the process environment after GPU driver threads exist.
pub fn configure_gtk_presentation() -> Result<(), String> {
    if INTEL.get().is_none() {
        return Err("Select the GTK GPU backend before configuring presentation".into());
    }
    let intel = intel_backend_selected();
    let prime = if intel {
        let path = shrimply_render_vulkan::device::Device::render_device_path()?
            .canonicalize()
            .map_err(|error| format!("Resolve Intel DRM render device: {error}"))?;
        let node = path.file_name().ok_or("Intel DRM device has no name")?;
        let pci = std::path::Path::new("/sys/class/drm")
            .join(node)
            .join("device")
            .canonicalize()
            .map_err(|error| format!("Find Intel presentation device: {error}"))?;
        let address = pci
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("Intel PCI address is not UTF-8")?;
        Some(format!("pci-{}", address.replace([':', '.'], "_")))
    } else {
        None
    };
    let mut command =
        std::process::Command::new(std::env::current_exe().map_err(|error| error.to_string())?);
    command.args(std::env::args_os().skip(1));
    let mut restart = false;
    let provider = std::env::var("__NV_PRIME_RENDER_OFFLOAD_PROVIDER").ok();
    for (key, value) in [
        ("DRI_PRIME", prime.as_deref()),
        (
            "__NV_PRIME_RENDER_OFFLOAD",
            Some(if intel { "0" } else { "1" }),
        ),
        (
            "__GLX_VENDOR_LIBRARY_NAME",
            Some(if intel { "mesa" } else { "nvidia" }),
        ),
        (
            "__NV_PRIME_RENDER_OFFLOAD_PROVIDER",
            provider.as_deref().filter(|_| !intel),
        ),
    ] {
        if std::env::var_os(key).as_deref() == value.map(std::ffi::OsStr::new) {
            continue;
        }
        restart = true;
        if let Some(value) = value {
            command.env(key, value);
        } else {
            command.env_remove(key);
        }
    }
    // GTK's UI and GLArea can share one OpenGL device and context family.
    // Retain an explicitly requested GSK renderer.
    if std::env::var_os("GSK_RENDERER").is_none() {
        command.env("GSK_RENDERER", "gl");
        restart = true;
    }
    // GLArea requires desktop OpenGL. NVIDIA cannot share its textures with
    // GTK's default GLES context, so select the same API for the whole display.
    let disabled = std::env::var("GDK_DISABLE").unwrap_or_default();
    if !disabled
        .split([':', ';', ',', ' ', '\t'])
        .any(|feature| feature == "gles-api" || feature == "all")
    {
        command.env(
            "GDK_DISABLE",
            if disabled.is_empty() {
                "gles-api".into()
            } else {
                format!("{disabled},gles-api")
            },
        );
        restart = true;
    }
    if restart {
        return Err(format!(
            "Restart with selected GTK presentation device: {}",
            command.exec()
        ));
    }
    Ok(())
}
