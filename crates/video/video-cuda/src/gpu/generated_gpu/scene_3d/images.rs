use super::*;

impl Scene3dRenderer {
    pub(super) fn upload_environment(
        &self,
        width: u32,
        height: u32,
        pixels: &[Color],
    ) -> Result<VulkanTexture, String> {
        if width == 0 || height == 0 {
            return Err("environment dimensions must be nonzero".to_string());
        }
        let expected = width as usize * height as usize;
        if pixels.len() != expected {
            return Err(format!(
                "environment has {} pixels, expected {expected}",
                pixels.len()
            ));
        }
        let properties = unsafe {
            self.vulkan
                .instance
                .get_physical_device_format_properties(self.physical_device, ENVIRONMENT_FORMAT)
        };
        let required = vk::FormatFeatureFlags::SAMPLED_IMAGE
            | vk::FormatFeatureFlags::SAMPLED_IMAGE_FILTER_LINEAR
            | vk::FormatFeatureFlags::BLIT_SRC
            | vk::FormatFeatureFlags::BLIT_DST;
        if !properties.optimal_tiling_features.contains(required) {
            return Err(
                "Vulkan device cannot linearly sample and mipmap RGBA32F environments".to_string(),
            );
        }
        let mip_levels = 32 - width.max(height).leading_zeros();
        let staging = VulkanBuffer::new(
            self.vulkan.clone(),
            self.physical_device,
            size_of_val(pixels) as u64,
            vk::BufferUsageFlags::TRANSFER_SRC,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        staging.write(pixels)?;
        let texture = VulkanTexture::new(
            self.vulkan.clone(),
            self.physical_device,
            width,
            height,
            mip_levels,
            ENVIRONMENT_FORMAT,
            vk::ImageUsageFlags::TRANSFER_SRC
                | vk::ImageUsageFlags::TRANSFER_DST
                | vk::ImageUsageFlags::SAMPLED,
            vk::ImageAspectFlags::COLOR,
        )?;
        let command = self.begin_commands("allocate environment upload command buffer")?;
        transition_image(
            &self.vulkan.device,
            command.handle,
            texture.image,
            vk::ImageLayout::UNDEFINED,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
            vk::AccessFlags::empty(),
            vk::AccessFlags::TRANSFER_WRITE,
            0,
            mip_levels,
        );
        let copy = vk::BufferImageCopy::default()
            .image_subresource(vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            })
            .image_extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            });
        unsafe {
            self.vulkan.device.cmd_copy_buffer_to_image(
                command.handle,
                staging.buffer,
                texture.image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[copy],
            );
        }
        let mut mip_width = width as i32;
        let mut mip_height = height as i32;
        for mip in 1..mip_levels {
            transition_image(
                &self.vulkan.device,
                command.handle,
                texture.image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::TRANSFER,
                vk::AccessFlags::TRANSFER_WRITE,
                vk::AccessFlags::TRANSFER_READ,
                mip - 1,
                1,
            );
            let next_width = (mip_width / 2).max(1);
            let next_height = (mip_height / 2).max(1);
            let blit = vk::ImageBlit::default()
                .src_subresource(vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: mip - 1,
                    base_array_layer: 0,
                    layer_count: 1,
                })
                .src_offsets([
                    vk::Offset3D { x: 0, y: 0, z: 0 },
                    vk::Offset3D {
                        x: mip_width,
                        y: mip_height,
                        z: 1,
                    },
                ])
                .dst_subresource(vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: mip,
                    base_array_layer: 0,
                    layer_count: 1,
                })
                .dst_offsets([
                    vk::Offset3D { x: 0, y: 0, z: 0 },
                    vk::Offset3D {
                        x: next_width,
                        y: next_height,
                        z: 1,
                    },
                ]);
            unsafe {
                self.vulkan.device.cmd_blit_image(
                    command.handle,
                    texture.image,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    texture.image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[blit],
                    vk::Filter::LINEAR,
                );
            }
            mip_width = next_width;
            mip_height = next_height;
        }
        if mip_levels > 1 {
            transition_image(
                &self.vulkan.device,
                command.handle,
                texture.image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::RAY_TRACING_SHADER_KHR,
                vk::AccessFlags::TRANSFER_READ,
                vk::AccessFlags::SHADER_READ,
                0,
                mip_levels - 1,
            );
        }
        transition_image(
            &self.vulkan.device,
            command.handle,
            texture.image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::RAY_TRACING_SHADER_KHR,
            vk::AccessFlags::TRANSFER_WRITE,
            vk::AccessFlags::SHADER_READ,
            mip_levels - 1,
            1,
        );
        self.submit_and_wait(command, "upload environment image")?;
        Ok(texture)
    }

    pub(super) fn upload_material_texture(
        &self,
        atlas: &shrimply_scene_3d::TextureAtlas,
    ) -> Result<VulkanTexture, String> {
        let expected = atlas.width as usize * atlas.height as usize;
        if atlas.width == 0 || atlas.height == 0 || atlas.pixels.len() != expected {
            return Err("3D material texture atlas has invalid dimensions".to_string());
        }
        let properties = unsafe {
            self.vulkan
                .instance
                .get_physical_device_format_properties(self.physical_device, COLOR_FORMAT)
        };
        let required = vk::FormatFeatureFlags::SAMPLED_IMAGE
            | vk::FormatFeatureFlags::SAMPLED_IMAGE_FILTER_LINEAR;
        if !properties.optimal_tiling_features.contains(required) {
            return Err("Vulkan device cannot linearly sample RGBA8 material textures".to_string());
        }
        let staging = VulkanBuffer::new(
            self.vulkan.clone(),
            self.physical_device,
            size_of_val(atlas.pixels.as_slice()) as u64,
            vk::BufferUsageFlags::TRANSFER_SRC,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        staging.write(&atlas.pixels)?;
        let texture = VulkanTexture::new(
            self.vulkan.clone(),
            self.physical_device,
            atlas.width,
            atlas.height,
            1,
            COLOR_FORMAT,
            vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED,
            vk::ImageAspectFlags::COLOR,
        )?;
        let command = self.begin_commands("allocate material texture upload command buffer")?;
        transition_image(
            &self.vulkan.device,
            command.handle,
            texture.image,
            vk::ImageLayout::UNDEFINED,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
            vk::AccessFlags::empty(),
            vk::AccessFlags::TRANSFER_WRITE,
            0,
            1,
        );
        let copy = vk::BufferImageCopy::default()
            .image_subresource(vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            })
            .image_extent(vk::Extent3D {
                width: atlas.width,
                height: atlas.height,
                depth: 1,
            });
        unsafe {
            self.vulkan.device.cmd_copy_buffer_to_image(
                command.handle,
                staging.buffer,
                texture.image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[copy],
            );
        }
        transition_image(
            &self.vulkan.device,
            command.handle,
            texture.image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::RAY_TRACING_SHADER_KHR,
            vk::AccessFlags::TRANSFER_WRITE,
            vk::AccessFlags::SHADER_READ,
            0,
            1,
        );
        self.submit_and_wait(command, "upload material texture atlas")?;
        Ok(texture)
    }
}
