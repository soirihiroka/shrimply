struct VulkanBuffer {
    vulkan: Arc<VulkanDevice>,
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    size: u64,
}

struct VulkanGeometry {
    blases: Vec<VulkanAccelerationStructure>,
    positions: VulkanBuffer,
    normals: VulkanBuffer,
    tangents: VulkanBuffer,
    tex_coords_0: VulkanBuffer,
    tex_coords_1: VulkanBuffer,
    colors: VulkanBuffer,
    texture_atlas: VulkanTexture,
}

struct VulkanScene {
    tlas: VulkanAccelerationStructure,
    _instances: VulkanBuffer,
    materials: VulkanBuffer,
    mesh_instances: VulkanBuffer,
    instance_count: u32,
    tlas_update_pending: bool,
}

struct TransmissionBackgroundSource {
    buffer: vk::Buffer,
    width: u32,
    height: u32,
}

struct TransmissionBackgroundTexture {
    texture: VulkanTexture,
    width: u32,
    height: u32,
    initialized: bool,
}

struct VulkanAccelerationStructure {
    loader: ash::khr::acceleration_structure::Device,
    handle: vk::AccelerationStructureKHR,
    _storage: VulkanBuffer,
}

impl VulkanAccelerationStructure {
    fn device_address(&self) -> vk::DeviceAddress {
        let info = vk::AccelerationStructureDeviceAddressInfoKHR::default()
            .acceleration_structure(self.handle);
        unsafe { self.loader.get_acceleration_structure_device_address(&info) }
    }
}

impl Drop for VulkanAccelerationStructure {
    fn drop(&mut self) {
        unsafe {
            self.loader
                .destroy_acceleration_structure(self.handle, None)
        };
    }
}

struct ShaderBindingTable {
    _buffer: VulkanBuffer,
    raygen: vk::StridedDeviceAddressRegionKHR,
    outline_distance: vk::StridedDeviceAddressRegionKHR,
    outline: vk::StridedDeviceAddressRegionKHR,
    miss: vk::StridedDeviceAddressRegionKHR,
    hit: vk::StridedDeviceAddressRegionKHR,
}

