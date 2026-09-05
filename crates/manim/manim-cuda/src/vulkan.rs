use ash_wgpu::{khr, vk};
use hashbrown::HashMap;
use shrimply_manim_wgpu::{ExternalFrameDescriptor, PreparedAnimation, Renderer as SharedRenderer};
use std::{
    borrow::Cow,
    cell::Cell,
    os::fd::{FromRawFd, OwnedFd},
};
const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
pub struct ExportedFrame {
    pub fd: OwnedFd,
    pub semaphore_fd: OwnedFd,
    pub allocation_size: u64,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenderedExternalFrame {
    pub descriptor: ExternalFrameDescriptor,
    pub semaphore_value: u64,
}

pub struct Renderer {
    shared: SharedRenderer,
    targets: HashMap<usize, Target>,
}
struct Target {
    width: u32,
    height: u32,
    samples: u32,
    output: wgpu::Texture,
    external_layout_pipeline: wgpu::ComputePipeline,
    external_layout_bind_group: wgpu::BindGroup,
    export_memory: vk::DeviceMemory,
    export_allocation_size: u64,
    semaphore: vk::Semaphore,
    next_semaphore_value: Cell<u64>,
    raw_device: ash_wgpu::Device,
}
impl Renderer {
    pub fn new(cuda_device_uuid: [u8; vk::UUID_SIZE]) -> Result<Self, String> {
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
                        identity.device_uuid == cuda_device_uuid
                    })
            })
            .ok_or("Manim cannot find the Vulkan device used by the CUDA context")?;
        let info = adapter.get_info();
        let descriptor = wgpu::DeviceDescriptor {
            label: Some("Shrimply Manim WGPU device"),
            ..Default::default()
        };
        let hal_device = {
            let hal = unsafe { adapter.as_hal::<wgpu::hal::api::Vulkan>() }
                .ok_or_else(|| "Manim WGPU adapter is not Vulkan".to_string())?;
            let hal: &wgpu::hal::vulkan::Adapter = &hal;
            unsafe {
                hal.open_with_callback(
                    descriptor.required_features,
                    &descriptor.required_limits,
                    &descriptor.memory_hints,
                    Some(Box::new(|args| {
                        if !args.extensions.contains(&khr::external_semaphore_fd::NAME) {
                            args.extensions.push(khr::external_semaphore_fd::NAME);
                        }
                    })),
                )
            }
            .map_err(|error| format!("open Manim WGPU Vulkan device: {error:?}"))?
        };
        let (device, queue) = unsafe {
            adapter.create_device_from_hal::<wgpu::hal::api::Vulkan>(hal_device, &descriptor)
        }
        .map_err(|error| format!("create Manim WGPU device: {error}"))?;
        tracing::info!(adapter = %info.name, backend = ?info.backend, "Manim WGPU renderer initialized");
        Ok(Self {
            shared: SharedRenderer::from_device(device, queue),
            targets: HashMap::new(),
        })
    }

    pub fn target_descriptor(&self, slot: usize) -> Option<ExternalFrameDescriptor> {
        self.shared.target_descriptor(slot)
    }
    pub fn release_render_surfaces(&mut self) -> bool {
        let released = self.shared.release_render_surfaces() || !self.targets.is_empty();
        self.targets.clear();
        released
    }
    pub fn release_gpu_animation_resources(&mut self) -> bool {
        self.shared.release_gpu_animation_resources()
    }
    pub fn remove_target(&mut self, slot: usize) -> bool {
        self.shared.remove_target(slot) | self.targets.remove(&slot).is_some()
    }
    pub fn render_external(
        &mut self,
        slot: usize,
        animation: &PreparedAnimation,
        frame_index: usize,
    ) -> Result<RenderedExternalFrame, String> {
        let descriptor = SharedRenderer::external_frame_descriptor(animation);
        if self.target_descriptor(slot) != Some(descriptor) {
            self.remove_target(slot);
            self.targets
                .insert(slot, Target::new(&self.shared.device, descriptor)?);
        }
        let target = self
            .targets
            .get(&slot)
            .expect("initialized Manim Vulkan target");
        let mut encoder =
            self.shared
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Manim Vulkan frame"),
                });
        self.shared.encode(
            slot,
            animation,
            frame_index,
            target.output.clone(),
            &mut encoder,
        )?;
        let semaphore_value = target.prepare_submit(&self.shared.queue, &mut encoder)?;
        self.shared.queue.submit([encoder.finish()]);
        Ok(RenderedExternalFrame {
            descriptor,
            semaphore_value,
        })
    }
    pub fn export_frame(&self, slot: usize) -> Result<ExportedFrame, String> {
        self.targets
            .get(&slot)
            .ok_or("Manim target is not initialized")?
            .export(&self.shared.device)
    }
}
impl Target {
    fn new(device: &wgpu::Device, descriptor: ExternalFrameDescriptor) -> Result<Self, String> {
        let ExternalFrameDescriptor {
            width,
            height,
            samples,
        } = descriptor;
        if !matches!(samples, 1 | 2 | 4 | 8 | 16) {
            return Err(format!("unsupported Manim sample count {samples}"));
        }
        let (output, export_memory, export_allocation_size) =
            make_export_texture(device, width, height)?;
        let output_view = output.create_view(&Default::default());
        let external_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Manim external image layout"),
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
            label: Some("Manim external image layout"),
            layout: &external_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&output_view),
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Manim external image layout"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(
                "@group(0) @binding(0) var output: texture_storage_2d<rgba8unorm, write>;\n\
                 @compute @workgroup_size(1) fn main() {}",
            )),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Manim external image layout"),
            bind_group_layouts: &[Some(&external_layout)],
            immediate_size: 0,
        });
        let external_layout_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("Manim external image layout"),
                layout: Some(&layout),
                module: &shader,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
        let (semaphore, raw_device) = make_export_semaphore(device)?;
        Ok(Self {
            width,
            height,
            samples,
            output,

            external_layout_pipeline,
            external_layout_bind_group,
            export_memory,
            export_allocation_size,
            semaphore,
            next_semaphore_value: Cell::new(0),
            raw_device,
        })
    }

    fn prepare_submit(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<u64, String> {
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Manim external image layout"),
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
            .ok_or_else(|| "Manim WGPU timeline semaphore overflowed".to_string())?;
        self.next_semaphore_value.set(semaphore_value);
        let hal_queue = unsafe { queue.as_hal::<wgpu::hal::api::Vulkan>() }
            .ok_or_else(|| "Manim WGPU queue is not Vulkan".to_string())?;
        hal_queue.add_signal_semaphore(self.semaphore, Some(semaphore_value));
        drop(hal_queue);

        Ok(semaphore_value)
    }

    fn export(&self, device: &wgpu::Device) -> Result<ExportedFrame, String> {
        let hal = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }
            .ok_or_else(|| "Manim WGPU device is not Vulkan".to_string())?;
        let external_memory = khr::external_memory_fd::Device::new(
            hal.shared_instance().raw_instance(),
            hal.raw_device(),
        );
        let info = vk::MemoryGetFdInfoKHR::default()
            .memory(self.export_memory)
            .handle_type(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);
        let fd = unsafe { external_memory.get_memory_fd(&info) }
            .map_err(|error| format!("export Manim WGPU memory fd: {error:?}"))?;
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let external_semaphore = khr::external_semaphore_fd::Device::new(
            hal.shared_instance().raw_instance(),
            hal.raw_device(),
        );
        let semaphore_info = vk::SemaphoreGetFdInfoKHR::default()
            .semaphore(self.semaphore)
            .handle_type(vk::ExternalSemaphoreHandleTypeFlags::OPAQUE_FD);
        let semaphore_fd = unsafe { external_semaphore.get_semaphore_fd(&semaphore_info) }
            .map_err(|error| format!("export Manim WGPU semaphore fd: {error:?}"))?;
        Ok(ExportedFrame {
            fd,
            semaphore_fd: unsafe { OwnedFd::from_raw_fd(semaphore_fd) },
            allocation_size: self.export_allocation_size,
            width: self.width,
            height: self.height,
        })
    }
}
impl Drop for Target {
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
) -> Result<(wgpu::Texture, vk::DeviceMemory, u64), String> {
    let hal = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }
        .ok_or_else(|| "Manim WGPU device is not Vulkan".to_string())?;
    if !hal
        .enabled_device_extensions()
        .contains(&khr::external_memory_fd::NAME)
    {
        return Err("Manim WGPU device does not support external memory fd".to_string());
    }
    let raw_device = hal.raw_device();
    let mut external = vk::ExternalMemoryImageCreateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);
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
        .map_err(|error| format!("create Manim WGPU export image: {error:?}"))?;
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
        .ok_or_else(|| "Manim WGPU export image has no device-local memory type".to_string());
    let memory_type = match memory_type {
        Ok(memory_type) => memory_type,
        Err(error) => {
            unsafe { raw_device.destroy_image(image, None) };
            return Err(error);
        }
    };
    let mut export = vk::ExportMemoryAllocateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);
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
            return Err(format!(
                "allocate Manim WGPU export image memory: {error:?}"
            ));
        }
    };
    if let Err(error) = unsafe { raw_device.bind_image_memory(image, memory, 0) } {
        unsafe {
            raw_device.destroy_image(image, None);
            raw_device.free_memory(memory, None);
        }
        return Err(format!("bind Manim WGPU export image memory: {error:?}"));
    }
    let usage = wgpu::TextureUsages::RENDER_ATTACHMENT
        | wgpu::TextureUsages::STORAGE_BINDING
        | wgpu::TextureUsages::COPY_SRC;
    let descriptor = wgpu::TextureDescriptor {
        label: Some("Manim WGPU export image"),
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
    Ok((texture, memory, requirements.size))
}

fn make_export_semaphore(
    device: &wgpu::Device,
) -> Result<(vk::Semaphore, ash_wgpu::Device), String> {
    let hal = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }
        .ok_or_else(|| "Manim WGPU device is not Vulkan".to_string())?;
    if !hal
        .enabled_device_extensions()
        .contains(&khr::external_semaphore_fd::NAME)
    {
        return Err("Manim WGPU device does not support external semaphore fd".to_string());
    }
    let raw_device = hal.raw_device().clone();
    let mut external = vk::ExportSemaphoreCreateInfo::default()
        .handle_types(vk::ExternalSemaphoreHandleTypeFlags::OPAQUE_FD);
    let mut timeline = vk::SemaphoreTypeCreateInfo::default()
        .semaphore_type(vk::SemaphoreType::TIMELINE)
        .initial_value(0);
    let info = vk::SemaphoreCreateInfo::default()
        .push_next(&mut external)
        .push_next(&mut timeline);
    let semaphore = unsafe { raw_device.create_semaphore(&info, None) }
        .map_err(|error| format!("create Manim WGPU timeline semaphore: {error:?}"))?;
    Ok((semaphore, raw_device))
}
