use std::{ptr::NonNull, sync::Arc};

use ffmpeg_next::format::Pixel;
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_core_video::{
    CVMetalTexture, CVMetalTextureCache, CVMetalTextureGetTexture, CVMetalTextureIsFlipped,
    CVPixelBuffer, CVPixelBufferGetHeightOfPlane, CVPixelBufferGetPixelFormatType,
    CVPixelBufferGetPlaneCount, CVPixelBufferGetWidthOfPlane,
    kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
    kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
    kCVPixelFormatType_420YpCbCr10BiPlanarFullRange,
    kCVPixelFormatType_420YpCbCr10BiPlanarVideoRange, kCVReturnSuccess,
};
use objc2_metal::{
    MTLCommandBuffer, MTLCommandEncoder, MTLCommandQueue, MTLComputeCommandEncoder, MTLPixelFormat,
    MTLResource, MTLResourceUsage, MTLSize,
};
use shrimply_visual_frame::{GpuFrame, VideoFrame, math::video_color};

use crate::{Buffer, Renderer, Submission};

/// Decoder textures and their native owner stay alive through GPU completion.
pub struct ImportedFrame {
    submission: Submission,
    pub output: Buffer,
    _textures: Vec<Retained<CVMetalTexture>>,
    source: Arc<VideoFrame>,
}

impl Drop for ImportedFrame {
    fn drop(&mut self) {
        if self.submission.wait().is_err() {
            std::process::abort();
        }
    }
}

impl GpuFrame for ImportedFrame {
    type Context = Renderer;

    fn source(&self) -> &Arc<VideoFrame> {
        &self.source
    }