impl ShaderBindingTable {
    fn new(
        vulkan: Arc<VulkanDevice>,
        physical_device: vk::PhysicalDevice,
        ray_tracing: &ash::khr::ray_tracing_pipeline::Device,
        pipeline: vk::Pipeline,
    ) -> Result<Self, String> {
        let mut properties = vk::PhysicalDeviceRayTracingPipelinePropertiesKHR::default();
        let mut device_properties =
            vk::PhysicalDeviceProperties2::default().push_next(&mut properties);
        unsafe {
            vulkan
                .instance
                .get_physical_device_properties2(physical_device, &mut device_properties)
        };
        let handle_size = u64::from(properties.shader_group_handle_size);
        let stride =
            handle_size.next_multiple_of(u64::from(properties.shader_group_handle_alignment));
        let base_alignment = u64::from(properties.shader_group_base_alignment);
        let raygen_offset = 0;
        let outline_distance_offset = stride.next_multiple_of(base_alignment);
        let outline_offset = (outline_distance_offset + stride).next_multiple_of(base_alignment);
        let miss_offset = (outline_offset + stride).next_multiple_of(base_alignment);
        let hit_offset = (miss_offset + stride * 2).next_multiple_of(base_alignment);
        let total_size = hit_offset + stride * 2;
        let handles = unsafe {
            ray_tracing.get_ray_tracing_shader_group_handles(
                pipeline,
                0,
                RAY_TRACING_GROUP_COUNT,
                (handle_size * u64::from(RAY_TRACING_GROUP_COUNT)) as usize,
            )
        }
        .map_err(|error| format!("read Vulkan ray-tracing shader handles: {error:?}"))?;
        let buffer = VulkanBuffer::new(
            vulkan,
            physical_device,
            total_size + base_alignment - 1,
            vk::BufferUsageFlags::SHADER_BINDING_TABLE_KHR
                | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        let raw_address = buffer.device_address();
        let address = raw_address.next_multiple_of(base_alignment);
        let prefix = address - raw_address;
        let mut bytes = vec![0u8; buffer.size as usize];
        for (group, offset) in [
            raygen_offset,
            outline_distance_offset,
            outline_offset,
            miss_offset,
            miss_offset + stride,
            hit_offset,
            hit_offset + stride,
        ]
        .into_iter()
        .enumerate()
        {
            let source = group * handle_size as usize;
            let destination = (prefix + offset) as usize;
            bytes[destination..destination + handle_size as usize]
                .copy_from_slice(&handles[source..source + handle_size as usize]);
        }
        buffer.write(&bytes)?;
        tracing::info!(
            handle_size,
            stride,
            base_alignment,
            address,
            "Created Vulkan ray-tracing shader binding table"
        );
        Ok(Self {
            raygen: vk::StridedDeviceAddressRegionKHR {
                device_address: address + raygen_offset,
                stride,
                size: stride,
            },
            outline_distance: vk::StridedDeviceAddressRegionKHR {
                device_address: address + outline_distance_offset,
                stride,
                size: stride,
            },
            outline: vk::StridedDeviceAddressRegionKHR {
                device_address: address + outline_offset,
                stride,
                size: stride,
            },
            miss: vk::StridedDeviceAddressRegionKHR {
                device_address: address + miss_offset,
                stride,
                size: stride * 2,
            },
            hit: vk::StridedDeviceAddressRegionKHR {
                device_address: address + hit_offset,
                stride,
                size: stride * 2,
            },
            _buffer: buffer,
        })
    }
}

impl VulkanBuffer {
    fn new(
        vulkan: Arc<VulkanDevice>,
        physical_device: vk::PhysicalDevice,
        size: u64,
        usage: vk::BufferUsageFlags,
        properties: vk::MemoryPropertyFlags,
    ) -> Result<Self, String> {
        let info = vk::BufferCreateInfo::default()
            .size(size.max(1))
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let buffer = unsafe { vulkan.device.create_buffer(&info, None) }
            .map_err(|error| format!("create Vulkan 3D buffer: {error:?}"))?;
        let requirements = unsafe { vulkan.device.get_buffer_memory_requirements(buffer) };
        let memory_type = match memory_type(
            &vulkan,
            physical_device,
            requirements.memory_type_bits,
            properties,
        ) {
            Ok(index) => index,
            Err(error) => {
                unsafe { vulkan.device.destroy_buffer(buffer, None) };
                return Err(error);
            }
        };
        let mut allocation_flags = vk::MemoryAllocateFlagsInfo::default()
            .flags(vk::MemoryAllocateFlags::DEVICE_ADDRESS)
            ;
        let allocation = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(memory_type)
            .push_next(&mut allocation_flags);
        let memory = match unsafe { vulkan.device.allocate_memory(&allocation, None) } {
            Ok(memory) => memory,
            Err(error) => {
                unsafe { vulkan.device.destroy_buffer(buffer, None) };
                return Err(format!("allocate Vulkan 3D buffer memory: {error:?}"));
            }
        };
        if let Err(error) = unsafe { vulkan.device.bind_buffer_memory(buffer, memory, 0) } {
            unsafe {
                vulkan.device.destroy_buffer(buffer, None);
                vulkan.device.free_memory(memory, None);
            }
            return Err(format!("bind Vulkan 3D buffer memory: {error:?}"));
        }
        Ok(Self {
            vulkan,
            buffer,
            memory,
            size,
        })
    }

    fn write<T>(&self, values: &[T]) -> Result<(), String> {
        self.write_at(0, values)
    }

