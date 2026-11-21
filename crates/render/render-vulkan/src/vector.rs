use crate::{Buffer, Renderer, Submission};
use ash::{vk, vk::Handle};
use skia_safe::{AlphaType, Canvas, ColorType, ImageInfo, gpu};

pub(super) enum SurfaceFormat {
    Rgba8,
    OpaqueRgba8,
    RgbaF16,
}

impl Renderer {
    pub fn mesh_flow(
        &mut self,
        input: Buffer,
        width: u32,
        height: u32,
        grid_width: u32,
        grid_height: u32,
        offsets: &[[f32; 2]],
    ) -> Result<(Buffer, Submission), String> {
        if width < 2
            || height < 2
            || grid_width < 2
            || grid_height < 2
            || (grid_width as usize).checked_mul(grid_height as usize) != Some(offsets.len())
            || offsets.iter().flatten().any(|value| !value.is_finite())
        {
            return Err("Invalid Vulkan MeshFlow field".into());
        }
        let count = (width as usize)
            .checked_mul(height as usize)
            .ok_or("MeshFlow dimensions overflow")?;
        if input.len() != count.checked_mul(4).ok_or("MeshFlow size overflow")? {
            return Err("MeshFlow source dimensions do not match its storage".into());
        }
        let bytes = offsets
            .iter()
            .flat_map(|offset| offset.iter().flat_map(|value| value.to_ne_bytes()))
            .collect::<Vec<_>>();
        let offsets = self.upload(&bytes)?;
        self.mesh_flow_buffer(
            input,
            (width, height),
            (grid_width, grid_height),
            offsets,
            1.0,
        )
    }

    pub fn mesh_flow_buffer(
        &mut self,
        input: Buffer,
        size: (u32, u32),
        grid: (u32, u32),
        offsets: Buffer,
        scale: f32,
    ) -> Result<(Buffer, Submission), String> {
        let (width, height) = size;
        let (grid_width, grid_height) = grid;
        let count = (width as usize)
            .checked_mul(height as usize)
            .ok_or("MeshFlow size overflow")?;
        if width < 2
            || height < 2
            || grid_width < 2
            || grid_height < 2
            || !scale.is_finite()
            || count.checked_mul(4) != Some(input.len())
            || (grid_width as usize)
                .checked_mul(grid_height as usize)
                .and_then(|n| n.checked_mul(size_of::<[f32; 2]>()))
                != Some(offsets.len())
        {
            return Err("Invalid GPU MeshFlow storage".into());
        }
        let output = self.allocate(input.len())?;
        let mut arguments = self.arguments("mesh_flow_warp")?;
        arguments
            .set("input", &input.address().to_ne_bytes())?
            .set("offsets", &offsets.address().to_ne_bytes())?
            .set("output", &output.address().to_ne_bytes())?
            .set("width", &width.to_ne_bytes())?
            .set("height", &height.to_ne_bytes())?
            .set("grid_width", &grid_width.to_ne_bytes())?
            .set("grid_height", &grid_height.to_ne_bytes())?
            .set("offset_scale", &scale.to_ne_bytes())?;
        let submission = unsafe {
            self.dispatch(
                arguments,
                vec![input, offsets, output.clone()],
                [count, 1, 1],
            )
        }?;
        Ok((output, submission))
    }
    pub fn draw_vector(
        &mut self,
        size: (u32, u32),
        draw: impl FnOnce(&Canvas) -> Result<(), String>,
    ) -> Result<(Buffer, Submission), String> {
        self.draw_surface(size, SurfaceFormat::Rgba8, draw)
    }

