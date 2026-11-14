use super::*;
use ash::vk::Handle;
use ffmpeg_next::sys;
use std::{ffi::CString, ptr};

static SELECTED_DEVICE: OnceLock<std::path::PathBuf> = OnceLock::new();

impl Device {
    pub fn select_render_device(path: std::path::PathBuf) -> Result<(), String> {
        if SELECTED_DEVICE.set(path.clone()).is_err() && SELECTED_DEVICE.get() != Some(&path) {
            return Err("Intel GPU device cannot change after startup".into());
        }
        Ok(())
    }
    pub fn render_device_path() -> Result<std::path::PathBuf, String> {
        let path = match std::env::var("SHRIMPLY_INTEL_DEVICE") {
            Ok(path) => path,
            Err(std::env::VarError::NotPresent) => {
                if let Some(path) = SELECTED_DEVICE.get() {
                    path.to_str()
                        .ok_or("Intel DRM path is not UTF-8")?
                        .to_owned()
                } else {
                    let mut devices = std::fs::read_dir("/dev/dri")
                        .map_err(|e| format!("Find Intel DRM render device: {e}"))?
                        .filter_map(Result::ok)
                        .filter(|entry| {
                            let name = entry.file_name();
                            let name = name.to_string_lossy();
                            name.starts_with("renderD")
                                && std::fs::read_to_string(format!(
                                    "/sys/class/drm/{name}/device/vendor"
                                ))
                                .is_ok_and(|vendor| vendor.trim() == "0x8086")
                        })
                        .map(|entry| entry.path())
                        .collect::<Vec<_>>();
                    devices.sort();
                    devices
                        .first()
                        .ok_or("No Intel DRM render device found; set SHRIMPLY_INTEL_DEVICE")?
                        .to_str()
                        .ok_or("Intel DRM path is not UTF-8")?
                        .to_owned()
                }
            }
            Err(error) => return Err(format!("Invalid SHRIMPLY_INTEL_DEVICE: {error}")),
        };
        Ok(std::path::PathBuf::from(path))
    }

