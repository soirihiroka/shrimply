//! Ordinary image composition uses the same persistent Skia GPU context as vectors.
use super::{Buffer, Frame, Renderer};
use ash::vk;
use shrimply_render_core::{LayerKind, Nv12LayerParams, VideoSampleMethod};
use skia_safe::{AlphaType, ColorType, Paint, Rect, SamplingOptions, TileMode, gpu};

impl Renderer {
    pub(super) fn composite_quads(
        &mut self,
        layers: &[(Nv12LayerParams, Buffer)],
        size: (u32, u32),
        background: u32,
    ) -> Result<Option<Frame>, String> {
        let Some(geometry) = layers
            .iter()
            .map(|(p, _)| shrimply_render_core::math::rgba_quad(p))
            .collect::<Option<Vec<_>>>()
        else {
            return Ok(None);
        };
        let _measurement = shrimply_profiling::measure("Vulkan rendering / Skia quad composition");
        let mut textures = Vec::with_capacity(layers.len());
        let mut prior = Vec::with_capacity(layers.len());
        for (p, buffer) in layers {
            let (texture, copy) =
                self.upload_texture(buffer, (p.source_width, p.source_height), false)?;
            textures.push(texture);
            prior.push(copy);
        }
        if self.skia.is_none() {
            self.skia = Some(self.device.skia_context(false)?);
        }
        let context = self.skia.as_mut().expect("initialized Skia compositor");
        let mut images = Vec::with_capacity(layers.len());
        for (texture, (p, _)) in textures.iter().zip(layers) {
            let mut info = unsafe {
                gpu::vk::ImageInfo::new(
                    texture.id() as _,
                    gpu::vk::Alloc::default(),
                    gpu::vk::ImageTiling::OPTIMAL,
                    gpu::vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                    gpu::vk::Format::R8G8B8A8_UNORM,
                    1,
                    self.device.family,
                    None,
                    None,
                    None,
                )
            };
            info.image_usage_flags = (vk::ImageUsageFlags::SAMPLED
                | vk::ImageUsageFlags::TRANSFER_SRC
                | vk::ImageUsageFlags::TRANSFER_DST)
                .as_raw();
            let backend = unsafe {
                gpu::backend_textures::make_vk(
                    (p.source_width as i32, p.source_height as i32),
                    &info,
                    "compositor source",
                )
            };
            images.push(
                gpu::images::borrow_texture_from(
                    context,
                    &backend,
                    gpu::SurfaceOrigin::TopLeft,
                    ColorType::RGBA8888,
                    if p.kind == LayerKind::RgbaOpaque {
                        AlphaType::Opaque
                    } else {
                        AlphaType::Unpremul
                    },
                    None,
                )
                .ok_or("Borrow Skia compositor source")?,
            );
        }
        let opaque = background.to_le_bytes()[3] == u8::MAX
            || layers.iter().any(|(p, _)| {
                p.kind == LayerKind::RgbaOpaque
                    && p.opacity >= 1.
                    && shrimply_render_core::math::aligned_rgba_layer(p, size.0, size.1)
            });
        let format = if opaque {
            super::vector::SurfaceFormat::OpaqueRgba8
        } else {
            super::vector::SurfaceFormat::RgbaF16
        };
        let (output, mut submission) = self.draw_surface(size, format, |canvas| {
            let [r, g, b, a] = background.to_le_bytes();
            canvas.clear(skia_safe::Color::from_argb(a, r, g, b));
            for (((p, _), image), (transform, crop)) in layers.iter().zip(&images).zip(&geometry) {
                let sampling = match p.sample_method {
                    VideoSampleMethod::Nearest => {
                        SamplingOptions::from(skia_safe::FilterMode::Nearest)
                    }
                    VideoSampleMethod::Bilinear => {
                        SamplingOptions::from(skia_safe::FilterMode::Linear)
                    }
                    VideoSampleMethod::Bicubic => {
                        SamplingOptions::from(skia_safe::CubicResampler::catmull_rom())
                    }
                    _ => unreachable!("validated Skia quad sampling"),
                };
                let mut paint = Paint::default();
                paint.set_alpha_f(p.opacity.clamp(0., 1.));
                paint.set_shader(
                    image
                        .to_shader((TileMode::Decal, TileMode::Decal), sampling, None)
                        .ok_or("Create compositor image shader")?,
                );
                let save = canvas.save();
                canvas.concat(&shrimply_math_geometry::to_skia_matrix(*transform));
                let rect = Rect::new(crop[0], crop[1], crop[2], crop[3]);
                canvas.draw_rect(rect, &paint);
                canvas.restore_to_count(save);
            }
            Ok(())
        })?;
        submission._prior.extend(prior);
        submission._images.extend(images);
        submission._textures.extend(textures);
        shrimply_profiling::increment("Vulkan rendering / Skia quad compositions");
        Ok(Some(Frame { submission, output }))
    }
}
