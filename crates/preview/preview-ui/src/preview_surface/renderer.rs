use glam::IVec2;
use glow::HasContext;
use shrimply_math_color::Color;

use crate::cuda_gl::CudaTexture;
use crate::gl_loader;
use crate::preferences::store::{PreviewDownsampleMethod, PreviewUpsampleMethod};
use crate::timeline::renderer::{Rect, TimelinePainter, TimelineRenderer};
use crate::video::gpu::{CompositedFrameStorageKey, CompositedVideoFrame};

#[derive(Clone, Copy)]
pub(super) struct Appearance {
    pub content_rect: Rect,
    pub shadow_size_px: u32,
    pub background_color: Color,
    pub upsample_method: PreviewUpsampleMethod,
    pub downsample_method: PreviewDownsampleMethod,
}

pub(super) struct VideoRenderer {
    gl: glow::Context,
    program: glow::NativeProgram,
    vao: glow::NativeVertexArray,
    rgba_texture: glow::NativeTexture,
    rgba_cuda: Option<CudaTexture>,
    cuda_context: Option<cuda_core::sys::CUcontext>,
    texture_width: u32,
    texture_height: u32,
    last_frame_key: Option<CompositedFrameStorageKey>,
    mipmapped_frame_key: Option<CompositedFrameStorageKey>,
    has_frame_uniform: Option<glow::NativeUniformLocation>,
    surface_size_uniform: Option<glow::NativeUniformLocation>,
    content_rect_uniform: Option<glow::NativeUniformLocation>,
    background_color_uniform: Option<glow::NativeUniformLocation>,
    shadow_size_uniform: Option<glow::NativeUniformLocation>,
    draw_frame_uniform: Option<glow::NativeUniformLocation>,
    overlay_renderer: TimelineRenderer,
}

impl VideoRenderer {
    pub fn new() -> Result<Self, String> {
        let gl = gl_loader::context();
        unsafe {
            let program = create_program(&gl)?;
            let vao = gl.create_vertex_array()?;
            let rgba_texture = create_video_texture(&gl)?;
            let has_frame_uniform = gl.get_uniform_location(program, "u_has_frame");
            let surface_size_uniform = gl.get_uniform_location(program, "u_surface_size");
            let content_rect_uniform = gl.get_uniform_location(program, "u_content_rect");
            let background_color_uniform = gl.get_uniform_location(program, "u_background_color");
            let shadow_size_uniform = gl.get_uniform_location(program, "u_shadow_size");
            let draw_frame_uniform = gl.get_uniform_location(program, "u_draw_frame");
            let rgba_texture_uniform = gl.get_uniform_location(program, "u_rgba_texture");
            gl.use_program(Some(program));
            gl.uniform_1_i32(rgba_texture_uniform.as_ref(), 0);
            gl.use_program(None);
            Ok(Self {
                gl,
                program,
                vao,
                rgba_texture,
                rgba_cuda: None,
                cuda_context: None,
                texture_width: 0,
                texture_height: 0,
                last_frame_key: None,
                mipmapped_frame_key: None,
                has_frame_uniform,
                surface_size_uniform,
                content_rect_uniform,
                background_color_uniform,
                shadow_size_uniform,
                draw_frame_uniform,
                overlay_renderer: TimelineRenderer::new(),
            })
        }
    }

    pub fn render(
        &mut self,
        surface: IVec2,
        pixels_per_point: f32,
        frame: Option<&CompositedVideoFrame>,
        appearance: Appearance,
        draw_overlay: impl FnOnce(&TimelinePainter),
    ) -> Result<(), String> {
        let width = surface.x.max(1);
        let height = surface.y.max(1);
        unsafe {
            self.gl.viewport(0, 0, width, height);
            self.gl.clear_color(0.0, 0.0, 0.0, 0.0);
            self.gl.clear(glow::COLOR_BUFFER_BIT);
            self.gl.disable(glow::DEPTH_TEST);
            self.gl.disable(glow::CULL_FACE);
            self.gl.use_program(Some(self.program));
            self.gl.bind_vertex_array(Some(self.vao));
            self.gl.uniform_2_f32(
                self.surface_size_uniform.as_ref(),
                width as f32,
                height as f32,
            );
            self.gl.uniform_4_f32(
                self.content_rect_uniform.as_ref(),
                appearance.content_rect.left() * pixels_per_point,
                appearance.content_rect.top() * pixels_per_point,
                appearance.content_rect.width() * pixels_per_point,
                appearance.content_rect.height() * pixels_per_point,
            );
            self.gl.uniform_4_f32(
                self.background_color_uniform.as_ref(),
                appearance.background_color.r,
                appearance.background_color.g,
                appearance.background_color.b,
                appearance.background_color.a,
            );
            self.gl.uniform_1_f32(
                self.shadow_size_uniform.as_ref(),
                appearance.shadow_size_px as f32 * pixels_per_point,
            );
            self.gl.active_texture(glow::TEXTURE0);
            self.gl
                .bind_texture(glow::TEXTURE_2D, Some(self.rgba_texture));
            if let Some(frame) = frame {
                self.upload_frame(frame)?;
            }
            self.set_sampling(appearance.upsample_method, appearance.downsample_method);
            self.gl.disable(glow::BLEND);
            self.gl
                .uniform_1_i32(self.has_frame_uniform.as_ref(), i32::from(frame.is_some()));
            self.gl.uniform_1_i32(self.draw_frame_uniform.as_ref(), 0);
            self.gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            self.gl.uniform_1_i32(self.draw_frame_uniform.as_ref(), 1);
            self.gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            self.gl.bind_vertex_array(None);
            self.gl.use_program(None);
        }
        let painter = self
            .overlay_renderer
            .begin_overlay_frame(
                glam::UVec2::new(width as u32, height as u32),
                pixels_per_point,
            )?;
        draw_overlay(&painter);
        self.overlay_renderer.end_frame()
    }

