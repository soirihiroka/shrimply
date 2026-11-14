use super::*;
use std::ffi::CStr;

/// The shared shaders use Vulkan 1.2 scalar layout, byte storage and device
/// addresses. Android API level alone says nothing about those capabilities.
pub const API_VERSION: u32 = vk::API_VERSION_1_2;

pub struct HardwareBufferProperties {
    pub allocation_size: u64,
    pub memory_type_bits: u32,
    pub format: vk::AndroidHardwareBufferFormatPropertiesANDROID<'static>,
}

impl Device {
    pub fn hardware_buffer_properties(
        &self,
        buffer: &ndk::hardware_buffer::HardwareBuffer,
    ) -> Result<HardwareBufferProperties, String> {
        let loader = ash::android::external_memory_android_hardware_buffer::Device::new(
            &self.instance,
            &self.raw,
        );
        let mut format = vk::AndroidHardwareBufferFormatPropertiesANDROID::default();
        let mut properties =
            vk::AndroidHardwareBufferPropertiesANDROID::default().push_next(&mut format);
        unsafe {
            loader.get_android_hardware_buffer_properties(buffer.as_ptr().cast(), &mut properties)
        }
        .map_err(|error| format!("Query decoder hardware allocation: {error:?}"))?;
        let allocation_size = properties.allocation_size;
        let memory_type_bits = properties.memory_type_bits;
        Ok(HardwareBufferProperties {
            allocation_size,
            memory_type_bits,
            format,
        })
    }

    pub(super) fn new() -> Result<Self, String> {
        let entry =
            unsafe { ash::Entry::load() }.map_err(|error| format!("load Vulkan: {error}"))?;
        let version = unsafe { entry.try_enumerate_instance_version() }
            .map_err(|error| format!("query Vulkan loader: {error:?}"))?
            .unwrap_or(vk::API_VERSION_1_0);
        if version < API_VERSION {
            return Err("The shared GPU renderer requires Vulkan 1.2".into());
        }
        let extensions = [
            ash::khr::surface::NAME.as_ptr(),
            ash::khr::android_surface::NAME.as_ptr(),
        ];
        let application = vk::ApplicationInfo::default()
            .application_name(c"Shrimply")
            .api_version(API_VERSION);
        let instance = unsafe {
            entry.create_instance(
                &vk::InstanceCreateInfo::default()
                    .application_info(&application)
                    .enabled_extension_names(&extensions),
                None,
            )
        }
        .map_err(|error| format!("create Vulkan instance: {error:?}"))?;
        let result = (|| {
            let required_extensions = [
                ash::khr::swapchain::NAME,
                ash::android::external_memory_android_hardware_buffer::NAME,
                ash::khr::external_semaphore_fd::NAME,
                ash::ext::queue_family_foreign::NAME,
            ];
            let devices = unsafe { instance.enumerate_physical_devices() }
                .map_err(|error| format!("enumerate Vulkan devices: {error:?}"))?;
            for physical in devices {
                let properties = unsafe { instance.get_physical_device_properties(physical) };
                if properties.api_version < API_VERSION
                    || properties.device_type == vk::PhysicalDeviceType::CPU
                {
                    continue;
                }
                let extensions =
                    unsafe { instance.enumerate_device_extension_properties(physical) }
                        .map_err(|error| format!("query Vulkan extensions: {error:?}"))?;
                if required_extensions.iter().any(|required| !extensions.iter().any(|extension| unsafe { CStr::from_ptr(extension.extension_name.as_ptr()) } == *required)) { continue; }
                let mut features11 = vk::PhysicalDeviceVulkan11Features::default();
                let mut features12 = vk::PhysicalDeviceVulkan12Features::default();
                let mut features = vk::PhysicalDeviceFeatures2::default()
                    .push_next(&mut features11)
                    .push_next(&mut features12);
                unsafe { instance.get_physical_device_features2(physical, &mut features) };
                let core = features.features;
                if core.shader_int16 == 0
                    || features11.sampler_ycbcr_conversion == 0
                    || features12.buffer_device_address == 0
                    || features12.scalar_block_layout == 0
                    || features12.storage_buffer8_bit_access == 0
                    || features12.shader_int8 == 0
                {
                    continue;
                }
                let families =
                    unsafe { instance.get_physical_device_queue_family_properties(physical) };
                let Some(family) = families.iter().position(|family| {
                    family.queue_count > 0
                        && family
                            .queue_flags
                            .contains(vk::QueueFlags::GRAPHICS | vk::QueueFlags::COMPUTE)
                }) else {
                    continue;
                };
                let family = family as u32;
                let priorities = [1.0];
                let queues = [vk::DeviceQueueCreateInfo::default()
                    .queue_family_index(family)
                    .queue_priorities(&priorities)];
                let mut extension_names = required_extensions.map(CStr::as_ptr).to_vec();
                let presentation_timing = extensions.iter().any(|extension| unsafe { CStr::from_ptr(extension.extension_name.as_ptr()) } == ash::google::display_timing::NAME);
                if presentation_timing {
                    extension_names.push(ash::google::display_timing::NAME.as_ptr());
                }
                let mut enabled = vk::PhysicalDeviceVulkan12Features::default()
                    .buffer_device_address(true)
                    .scalar_block_layout(true)
                    .storage_buffer8_bit_access(true)
                    .shader_int8(true)
                    .timeline_semaphore(features12.timeline_semaphore != 0)
                    .storage_push_constant8(features12.storage_push_constant8 != 0);
                let mut enabled11 =
                    vk::PhysicalDeviceVulkan11Features::default().sampler_ycbcr_conversion(true);
                let core = vk::PhysicalDeviceFeatures::default()
                    .shader_int64(core.shader_int64 != 0)
                    .shader_int16(true);
                let raw = unsafe {
                    instance.create_device(
                        physical,
                        &vk::DeviceCreateInfo::default()
                            .queue_create_infos(&queues)
                            .enabled_extension_names(&extension_names)
                            .enabled_features(&core)
                            .push_next(&mut enabled11)
                            .push_next(&mut enabled),
                        None,
                    )
                }
                .map_err(|error| format!("create shared Vulkan device: {error:?}"))?;
                let queue = unsafe { raw.get_device_queue(family, 0) };
                let memory = unsafe { instance.get_physical_device_memory_properties(physical) };
                return Ok(Self {
                    instance: instance.clone(),
                    raw,
                    physical,
                    queue,
                    family,
                    memory,
                    limits: properties.limits,
                    compute_subgroup: None,
                    api_version: API_VERSION,
                    presentation_timing,
                    portable_indices: core.shader_int64 == 0
                        || features12.storage_push_constant8 == 0,
                    submission: Mutex::new(()),
                    pipeline_cache: Mutex::new(None),
                    entry: entry.clone(),
                });
            }
            Err("No GPU supports the shared renderer's Vulkan 1.2 shader features and Android hardware-buffer interop".into())
        })();
        if result.is_err() {
            unsafe { instance.destroy_instance(None) };
        }
        result
    }
}
