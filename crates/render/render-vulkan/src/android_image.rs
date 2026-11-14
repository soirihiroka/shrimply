//! Android supplies the acquired image and native allocation. Sampling,
//! dispatch, synchronization and downstream effects use the shared GPU renderer.
use super::{Buffer, DispatchBindings, Renderer, Submission, device::Device};
use ash::vk;
use shrimply_visual_frame::{VideoFrame, android::AndroidFramePool};
use std::{cell::Cell, rc::Rc, sync::Arc};

pub fn decoder_backend(
    width: u32,
    height: u32,
    maximum_images: u32,
) -> Result<shrimply_video_decoder::DecoderBackend, String> {
    let device = Device::get()?;
    let pool = AndroidFramePool::new(width, height, maximum_images, move |buffer| {
        device
            .hardware_buffer_properties(buffer)
            .map(|properties| properties.allocation_size)
    })?;
    shrimply_video_decoder::DecoderBackend::mediacodec(pool)
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct SamplerKey {
    format: vk::Format,
    external: u64,
    model: vk::SamplerYcbcrModelConversion,
    range: vk::SamplerYcbcrRange,
    components: [vk::ComponentSwizzle; 4],
    chroma: [vk::ChromaLocation; 2],
    converted: bool,
}

pub(super) struct Sampler {
    device: Arc<Device>,
    conversion: vk::SamplerYcbcrConversion,
    sampler: vk::Sampler,
    layout: vk::DescriptorSetLayout,
    pipeline_layout: vk::PipelineLayout,
    descriptors_per_image: Cell<u32>,
}

impl Sampler {
    fn new(device: Arc<Device>, key: SamplerKey) -> Result<Self, String> {
        let mut result = Self {
            device,
            conversion: vk::SamplerYcbcrConversion::null(),
            sampler: vk::Sampler::null(),
            layout: vk::DescriptorSetLayout::null(),
            pipeline_layout: vk::PipelineLayout::null(),
            descriptors_per_image: Cell::new(1),
        };
        let raw = &result.device.raw;
        if key.converted {
            let mut external = vk::ExternalFormatANDROID::default().external_format(key.external);
            let mut info = vk::SamplerYcbcrConversionCreateInfo::default()
                .format(key.format)
                .ycbcr_model(key.model)
                .ycbcr_range(key.range)
                .components(vk::ComponentMapping {
                    r: key.components[0],
                    g: key.components[1],
                    b: key.components[2],
                    a: key.components[3],
                })
                .x_chroma_offset(key.chroma[0])
                .y_chroma_offset(key.chroma[1])
                .chroma_filter(vk::Filter::NEAREST);
            if key.format == vk::Format::UNDEFINED {
                info = info.push_next(&mut external);
            }
            result.conversion = unsafe { raw.create_sampler_ycbcr_conversion(&info, None) }
                .map_err(|error| format!("Create hardware color conversion: {error:?}"))?;
        }
        let mut conversion =
            vk::SamplerYcbcrConversionInfo::default().conversion(result.conversion);
        let mut info = vk::SamplerCreateInfo::default()
            .mag_filter(vk::Filter::NEAREST)
            .min_filter(vk::Filter::NEAREST)
            .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
            .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE);
        if key.converted {
            info = info.push_next(&mut conversion);
        }
        result.sampler = unsafe { raw.create_sampler(&info, None) }
            .map_err(|error| format!("Create hardware image sampler: {error:?}"))?;
        let samplers = [result.sampler];
        let bindings = [vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::COMPUTE)
            .immutable_samplers(&samplers)];
        result.layout = unsafe {
            raw.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                None,
            )
        }
        .map_err(|error| format!("Create hardware sampler layout: {error:?}"))?;
        let layouts = [result.layout];
        let push = [vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::COMPUTE)
            .size(super::PUSH_CONSTANT_BYTES)];
        result.pipeline_layout = unsafe {
            raw.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default()
                    .set_layouts(&layouts)
                    .push_constant_ranges(&push),
                None,
            )
        }
        .map_err(|error| format!("Create hardware image pipeline layout: {error:?}"))?;
        Ok(result)
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        unsafe {
            self.device
                .raw
                .destroy_pipeline_layout(self.pipeline_layout, None);
            self.device
                .raw
                .destroy_descriptor_set_layout(self.layout, None);
            self.device.raw.destroy_sampler(self.sampler, None);
            self.device
                .raw
                .destroy_sampler_ycbcr_conversion(self.conversion, None);
        }
    }
}

