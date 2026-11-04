use ash::{vk, vk::Handle};
use ffmpeg_next::sys;
use std::{
    ffi::CString,
    ptr,
    sync::{Arc, Mutex, OnceLock},
};

/// VA-API and Vulkan share one DRM device. FFmpeg owns both native contexts.
pub struct Device {
    pub instance: ash::Instance,
    pub raw: ash::Device,
    pub physical: vk::PhysicalDevice,
    pub queue: vk::Queue,
    pub family: u32,
    pub memory: vk::PhysicalDeviceMemoryProperties,
    pub submission: Mutex<()>,
    pub va: *mut sys::AVBufferRef,
    pub vulkan: *mut sys::AVBufferRef,
    pub(crate) _entry: ash::Entry,
}

// Native handles are shared immutably; our queue is separate from FFmpeg's queue.
unsafe impl Send for Device {}
unsafe impl Sync for Device {}

impl Device {
    pub fn get() -> Result<Arc<Self>, String> {
        static DEVICE: OnceLock<Result<Arc<Device>, String>> = OnceLock::new();
        DEVICE.get_or_init(|| Self::new().map(Arc::new)).clone()
    }

    fn new() -> Result<Self, String> {
        ffmpeg_next::init().map_err(|e| e.to_string())?;
        let path = std::env::var("SHRIMPLY_INTEL_DEVICE")
            .unwrap_or_else(|_| "/dev/dri/by-path/pci-0000:00:02.0-render".into());
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
                .find(|family| family.flags & vk::QueueFlags::COMPUTE.as_raw() != 0)
                .ok_or("Intel Vulkan device has no compute queue")?;
            // This context is used only for importing VA surfaces. All Vulkan
            // commands are submitted by our render worker, never a codec/filter.
            let family = family.idx as u32;
            let queue = unsafe { raw.get_device_queue(family, 0) };
            let memory = unsafe { instance.get_physical_device_memory_properties(physical) };
            let properties = unsafe { instance.get_physical_device_properties(physical) };
            if properties.vendor_id != 0x8086 {
                return Err("VA-API render device is not an Intel GPU".into());
            }
            tracing::info!(device = %unsafe { std::ffi::CStr::from_ptr(properties.device_name.as_ptr()) }.to_string_lossy(), "Intel Vulkan renderer initialized");
            Ok(Self {
                instance,
                raw,
                physical,
                queue,
                family,
                memory,
                submission: Mutex::new(()),
                va,
                vulkan,
                _entry: entry,
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

    pub fn submit(
        &self,
        commands: &[vk::CommandBuffer],
        fence: vk::Fence,
        signals: &[vk::Semaphore],
    ) -> Result<(), String> {
        let _guard = self
            .submission
            .lock()
            .expect("Vulkan submission mutex poisoned");
        unsafe {
            self.raw.queue_submit(
                self.queue,
                &[vk::SubmitInfo::default()
                    .command_buffers(commands)
                    .signal_semaphores(signals)],
                fence,
            )
        }
        .map_err(|e| format!("submit Intel Vulkan commands: {e:?}"))
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        if unsafe { self.raw.device_wait_idle() }.is_err() {
            std::process::abort();
        }
        unsafe {
            sys::av_buffer_unref(&mut self.vulkan);
            sys::av_buffer_unref(&mut self.va);
        }
    }
}