    pub(super) fn new() -> Result<Self, String> {
        ffmpeg_next::init().map_err(|e| e.to_string())?;
        let drm_path = Self::render_device_path()?;
        let path = drm_path.to_str().ok_or("Intel DRM path is not UTF-8")?;
        let path = CString::new(path).map_err(|e| e.to_string())?;
        let mut va = ptr::null_mut();
        let result = unsafe {
            sys::av_hwdevice_ctx_create(
                &mut va,
                sys::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI,
                path.as_ptr(),
                ptr::null_mut(),
                0,
            )
        };
        if result < 0 {
            return Err(format!(
                "create Intel VA-API device: {}",
                ffmpeg_next::Error::from(result)
            ));
        }
        let mut vulkan = ptr::null_mut();
        let result = unsafe {
            sys::av_hwdevice_ctx_create_derived(
                &mut vulkan,
                sys::AVHWDeviceType::AV_HWDEVICE_TYPE_VULKAN,
                va,
                0,
            )
        };
        if result < 0 {
            unsafe {
                sys::av_buffer_unref(&mut va);
            }
            return Err(format!(
                "derive Vulkan device from VA-API: {}",
                ffmpeg_next::Error::from(result)
            ));
        }
        let result = (|| {
            let context = unsafe { &*((*vulkan).data.cast::<sys::AVHWDeviceContext>()) };
            let native = unsafe { &*context.hwctx.cast::<sys::AVVulkanDeviceContext>() };
            let mut next = native.device_features.pNext.cast::<vk::BaseInStructure>();
            let mut shader_features = false;
            let mut subgroup_size_control = false;
            while let Some(header) = unsafe { next.as_ref() } {
                if header.s_type == vk::StructureType::PHYSICAL_DEVICE_VULKAN_1_2_FEATURES {
                    let features = unsafe { &*next.cast::<vk::PhysicalDeviceVulkan12Features>() };
                    shader_features = features.scalar_block_layout != 0
                        && features.buffer_device_address != 0
                        && features.storage_buffer8_bit_access != 0
                        && features.shader_int8 != 0;
                } else if header.s_type == vk::StructureType::PHYSICAL_DEVICE_VULKAN_1_3_FEATURES {
                    let features = unsafe { &*next.cast::<vk::PhysicalDeviceVulkan13Features>() };
                    subgroup_size_control = features.subgroup_size_control != 0;
                }
                next = header.p_next.cast_mut();
            }
            if !shader_features {
                return Err("FFmpeg's Vulkan device lacks required shader features; use FFmpeg 8+ or Shrimply's bundled FFmpeg with Vulkan scalar layout enabled".into());
            }
            let entry = unsafe { ash::Entry::load() }.map_err(|e| format!("load Vulkan: {e}"))?;
            let instance = unsafe {
                ash::Instance::load(
                    entry.static_fn(),
                    vk::Instance::from_raw(native.inst as u64),
                )
            };
            let physical = vk::PhysicalDevice::from_raw(native.phys_dev as u64);
            let raw = unsafe {
                ash::Device::load(
                    instance.fp_v1_0(),
                    vk::Device::from_raw(native.act_dev as u64),
                )
            };
            let family = native.qf[..native.nb_qf as usize]
                .iter()
                .find(|family| {
                    family.num > 0
                        && family.flags
                            & (vk::QueueFlags::COMPUTE | vk::QueueFlags::GRAPHICS).as_raw()
                            == (vk::QueueFlags::COMPUTE | vk::QueueFlags::GRAPHICS).as_raw()
                })
                .ok_or("Intel Vulkan device has no graphics/compute queue")?;
            let family = family.idx as u32;
            #[cfg(ffmpeg_vulkan_queue_flags)]
            let flags = vk::DeviceQueueCreateFlags::from_raw(native.queue_flags);
            #[cfg(not(ffmpeg_vulkan_queue_flags))]
            let flags = vk::DeviceQueueCreateFlags::empty();
            let queue = unsafe {
                raw.get_device_queue2(
                    &vk::DeviceQueueInfo2::default()
                        .flags(flags)
                        .queue_family_index(family)
                        .queue_index(0),
                )
            };
            if queue == vk::Queue::null() {
                return Err("FFmpeg's Intel Vulkan queue is unavailable".into());
            }
            let memory = unsafe { instance.get_physical_device_memory_properties(physical) };
            let mut subgroups = vk::PhysicalDeviceSubgroupSizeControlProperties::default();
            let mut properties = vk::PhysicalDeviceProperties2::default().push_next(&mut subgroups);
            unsafe { instance.get_physical_device_properties2(physical, &mut properties) };
            let properties = properties.properties;
            let compute_subgroup = (subgroup_size_control
                && subgroups
                    .required_subgroup_size_stages
                    .contains(vk::ShaderStageFlags::COMPUTE))
            .then_some((
                subgroups.min_subgroup_size,
                subgroups.max_compute_workgroup_subgroups,
            ));
            if properties.vendor_id != 0x8086 {
                return Err("VA-API render device is not an Intel GPU".into());
            }
            tracing::info!(
                device = %unsafe { std::ffi::CStr::from_ptr(properties.device_name.as_ptr()) }.to_string_lossy(),
                compute_subgroup_size = compute_subgroup.map(|(size, _)| size),
                "Intel Vulkan renderer initialized"
            );
            Ok(Self {
                instance,
                raw,
                physical,
                queue,
                family,
                memory,
                limits: properties.limits,
                compute_subgroup,
                portable_indices: false,
                api_version: vk::API_VERSION_1_3,
                submission: Mutex::new(()),
                pipeline_cache: Mutex::new(None),
                va,
                drm_path,
                vulkan,
                entry,
            })
        })();
        if result.is_err() {
            unsafe {
                sys::av_buffer_unref(&mut vulkan);
                sys::av_buffer_unref(&mut va);
            }
        }
        result
    }
}
