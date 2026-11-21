//! Skia records independently on each rendering thread. Only Vulkan queue host
//! operations require the shared queue lock, including Skia's implicit submits.
use super::Device;
use ash::{vk, vk::Handle};
use skia_safe::gpu::{self, vk as skvk};
use std::{ffi::c_void, sync::Arc};

impl Device {
    pub(crate) fn skia_context(&self, presentation: bool) -> Result<gpu::DirectContext, String> {
        let get_proc = |request: skvk::GetProcOf| unsafe {
            let native = match request {
                skvk::GetProcOf::Instance(handle, name) => self
                    .entry
                    .get_instance_proc_addr(vk::Instance::from_raw(handle as u64), name),
                skvk::GetProcOf::Device(handle, name) => self
                    .instance
                    .get_device_proc_addr(vk::Device::from_raw(handle as u64), name),
            };
            let Some(native) = native else {
                return std::ptr::null();
            };
            // The locked Skia m153 VulkanInterface loads these four queue-host
            // operations. Audit this list when upgrading Skia:
            // https://github.com/rust-skia/skia/blob/m153-0.101.2/src/gpu/vk/VulkanInterface.cpp
            match request.name().to_bytes() {
                b"vkQueueSubmit" => queue_submit as *const () as *const c_void,
                b"vkQueueBindSparse" => queue_bind_sparse as *const () as *const c_void,
                b"vkQueueWaitIdle" => queue_wait_idle as *const () as *const c_void,
                b"vkDeviceWaitIdle" => device_wait_idle as *const () as *const c_void,
                _ => native as *const c_void,
            }
        };
        let version = if presentation {
            self.api_version.min(vk::API_VERSION_1_2)
        } else {
            self.api_version
        };
        let backend = unsafe {
            skvk::BackendContext::new_builder(
                self.instance.handle().as_raw() as _,
                self.physical.as_raw() as _,
                self.raw.handle().as_raw() as _,
                (self.queue.as_raw() as _, self.family as usize),
                &get_proc,
                Some(version.into()),
            )
            .with_extensions(
                if presentation {
                    &["VK_KHR_surface"]
                } else {
                    &[]
                },
                if presentation {
                    &["VK_KHR_swapchain"]
                } else {
                    &[]
                },
            )
            .build()
        };
        gpu::direct_contexts::make_vulkan(&backend, None)
            .ok_or_else(|| "Create Skia Vulkan context".into())
    }
}

fn shared_queue(queue: vk::Queue) -> Arc<Device> {
    let shared = Device::get().expect("shared Skia Vulkan device");
    assert_eq!(queue, shared.queue, "Skia must use the shared Vulkan queue");
    shared
}

unsafe extern "system" fn queue_submit(
    queue: vk::Queue,
    count: u32,
    submits: *const vk::SubmitInfo<'_>,
    fence: vk::Fence,
) -> vk::Result {
    let shared = shared_queue(queue);
    let _guard = shared.queue_lock();
    let _measure = shrimply_profiling::measure("Vulkan queue / Skia submit");
    unsafe { (shared.raw.fp_v1_0().queue_submit)(queue, count, submits, fence) }
}

unsafe extern "system" fn queue_bind_sparse(
    queue: vk::Queue,
    count: u32,
    binds: *const vk::BindSparseInfo<'_>,
    fence: vk::Fence,
) -> vk::Result {
    let shared = shared_queue(queue);
    let _guard = shared.queue_lock();
    unsafe { (shared.raw.fp_v1_0().queue_bind_sparse)(queue, count, binds, fence) }
}

unsafe extern "system" fn queue_wait_idle(queue: vk::Queue) -> vk::Result {
    let shared = shared_queue(queue);
    let _guard = shared.queue_lock();
    unsafe { (shared.raw.fp_v1_0().queue_wait_idle)(queue) }
}

unsafe extern "system" fn device_wait_idle(device: vk::Device) -> vk::Result {
    let shared = Device::get().expect("shared Skia Vulkan device");
    assert_eq!(
        device,
        shared.raw.handle(),
        "Skia must use the shared Vulkan device"
    );
    shared
        .wait_idle()
        .map_or_else(|error| error, |()| vk::Result::SUCCESS)
}