    fn write_at<T>(&self, offset: u64, values: &[T]) -> Result<(), String> {
        let bytes = size_of_val(values) as u64;
        if offset
            .checked_add(bytes)
            .is_none_or(|end| end > self.size)
        {
            return Err("Vulkan 3D buffer upload exceeds allocation".to_string());
        }
        if bytes == 0 {
            return Ok(());
        }
        let mapped = unsafe {
            self.vulkan
                .device
                .map_memory(self.memory, 0, offset + bytes, vk::MemoryMapFlags::empty())
        }
        .map_err(|error| format!("map Vulkan 3D buffer: {error:?}"))?;
        unsafe {
            ptr::copy_nonoverlapping(
                values.as_ptr().cast::<u8>(),
                mapped.cast::<u8>().add(offset as usize),
                bytes as usize,
            );
            self.vulkan.device.unmap_memory(self.memory);
        }
        Ok(())
    }

    fn read(&self) -> Result<Vec<u8>, String> {
        let mapped = unsafe {
            self.vulkan
                .device
                .map_memory(self.memory, 0, self.size, vk::MemoryMapFlags::empty())
        }
        .map_err(|error| format!("map Vulkan 3D diagnostic buffer: {error:?}"))?;
        let bytes =
            unsafe { std::slice::from_raw_parts(mapped.cast::<u8>(), self.size as usize) }.to_vec();
        unsafe { self.vulkan.device.unmap_memory(self.memory) };
        Ok(bytes)
    }

    fn device_address(&self) -> vk::DeviceAddress {
        let info = vk::BufferDeviceAddressInfo::default().buffer(self.buffer);
        unsafe { self.vulkan.device.get_buffer_device_address(&info) }
    }
}

impl Drop for VulkanBuffer {
    fn drop(&mut self) {
        unsafe {
            self.vulkan.device.destroy_buffer(self.buffer, None);
            self.vulkan.device.free_memory(self.memory, None);
        }
    }
}

struct VulkanTexture {
    vulkan: Arc<VulkanDevice>,
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
}

impl VulkanTexture {
    fn empty(vulkan: Arc<VulkanDevice>) -> Self {
        Self {
            vulkan,
            image: vk::Image::null(),
            memory: vk::DeviceMemory::null(),
            view: vk::ImageView::null(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn new(
        vulkan: Arc<VulkanDevice>,
        physical_device: vk::PhysicalDevice,
        width: u32,
        height: u32,
        mip_levels: u32,
        format: vk::Format,
        usage: vk::ImageUsageFlags,
        aspect: vk::ImageAspectFlags,
    ) -> Result<Self, String> {
        let info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            })
            .mip_levels(mip_levels)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let image = unsafe { vulkan.device.create_image(&info, None) }
            .map_err(|error| format!("create Vulkan 3D image: {error:?}"))?;
        let requirements = unsafe { vulkan.device.get_image_memory_requirements(image) };
        let memory_type = match memory_type(
            &vulkan,
            physical_device,
            requirements.memory_type_bits,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        ) {
            Ok(index) => index,
            Err(error) => {
                unsafe { vulkan.device.destroy_image(image, None) };
                return Err(error);
            }
        };
        let allocation = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(memory_type);
        let memory = match unsafe { vulkan.device.allocate_memory(&allocation, None) } {
            Ok(memory) => memory,
            Err(error) => {
                unsafe { vulkan.device.destroy_image(image, None) };
                return Err(format!("allocate Vulkan 3D image memory: {error:?}"));
            }
        };
        if let Err(error) = unsafe { vulkan.device.bind_image_memory(image, memory, 0) } {
            unsafe {
                vulkan.device.destroy_image(image, None);
                vulkan.device.free_memory(memory, None);
            }
            return Err(format!("bind Vulkan 3D image memory: {error:?}"));
        }
        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(format)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: aspect,
                base_mip_level: 0,
                level_count: mip_levels,
                base_array_layer: 0,
                layer_count: 1,
            });
        let view = match unsafe { vulkan.device.create_image_view(&view_info, None) } {
            Ok(view) => view,
            Err(error) => {
                unsafe {
                    vulkan.device.destroy_image(image, None);
                    vulkan.device.free_memory(memory, None);
                }
                return Err(format!("create Vulkan 3D image view: {error:?}"));
            }
        };
        Ok(Self {
            vulkan,
            image,
            memory,
            view,
        })
    }
}

