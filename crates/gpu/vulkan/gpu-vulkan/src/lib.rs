#![cfg(any(target_os = "linux", windows))]

use ash_wgpu::{khr, vk};
#[cfg(target_os = "linux")]
use std::os::fd::{FromRawFd, OwnedFd};
#[cfg(windows)]
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::{borrow::Cow, cell::Cell};
const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
pub struct ExportedFrame {
    #[cfg(target_os = "linux")]
    pub fd: OwnedFd,
    #[cfg(target_os = "linux")]
    pub semaphore_fd: OwnedFd,
    #[cfg(windows)]
    pub handle: OwnedHandle,
    #[cfg(windows)]
    pub semaphore_handle: OwnedHandle,
    pub allocation_size: u64,
    pub memory_type_index: u32,
    pub width: u32,
    pub height: u32,
}

pub struct ExternalImage {
    width: u32,
    height: u32,
    pub output: wgpu::Texture,
    external_layout_pipeline: wgpu::ComputePipeline,
    external_layout_bind_group: wgpu::BindGroup,
    export_memory: vk::DeviceMemory,
    export_allocation_size: u64,
    memory_type_index: u32,
    external_owner: Cell<bool>,
    queue_family: u32,
    semaphore: vk::Semaphore,
    next_semaphore_value: Cell<u64>,
    raw_device: ash_wgpu::Device,
}
pub fn device_for_uuid(
    device_uuid: [u8; vk::UUID_SIZE],
) -> Result<(wgpu::Device, wgpu::Queue), String> {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::VULKAN;
    let instance = wgpu::Instance::new(descriptor);
    let adapter = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN))
        .into_iter()
        .find(|adapter| unsafe {
            adapter
                .as_hal::<wgpu::hal::api::Vulkan>()
                .is_some_and(|adapter| {
                    let mut identity = vk::PhysicalDeviceIDProperties::default();
                    let mut properties =
                        vk::PhysicalDeviceProperties2::default().push_next(&mut identity);
                    adapter
                        .shared_instance()
                        .raw_instance()
                        .get_physical_device_properties2(
                            adapter.raw_physical_device(),
                            &mut properties,
                        );
                    identity.device_uuid == device_uuid
                })
        })
        .ok_or("Vulkan cannot find the requested physical device")?;
    let info = adapter.get_info();
    let descriptor = wgpu::DeviceDescriptor {
        label: Some("Shrimply Vulkan device"),
        ..Default::default()
    };
    let hal_device = {
        let hal = unsafe { adapter.as_hal::<wgpu::hal::api::Vulkan>() }
            .ok_or_else(|| "Vulkan adapter is not Vulkan".to_string())?;
        let hal: &wgpu::hal::vulkan::Adapter = &hal;
        unsafe {
            hal.open_with_callback(
                descriptor.required_features,
                &descriptor.required_limits,
                &descriptor.memory_hints,
                Some(Box::new(|args| {
                    for extension in [external_memory_extension(), external_semaphore_extension()] {
                        if !args.extensions.contains(&extension) {
                            args.extensions.push(extension);
                        }
                    }
                })),
            )
        }
        .map_err(|error| format!("open Vulkan device: {error:?}"))?
    };
    let (device, queue) = unsafe {
        adapter.create_device_from_hal::<wgpu::hal::api::Vulkan>(hal_device, &descriptor)
    }
    .map_err(|error| format!("create Vulkan device: {error}"))?;
    tracing::info!(adapter = %info.name, backend = ?info.backend, "Vulkan renderer initialized");
    Ok((device, queue))
}

