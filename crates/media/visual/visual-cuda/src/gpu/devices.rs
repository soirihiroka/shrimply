use ash::{Entry, vk};
use std::{ffi::CStr, os::unix::fs::MetadataExt, path::PathBuf};

pub struct AccelerationDevice {
    pub id: String,
    pub name: String,
    pub(super) selection: Selection,
}

pub(super) enum Selection {
    Cuda(usize),
    Intel(PathBuf),
}

pub fn acceleration_devices() -> Result<Vec<AccelerationDevice>, String> {
    let mut devices = shrimply_gpu_cuda::devices()
        .unwrap_or_default()
        .into_iter()
        .map(|device| AccelerationDevice {
            id: format!("cuda:{}", uuid::Uuid::from_bytes(device.uuid)),
            name: device.name,
            selection: Selection::Cuda(device.ordinal),
        })
        .collect::<Vec<_>>();
    match intel_devices() {
        Ok(intel) => devices.extend(intel),
        Err(error) if devices.is_empty() => return Err(error),
        Err(error) => tracing::warn!(%error, "Could not enumerate Intel acceleration devices"),
    }
    Ok(devices)
}

fn intel_devices() -> Result<Vec<AccelerationDevice>, String> {
    let entry = unsafe { Entry::load() }.map_err(|error| error.to_string())?;
    let application = vk::ApplicationInfo::default().api_version(vk::API_VERSION_1_2);
    let instance = unsafe {
        entry.create_instance(
            &vk::InstanceCreateInfo::default().application_info(&application),
            None,
        )
    }
    .map_err(|error| error.to_string())?;
    let result = (|| {
        let nodes = std::fs::read_dir("/dev/dri")
            .map_err(|error| error.to_string())?
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("renderD"))
            .map(|entry| entry.path())
            .collect::<Vec<_>>();
        let mut result = Vec::new();
        let mut render_devices = std::collections::HashSet::new();
        for physical in
            unsafe { instance.enumerate_physical_devices() }.map_err(|error| error.to_string())?
        {
            let base = unsafe { instance.get_physical_device_properties(physical) };
            if base.vendor_id != 0x8086 {
                continue;
            }
            let extensions = unsafe { instance.enumerate_device_extension_properties(physical) }
                .map_err(|error| error.to_string())?;
            if !extensions.iter().any(|extension| unsafe { CStr::from_ptr(extension.extension_name.as_ptr()) } == ash::ext::physical_device_drm::NAME) { continue; }
            let mut identity = vk::PhysicalDeviceIDProperties::default();
            let mut drm = vk::PhysicalDeviceDrmPropertiesEXT::default();
            let mut properties = vk::PhysicalDeviceProperties2::default()
                .push_next(&mut identity)
                .push_next(&mut drm);
            unsafe {
                instance.get_physical_device_properties2(physical, &mut properties);
            }
            if drm.has_render == 0 {
                continue;
            }
            let Some(path) = nodes.iter().find(|path| {
                path.metadata().is_ok_and(|metadata| {
                    libc::major(metadata.rdev()) == drm.render_major as u32
                        && libc::minor(metadata.rdev()) == drm.render_minor as u32
                })
            }) else {
                continue;
            };
            // Multiple ICDs can expose the same physical DRM render device.
            if !render_devices.insert((drm.render_major, drm.render_minor)) {
                continue;
            }
            result.push(AccelerationDevice {
                id: format!("intel:{}", uuid::Uuid::from_bytes(identity.device_uuid)),
                name: unsafe { CStr::from_ptr(base.device_name.as_ptr()) }
                    .to_string_lossy()
                    .into_owned(),
                selection: Selection::Intel(path.clone()),
            });
        }
        Ok(result)
    })();
    unsafe {
        instance.destroy_instance(None);
    }
    result
}
