use ash::{vk, vk::Handle};
use ffmpeg_next::sys;
use std::{
    ffi::CString,
    ptr,
    sync::{Arc, Mutex, MutexGuard, OnceLock},
};

/// VA-API and Vulkan share one DRM device. FFmpeg owns both native contexts.
pub struct Device {
    pub instance: ash::Instance,
    pub raw: ash::Device,
    pub physical: vk::PhysicalDevice,
    pub queue: vk::Queue,
    pub family: u32,
    pub memory: vk::PhysicalDeviceMemoryProperties,
    pub limits: vk::PhysicalDeviceLimits,
    submission: Mutex<()>,
    pub va: *mut sys::AVBufferRef,
    pub vulkan: *mut sys::AVBufferRef,
    pub(crate) _entry: ash::Entry,
}

pub(crate) struct QueueLock<'a> {
    device: &'a Device,
    _guard: MutexGuard<'a, ()>,
}

impl Drop for QueueLock<'_> {
    fn drop(&mut self) {
        unsafe {
            let context = (*self.device.vulkan).data.cast::<sys::AVHWDeviceContext>();
            let native = &*(*context).hwctx.cast::<sys::AVVulkanDeviceContext>();
            if let Some(unlock) = native.unlock_queue {
                unlock(context, self.device.family, 0);
            }
        }
    }
}

// Native handles are shared immutably; queue submissions are serialized.
unsafe impl Send for Device {}
unsafe impl Sync for Device {}

impl Device {
    pub fn get() -> Result<Arc<Self>, String> {
        static DEVICE: OnceLock<Result<Arc<Device>, String>> = OnceLock::new();
        DEVICE.get_or_init(|| Self::new().map(Arc::new)).clone()
    }

    fn new() -> Result<Self, String> {
        ffmpeg_next::init().map_err(|e| e.to_string())?;
        let path = match std::env::var("SHRIMPLY_INTEL_DEVICE") {
            Ok(path) => path,
            Err(std::env::VarError::NotPresent) => {
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
            Err(error) => return Err(format!("Invalid SHRIMPLY_INTEL_DEVICE: {error}")),
        };
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
                .find(|family| {
                    family.flags & (vk::QueueFlags::COMPUTE | vk::QueueFlags::GRAPHICS).as_raw()
                        == (vk::QueueFlags::COMPUTE | vk::QueueFlags::GRAPHICS).as_raw()
                })
                .ok_or("Intel Vulkan device has no graphics/compute queue")?;
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
                limits: properties.limits,
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

    pub(crate) fn queue_lock(&self) -> QueueLock<'_> {
        let guard = self
            .submission
            .lock()
            .expect("Vulkan submission mutex poisoned");
        unsafe {
            let context = (*self.vulkan).data.cast::<sys::AVHWDeviceContext>();
            let native = &*(*context).hwctx.cast::<sys::AVVulkanDeviceContext>();
            if let Some(lock) = native.lock_queue {
                lock(context, self.family, 0);
            }
        }
        QueueLock {
            device: self,
            _guard: guard,
        }
    }

    pub fn submit(
        &self,
        commands: &[vk::CommandBuffer],
        fence: vk::Fence,
        signals: &[vk::Semaphore],
    ) -> Result<(), String> {
        let _guard = self.queue_lock();
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