impl Drop for VulkanTexture {
    fn drop(&mut self) {
        unsafe {
            if self.view != vk::ImageView::null() {
                self.vulkan.device.destroy_image_view(self.view, None);
            }
            if self.image != vk::Image::null() {
                self.vulkan.device.destroy_image(self.image, None);
            }
            if self.memory != vk::DeviceMemory::null() {
                self.vulkan.device.free_memory(self.memory, None);
            }
        }
    }
}

struct RenderTarget {
    color: VulkanTexture,
    outline_guide: VulkanTexture,
    outline_distance: VulkanTexture,
    denoiser: Option<DenoiserTargets>,
    width: u32,
    height: u32,
    denoise: bool,
    rendered: bool,
    cache_key: Option<RenderCacheKey>,
}

#[derive(Clone, PartialEq)]
struct RenderCacheKey {
    mesh: shrimply_render_3d::SceneIdentity,
    environment: Option<AssetSnapshot>,
    uniforms: shrimply_render_3d::obj::SceneUniforms,
}

struct DenoiserTargets {
    beauty: VulkanTexture,
    albedo: VulkanTexture,
    normal: VulkanTexture,
    background: VulkanTexture,
}

#[derive(Clone, Copy)]
enum SceneOutput {
    Rgba {
        image: vk::Image,
        source: ImageCopySource,
    },
    Denoise {
        beauty: vk::Image,
        albedo: vk::Image,
        normal: vk::Image,
        background: vk::Image,
        source: ImageCopySource,
    },
}

impl RenderTarget {
    fn new(
        vulkan: Arc<VulkanDevice>,
        physical_device: vk::PhysicalDevice,
        width: u32,
        height: u32,
        denoise: bool,
    ) -> Result<Self, String> {
        let color = VulkanTexture::new(
            vulkan.clone(),
            physical_device,
            width,
            height,
            1,
            COLOR_FORMAT,
            vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::TRANSFER_SRC,
            vk::ImageAspectFlags::COLOR,
        )?;
        let outline_guide = VulkanTexture::new(
            vulkan.clone(),
            physical_device,
            width,
            height,
            1,
            ENVIRONMENT_FORMAT,
            vk::ImageUsageFlags::STORAGE,
            vk::ImageAspectFlags::COLOR,
        )?;
        let outline_distance = VulkanTexture::new(
            vulkan.clone(),
            physical_device,
            width,
            height,
            1,
            OUTLINE_DISTANCE_FORMAT,
            vk::ImageUsageFlags::STORAGE,
            vk::ImageAspectFlags::COLOR,
        )?;
        let denoiser = denoise
            .then(|| {
                let create = || {
                    VulkanTexture::new(
                        vulkan.clone(),
                        physical_device,
                        width,
                        height,
                        1,
                        ENVIRONMENT_FORMAT,
                        vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::TRANSFER_SRC,
                        vk::ImageAspectFlags::COLOR,
                    )
                };
                Ok::<_, String>(DenoiserTargets {
                    beauty: create()?,
                    albedo: create()?,
                    normal: create()?,
                    background: create()?,
                })
            })
            .transpose()?;
        Ok(Self {
            color,
            outline_guide,
            outline_distance,
            denoiser,
            width,
            height,
            denoise,
            rendered: false,
            cache_key: None,
        })
    }

    fn denoiser_images(&self) -> [&VulkanTexture; 4] {
        self.denoiser.as_ref().map_or(
            [&self.color, &self.color, &self.color, &self.color],
            |targets| {
                [
                    &targets.beauty,
                    &targets.albedo,
                    &targets.normal,
                    &targets.background,
                ]
            },
        )
    }

    fn output(&self, source: ImageCopySource) -> SceneOutput {
        if let Some(targets) = &self.denoiser {
            SceneOutput::Denoise {
                beauty: targets.beauty.image,
                albedo: targets.albedo.image,
                normal: targets.normal.image,
                background: targets.background.image,
                source,
            }
        } else {
            SceneOutput::Rgba {
                image: self.color.image,
                source,
            }
        }
    }
}
