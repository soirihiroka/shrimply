use ash::vk;
#[cfg(target_os = "linux")]
use ffmpeg_next::sys;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
mod cache;

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "android")]
pub use android::{API_VERSION, HardwareBufferProperties};

/// A GPU and its serialized queue, shared by presentation and compute clients.
/// Linux retains FFmpeg's native context; Android owns its Vulkan handles.
pub struct Device {
    pub instance: ash::Instance,
    pub raw: ash::Device,
    pub physical: vk::PhysicalDevice,
    pub queue: vk::Queue,
    pub family: u32,
    pub memory: vk::PhysicalDeviceMemoryProperties,
    pub limits: vk::PhysicalDeviceLimits,
    pub compute_subgroup: Option<(u32, u32)>,
    pub portable_indices: bool,
    pub api_version: u32,
    #[cfg(target_os = "android")]
    pub presentation_timing: bool,
    submission: Mutex<()>,
    pipeline_cache: Mutex<Option<cache::Cache>>,
    #[cfg(target_os = "linux")]
    pub va: *mut sys::AVBufferRef,
    #[cfg(target_os = "linux")]
    pub drm_path: std::path::PathBuf,
    #[cfg(target_os = "linux")]
    pub vulkan: *mut sys::AVBufferRef,
    pub entry: ash::Entry,
}

pub struct QueueLock<'a> {
    _device: &'a Device,
    _guard: MutexGuard<'a, ()>,
}

impl Drop for QueueLock<'_> {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        unsafe {
            let context = (*self._device.vulkan).data.cast::<sys::AVHWDeviceContext>();
            let native = &*(*context).hwctx.cast::<sys::AVVulkanDeviceContext>();
            if let Some(unlock) = native.unlock_queue {
                unlock(context, self._device.family, 0);
            }
        }
    }
}

// Native handles are shared immutably; queue submissions are serialized.
unsafe impl Send for Device {}
unsafe impl Sync for Device {}

impl Device {
    pub fn device_uuid(&self) -> [u8; vk::UUID_SIZE] {
        let mut identity = vk::PhysicalDeviceIDProperties::default();
        let mut properties = vk::PhysicalDeviceProperties2::default().push_next(&mut identity);
        unsafe {
            self.instance
                .get_physical_device_properties2(self.physical, &mut properties);
        }
        identity.device_uuid
    }

    pub fn get() -> Result<Arc<Self>, String> {
        static DEVICE: OnceLock<Result<Arc<Device>, String>> = OnceLock::new();
        DEVICE.get_or_init(|| Self::new().map(Arc::new)).clone()
    }

    pub fn queue_lock(&self) -> QueueLock<'_> {
        let guard = self
            .submission
            .lock()
            .expect("Vulkan submission mutex poisoned");
        #[cfg(target_os = "linux")]
        unsafe {
            let context = (*self.vulkan).data.cast::<sys::AVHWDeviceContext>();
            let native = &*(*context).hwctx.cast::<sys::AVVulkanDeviceContext>();
            if let Some(lock) = native.lock_queue {
                lock(context, self.family, 0);
            }
        }
        QueueLock {
            _device: self,
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
        .map_err(|error| format!("submit Vulkan commands: {error:?}"))
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        if unsafe { self.raw.device_wait_idle() }.is_err() {
            std::process::abort();
        }
        self.pipeline_cache
            .get_mut()
            .expect("Vulkan pipeline cache poisoned")
            .take();
        #[cfg(target_os = "linux")]
        unsafe {
            sys::av_buffer_unref(&mut self.vulkan);
            sys::av_buffer_unref(&mut self.va);
        }
        #[cfg(target_os = "android")]
        unsafe {
            self.raw.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}