pub(super) struct ImportedImage {
    device: Arc<Device>,
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
    descriptors: vk::DescriptorPool,
    sampler: Rc<Sampler>,
    // The acquired AImage, not just its allocation, must survive the GPU fence.
    _source: Arc<VideoFrame>,
}

impl Drop for ImportedImage {
    fn drop(&mut self) {
        // Submission waits before dropping its retained images. Its release
        // barrier has already returned ownership to the foreign producer.
        unsafe {
            self.device
                .raw
                .destroy_descriptor_pool(self.descriptors, None);
            self.device.raw.destroy_image_view(self.view, None);
            self.device.raw.destroy_image(self.image, None);
            self.device.raw.free_memory(self.memory, None);
        }
    }
}

impl Renderer {
    pub(super) fn import_video(
        &mut self,
        source: &Arc<VideoFrame>,
    ) -> Result<(Buffer, Submission), String> {
        let native = source
            .android()
            .ok_or("Video frame is not an acquired Android hardware image")?;
        let buffer = native.buffer();
        let description = buffer.describe();
        let properties = self.device.hardware_buffer_properties(buffer)?;
        let format = properties.format;
        if !format
            .format_features
            .contains(vk::FormatFeatureFlags::SAMPLED_IMAGE)
        {
            return Err("Decoder hardware buffer cannot be sampled on this GPU".into());
        }
        let converted = format.format == vk::Format::UNDEFINED
            || format.format_features.intersects(
                vk::FormatFeatureFlags::MIDPOINT_CHROMA_SAMPLES
                    | vk::FormatFeatureFlags::COSITED_CHROMA_SAMPLES,
            );
        if format.format == vk::Format::UNDEFINED && format.external_format == 0 {
            return Err("Decoder hardware buffer has no supported native format".into());
        }
        let components = format.sampler_ycbcr_conversion_components;
        let key = SamplerKey {
            format: format.format,
            external: if format.format == vk::Format::UNDEFINED {
                format.external_format
            } else {
                0
            },
            model: format.suggested_ycbcr_model,
            range: format.suggested_ycbcr_range,
            components: [components.r, components.g, components.b, components.a],
            chroma: [
                format.suggested_x_chroma_offset,
                format.suggested_y_chroma_offset,
            ],
            converted,
        };
        if !self.native_samplers.contains_key(&key) {
            self.native_samplers
                .insert(key, Rc::new(Sampler::new(self.device.clone(), key)?));
        }
        let sampler = self
            .native_samplers
            .get(&key)
            .expect("initialized hardware sampler")
            .clone();
        let mut imported = ImportedImage {
            device: self.device.clone(),
            image: vk::Image::null(),
            memory: vk::DeviceMemory::null(),
            view: vk::ImageView::null(),
            descriptors: vk::DescriptorPool::null(),
            sampler,
            _source: source.clone(),
        };
        let raw = &self.device.raw;
        let mut external = vk::ExternalMemoryImageCreateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::ANDROID_HARDWARE_BUFFER_ANDROID);
        let mut external_format =
            vk::ExternalFormatANDROID::default().external_format(key.external);
        let mut info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(key.format)
            .extent(vk::Extent3D {
                width: description.width,
                height: description.height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::SAMPLED)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .push_next(&mut external);
        if key.format == vk::Format::UNDEFINED {
            info = info.push_next(&mut external_format);
        }
        imported.image = unsafe { raw.create_image(&info, None) }
            .map_err(|error| format!("Import decoder image: {error:?}"))?;
        let memory_type = (0..self.device.memory.memory_type_count)
            .filter(|index| properties.memory_type_bits & (1 << index) != 0)
            .max_by_key(|index| {
                self.device.memory.memory_types[*index as usize]
                    .property_flags
                    .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
            })
            .ok_or("Decoder image has no compatible Vulkan memory type")?;
        let mut hardware =
            vk::ImportAndroidHardwareBufferInfoANDROID::default().buffer(buffer.as_ptr().cast());
        let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(imported.image);
        let info = vk::MemoryAllocateInfo::default()
            .allocation_size(properties.allocation_size)
            .memory_type_index(memory_type)
            .push_next(&mut hardware)
            .push_next(&mut dedicated);
        imported.memory = unsafe { raw.allocate_memory(&info, None) }
            .map_err(|error| format!("Bind decoder hardware allocation: {error:?}"))?;
        unsafe { raw.bind_image_memory(imported.image, imported.memory, 0) }
            .map_err(|error| format!("Bind decoder image memory: {error:?}"))?;
        let range = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .level_count(1)
            .layer_count(1);
        let mut conversion =
            vk::SamplerYcbcrConversionInfo::default().conversion(imported.sampler.conversion);
        let mut info = vk::ImageViewCreateInfo::default()
            .image(imported.image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(key.format)
            .subresource_range(range);
        if key.converted {
            info = info.push_next(&mut conversion);
        }
        imported.view = unsafe { raw.create_image_view(&info, None) }
            .map_err(|error| format!("Create decoder image view: {error:?}"))?;
        // Opaque YCbCr formats can consume several sampler descriptors. Vulkan
        // 1.2 reports an undersized pool through OUT_OF_POOL_MEMORY; grow only
        // until this layout fits, then retain that size for subsequent images.
        let limit = self.device.limits.max_per_stage_descriptor_samplers;
        let mut count = imported.sampler.descriptors_per_image.get();
        let layouts = [imported.sampler.layout];
        let sets = loop {
            let sizes = [vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(count)];
            let pool = unsafe {
                raw.create_descriptor_pool(
                    &vk::DescriptorPoolCreateInfo::default()
                        .max_sets(1)
                        .pool_sizes(&sizes),
                    None,
                )
            }
            .map_err(|error| format!("Allocate decoder image descriptors: {error:?}"))?;
            let result = unsafe {
                raw.allocate_descriptor_sets(
                    &vk::DescriptorSetAllocateInfo::default()
                        .descriptor_pool(pool)
                        .set_layouts(&layouts),
                )
            };
            match result {
                Ok(sets) => {
                    imported.descriptors = pool;
                    imported.sampler.descriptors_per_image.set(count);
                    break sets;
                }
                Err(error) => {
                    unsafe {
                        raw.destroy_descriptor_pool(pool, None);
                    }
                    if error == vk::Result::ERROR_OUT_OF_POOL_MEMORY && count < limit {
                        count = count.saturating_mul(2).min(limit);
                    } else {
                        return Err(format!("Bind decoder image descriptor: {error:?}"));
                    }
                }
            }
        };
        let images = [vk::DescriptorImageInfo::default()
            .image_view(imported.view)
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
        unsafe {
            raw.update_descriptor_sets(
                &[vk::WriteDescriptorSet::default()
                    .dst_set(sets[0])
                    .dst_binding(0)
                    .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                    .image_info(&images)],
                &[],
            )
        };
        let acquire = [vk::ImageMemoryBarrier::default()
            .image(imported.image)
            .subresource_range(range)
            .old_layout(vk::ImageLayout::GENERAL)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_FOREIGN_EXT)
            .dst_queue_family_index(self.device.family)
            .dst_access_mask(vk::AccessFlags::SHADER_READ)];
        let release = [vk::ImageMemoryBarrier::default()
            .image(imported.image)
            .subresource_range(range)
            .old_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .new_layout(vk::ImageLayout::GENERAL)
            .src_queue_family_index(self.device.family)
            .dst_queue_family_index(vk::QUEUE_FAMILY_FOREIGN_EXT)
            .src_access_mask(vk::AccessFlags::SHADER_READ)];
        let size = (source.native().width(), source.native().height());
        let crop = native.crop_rect()?;
        if crop[0] < 0
            || crop[1] < 0
            || crop[2] <= crop[0]
            || crop[3] <= crop[1]
            || crop[2] as u32 > description.width
            || crop[3] as u32 > description.height
        {
            return Err("Decoder image crop lies outside its hardware buffer".into());
        }
        let length = (size.0 as usize)
            .checked_mul(size.1 as usize)
            .and_then(|count| count.checked_mul(4))
            .ok_or("Decoded image size overflow")?;
        let output = self.allocate(length)?;
        let crop = [
            crop[0] as f32,
            crop[1] as f32,
            (crop[2] - crop[0]) as f32,
            (crop[3] - crop[1]) as f32,
        ];
        let dimensions = [description.width as f32, description.height as f32];
        let mut arguments = self.arguments("sample_image_to_rgba")?;
        arguments
            .set("output", &output.address().to_ne_bytes())?
            .set("width", &size.0.to_ne_bytes())?
            .set("height", &size.1.to_ne_bytes())?
            .set("crop", &crop.map(f32::to_ne_bytes).concat())?
            .set("source_size", &dimensions.map(f32::to_ne_bytes).concat())?;
        let mut submission = unsafe {
            self.dispatch_bound(
                arguments,
                vec![output.clone()],
                [size.0 as usize, size.1 as usize, 1],
                DispatchBindings {
                    layout: imported.sampler.pipeline_layout,
                    sets: &sets,
                    acquire: &acquire,
                    release: &release,
                },
            )
        }?;
        submission._native_images.push(imported);
        Ok((output, submission))
    }
}
