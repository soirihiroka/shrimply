#![cfg(target_os = "macos")]

use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_metal::{
    MTLCommandBuffer, MTLCommandQueue, MTLCreateSystemDefaultDevice, MTLDevice, MTLDrawable,
    MTLPixelFormat,
};
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};
use skia_safe::{
    Canvas, ColorType,
    gpu::{self, DirectContext, SurfaceOrigin, backend_render_targets, mtl},
};

pub struct Renderer {
    context: DirectContext,
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    layer: Retained<CAMetalLayer>,
}

impl Default for Renderer {
    fn default() -> Self {
        let device = MTLCreateSystemDefaultDevice().expect("Metal device unavailable");
        let queue = device
            .newCommandQueue()
            .expect("create Metal command queue");
        // Skia retains both native objects for the lifetime of the context.
        let backend = unsafe {
            mtl::BackendContext::new(
                Retained::as_ptr(&device) as mtl::Handle,
                Retained::as_ptr(&queue) as mtl::Handle,
            )
        };
        let context =
            gpu::direct_contexts::make_metal(&backend, None).expect("create Skia Metal context");
        let layer = CAMetalLayer::new();
        layer.setDevice(Some(&device));
        layer.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
        layer.setFramebufferOnly(false);
        Self {
            context,
            queue,
            layer,
        }
    }
}

impl Renderer {
    pub fn layer(&self) -> &CAMetalLayer {
        &self.layer
    }

    pub fn draw(&mut self, paint: impl FnOnce(&Canvas)) {
        // An occluded or detached layer may have no drawable available.
        let Some(drawable) = self.layer.nextDrawable() else {
            return;
        };
        let size = self.layer.drawableSize();
        let texture = drawable.texture();
        // The drawable retains its texture until Skia's work has been submitted.
        let texture = unsafe { mtl::TextureInfo::new(Retained::as_ptr(&texture) as mtl::Handle) };
        let target =
            backend_render_targets::make_mtl((size.width as i32, size.height as i32), &texture);
        let mut surface = gpu::surfaces::wrap_backend_render_target(
            &mut self.context,
            &target,
            SurfaceOrigin::TopLeft,
            ColorType::BGRA8888,
            None,
            None,
        )
        .expect("wrap Metal drawable in Skia surface");
        paint(surface.canvas());
        self.context.flush_and_submit();
        drop(surface);
        let command = self
            .queue
            .commandBuffer()
            .expect("create Metal presentation command");
        let drawable: Retained<ProtocolObject<dyn MTLDrawable>> = (&drawable).into();
        command.presentDrawable(&drawable);
        command.commit();
    }
}
