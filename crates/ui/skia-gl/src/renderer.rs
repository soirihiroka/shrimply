use crate::gl_loader;
use glow::HasContext;
use shrimply_skia_adw_core::canvas::{Color, TimelinePainter, UVec2};
use skia_safe::{
    ColorType,
    gpu::{
        self, SurfaceOrigin, backend_render_targets, direct_contexts,
        gl::{Format as GlFormat, FramebufferInfo},
        surfaces,
    },
};

pub struct TimelineRenderer {
    context: Option<gpu::DirectContext>,
    surface: Option<skia_safe::Surface>,
    interface: Option<skia_safe::gpu::gl::Interface>,
}

impl Default for TimelineRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl TimelineRenderer {
    pub fn new() -> Self {
        Self {
            context: None,
            surface: None,
            interface: None,
        }
    }

    pub fn begin_frame(
        &mut self,
        screen_size_px: UVec2,
        pixels_per_point: f32,
        clear_color: Color,
    ) -> Result<TimelinePainter, String> {
        self.begin_frame_inner(screen_size_px, pixels_per_point, Some(clear_color))
    }

    pub fn begin_overlay_frame(
        &mut self,
        screen_size_px: UVec2,
        pixels_per_point: f32,
    ) -> Result<TimelinePainter, String> {
        self.begin_frame_inner(screen_size_px, pixels_per_point, None)
    }

    fn begin_frame_inner(
        &mut self,
        screen_size_px: UVec2,
        pixels_per_point: f32,
        clear_color: Option<Color>,
    ) -> Result<TimelinePainter, String> {
        if screen_size_px.x == 0 || screen_size_px.y == 0 {
            return Err(String::from("Invalid timeline surface size"));
        }
        if !pixels_per_point.is_finite() || pixels_per_point <= 0.0 {
            return Err(String::from("Invalid timeline pixels-per-point"));
        }

        if self.context.is_none() || self.interface.is_none() {
            let interface =
                gpu::gl::Interface::new_load_with(gl_loader::proc_address).ok_or_else(|| {
                    String::from("Could not initialize Skia OpenGL interface for timeline renderer")
                })?;
            let context = direct_contexts::make_gl(interface.clone(), None).ok_or_else(|| {
                String::from("Could not initialize Skia GL context for timeline renderer")
            })?;
            self.interface = Some(interface);
            self.context = Some(context);
        }

        let UVec2 {
            x: width,
            y: height,
        } = screen_size_px;
        let width = i32::try_from(width).map_err(|error| error.to_string())?;
        let height = i32::try_from(height).map_err(|error| error.to_string())?;
        if width <= 0 || height <= 0 {
            return Err(String::from("Invalid timeline surface size"));
        }

        let context = self
            .context
            .as_mut()
            .ok_or_else(|| String::from("Timeline Skia context missing when beginning a frame"))?;
        context.reset(None);

        let gl = gl_loader::context();
        let framebuffer = unsafe { gl.get_parameter_i32(glow::FRAMEBUFFER_BINDING) };
        let framebuffer_id =
            u32::try_from(framebuffer.max(0)).map_err(|error| error.to_string())?;

        let render_target = backend_render_targets::make_gl(
            (width, height),
            1,
            0,
            FramebufferInfo {
                fboid: framebuffer_id,
                format: GlFormat::RGBA8.into(),
                ..FramebufferInfo::default()
            },
        );

        let mut surface = match surfaces::wrap_backend_render_target(
            context,
            &render_target,
            SurfaceOrigin::BottomLeft,
            ColorType::RGBA8888,
            None,
            None,
        ) {
            Some(surface) => surface,
            None if context.oomed() => {
                context.free_gpu_resources();
                surfaces::wrap_backend_render_target(
                    context,
                    &render_target,
                    SurfaceOrigin::BottomLeft,
                    ColorType::RGBA8888,
                    None,
                    None,
                )
                .ok_or_else(|| {
                    String::from(
                        "Could not create timeline Skia surface after clearing its GPU cache",
                    )
                })?
            }
            None => return Err(String::from("Could not create timeline Skia surface")),
        };
        let canvas = surface.canvas();
        if let Some(clear_color) = clear_color {
            canvas.clear(clear_color);
        }
        canvas.scale((pixels_per_point, pixels_per_point));
        self.surface = Some(surface);

        let canvas = self
            .surface
            .as_mut()
            .ok_or_else(|| String::from("Could not access timeline Skia surface"))?
            .canvas();
        Ok(TimelinePainter::new(canvas))
    }

    pub fn end_frame(&mut self) -> Result<(), String> {
        let context = self
            .context
            .as_mut()
            .ok_or_else(|| String::from("Timeline Skia context missing when ending a frame"))?;
        context.flush_and_submit();
        if context.oomed() {
            context.free_gpu_resources();
            return Err(String::from(
                "Timeline Skia ran out of GPU memory and cleared its cache",
            ));
        }
        Ok(())
    }

    pub fn destroy(&mut self) {
        self.surface = None;
        self.context = None;
        self.interface = None;
    }
}