impl ExternalImage {
    pub fn new(device: &wgpu::Device, size: [u32; 2]) -> Result<Self, String> {
        let [width, height] = size;
        let (output, export_memory, export_allocation_size, memory_type_index) =
            make_export_texture(device, width, height)?;
        let output_view = output.create_view(&Default::default());
        let external_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Vulkan external image layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: COLOR_FORMAT,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            }],
        });
        let external_layout_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Vulkan external image layout"),
            layout: &external_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&output_view),
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Vulkan external image layout"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(
                "@group(0) @binding(0) var output: texture_storage_2d<rgba8unorm, write>;\n\
                 @compute @workgroup_size(1) fn main() {}",
            )),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Vulkan external image layout"),
            bind_group_layouts: &[Some(&external_layout)],
            immediate_size: 0,
        });
        let external_layout_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("Vulkan external image layout"),
                layout: Some(&layout),
                module: &shader,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
        let (semaphore, raw_device) = make_export_semaphore(device)?;
        let queue_family = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }
            .ok_or("External output device is not Vulkan")?
            .queue_family_index();
        Ok(Self {
            width,
            height,
            output,

            external_layout_pipeline,
            external_layout_bind_group,
            export_memory,
            export_allocation_size,
            memory_type_index,
            external_owner: Cell::new(false),
            queue_family,
            semaphore,
            next_semaphore_value: Cell::new(0),
            raw_device,
        })
    }

    pub fn prepare_submit(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<u64, String> {
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Vulkan external image layout"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.external_layout_pipeline);
            pass.set_bind_group(0, &self.external_layout_bind_group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        let semaphore_value = self
            .next_semaphore_value
            .get()
            .checked_add(1)
            .ok_or_else(|| "Vulkan timeline semaphore overflowed".to_string())?;
        self.next_semaphore_value.set(semaphore_value);
        let hal_queue = unsafe { queue.as_hal::<wgpu::hal::api::Vulkan>() }
            .ok_or_else(|| "Vulkan queue is not Vulkan".to_string())?;
        hal_queue.add_signal_semaphore(self.semaphore, Some(semaphore_value));
        drop(hal_queue);

        Ok(semaphore_value)
    }

    /// The consumer must finish reading and release ownership before the next acquire.
    pub fn acquire_external(&self, encoder: &mut wgpu::CommandEncoder) -> Result<(), String> {
        if self.external_owner.replace(false) {
            self.transfer_ownership(encoder, true)?;
        }
        Ok(())
    }

    pub fn release_external(&self, encoder: &mut wgpu::CommandEncoder) -> Result<(), String> {
        self.transfer_ownership(encoder, false)?;
        self.external_owner.set(true);
        Ok(())
    }

    fn transfer_ownership(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        acquire: bool,
    ) -> Result<(), String> {
        let texture = unsafe { self.output.as_hal::<wgpu::hal::api::Vulkan>() }
            .ok_or("External output is not a Vulkan texture")?;
        let image = unsafe { texture.raw_handle() };
        // Vulkan has the same queue family for graphics, compute, and transfer in WGPU.
        // The renderer stores the device family when the target is created.
        let family = self.queue_family;
        let (source, destination, source_access, destination_access) = if acquire {
            (
                vk::QUEUE_FAMILY_EXTERNAL,
                family,
                vk::AccessFlags::empty(),
                vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE,
            )
        } else {
            (
                family,
                vk::QUEUE_FAMILY_EXTERNAL,
                vk::AccessFlags::MEMORY_WRITE,
                vk::AccessFlags::empty(),
            )
        };
        unsafe {
            encoder.as_hal_mut::<wgpu::hal::api::Vulkan, _, _>(|encoder| {
                let encoder = encoder.ok_or("External output encoder is not Vulkan")?;
                self.raw_device.cmd_pipeline_barrier(
                    encoder.raw_handle(),
                    vk::PipelineStageFlags::ALL_COMMANDS,
                    vk::PipelineStageFlags::ALL_COMMANDS,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[vk::ImageMemoryBarrier::default()
                        .old_layout(vk::ImageLayout::GENERAL)
                        .new_layout(vk::ImageLayout::GENERAL)
                        .src_access_mask(source_access)
                        .dst_access_mask(destination_access)
                        .src_queue_family_index(source)
                        .dst_queue_family_index(destination)
                        .image(image)
                        .subresource_range(
                            vk::ImageSubresourceRange::default()
                                .aspect_mask(vk::ImageAspectFlags::COLOR)
                                .level_count(1)
                                .layer_count(1),
                        )],
                );
                Ok(())
            })
        }
    }

    pub fn export(&self, device: &wgpu::Device) -> Result<ExportedFrame, String> {
        let hal = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }
            .ok_or_else(|| "Vulkan device is not Vulkan".to_string())?;
        #[cfg(target_os = "linux")]
        let (memory, semaphore) = {
            let external_memory = khr::external_memory_fd::Device::new(
                hal.shared_instance().raw_instance(),
                hal.raw_device(),
            );
            let info = vk::MemoryGetFdInfoKHR::default()
                .memory(self.export_memory)
                .handle_type(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);
            let fd = unsafe { external_memory.get_memory_fd(&info) }
                .map_err(|error| format!("export Vulkan memory fd: {error:?}"))?;
            let memory = unsafe { OwnedFd::from_raw_fd(fd) };
            let external_semaphore = khr::external_semaphore_fd::Device::new(
                hal.shared_instance().raw_instance(),
                hal.raw_device(),
            );
            let semaphore_info = vk::SemaphoreGetFdInfoKHR::default()
                .semaphore(self.semaphore)
                .handle_type(vk::ExternalSemaphoreHandleTypeFlags::OPAQUE_FD);
            let semaphore_fd = unsafe { external_semaphore.get_semaphore_fd(&semaphore_info) }
                .map_err(|error| format!("export Vulkan semaphore fd: {error:?}"))?;
            (memory, unsafe { OwnedFd::from_raw_fd(semaphore_fd) })
        };
        #[cfg(windows)]
        let (memory, semaphore) = {
            let external_memory = khr::external_memory_win32::Device::new(
                hal.shared_instance().raw_instance(),
                hal.raw_device(),
            );
            let info = vk::MemoryGetWin32HandleInfoKHR::default()
                .memory(self.export_memory)
                .handle_type(vk::ExternalMemoryHandleTypeFlags::OPAQUE_WIN32);
            let handle = unsafe { external_memory.get_memory_win32_handle(&info) }
                .map_err(|error| format!("export Vulkan memory handle: {error:?}"))?;
            let memory = unsafe { OwnedHandle::from_raw_handle(handle as *mut _) };
            let external_semaphore = khr::external_semaphore_win32::Device::new(
                hal.shared_instance().raw_instance(),
                hal.raw_device(),
            );
            let semaphore_info = vk::SemaphoreGetWin32HandleInfoKHR::default()
                .semaphore(self.semaphore)
                .handle_type(vk::ExternalSemaphoreHandleTypeFlags::OPAQUE_WIN32);
            let semaphore_handle =
                unsafe { external_semaphore.get_semaphore_win32_handle(&semaphore_info) }
                    .map_err(|error| format!("export Vulkan semaphore handle: {error:?}"))?;
            (memory, unsafe {
                OwnedHandle::from_raw_handle(semaphore_handle as *mut _)
            })
        };
        Ok(ExportedFrame {
            #[cfg(target_os = "linux")]
            fd: memory,
            #[cfg(target_os = "linux")]
            semaphore_fd: semaphore,
            #[cfg(windows)]
            handle: memory,
            #[cfg(windows)]
            semaphore_handle: semaphore,
            allocation_size: self.export_allocation_size,
            memory_type_index: self.memory_type_index,
            width: self.width,
            height: self.height,
        })
    }
}
impl Drop for ExternalImage {
    fn drop(&mut self) {
        if unsafe { self.raw_device.device_wait_idle() }.is_err() {
            std::process::abort();
        }
        unsafe { self.raw_device.destroy_semaphore(self.semaphore, None) };
    }
}

