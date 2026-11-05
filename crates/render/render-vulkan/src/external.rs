use crate::{Buffer, Renderer, device::Device};
use ash::vk;
use shrimply_gpu_vulkan::ExportedFrame;
use std::{
    os::fd::{AsRawFd, IntoRawFd},
    sync::Arc,
};

/// A persistent import of a WGPU render target on the same physical GPU.
pub struct ExternalImage {
    device: Arc<Device>,
    image: vk::Image,
    memory: vk::DeviceMemory,
    semaphore: vk::Semaphore,
    width: u32,
    height: u32,
}

impl ExternalImage {
    pub fn new(exported: ExportedFrame) -> Result<Self, String> {
        let device = Device::get()?;
        let mut external = vk::ExternalMemoryImageCreateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);
        let image = unsafe {
            device.raw.create_image(
                &vk::ImageCreateInfo::default()
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(vk::Format::R8G8B8A8_UNORM)
                    .extent(vk::Extent3D {
                        width: exported.width,
                        height: exported.height,
                        depth: 1,
                    })
                    .mip_levels(1)
                    .array_layers(1)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .tiling(vk::ImageTiling::OPTIMAL)
                    .usage(
                        vk::ImageUsageFlags::COLOR_ATTACHMENT
                            | vk::ImageUsageFlags::STORAGE
                            | vk::ImageUsageFlags::TRANSFER_SRC,
                    )
                    .sharing_mode(vk::SharingMode::EXCLUSIVE)
                    .push_next(&mut external),
                None,
            )
        }
        .map_err(|error| format!("Create shared Vulkan image: {error:?}"))?;
        let mut imported = Self {
            device,
            image,
            memory: vk::DeviceMemory::null(),
            semaphore: vk::Semaphore::null(),
            width: exported.width,
            height: exported.height,
        };
        let requirements = unsafe { imported.device.raw.get_image_memory_requirements(image) };
        if exported.memory_type_index >= imported.device.memory.memory_type_count
            || requirements.memory_type_bits & (1 << exported.memory_type_index) == 0
            || requirements.size > exported.allocation_size
        {
            return Err(
                "Shared Vulkan image allocation is incompatible with the compositor".into(),
            );
        }
        let mut fd = vk::ImportMemoryFdInfoKHR::default()
            .handle_type(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD)
            .fd(exported.fd.as_raw_fd());
        let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
        imported.memory = unsafe {
            imported.device.raw.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(exported.allocation_size)
                    .memory_type_index(exported.memory_type_index)
                    .push_next(&mut fd)
                    .push_next(&mut dedicated),
                None,
            )
        }
        .map_err(|error| format!("Import shared Vulkan memory: {error:?}"))?;
        let _ = exported.fd.into_raw_fd(); // Vulkan owns the FD after a successful import.
        unsafe {
            imported
                .device
                .raw
                .bind_image_memory(image, imported.memory, 0)
        }
        .map_err(|error| format!("Bind shared Vulkan image: {error:?}"))?;
        let mut timeline =
            vk::SemaphoreTypeCreateInfo::default().semaphore_type(vk::SemaphoreType::TIMELINE);
        imported.semaphore = unsafe {
            imported.device.raw.create_semaphore(
                &vk::SemaphoreCreateInfo::default().push_next(&mut timeline),
                None,
            )
        }
        .map_err(|error| format!("Create shared Vulkan semaphore: {error:?}"))?;
        let extension = ash::khr::external_semaphore_fd::Device::new(
            &imported.device.instance,
            &imported.device.raw,
        );
        unsafe {
            extension.import_semaphore_fd(
                &vk::ImportSemaphoreFdInfoKHR::default()
                    .semaphore(imported.semaphore)
                    .handle_type(vk::ExternalSemaphoreHandleTypeFlags::OPAQUE_FD)
                    .fd(exported.semaphore_fd.as_raw_fd()),
            )
        }
        .map_err(|error| format!("Import shared Vulkan semaphore: {error:?}"))?;
        let _ = exported.semaphore_fd.into_raw_fd();
        Ok(imported)
    }
}

impl Drop for ExternalImage {
    fn drop(&mut self) {
        unsafe {
            self.device.raw.destroy_semaphore(self.semaphore, None);
            self.device.raw.destroy_image(self.image, None);
            self.device.raw.free_memory(self.memory, None);
        }
    }
}

impl Renderer {
    pub fn device_uuid(&self) -> [u8; vk::UUID_SIZE] {
        self.device.device_uuid()
    }

    /// Copies GPU storage only. Completing the transfer lets WGPU reuse its target.
    pub fn copy_external(&self, source: &ExternalImage, value: u64) -> Result<Buffer, String> {
        let length = (source.width as usize)
            .checked_mul(source.height as usize)
            .and_then(|count| count.checked_mul(4))
            .ok_or("External frame size overflow")?;
        let output = self.allocate(length)?;
        let command = self.command()?;
        let range = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .level_count(1)
            .layer_count(1);
        unsafe {
            self.device.raw.cmd_pipeline_barrier(
                command,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[vk::ImageMemoryBarrier::default()
                    .image(source.image)
                    .subresource_range(range)
                    .old_layout(vk::ImageLayout::GENERAL)
                    .new_layout(vk::ImageLayout::GENERAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_EXTERNAL)
                    .dst_queue_family_index(self.device.family)
                    .dst_access_mask(vk::AccessFlags::TRANSFER_READ)],
            );
            self.device.raw.cmd_copy_image_to_buffer(
                command,
                source.image,
                vk::ImageLayout::GENERAL,
                output.raw(),
                &[vk::BufferImageCopy::default()
                    .image_subresource(
                        vk::ImageSubresourceLayers::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .layer_count(1),
                    )
                    .image_extent(vk::Extent3D {
                        width: source.width,
                        height: source.height,
                        depth: 1,
                    })],
            );
            self.device.raw.cmd_pipeline_barrier(
                command,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::DependencyFlags::empty(),
                &[vk::MemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(vk::AccessFlags::MEMORY_READ)],
                &[],
                &[vk::ImageMemoryBarrier::default()
                    .image(source.image)
                    .subresource_range(range)
                    .old_layout(vk::ImageLayout::GENERAL)
                    .new_layout(vk::ImageLayout::GENERAL)
                    .src_queue_family_index(self.device.family)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_EXTERNAL)
                    .src_access_mask(vk::AccessFlags::TRANSFER_READ)],
            );
        }
        self.submit_with_semaphores(
            command,
            vec![output.clone()],
            &[(source.semaphore, value)],
            &[],
        )?
        .wait()?;
        Ok(output)
    }
}
