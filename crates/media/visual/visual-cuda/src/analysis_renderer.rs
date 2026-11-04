//! Explicit CPU images for analysis and cache baking; normal preview/export stays on GPU.
use crate::compositor::{EXPORT_ASSETS_LOADING, VideoExportRenderer};
use ffmpeg_next::{format::Pixel, frame::Video};
use shrimply_project_document::project::{ItemAddress, Project, Time, TrackAddress};

pub(crate) enum Renderer {
    Cuda(Box<VideoExportRenderer>),
    #[cfg(target_os = "linux")]
    Intel(Box<shrimply_preview_render_vulkan::Compositor>),
}

pub(crate) enum Target<'a> {
    ModifierInput {
        address: &'a ItemAddress,
        snap_content: bool,
    },
    Track(&'a TrackAddress),
}

impl Renderer {
    pub fn new(sample_rate: u32) -> Result<Self, String> {
        #[cfg(target_os = "linux")]
        if crate::gpu::intel_backend_selected() {
            return shrimply_preview_render_vulkan::Compositor::new()
                .map(|renderer| Self::Intel(Box::new(renderer)));
        }
        VideoExportRenderer::new(sample_rate).map(|renderer| Self::Cuda(Box::new(renderer)))
    }

    pub fn alpha_mode(&self) -> shrimply_visual_core::modifier_cache::encoder::RgbaAlpha {
        match self {
            Self::Cuda(_) => {
                shrimply_visual_core::modifier_cache::encoder::RgbaAlpha::Premultiplied
            }
            #[cfg(target_os = "linux")]
            Self::Intel(_) => shrimply_visual_core::modifier_cache::encoder::RgbaAlpha::Straight,
        }
    }

    pub fn frame(
        &mut self,
        project: &Project,
        position: Time,
        target: Target<'_>,
    ) -> Result<Option<Video>, String> {
        let size = project.canvas_size;
        let mut output = Video::new(Pixel::RGBA, size.width, size.height);
        match self {
            Self::Cuda(renderer) => {
                let rendered = match target {
                    Target::ModifierInput {
                        address,
                        snap_content: true,
                    } => renderer.render_transparent_fill_input(project, position, address),
                    Target::ModifierInput {
                        address,
                        snap_content: false,
                    } => renderer.render_cache_item(project, position, address),
                    Target::Track(track) => renderer.render_track(project, position, track),
                };
                let frame = match rendered {
                    Ok(frame) => frame,
                    Err(error) if error == EXPORT_ASSETS_LOADING => return Ok(None),
                    Err(error) => return Err(error),
                };
                renderer.copy_to_rgba_frame(frame, &mut output)?;
            }
            #[cfg(target_os = "linux")]
            Self::Intel(renderer) => {
                let target = match target {
                    Target::ModifierInput {
                        address,
                        snap_content,
                    } => shrimply_preview_render_core::CaptureTarget::ModifierInput {
                        address: address.clone(),
                        snap_content,
                    },
                    Target::Track(track) => {
                        shrimply_preview_render_core::CaptureTarget::Track(track.clone())
                    }
                };
                let Some(pixels) = renderer.capture_rgba(project, position, target)? else {
                    return Ok(None);
                };
                let row_bytes = size.width as usize * 4;
                let stride = output.stride(0);
                if pixels.len() != row_bytes * size.height as usize {
                    return Err("Intel analysis frame has invalid dimensions".into());
                }
                for (source, destination) in pixels
                    .chunks_exact(row_bytes)
                    .zip(output.data_mut(0).chunks_exact_mut(stride))
                {
                    destination[..row_bytes].copy_from_slice(source);
                }
            }
        }
        Ok(Some(output))
    }
}