    fn import(renderer: &mut Renderer, source: Arc<VideoFrame>) -> Result<Self, String> {
        if source.native().format() != Pixel::VIDEOTOOLBOX {
            return Err("Metal video import requires a VideoToolbox hardware frame".into());
        }
        let native = unsafe { &*source.native().as_ptr() };
        // FFmpeg's VideoToolbox AVFrame owns a CVPixelBufferRef in data[3].
        let pixel = unsafe { native.data[3].cast::<CVPixelBuffer>().as_ref() }
            .ok_or("VideoToolbox frame has no pixel buffer")?;
        if CVPixelBufferGetPlaneCount(pixel) != 2 || native.crop_left != 0 || native.crop_top != 0 {
            return Err("unsupported VideoToolbox plane layout or origin crop".into());
        }
        let format = CVPixelBufferGetPixelFormatType(pixel);
        let (p010, full_range, formats) = match format {
            kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
            | kCVPixelFormatType_420YpCbCr8BiPlanarFullRange => (
                false,
                format == kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
                [MTLPixelFormat::R8Unorm, MTLPixelFormat::RG8Unorm],
            ),
            kCVPixelFormatType_420YpCbCr10BiPlanarVideoRange
            | kCVPixelFormatType_420YpCbCr10BiPlanarFullRange => (
                true,
                format == kCVPixelFormatType_420YpCbCr10BiPlanarFullRange,
                [MTLPixelFormat::R16Unorm, MTLPixelFormat::RG16Unorm],
            ),
            format => {
                return Err(format!(
                    "unsupported VideoToolbox pixel format: {format:#x}"
                ));
            }
        };
        if renderer.video_cache.is_none() {
            let mut raw = std::ptr::null_mut();
            let status = unsafe {
                CVMetalTextureCache::create(
                    None,
                    None,
                    &renderer.context.device,
                    None,
                    NonNull::from(&mut raw),
                )
            };
            if status != kCVReturnSuccess {
                return Err(format!("create Core Video Metal texture cache: {status}"));
            }
            renderer.video_cache = Some(
                unsafe { Retained::from_raw(raw) }
                    .ok_or("Core Video returned no Metal texture cache")?,
            );
        }
        let cache = renderer
            .video_cache
            .as_ref()
            .expect("initialized video texture cache");
        let mut views = Vec::with_capacity(formats.len());
        let mut textures = Vec::with_capacity(formats.len());
        let width = source.native().width();
        let height = source.native().height();
        for (plane, format) in formats.into_iter().enumerate() {
            let plane_width = CVPixelBufferGetWidthOfPlane(pixel, plane);
            let plane_height = CVPixelBufferGetHeightOfPlane(pixel, plane);
            let expected = if plane == 0 {
                (width as usize, height as usize)
            } else {
                (width.div_ceil(2) as usize, height.div_ceil(2) as usize)
            };
            if plane_width < expected.0 || plane_height < expected.1 {
                return Err("VideoToolbox plane is smaller than its visible image".into());
            }
            let mut raw = std::ptr::null_mut();
            let status = unsafe {
                CVMetalTextureCache::create_texture_from_image(
                    None,
                    cache,
                    pixel,
                    None,
                    format,
                    plane_width,
                    plane_height,
                    plane,
                    NonNull::from(&mut raw),
                )
            };
            if status != kCVReturnSuccess {
                return Err(format!(
                    "import VideoToolbox plane {plane} into Metal: {status}"
                ));
            }
            let view =
                unsafe { Retained::from_raw(raw) }.ok_or("Core Video returned no Metal plane")?;
            textures.push(
                CVMetalTextureGetTexture(&view).ok_or("Core Video plane has no Metal texture")?,
            );
            views.push(view);
        }
        // Core Video calls an upper-left origin "flipped"; our output is upper-left too.
        let upper_left = CVMetalTextureIsFlipped(&views[0]);
        if CVMetalTextureIsFlipped(&views[1]) != upper_left {
            return Err("VideoToolbox plane orientations disagree".into());
        }
        let count = (width as usize)
            .checked_mul(height as usize)
            .ok_or("video pixel count overflow")?;
        let output = renderer.allocate(
            count
                .checked_mul(size_of::<u32>())
                .ok_or("video buffer size overflow")?,
        )?;
        let color = video_color(source.native())?;
        let mut arguments = renderer.arguments("video_texture_to_rgba")?;
        arguments
            .set("output", &output.address().to_ne_bytes())?
            .set("width", &width.to_ne_bytes())?
            .set("height", &height.to_ne_bytes())?
            .set(
                "coefficients",
                color.coefficients.map(i32::to_ne_bytes).as_flattened(),
            )?
            .set(
                "chroma_offset",
                color.chroma_offset.map(f32::to_ne_bytes).as_flattened(),
            )?
            .set("full_range", &[u8::from(full_range)])?
            .set("p010", &[u8::from(p010)])?
            .set("flip_y", &[u8::from(!upper_left)])?;
        let argument_buffer = renderer.upload(&arguments.bytes)?;
        let command = renderer
            .context
            .queue
            .commandBuffer()
            .ok_or("create Metal video conversion command")?;
        let encoder = command
            .computeCommandEncoder()
            .ok_or("create Metal video conversion encoder")?;
        let kernel = renderer.kernel("video_texture_to_rgba")?;
        encoder.setComputePipelineState(&kernel.pipeline);
        unsafe {
            encoder.setBuffer_offset_atIndex(Some(argument_buffer.metal()), 0, kernel.binding)
        };
        for (name, texture) in ["y_texture", "uv_texture"].into_iter().zip(&textures) {
            let (_, index) = kernel
                .layout
                .textures
                .iter()
                .find(|(field, _)| *field == name)
                .ok_or_else(|| format!("video shader has no {name} texture binding"))?;
            unsafe { encoder.setTexture_atIndex(Some(texture), *index) };
        }
        for buffer in [&output, &argument_buffer] {
            let resource: &ProtocolObject<dyn MTLResource> =
                ProtocolObject::from_ref(&**buffer.metal());
            encoder.useResource_usage(resource, MTLResourceUsage::Read | MTLResourceUsage::Write);
        }
        let [x, y, z] = kernel.layout.group;
        encoder.dispatchThreads_threadsPerThreadgroup(
            MTLSize {
                width: count,
                height: 1,
                depth: 1,
            },
            MTLSize {
                width: x,
                height: y,
                depth: z,
            },
        );
        encoder.endEncoding();
        command.commit();
        let submission = Submission::new(command, vec![argument_buffer, output.clone()]);
        Ok(Self {
            submission,
            output,
            _textures: views,
            source,
        })
    }
}