    fn upload_frame(&mut self, frame: &CompositedVideoFrame) -> Result<(), String> {
        self.ensure_texture(frame)?;
        if self.last_frame_key == Some(frame.storage_key) {
            return Ok(());
        }
        self.rgba_cuda
            .as_ref()
            .ok_or_else(|| "CUDA RGBA texture is not registered".to_string())?
            .copy_from_device(
                frame.buffer.cu_deviceptr(),
                frame.buffer.memory_kind(),
                frame.width as usize * std::mem::size_of::<u32>(),
                frame.width as usize * std::mem::size_of::<u32>(),
                frame.height as usize,
            )
            .map_err(|error| {
                format!(
                    "upload preview frame {} to CUDA-GL texture: {error}",
                    frame.debug_label()
                )
            })?;
        self.last_frame_key = Some(frame.storage_key);
        Ok(())
    }

    fn set_sampling(
        &mut self,
        upsample_method: PreviewUpsampleMethod,
        downsample_method: PreviewDownsampleMethod,
    ) {
        unsafe {
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                match upsample_method {
                    PreviewUpsampleMethod::Nearest => glow::NEAREST,
                    PreviewUpsampleMethod::Bilinear => glow::LINEAR,
                } as i32,
            );
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                match downsample_method {
                    PreviewDownsampleMethod::Nearest => glow::NEAREST,
                    PreviewDownsampleMethod::Bilinear => glow::LINEAR,
                    PreviewDownsampleMethod::Trilinear => glow::LINEAR_MIPMAP_LINEAR,
                } as i32,
            );
            if matches!(downsample_method, PreviewDownsampleMethod::Trilinear)
                && self.last_frame_key.is_some()
                && self.mipmapped_frame_key != self.last_frame_key
            {
                self.gl.generate_mipmap(glow::TEXTURE_2D);
                self.mipmapped_frame_key = self.last_frame_key;
            }
        }
    }

    fn ensure_texture(&mut self, frame: &CompositedVideoFrame) -> Result<(), String> {
        let cuda_context = frame.buffer.context().cu_ctx();
        if self.texture_width == frame.width
            && self.texture_height == frame.height
            && self.cuda_context == Some(cuda_context)
        {
            return Ok(());
        }
        self.rgba_cuda.take();
        self.cuda_context = None;
        unsafe {
            self.gl
                .bind_texture(glow::TEXTURE_2D, Some(self.rgba_texture));
            // CUDA requires the OpenGL texture storage to remain stable after registration.
            let mipmap_levels = frame.width.max(frame.height).ilog2() + 1;
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAX_LEVEL,
                mipmap_levels as i32 - 1,
            );
            for level in 0..mipmap_levels {
                self.gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    level as i32,
                    glow::RGBA8 as i32,
                    (frame.width >> level).max(1) as i32,
                    (frame.height >> level).max(1) as i32,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(None),
                );
            }
        }
        self.rgba_cuda = Some(CudaTexture::register(
            self.rgba_texture.0.get(),
            glow::TEXTURE_2D,
            frame.buffer.context().clone(),
        )?);
        self.cuda_context = Some(cuda_context);
        self.texture_width = frame.width;
        self.texture_height = frame.height;
        self.last_frame_key = None;
        self.mipmapped_frame_key = None;
        Ok(())
    }

    pub fn destroy(&mut self) {
        self.overlay_renderer.destroy();
        self.rgba_cuda.take();
        self.cuda_context = None;
        unsafe {
            self.gl.delete_texture(self.rgba_texture);
            self.gl.delete_vertex_array(self.vao);
            self.gl.delete_program(self.program);
        }
    }
}

fn create_video_texture(gl: &glow::Context) -> Result<glow::NativeTexture, String> {
    unsafe {
        let texture = gl.create_texture()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_S,
            glow::CLAMP_TO_EDGE as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_T,
            glow::CLAMP_TO_EDGE as i32,
        );
        Ok(texture)
    }
}

fn create_program(gl: &glow::Context) -> Result<glow::NativeProgram, String> {
    unsafe {
        let vertex = compile_shader(gl, glow::VERTEX_SHADER, VIDEO_VERTEX_SHADER)?;
        let fragment = compile_shader(gl, glow::FRAGMENT_SHADER, VIDEO_FRAGMENT_SHADER)?;
        let program = gl.create_program()?;
        gl.attach_shader(program, vertex);
        gl.attach_shader(program, fragment);
        gl.link_program(program);
        let linked = gl.get_program_link_status(program);
        gl.detach_shader(program, vertex);
        gl.detach_shader(program, fragment);
        gl.delete_shader(vertex);
        gl.delete_shader(fragment);
        if !linked {
            let error = gl.get_program_info_log(program);
            gl.delete_program(program);
            return Err(error);
        }
        Ok(program)
    }
}

fn compile_shader(
    gl: &glow::Context,
    kind: u32,
    source: &str,
) -> Result<glow::NativeShader, String> {
    unsafe {
        let shader = gl.create_shader(kind)?;
        gl.shader_source(shader, source);
        gl.compile_shader(shader);
        if !gl.get_shader_compile_status(shader) {
            let error = gl.get_shader_info_log(shader);
            gl.delete_shader(shader);
            return Err(error);
        }
        Ok(shader)
    }
}

const VIDEO_VERTEX_SHADER: &str = include_str!("../shaders/video.vert.glsl");
const VIDEO_FRAGMENT_SHADER: &str = include_str!("../shaders/video.frag.glsl");
