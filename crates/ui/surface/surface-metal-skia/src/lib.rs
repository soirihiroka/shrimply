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
use std::time::{Duration, Instant};

// Diagnostic thresholds only; these do not control rendering cadence.
const SLOW_SUBMISSION: Duration = Duration::from_millis(20);
const SLOW_LOG_INTERVAL: Duration = Duration::from_secs(1);

pub struct Renderer {
    context: DirectContext,
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    layer: Retained<CAMetalLayer>,
    last_slow_log: Option<Instant>,
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
            last_slow_log: None,
        }
    }
}

impl Renderer {
    pub fn layer(&self) -> &CAMetalLayer {
        &self.layer
    }

    pub fn draw(&mut self, label: &'static str, paint: impl FnOnce(&Canvas)) {
        let started = Instant::now();
        // Drain autoreleased drawable/command references after each submission,
        // rather than retaining them until the surrounding AppKit run-loop pool drains.
        let phases = objc2::rc::autoreleasepool(|_| {
            // An occluded or detached layer may have no drawable available.
            let drawable = self.layer.nextDrawable();
            let acquired = Instant::now();
            let Some(drawable) = drawable else {
                tracing::warn!(
                    surface = label,
                    elapsed_us = started.elapsed().as_micros(),
                    "UI drawable unavailable"
                );
                return None;
            };
            let size = self.layer.drawableSize();
            let texture = drawable.texture();
            // The drawable retains its texture until Skia's work has been submitted.
            let texture =
                unsafe { mtl::TextureInfo::new(Retained::as_ptr(&texture) as mtl::Handle) };
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
            let wrapped = Instant::now();
            paint(surface.canvas());
            let painted = Instant::now();
            self.context.flush_and_submit();
            let flushed = Instant::now();
            drop(surface);
            let released = Instant::now();
            let command = self
                .queue
                .commandBuffer()
                .expect("create Metal presentation command");
            let drawable: Retained<ProtocolObject<dyn MTLDrawable>> = (&drawable).into();
            command.presentDrawable(&drawable);
            command.commit();
            Some((
                acquired,
                wrapped,
                painted,
                flushed,
                released,
                Instant::now(),
            ))
        });
        let elapsed = started.elapsed();
        if elapsed >= SLOW_SUBMISSION
            && self
                .last_slow_log
                .is_none_or(|last| last.elapsed() >= SLOW_LOG_INTERVAL)
            && let Some((acquired, wrapped, painted, flushed, released, committed)) = phases
        {
            self.last_slow_log = Some(Instant::now());
            let size = self.layer.drawableSize();
            tracing::warn!(
                surface = label,
                width = size.width,
                height = size.height,
                total_us = elapsed.as_micros(),
                acquire_us = acquired.duration_since(started).as_micros(),
                wrap_us = wrapped.duration_since(acquired).as_micros(),
                paint_us = painted.duration_since(wrapped).as_micros(),
                flush_us = flushed.duration_since(painted).as_micros(),
                surface_release_us = released.duration_since(flushed).as_micros(),
                present_us = committed.duration_since(released).as_micros(),
                autorelease_us = committed.elapsed().as_micros(),
                "Slow UI submission: phases from the same draw"
            );
        }
    }
}
