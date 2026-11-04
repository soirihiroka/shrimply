use shrimply_gpu_cuda::sys;
use std::sync::OnceLock;

static INTEL: OnceLock<bool> = OnceLock::new();

/// GTK selects once, before any render worker starts. Existing CUDA hosts keep
/// their current backend; SHRIMPLY_GPU=intel explicitly selects the Intel GPU.
pub fn configure_gtk_backend() -> Result<(), String> {
    let preference = if std::env::var_os("SHRIMPLY_GPU").is_none() {
        shrimply_preferences_core::snapshot(&shrimply_preferences_core::open_with_defaults())
            .acceleration_device
    } else {
        String::new()
    };
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
    tracing::info!(
        backend = if intel { "intel" } else { "cuda" },
        "GTK GPU backend selected"
    );
    Ok(())
}

pub fn intel_backend_selected() -> bool {
    INTEL.get().copied().unwrap_or(false)
}