fn make_export_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> Result<(wgpu::Texture, vk::DeviceMemory, u64, u32), String> {
    let hal = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }
        .ok_or_else(|| "Vulkan device is not Vulkan".to_string())?;
    if !hal
        .enabled_device_extensions()
        .contains(&external_memory_extension())
    {
        return Err("Vulkan device does not support external memory export".to_string());
    }
    let raw_device = hal.raw_device();
    let mut external =
        vk::ExternalMemoryImageCreateInfo::default().handle_types(external_memory_handle_type());
    let create_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(vk::Format::R8G8B8A8_UNORM)
        .extent(vk::Extent3D {
            width,
            height,
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
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .push_next(&mut external);
    let image = unsafe { raw_device.create_image(&create_info, None) }
        .map_err(|error| format!("create Vulkan export image: {error:?}"))?;
    let requirements = unsafe { raw_device.get_image_memory_requirements(image) };
    let memory_properties = unsafe {
        hal.shared_instance()
            .raw_instance()
            .get_physical_device_memory_properties(hal.raw_physical_device())
    };
    let memory_type = (0..memory_properties.memory_type_count)
        .find(|index| {
            requirements.memory_type_bits & (1 << index) != 0
                && memory_properties.memory_types[*index as usize]
                    .property_flags
                    .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
        })
        .ok_or_else(|| "Vulkan export image has no device-local memory type".to_string());
    let memory_type = match memory_type {
        Ok(memory_type) => memory_type,
        Err(error) => {
            unsafe { raw_device.destroy_image(image, None) };
            return Err(error);
        }
    };
    let mut export =
        vk::ExportMemoryAllocateInfo::default().handle_types(external_memory_handle_type());
    let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
    let allocate_info = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(memory_type)
        .push_next(&mut export)
        .push_next(&mut dedicated);
    let memory = match unsafe { raw_device.allocate_memory(&allocate_info, None) } {
        Ok(memory) => memory,
        Err(error) => {
            unsafe { raw_device.destroy_image(image, None) };
            return Err(format!("allocate Vulkan export image memory: {error:?}"));
        }
    };
    if let Err(error) = unsafe { raw_device.bind_image_memory(image, memory, 0) } {
        unsafe {
            raw_device.destroy_image(image, None);
            raw_device.free_memory(memory, None);
        }
        return Err(format!("bind Vulkan export image memory: {error:?}"));
    }
    let usage = wgpu::TextureUsages::RENDER_ATTACHMENT
        | wgpu::TextureUsages::STORAGE_BINDING
        | wgpu::TextureUsages::COPY_SRC;
    let descriptor = wgpu::TextureDescriptor {
        label: Some("Vulkan export image"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: COLOR_FORMAT,
        usage,
        view_formats: &[],
    };
    let hal_descriptor = wgpu::hal::TextureDescriptor {
        label: descriptor.label,
        size: descriptor.size,
        mip_level_count: descriptor.mip_level_count,
        sample_count: descriptor.sample_count,
        dimension: descriptor.dimension,
        format: descriptor.format,
        usage: wgpu::TextureUses::COLOR_TARGET
            | wgpu::TextureUses::STORAGE_WRITE_ONLY
            | wgpu::TextureUses::COPY_SRC,
        memory_flags: wgpu::hal::MemoryFlags::empty(),
        view_formats: Vec::new(),
    };
    let hal_texture = unsafe {
        hal.texture_from_raw(
            image,
            &hal_descriptor,
            None,
            wgpu::hal::vulkan::TextureMemory::Dedicated(memory),
        )
    };
    drop(hal);
    let texture = unsafe {
        device.create_texture_from_hal::<wgpu::hal::api::Vulkan>(
            hal_texture,
            &descriptor,
            wgpu::TextureUses::UNINITIALIZED,
        )
    };
    Ok((texture, memory, requirements.size, memory_type))
}

fn make_export_semaphore(
    device: &wgpu::Device,
) -> Result<(vk::Semaphore, ash_wgpu::Device), String> {
    let hal = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }
        .ok_or_else(|| "Vulkan device is not Vulkan".to_string())?;
    if !hal
        .enabled_device_extensions()
        .contains(&external_semaphore_extension())
    {
        return Err("Vulkan device does not support external semaphore export".to_string());
    }
    let raw_device = hal.raw_device().clone();
    let mut external =
        vk::ExportSemaphoreCreateInfo::default().handle_types(external_semaphore_handle_type());
    let mut timeline = vk::SemaphoreTypeCreateInfo::default()
        .semaphore_type(vk::SemaphoreType::TIMELINE)
        .initial_value(0);
    let info = vk::SemaphoreCreateInfo::default()
        .push_next(&mut external)
        .push_next(&mut timeline);
    let semaphore = unsafe { raw_device.create_semaphore(&info, None) }
        .map_err(|error| format!("create Vulkan timeline semaphore: {error:?}"))?;
    Ok((semaphore, raw_device))
}

#[cfg(target_os = "linux")]
fn external_memory_extension() -> &'static std::ffi::CStr {
    khr::external_memory_fd::NAME
}

#[cfg(windows)]
fn external_memory_extension() -> &'static std::ffi::CStr {
    khr::external_memory_win32::NAME
}

#[cfg(target_os = "linux")]
fn external_semaphore_extension() -> &'static std::ffi::CStr {
    khr::external_semaphore_fd::NAME
}

#[cfg(windows)]
fn external_semaphore_extension() -> &'static std::ffi::CStr {
    khr::external_semaphore_win32::NAME
}

#[cfg(target_os = "linux")]
fn external_memory_handle_type() -> vk::ExternalMemoryHandleTypeFlags {
    vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD
}

#[cfg(windows)]
fn external_memory_handle_type() -> vk::ExternalMemoryHandleTypeFlags {
    vk::ExternalMemoryHandleTypeFlags::OPAQUE_WIN32
}

#[cfg(target_os = "linux")]
fn external_semaphore_handle_type() -> vk::ExternalSemaphoreHandleTypeFlags {
    vk::ExternalSemaphoreHandleTypeFlags::OPAQUE_FD
}

#[cfg(windows)]
fn external_semaphore_handle_type() -> vk::ExternalSemaphoreHandleTypeFlags {
    vk::ExternalSemaphoreHandleTypeFlags::OPAQUE_WIN32
}