    pub(super) fn draw_surface(
        &mut self,
        size: (u32, u32),
        format: SurfaceFormat,
        draw: impl FnOnce(&Canvas) -> Result<(), String>,
    ) -> Result<(Buffer, Submission), String> {
        let _total = shrimply_profiling::measure("Vulkan vector / Total");
        let allocation = shrimply_profiling::measure("Vulkan vector / Allocate surface");
        let dimensions = (
            i32::try_from(size.0).map_err(|_| "Vector width exceeds Skia limits")?,
            i32::try_from(size.1).map_err(|_| "Vector height exceeds Skia limits")?,
        );
        let (color_type, pixel_bytes, conversion) = match format {
            SurfaceFormat::Rgba8 => (
                ColorType::RGBA8888,
                size_of::<u32>(),
                Some("unpremultiply_rgba"),
            ),
            SurfaceFormat::OpaqueRgba8 => (ColorType::RGBA8888, size_of::<u32>(), None),
            SurfaceFormat::RgbaF16 => (
                ColorType::RGBAF16,
                size_of::<[u16; 4]>(),
                Some("unpremultiply_rgba_f16"),
            ),
        };
        let count = (size.0 as usize)
            .checked_mul(size.1 as usize)
            .ok_or("Vector surface size overflow")?;
        let length = count
            .checked_mul(pixel_bytes)
            .ok_or("Vector surface byte size overflow")?;
        if self.skia.is_none() {
            self.skia = Some(self.device.skia_context(false)?);
        }
        let info = ImageInfo::new(dimensions, color_type, AlphaType::Premul, None);
        let skia = self.skia.as_mut().expect("initialized Vulkan Skia context");
        let mut surface = gpu::surfaces::render_target(
            skia,
            gpu::Budgeted::Yes,
            &info,
            None,
            gpu::SurfaceOrigin::TopLeft,
            None,
            None,
            None,
        )
        .ok_or("Allocate Vulkan vector surface")?;
        drop(allocation);
        let recording = shrimply_profiling::measure("Vulkan vector / Record geometry");
        surface.canvas().clear(skia_safe::Color::TRANSPARENT);
        draw(surface.canvas())?;
        drop(recording);
        let snapshot = shrimply_profiling::measure("Vulkan vector / Snapshot surface");
        let image = surface.image_snapshot();
        drop(snapshot);
        let flush = shrimply_profiling::measure("Vulkan vector / Flush Skia");
        skia.flush_and_submit();
        drop(flush);
        let copying = shrimply_profiling::measure("Vulkan vector / Submit image copy");
        let (texture, origin) = gpu::images::get_backend_texture_from_image(&image, false)
            .ok_or("Get Vulkan vector texture")?;
        if origin != gpu::SurfaceOrigin::TopLeft {
            return Err("Vulkan vector texture has an unexpected origin".into());
        }
        let native = gpu::backend_textures::get_vk_image_info(&texture)
            .ok_or("Vulkan vector texture is not Vulkan")?;
        let source = vk::Image::from_raw(*native.image() as u64);
        let old_layout = vk::ImageLayout::from_raw(native.layout as i32);
        let copied = self.allocate(length)?;
        let command = self.command()?;
        let range = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .level_count(1)
            .layer_count(1);
        let barrier = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::MEMORY_WRITE)
            .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
            .old_layout(old_layout)
            .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(source)
            .subresource_range(range);
        let restore = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::TRANSFER_READ)
            .dst_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE)
            .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .new_layout(old_layout)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(source)
            .subresource_range(range);
        unsafe {
            self.device.raw.cmd_pipeline_barrier(
                command,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier],
            );
            self.device.raw.cmd_copy_image_to_buffer(
                command,
                source,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                copied.raw(),
                &[vk::BufferImageCopy::default()
                    .image_subresource(
                        vk::ImageSubresourceLayers::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .layer_count(1),
                    )
                    .image_extent(vk::Extent3D {
                        width: size.0,
                        height: size.1,
                        depth: 1,
                    })],
            );
            self.device.raw.cmd_pipeline_barrier(
                command,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[restore],
            );
        }
        let mut copy = self.submit(command, vec![copied.clone()])?;
        copy._images.push(image);
        drop(copying);
        // Premultiplied and straight RGBA are identical for proven opaque output.
        let Some(conversion) = conversion else {
            return Ok((copied, copy));
        };
        let _conversion = shrimply_profiling::measure("Vulkan vector / Submit alpha conversion");
        let output = self.allocate(
            count
                .checked_mul(size_of::<u32>())
                .ok_or("Vector output byte size overflow")?,
        )?;
        let mut arguments = self.arguments(conversion)?;
        arguments
            .set("input", &copied.address().to_ne_bytes())?
            .set("output", &output.address().to_ne_bytes())?
            .set("count", &(count as u64).to_ne_bytes())?;
        let mut converted =
            unsafe { self.dispatch(arguments, vec![copied, output.clone()], [count, 1, 1]) }?;
        converted._prior.push(copy);
        Ok((output, converted))
    }

    pub fn background(
        &mut self,
        uniforms: &shrimply_render_core::background_spirv::BackgroundUniforms,
    ) -> Result<(Buffer, Submission), String> {
        let width = uniforms.common.width as usize;
        let height = uniforms.common.height as usize;
        let length = width
            .checked_mul(height)
            .and_then(|count| count.checked_mul(4))
            .ok_or("Background dimensions overflow")?;
        let output = self.allocate(length)?;
        let mut arguments = self.arguments("render_background")?;
        arguments.set("output", &output.address().to_ne_bytes())?;
        shrimply_render_core::background_arguments(uniforms, |name, bytes| {
            arguments.set(name, bytes).map(|_| ())
        })?;
        let submission =
            unsafe { self.dispatch(arguments, vec![output.clone()], [width, height, 1]) }?;
        Ok((output, submission))
    }
}
