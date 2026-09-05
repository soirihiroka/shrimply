use super::{Buffer, Renderer};
use objc2::rc::Retained;
use objc2_metal::MTLBuffer;
use skia_safe::{
    AlphaType, Canvas, ColorType, ImageInfo,
    gpu::{self, mtl},
};

impl Renderer {
    /// Draw shared vector geometry on a Metal Skia surface, then read straight
    /// RGBA into a fresh shared buffer for Slang. Readback blocks this worker,
    /// never the UI thread, and finishes before compute can access the buffer.
    pub fn draw_vector(
        &mut self,
        size: (u32, u32),
        draw: impl FnOnce(&Canvas) -> Result<(), String>,
    ) -> Result<Buffer, String> {
        let dimensions = (
            i32::try_from(size.0).map_err(|_| "Vector width exceeds Skia limits")?,
            i32::try_from(size.1).map_err(|_| "Vector height exceeds Skia limits")?,
        );
        let row_bytes = (size.0 as usize)
            .checked_mul(size_of::<u32>())
            .ok_or("Vector row size overflow")?;
        let length = row_bytes
            .checked_mul(size.1 as usize)
            .ok_or("Vector image size overflow")?;
        let output = self.allocate(length)?;
        if self.skia.is_none() {
            // The backend context retains this renderer's device and queue.
            let backend = unsafe {
                mtl::BackendContext::new(
                    Retained::as_ptr(&self.device) as mtl::Handle,
                    Retained::as_ptr(&self.queue) as mtl::Handle,
                )
            };
            self.skia = Some(
                gpu::direct_contexts::make_metal(&backend, None)
                    .ok_or("Could not create Metal Skia context")?,
            );
        }
        let info = ImageInfo::new(dimensions, ColorType::RGBA8888, AlphaType::Premul, None);
        let mut surface = gpu::surfaces::render_target(
            self.skia.as_mut().expect("initialized Metal Skia context"),
            gpu::Budgeted::Yes,
            &info,
            None,
            gpu::SurfaceOrigin::TopLeft,
            None,
            None,
            None,
        )
        .ok_or("Could not allocate Metal vector surface")?;
        draw(surface.canvas())?;
        // This newly allocated buffer has no aliases or GPU users. Skia's
        // synchronous read performs the premultiplied-to-straight conversion.
        let bytes =
            unsafe { std::slice::from_raw_parts_mut(output.0.contents().as_ptr().cast(), length) };
        if !surface.read_pixels(
            &info.with_alpha_type(AlphaType::Unpremul),
            bytes,
            row_bytes,
            (0, 0),
        ) {
            return Err("Could not read Metal vector surface".into());
        }
        Ok(output)
    }
}
