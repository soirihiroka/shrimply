use hashbrown::HashSet;
use std::ffi::CStr;
use std::mem::size_of;
use std::ptr;
use std::rc::Rc;
use std::sync::Arc;

use crate::decode::DecodeControl;
use crate::layer::VideoLayer;
use ffmpeg_next::{frame as ffmpeg_frame, sys as ffmpeg_sys};
use shrimply_cuda::{CudaContext, CudaEvent, CudaStream, DeviceCopy, LaunchConfig, sys};
use shrimply_gpu_memory::GpuBuffer as DeviceBuffer;
use shrimply_project::project::{CanvasSize, Color};
use shrimply_render_core::{LayerCompositeParams, math};
pub use shrimply_visual_frame::VisualFrame;
use shrimply_visual_frame::{
    Device, GPU_FRAME_ALLOCATION_EXHAUSTED, gpu_allocation_stats, gpu_oom_generation,
};

mod device_buffer;
mod frame;
pub(crate) mod generated_gpu;
mod kernels;
pub(crate) mod layered_image;
mod layers;
pub(crate) mod modifiers;

pub use frame::{CompositedFrameStorageKey, CompositedVideoFrame};

const DISPLAY_GPU_MEMORY_RESERVE_DIVISOR: u64 = 16;
const MIGRATE_ALL_REQUIRED_BYTES: u64 = u64::MAX;
const RENDER_SUPERSEDED: &str = "video render superseded";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportPixelFormat {
    Nv12,
    P010,
}

pub struct ExportGpuTiming {
    pub compositor_ns: u64,
    pub conversion_ns: u64,
}

impl ExportPixelFormat {
    pub fn sw_format(self) -> ffmpeg_sys::AVPixelFormat {
        match self {
            Self::Nv12 => ffmpeg_sys::AVPixelFormat::AV_PIX_FMT_NV12,
            Self::P010 => ffmpeg_sys::AVPixelFormat::AV_PIX_FMT_P010LE,
        }
    }
}

pub struct CudaVideoCompositor {
    generated_renderer: Option<generated_gpu::GeneratedGpuRenderer>,
    generated_renderer_generation: u64,
    module: kernels::PreviewModule,
    export_module: Option<kernels::ExportModule>,
    stream: Arc<CudaStream>,
    context: Arc<CudaContext>,
    next_serial: u64,
    render_control: Option<DecodeControl>,
    observed_gpu_oom_generation: u64,
    export_layer_params: Option<DeviceBuffer<kernels::Nv12LayerParams>>,
    export_motion_transforms: Option<DeviceBuffer<glam::Mat3>>,
    export_output: Option<DeviceBuffer<u32>>,
    export_render_events: Option<(CudaEvent, CudaEvent)>,
    export_conversion_events: Option<(CudaEvent, CudaEvent)>,
    verified_export_planes: HashSet<(sys::CUdeviceptr, sys::CUdeviceptr)>,
    modifier_workspace: modifiers::ModifierWorkspace,
    anime4k: shrimply_anime4k::Workspace,
    solid_layer: Option<(CanvasSize, Color<u8>, Rc<VisualFrame>)>,
    optical_flow: Option<shrimply_nvidia_optical_flow::OpticalFlow>,
}

impl CudaVideoCompositor {
    pub fn new() -> Result<Self, String> {
        configure_primary_context_flags()?;
        let context = CudaContext::new(0).map_err(|error| format!("CUDA context: {error:?}"))?;
        let stream = context
            .new_stream()
            .map_err(|error| format!("CUDA stream: {error:?}"))?;
        let module = kernels::load_preview(&context)
            .map_err(|error| format!("CUDA compositor module: {error:?}"))?;
        let anime4k = shrimply_anime4k::Workspace::new(context.clone())?;

        let compositor = Self {
            context,
            stream,
            module,
            export_module: None,
            generated_renderer: None,
            generated_renderer_generation: 0,
            next_serial: 1,
            render_control: None,
            observed_gpu_oom_generation: gpu_oom_generation(),
            export_layer_params: None,
            export_motion_transforms: None,
            export_output: None,
            export_render_events: None,
            export_conversion_events: None,
            verified_export_planes: HashSet::new(),
            modifier_workspace: modifiers::ModifierWorkspace::new(),
            anime4k,
            solid_layer: None,
            optical_flow: None,
        };
        compositor.record_gpu_memory_usage();
        Ok(compositor)
    }

    pub(crate) fn set_render_control(&mut self, control: Option<DecodeControl>) {
        self.render_control = control;
    }

    fn fail_if_superseded(&self) -> Result<(), String> {
        if self
            .render_control
            .as_ref()
            .is_some_and(DecodeControl::superseded)
        {
            Err(RENDER_SUPERSEDED.to_string())
        } else {
            Ok(())
        }
    }

    pub(crate) fn estimate_optical_flow(
        &mut self,
        input: &VisualFrame,
        reference: &VisualFrame,
    ) -> Result<shrimply_nvidia_optical_flow::FlowField, String> {
        self.release_after_reported_gpu_oom()?;
        if input.format() != shrimply_visual_frame::VisualFormat::Rgba8
            || reference.format() != shrimply_visual_frame::VisualFormat::Rgba8
            || input.width() != reference.width()
            || input.height() != reference.height()
        {
            return Err("Morph optical-flow endpoints must be matching RGBA frames".to_string());
        }
        let input_plane = input
            .plane(0)
            .ok_or("Morph optical-flow input is not on the GPU")?;
        let reference_plane = reference
            .plane(0)
            .ok_or("Morph optical-flow reference is not on the GPU")?;
        let row_bytes = input.width() as usize * 4;
        if input_plane.pitch_bytes != row_bytes || reference_plane.pitch_bytes != row_bytes {
            return Err("Morph optical-flow endpoints are not tightly packed".to_string());
        }
        let settings = shrimply_nvidia_optical_flow::Settings {
            quality: shrimply_nvidia_optical_flow::Quality::Quality,
            output_grid: shrimply_nvidia_optical_flow::OutputGrid::TwoByTwo,
            temporal_hints: false,
        };
        if self
            .optical_flow
            .as_ref()
            .is_none_or(|flow| !flow.matches(input.width(), input.height(), settings))
        {
            self.optical_flow = Some(shrimply_nvidia_optical_flow::OpticalFlow::new(
                &self.context,
                &self.stream,
                input.width(),
                input.height(),
                settings,
            )?);
        }
        self.optical_flow
            .as_mut()
            .expect("Morph optical-flow session was initialized")
            .estimate(input_plane.device_ptr, reference_plane.device_ptr, true)
    }

    pub(crate) fn render_vector_morph(
        &mut self,
        frame: &crate::vector_morph::MorphFrame,
        scene: &crate::vector_morph::MorphScene,
        drawing_strategy: shrimply_project::project::SkiaDrawingStrategy,
    ) -> Result<Rc<VisualFrame>, String> {
        self.render_generated_visual(
            scene.canvas_size,
            scene.canvas_size,
            frame,
            &scene.evaluation,
            &[],
            drawing_strategy,
        )
        .map(Rc::new)
    }

    pub(crate) fn solid_layer(
        &mut self,
        canvas_size: CanvasSize,
        color: Color<u8>,
    ) -> Result<Rc<VisualFrame>, String> {
        if let Some((cached_size, cached_color, layer)) = &self.solid_layer
            && *cached_size == canvas_size
            && *cached_color == color
        {
            return Ok(layer.clone());
        }
        let width = canvas_size.width.max(1);
        let height = canvas_size.height.max(1);
        let pixel = [color.r, color.g, color.b, color.a];
        let bytes = pixel.repeat(width as usize * height as usize);
        let layer =
            Rc::new(self.upload_frame(&VisualFrame::from_rgba_bytes(width, height, bytes)?)?);
        self.solid_layer = Some((canvas_size, color, layer.clone()));
        Ok(layer)
    }

    pub(crate) fn begin_sam2_analysis(&mut self, modifier_id: uuid::Uuid) {
        self.modifier_workspace.begin_sam2_analysis(modifier_id);
    }

    pub(crate) fn end_sam2_analysis(&mut self) {
        self.modifier_workspace.end_sam2_analysis();
    }

    pub(crate) fn take_sam2_proxy(&mut self) -> Option<Vec<u8>> {
        self.modifier_workspace.take_sam2_proxy()
    }

    pub(crate) fn apply_modifiers(
        &mut self,
        frame: CompositedVideoFrame,
        modifiers: &[&dyn modifiers::GpuModifier],
    ) -> Result<CompositedVideoFrame, String> {
        self.release_after_reported_gpu_oom()?;
        let serial = self.next_serial;
        self.next_serial = self.next_serial.wrapping_add(1).max(1);
        self.modifier_workspace
            .apply(&self.context, &self.stream, frame, modifiers, serial)
    }

    pub(crate) fn apply_rgba_modifier(
        &mut self,
        layer: &VisualFrame,
        modifier: &dyn modifiers::GpuModifier,
    ) -> Result<VisualFrame, String> {
        self.release_after_reported_gpu_oom()?;
        let serial = self.next_serial;
        self.next_serial = self.next_serial.wrapping_add(1).max(1);
        let frame = self.modifier_workspace.apply_layer(
            &self.context,
            &self.stream,
            layer,
            modifier,
            serial,
        )?;
        generated_gpu::visual_frame_from_canvas(self.context.clone(), frame)
    }

    /// Materialize one visual item to the canvas before running its modifier
    /// chain. Keeping this separate from the final layer composition prevents
    /// an item's effects from touching the items below it.
    pub(crate) fn render_layer_with_modifiers(
        &mut self,
        canvas_size: CanvasSize,
        layer: &VideoLayer,
        modifiers: &[&dyn modifiers::GpuModifier],
    ) -> Result<VisualFrame, String> {
        let frame = self.render(canvas_size, std::slice::from_ref(layer))?;
        let frame = self.apply_modifiers(frame, modifiers)?;
        generated_gpu::visual_frame_from_canvas(self.context.clone(), frame)
    }

    pub(crate) fn render_layer_to_rgba(
        &mut self,
        canvas_size: CanvasSize,
        layer: &VideoLayer,
    ) -> Result<Rc<VisualFrame>, String> {
        let frame = self.render(canvas_size, std::slice::from_ref(layer))?;
        Ok(Rc::new(generated_gpu::visual_frame_from_canvas(
            self.context.clone(),
            frame,
        )?))
    }

    pub(crate) fn render_layers_to_rgba(
        &mut self,
        canvas_size: CanvasSize,
        layers: &[VideoLayer],
        background_alpha: Option<u8>,
    ) -> Result<Rc<VisualFrame>, String> {
        let frame = self.render_frame(canvas_size, layers, background_alpha)?;
        Ok(Rc::new(generated_gpu::visual_frame_from_canvas(
            self.context.clone(),
            frame,
        )?))
    }

    pub fn render(
        &mut self,
        canvas_size: CanvasSize,
        layers: &[VideoLayer],
    ) -> Result<CompositedVideoFrame, String> {
        self.render_frame(canvas_size, layers, None)
    }

    pub fn render_export(
        &mut self,
        canvas_size: CanvasSize,
        layers: &[VideoLayer],
        background_alpha: u8,
    ) -> Result<CompositedVideoFrame, String> {
        let frame = self.render_frame(canvas_size, layers, Some(background_alpha))?;

        // Export callers release their decoded source frames when this returns.
        // Keep them alive until CUDA has finished reading from those buffers.
        self.stream
            .synchronize()
            .map_err(|error| format!("synchronize CUDA export compositor: {error:?}"))?;
        Ok(frame)
    }

    fn render_frame(
        &mut self,
        canvas_size: CanvasSize,
        layers: &[VideoLayer],
        background_alpha: Option<u8>,
    ) -> Result<CompositedVideoFrame, String> {
        self.fail_if_superseded()?;
        shrimply_gpu_memory::global().begin_frame();
        self.release_after_reported_gpu_oom()?;
        let requested_bytes = u64::from(canvas_size.width.max(1))
            .checked_mul(u64::from(canvas_size.height.max(1)))
            .and_then(|pixels| pixels.checked_mul(size_of::<u32>() as u64))
            .ok_or_else(|| "CUDA compositor canvas size overflow".to_string())?;
        let result = match self.render_frame_once(canvas_size, layers, background_alpha) {
            Err(error) if is_gpu_oom(&error) => {
                self.relieve_gpu_pressure(requested_bytes, "CUDA compositor output")?;
                tracing::warn!(%error, "retrying CUDA compositor frame after GPU pressure relief");
                self.render_frame_once(canvas_size, layers, background_alpha)
                    .map_err(|retry| {
                        format!("CUDA compositor failed after GPU pressure relief: {retry}")
                    })
            }
            result => result,
        };
        let frame = result?;
        self.fail_if_superseded()?;
        self.spill_stale_frames_for_display_memory()?;
        self.record_gpu_memory_usage();
        Ok(frame)
    }

    fn render_frame_once(
        &mut self,
        canvas_size: CanvasSize,
        layers: &[VideoLayer],
        background_alpha: Option<u8>,
    ) -> Result<CompositedVideoFrame, String> {
        let export = background_alpha.is_some();
        let width = canvas_size.width.max(1);
        let height = canvas_size.height.max(1);
        let pixel_count = width as usize * height as usize;
        let launch_count = u32::try_from(pixel_count)
            .map_err(|_| "CUDA compositor canvas is too large".to_string())?;
        let mut prepared = {
            let _measurement = shrimply_benchmarking::measure("CUDA compositor / Prepare layers");
            layers::prepare(
                self.context.cu_ctx(),
                &self.stream,
                canvas_size,
                layers,
                self.render_control.as_ref(),
            )?
        };
        let spare_output = if export {
            self.export_output.take()
        } else {
            None
        };
        let mut buffer = {
            let _measurement = shrimply_benchmarking::measure("CUDA compositor / Acquire output");
            match spare_output.filter(|buffer| buffer.len() == pixel_count) {
                Some(buffer) => buffer,
                None => self.allocate_buffer(pixel_count, "CUDA output frame")?,
            }
        };
        let _decode_waits = {
            let _measurement =
                shrimply_benchmarking::measure("CUDA compositor / Wait for decoded frames");
            let mut waits = Vec::with_capacity(prepared.frame_streams.len());
            for &stream in &prepared.frame_streams {
                waits.push(self.wait_for_frame_stream(stream)?);
            }
            waits
        };
        let _anime4k_buffers = {
            let _measurement = shrimply_benchmarking::measure("CUDA compositor / Anime4K");
            layers::apply_anime4k(&mut prepared, &mut self.anime4k, &self.stream)?
        };
        let background = background_alpha.map_or(0, |alpha| {
            math::Color::new(0.0, 0.0, 0.0, f32::from(alpha) / f32::from(u8::MAX)).to_rgba_u32()
        });
        let params = {
            let _measurement =
                shrimply_benchmarking::measure("CUDA compositor / Upload layer params");
            if prepared.params.is_empty() {
                None
            } else {
                Some(device_buffer::copy(
                    &self.context,
                    &self.stream,
                    &prepared.params,
                    if export {
                        self.export_layer_params.take()
                    } else {
                        None
                    },
                )?)
            }
        };
        let motion_transforms = device_buffer::copy(
            &self.context,
            &self.stream,
            &prepared.motion_transforms,
            if export {
                self.export_motion_transforms.take()
            } else {
                None
            },
        )?;
        if export && self.export_render_events.is_none() {
            let flags = Some(sys::CUevent_flags_enum_CU_EVENT_DEFAULT);
            self.export_render_events = Some((
                self.context
                    .new_event(flags)
                    .map_err(|error| format!("create CUDA compositor start event: {error:?}"))?,
                self.context
                    .new_event(flags)
                    .map_err(|error| format!("create CUDA compositor end event: {error:?}"))?,
            ));
        }
        if export {
            self.export_render_events
                .as_ref()
                .expect("export render events were initialized")
                .0
                .record(&self.stream)
                .map_err(|error| format!("record CUDA compositor start event: {error:?}"))?;
        }
        {
            let _measurement = shrimply_benchmarking::measure("CUDA compositor / Launch");
            if let Some(params) = params.as_ref() {
                let result = unsafe {
                    self.module.composite_nv12_layers(
                        &self.stream,
                        LaunchConfig::for_num_elems(launch_count),
                        params,
                        &motion_transforms,
                        &mut buffer,
                        background,
                    )
                };
                if let Err(error) = result {
                    let output_bytes = u64::try_from(pixel_count)
                        .unwrap_or(u64::MAX)
                        .saturating_mul(size_of::<u32>() as u64);
                    if error.0 != sys::cudaError_enum_CUDA_ERROR_OUT_OF_MEMORY {
                        return Err(format!("launch CUDA compositor kernel: {error:?}"));
                    }
                    self.relieve_gpu_pressure(output_bytes, "CUDA compositor kernel output")?;
                    unsafe {
                        self.module
                            .composite_nv12_layers(
                                &self.stream,
                                LaunchConfig::for_num_elems(launch_count),
                                params,
                                &motion_transforms,
                                &mut buffer,
                                background,
                            )
                            .map_err(|retry| {
                                format!(
                                    "launch CUDA compositor kernel after GPU pressure relief: {retry:?}"
                                )
                            })?;
                    }
                }
            } else if background != 0 {
                buffer
                    .copy_from_host(&self.stream, &vec![background; pixel_count])
                    .map_err(|error| format!("fill CUDA export background: {error:?}"))?;
            } else {
                buffer
                    .zero_async(&self.stream)
                    .map_err(|error| format!("clear CUDA output frame: {error:?}"))?;
            }
        }
        if export {
            self.export_render_events
                .as_ref()
                .expect("export render events were initialized")
                .1
                .record(&self.stream)
                .map_err(|error| format!("record CUDA compositor end event: {error:?}"))?;
        }

        if export {
            self.export_layer_params = params;
            self.export_motion_transforms = Some(motion_transforms);
        }

        if !export {
            let _measurement = shrimply_benchmarking::measure("CUDA compositor / Synchronize");
            self.stream
                .synchronize()
                .map_err(|error| format!("synchronize CUDA compositor: {error:?}"))?;
        }

        let serial = self.next_serial;
        self.next_serial = self.next_serial.wrapping_add(1).max(1);
        Ok(CompositedVideoFrame::new(buffer, width, height, serial))
    }

    pub(crate) fn render_generated_visual(
        &mut self,
        render_size: CanvasSize,
        canvas_size: CanvasSize,
        visual: &dyn generated_gpu::GeneratedVisual,
        evaluation: &shrimply_evaluation::VisualEvaluation,
        operations: &[crate::layer::VectorOperation],
        drawing_strategy: shrimply_project::project::SkiaDrawingStrategy,
    ) -> Result<VisualFrame, String> {
        let context = self.context.clone();
        let stream = self.stream.clone();
        render_with_generated_gpu(
            &mut self.generated_renderer,
            &mut self.generated_renderer_generation,
            &context,
            &stream,
            self.render_control.as_ref(),
            "generated visual render",
            |renderer| {
                renderer.render_visual(
                    context.clone(),
                    (render_size, canvas_size),
                    visual,
                    evaluation,
                    operations,
                    drawing_strategy,
                )
            },
        )
    }

    pub(crate) fn render_scene_3d(
        &mut self,
        session: &mut shrimply_render_3d::ObjRenderSession,
        width: u32,
        height: u32,
        params: &shrimply_render_3d::SceneRenderParams,
        transmission_background: Option<&VisualFrame>,
    ) -> Result<VisualFrame, String> {
        let module = &self.module;
        let stream = &self.stream;
        let context = self.context.clone();
        render_with_generated_gpu(
            &mut self.generated_renderer,
            &mut self.generated_renderer_generation,
            &context,
            stream,
            self.render_control.as_ref(),
            "3D scene render",
            |renderer| {
                renderer.render_scene_3d(
                    context.clone(),
                    stream,
                    module,
                    session,
                    width,
                    height,
                    params,
                    transmission_background,
                )
            },
        )
    }

    pub(crate) fn render_gaussian_3d(
        &mut self,
        session: &shrimply_3dgs::RenderSession,
        width: u32,
        height: u32,
        params: &shrimply_3dgs::RenderParams,
    ) -> Result<VisualFrame, String> {
        let context = self.context.clone();
        let stream = self.stream.clone();
        render_with_generated_gpu(
            &mut self.generated_renderer,
            &mut self.generated_renderer_generation,
            &context,
            &stream,
            self.render_control.as_ref(),
            "3DGS render",
            |renderer| renderer.render_gaussian_3d(context.clone(), session, width, height, params),
        )
    }

    pub(crate) fn render_background(
        &mut self,
        width: u32,
        height: u32,
        time: shrimply_project::project::Time,
        background: &shrimply_background::Background,
    ) -> Result<VisualFrame, String> {
        let context = self.context.clone();
        let stream = self.stream.clone();
        render_with_generated_gpu(
            &mut self.generated_renderer,
            &mut self.generated_renderer_generation,
            &context,
            &stream,
            self.render_control.as_ref(),
            "background Slang/Vulkan render",
            |renderer| {
                renderer.render_background(
                    context.clone(),
                    stream.clone(),
                    width,
                    height,
                    time,
                    background,
                )
            },
        )
    }

    pub(crate) fn render_manim(
        &mut self,
        slot: &Arc<()>,
        animation: &shrimply_manim_wgpu::PreparedAnimation,
        frame_index: usize,
        destination: &VisualFrame,
    ) -> Result<(), String> {
        let context = self.context.clone();
        let stream = self.stream.clone();
        render_with_generated_gpu(
            &mut self.generated_renderer,
            &mut self.generated_renderer_generation,
            &context,
            &stream,
            self.render_control.as_ref(),
            "Manim WGPU render",
            |renderer| {
                renderer.render_manim(
                    context.clone(),
                    stream.clone(),
                    slot,
                    animation,
                    frame_index,
                    destination,
                )
            },
        )
    }

    pub(crate) fn render_mesh_flow(
        &mut self,
        source: &VisualFrame,
        grid_width: u32,
        grid_height: u32,
        source_offsets: &[glam::Vec2],
    ) -> Result<VisualFrame, String> {
        let context = self.context.clone();
        let stream = self.stream.clone();
        render_with_generated_gpu(
            &mut self.generated_renderer,
            &mut self.generated_renderer_generation,
            &context,
            &stream,
            self.render_control.as_ref(),
            "MeshFlow Slang/Vulkan warp",
            |renderer| {
                renderer.render_mesh_flow(
                    context.clone(),
                    &stream,
                    source,
                    grid_width,
                    grid_height,
                    source_offsets,
                )
            },
        )
    }

    pub(crate) fn generated_renderer_generation(&self) -> u64 {
        self.generated_renderer_generation
    }

    fn allocate_buffer<T: DeviceCopy>(
        &mut self,
        length: usize,
        description: &str,
    ) -> Result<DeviceBuffer<T>, String> {
        match shrimply_gpu_memory::global().allocate_buffer(
            &self.stream,
            length,
            shrimply_gpu_memory::AllocationClass::Transient,
            description,
        ) {
            Ok(buffer) => Ok(buffer),
            Err(error) => {
                let bytes = length
                    .checked_mul(size_of::<T>())
                    .and_then(|bytes| u64::try_from(bytes).ok())
                    .ok_or_else(|| format!("{description} size overflow"))?;
                self.relieve_gpu_pressure(bytes, description)?;
                shrimply_gpu_memory::global()
                    .allocate_buffer(
                        &self.stream,
                        length,
                        shrimply_gpu_memory::AllocationClass::Transient,
                        description,
                    )
                    .map_err(|retry| {
                    format!(
                        "allocate {description} after GPU pressure relief: {retry}; initial error: {error}"
                    )
                })
            }
        }
    }

    fn relieve_gpu_pressure(
        &mut self,
        requested_bytes: u64,
        allocation_description: &str,
    ) -> Result<(), String> {
        self.fail_if_superseded()?;
        self.context
            .bind_to_thread()
            .map_err(|error| format!("bind CUDA context for GPU pressure relief: {error:?}"))?;
        if let Err(error) = self.stream.synchronize() {
            if error.0 != sys::cudaError_enum_CUDA_ERROR_OUT_OF_MEMORY {
                return Err(format!("synchronize before GPU pressure relief: {error:?}"));
            }
            tracing::warn!(
                ?error,
                "CUDA stream reported OOM while preparing pressure relief"
            );
        }
        self.fail_if_superseded()?;
        let before = self.gpu_memory_info().ok();
        let migrated_bytes = shrimply_gpu_memory::global().relieve_vram_pressure(
            &self.context,
            &self.stream,
            requested_bytes,
            false,
            self.render_control
                .as_ref()
                .map(DecodeControl::generation_check),
        )?;
        self.fail_if_superseded()?;
        let (free, total) = self.gpu_memory_info()?;
        let reserve = total / DISPLAY_GPU_MEMORY_RESERVE_DIVISOR;
        if free >= reserve.saturating_add(requested_bytes) {
            let telemetry = shrimply_gpu_memory::global().telemetry();
            tracing::warn!(
                allocation_description,
                requested_bytes,
                free_vram_bytes = free,
                total_vram_bytes = total,
                reserve_bytes = reserve,
                managed_bytes = telemetry.managed_bytes,
                host_reserved_bytes = telemetry.host_reserved_bytes,
                host_budget_bytes = telemetry.host_budget_bytes,
                migrated_bytes,
                relief_level = "managed migration",
                "restored CUDA reserve by migrating managed allocations"
            );
            return Ok(());
        }
        let mut released = self.export_layer_params.take().is_some();
        released |= self.export_motion_transforms.take().is_some();
        released |= self.export_output.take().is_some();
        released |= self.solid_layer.take().is_some();
        released |= self.modifier_workspace.clear_cached_gpu_resources();
        if let Some(renderer) = self.generated_renderer.as_mut() {
            released |= renderer.release_render_surfaces(requested_bytes)?;
        }
        let transient_sufficient = self
            .gpu_memory_info()
            .is_ok_and(|(free, _)| free >= reserve.saturating_add(requested_bytes));
        let mut relief_level = "transient surfaces";
        let mut last_resort_before = None;
        if !transient_sufficient {
            relief_level = "last-resort external resources";
            last_resort_before = self.gpu_memory_info().ok();
            released |= self.export_render_events.take().is_some();
            released |= self.export_conversion_events.take().is_some();
            released |= self.export_module.take().is_some();
            released |= self.optical_flow.take().is_some();
            released |= self.anime4k.clear_cached_models();
            if let Some(renderer) = self.generated_renderer.as_mut() {
                if renderer.release_gpu_animation_resources() {
                    released = true;
                }
                released |= renderer.release_external_gpu_resources();
            }
        }
        let after = self.gpu_memory_info().ok();
        let recovered_bytes = before
            .zip(after)
            .map_or(0, |((before, _), (after, _))| after.saturating_sub(before));
        shrimply_benchmarking::increment("GPU pressure / Events");
        shrimply_benchmarking::add_to_counter("GPU pressure / Bytes recovered", recovered_bytes);
        if relief_level == "last-resort external resources" {
            let last_resort_recovered = last_resort_before
                .zip(after)
                .map_or(0, |((before, _), (after, _))| after.saturating_sub(before));
            shrimply_gpu_memory::global().note_last_resort_cleanup(last_resort_recovered);
        }
        self.record_gpu_memory_usage();
        let telemetry = shrimply_gpu_memory::global().telemetry();
        tracing::warn!(
            allocation_description,
            requested_bytes,
            free_vram_bytes = after.map_or(0, |(free, _)| free),
            total_vram_bytes = after.map_or(total, |(_, total)| total),
            reserve_bytes = reserve,
            managed_bytes = telemetry.managed_bytes,
            host_reserved_bytes = telemetry.host_reserved_bytes,
            host_budget_bytes = telemetry.host_budget_bytes,
            migrated_bytes,
            recovered_bytes,
            released,
            relief_level,
            "released cached GPU resources after allocation pressure"
        );
        Ok(())
    }

    pub(crate) fn relieve_all_gpu_pressure(
        &mut self,
        allocation_description: &str,
    ) -> Result<(), String> {
        self.relieve_gpu_pressure(MIGRATE_ALL_REQUIRED_BYTES, allocation_description)?;
        self.context
            .synchronize()
            .map_err(|error| format!("finish blocking CUDA GPU garbage collection: {error:?}"))
    }

    pub(crate) fn relieve_decoder_gpu_pressure(
        &mut self,
        startup_bytes: u64,
    ) -> Result<(), String> {
        self.relieve_gpu_pressure(startup_bytes, "speculative video decoder startup")
    }

    fn release_after_reported_gpu_oom(&mut self) -> Result<(), String> {
        self.fail_if_superseded()?;
        let generation = gpu_oom_generation();
        if generation == self.observed_gpu_oom_generation {
            return Ok(());
        }
        self.observed_gpu_oom_generation = generation;
        self.relieve_gpu_pressure(0, "previously reported CUDA OOM")?;
        Ok(())
    }

    fn spill_stale_frames_for_display_memory(&mut self) -> Result<(), String> {
        self.fail_if_superseded()?;
        let (free, total) = self.gpu_memory_info()?;
        if free < total / DISPLAY_GPU_MEMORY_RESERVE_DIVISOR {
            shrimply_gpu_memory::global().relieve_vram_pressure(
                &self.context,
                &self.stream,
                0,
                true,
                self.render_control
                    .as_ref()
                    .map(DecodeControl::generation_check),
            )?;
        }
        Ok(())
    }

    fn gpu_memory_info(&self) -> Result<(u64, u64), String> {
        gpu_memory_info(&self.context)
    }

    fn record_gpu_memory_usage(&self) {
        if let Ok((free, total)) = self.gpu_memory_info() {
            shrimply_benchmarking::set_counter("CUDA memory / Free bytes", free);
            shrimply_benchmarking::set_counter(
                "CUDA memory / Used bytes",
                total.saturating_sub(free),
            );
            shrimply_benchmarking::set_counter("CUDA memory / Total bytes", total);
        }
        let (frames, bytes) = gpu_allocation_stats();
        shrimply_benchmarking::set_counter("Visual frame / GPU frames retained", frames);
        shrimply_benchmarking::set_counter("Visual frame / GPU bytes retained", bytes);
    }

    pub(crate) fn upload_frame(&mut self, frame: &VisualFrame) -> Result<VisualFrame, String> {
        self.release_after_reported_gpu_oom()?;
        let error = match frame.copy_to(Device::Cuda(0)) {
            Ok(frame) => return Ok(frame),
            Err(error) if is_gpu_oom(&error) => error,
            Err(error) => return Err(error),
        };
        self.observed_gpu_oom_generation = gpu_oom_generation();
        self.relieve_gpu_pressure(frame.bytes(), "persistent visual upload")?;
        frame.copy_to(Device::Cuda(0)).map_err(|retry| {
            format!(
                "upload visual frame after GPU pressure relief: {retry}; initial error: {error}"
            )
        })
    }

    pub fn upload_rgba_layer(
        &mut self,
        width: u32,
        height: u32,
        pixels: &[u8],
    ) -> Result<VisualFrame, String> {
        self.upload_frame(&VisualFrame::from_rgba_bytes(
            width,
            height,
            pixels.to_vec(),
        )?)
    }

    pub fn allocate_rgba_layer(&mut self, width: u32, height: u32) -> Result<VisualFrame, String> {
        self.release_after_reported_gpu_oom()?;
        let context = self.context.clone();
        let allocate = || {
            VisualFrame::allocate(
                context.clone(),
                shrimply_visual_frame::VisualFormat::Rgba8,
                width,
                height,
            )
        };
        let error = match allocate() {
            Ok(frame) => return Ok(frame),
            Err(error) if error.starts_with(GPU_FRAME_ALLOCATION_EXHAUSTED) => error,
            Err(error) => return Err(error),
        };
        self.observed_gpu_oom_generation = gpu_oom_generation();
        let requested_bytes = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|pixels| pixels.checked_mul(size_of::<u32>() as u64))
            .ok_or_else(|| "RGBA layer size overflow".to_string())?;
        self.relieve_gpu_pressure(requested_bytes, "active RGBA render output")?;
        allocate().map_err(|retry| {
            format!(
                "allocate RGBA layer after GPU pressure relief: {retry}; initial error: {error}"
            )
        })
    }

    pub(crate) fn allocate_cached_rgba_layer(
        &mut self,
        width: u32,
        height: u32,
        description: &str,
    ) -> Result<VisualFrame, String> {
        self.release_after_reported_gpu_oom()?;
        let context = self.context.clone();
        let allocate = || {
            VisualFrame::allocate_cached(
                context.clone(),
                shrimply_visual_frame::VisualFormat::Rgba8,
                width,
                height,
                description,
            )
        };
        let error = match allocate() {
            Ok(frame) => return Ok(frame),
            Err(error) if is_gpu_oom(&error) => error,
            Err(error) => return Err(error),
        };
        self.observed_gpu_oom_generation = gpu_oom_generation();
        let requested_bytes = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|pixels| pixels.checked_mul(size_of::<u32>() as u64))
            .ok_or_else(|| "cached RGBA layer size overflow".to_string())?;
        self.relieve_gpu_pressure(requested_bytes, description)?;
        allocate().map_err(|retry| {
            format!(
                "allocate {description} after GPU pressure relief: {retry}; initial error: {error}"
            )
        })
    }

    pub(crate) fn retain_host_backed_frame(
        &mut self,
        frame: &VisualFrame,
        description: &str,
    ) -> Result<Option<VisualFrame>, String> {
        self.fail_if_superseded()?;
        if shrimply_gpu_memory::global().telemetry().host_budget_bytes == 0 {
            return Ok(None);
        }
        if frame.is_cached() {
            return Ok(Some(frame.clone()));
        }
        self.release_after_reported_gpu_oom()?;
        let context = self.context.clone();
        let stream = self.stream.clone();
        let copy = || frame.copy_to_cached(context.clone(), stream.as_ref(), description);
        let error = match copy() {
            Ok(frame) => return Ok(frame.is_managed().then_some(frame)),
            Err(error) if is_gpu_oom(&error) => error,
            Err(error) => return Err(error),
        };
        self.observed_gpu_oom_generation = gpu_oom_generation();
        self.relieve_gpu_pressure(frame.bytes(), description)?;
        copy().map(|frame| frame.is_managed().then_some(frame))
            .map_err(|retry| {
                format!(
                    "retain {description} after GPU pressure relief: {retry}; initial error: {error}"
                )
            })
    }

    pub(crate) fn prepare_host_backed_frame(
        &mut self,
        frame: &VisualFrame,
        description: &str,
    ) -> Result<(), String> {
        self.fail_if_superseded()?;
        if !frame.is_managed() {
            return Ok(());
        }
        if let Err(error) = frame.prefetch_to_device(&self.stream) {
            self.relieve_gpu_pressure(frame.bytes(), description)?;
            frame.prefetch_to_device(&self.stream).map_err(|retry| {
                format!(
                    "prefetch {description} after GPU pressure relief: {retry}; initial error: {error}"
                )
            })?;
        }
        self.fail_if_superseded()?;
        Ok(())
    }

    pub fn upload_rgba_layer_into(
        &self,
        destination: &VisualFrame,
        pixels: &[u8],
    ) -> Result<(), String> {
        let destination_memory = destination.memory_kind(0);
        let destination = destination
            .plane(0)
            .ok_or_else(|| "Manim preview image has no RGBA plane".to_string())?;
        let expected = destination
            .width_bytes
            .checked_mul(destination.height)
            .ok_or_else(|| "Manim preview image size overflow".to_string())?;
        if pixels.len() != expected {
            return Err(format!(
                "Manim preview image has {} bytes, expected {expected}",
                pixels.len()
            ));
        }
        bind_context(&self.context, "bind CUDA context for Manim preview upload")?;
        let copy = sys::CUDA_MEMCPY2D {
            srcXInBytes: 0,
            srcY: 0,
            srcMemoryType: sys::CUmemorytype_enum_CU_MEMORYTYPE_HOST,
            srcHost: pixels.as_ptr().cast(),
            srcDevice: 0,
            srcArray: ptr::null_mut(),
            srcPitch: destination.width_bytes,
            dstXInBytes: 0,
            dstY: 0,
            dstMemoryType: match destination_memory {
                Some(shrimply_gpu_memory::MemoryKind::Managed) => {
                    sys::CUmemorytype_enum_CU_MEMORYTYPE_UNIFIED
                }
                _ => sys::CUmemorytype_enum_CU_MEMORYTYPE_DEVICE,
            },
            dstHost: ptr::null_mut(),
            dstDevice: destination.device_ptr,
            dstArray: ptr::null_mut(),
            dstPitch: destination.pitch_bytes,
            WidthInBytes: destination.width_bytes,
            Height: destination.height,
        };
        cuda_check(
            unsafe { sys::cuMemcpy2DAsync_v2(&copy, self.stream.cu_stream()) },
            "upload Manim preview image",
        )
    }

    pub(crate) fn upload_blender_frame(
        &self,
        destination: &VisualFrame,
        pixels: &[u8],
    ) -> Result<(), String> {
        self.upload_rgba_layer_into(destination, pixels)?;
        self.stream
            .synchronize()
            .map_err(|error| format!("finish Blender frame upload: {error:?}"))
    }

    pub(crate) fn composite_layered_image_layers(
        &mut self,
        width: u32,
        height: u32,
        layers: &[layered_image::LayeredImageGpuLayer<'_>],
    ) -> Result<VisualFrame, String> {
        self.fail_if_superseded()?;
        let width = width.max(1);
        let height = height.max(1);
        let pixel_count = width as usize * height as usize;
        let launch_count = u32::try_from(pixel_count)
            .map_err(|_| "CUDA layered image composite is too large".to_string())?;
        for layer in layers {
            if layer.source.width() != width || layer.source.height() != height {
                return Err(
                    "CUDA layered image source layer dimensions do not match the document"
                        .to_string(),
                );
            }
            if let Some((base, _)) = layer.clipping_base
                && (base.width() != width || base.height() != height)
            {
                return Err(
                    "CUDA layered image clipping-base dimensions do not match the document"
                        .to_string(),
                );
            }
        }
        let mut output = self.allocate_buffer(pixel_count, "CUDA layered image composite")?;
        for layer in layers {
            self.fail_if_superseded()?;
            let source = layer.source.plane(0).expect("RGBA layer has no plane");
            let clipping_base = layer
                .clipping_base
                .map(|(base, _)| base.plane(0).expect("RGBA clipping base has no plane"));
            let clipping_base_opacity = layer.clipping_base.map_or(1.0, |(_, opacity)| opacity);
            unsafe {
                self.module
                    .composite_layered_image_layer(
                        &self.stream,
                        LaunchConfig::for_num_elems(launch_count),
                        LayerCompositeParams {
                            source: source.device_ptr as usize as *const u32,
                            clipping_base: clipping_base
                                .map_or(ptr::null(), |base| base.device_ptr as usize as *const u32),
                            source_pitch: source.pitch_bytes,
                            clipping_base_pitch: clipping_base.map_or(0, |base| base.pitch_bytes),
                            width,
                            mode: layer.mode,
                            opacity: layer.opacity,
                            clipping_base_opacity,
                            noise_seed: layer.noise_seed,
                            _padding_0: [0; 7],
                        },
                        &mut output,
                    )
                    .map_err(|error| format!("launch CUDA layered image compositor: {error:?}"))?;
            }
        }
        if let Err(error) = self.stream.synchronize() {
            tracing::error!(
                ?error,
                width,
                height,
                layer_count = layers.len(),
                "CUDA layered image compositor failed",
            );
            for (index, layer) in layers.iter().enumerate() {
                let source = layer.source.plane(0).expect("RGBA layer has no plane");
                let clipping_base = layer
                    .clipping_base
                    .map(|(base, _)| base.plane(0).expect("RGBA clipping base has no plane"));
                tracing::error!(
                    index,
                    source_ptr = source.device_ptr,
                    source_pitch = source.pitch_bytes,
                    clipping_base_ptr = clipping_base.map_or(0, |base| base.device_ptr),
                    clipping_base_pitch = clipping_base.map_or(0, |base| base.pitch_bytes),
                    ?layer.mode,
                    opacity = layer.opacity,
                    clipping_base_opacity = layer
                        .clipping_base
                        .map_or(1.0, |(_, opacity)| opacity),
                    "CUDA layered image input at failure",
                );
            }
            return Err(format!(
                "synchronize CUDA layered image compositor: {error:?}"
            ));
        }
        let serial = self.next_serial;
        self.next_serial = self.next_serial.wrapping_add(1).max(1);
        generated_gpu::visual_frame_from_canvas(
            self.context.clone(),
            CompositedVideoFrame::new(output, width, height, serial),
        )
    }

    pub fn copy_to_ffmpeg_hw_frame(
        &mut self,
        source: &CompositedVideoFrame,
        destination: &mut ffmpeg_frame::Video,
        pixel_format: ExportPixelFormat,
    ) -> Result<ExportGpuTiming, String> {
        let raw = unsafe { destination.as_mut_ptr() };
        if raw.is_null() {
            return Err("FFmpeg hardware frame is null".to_string());
        }
        let width = unsafe { (*raw).width }.max(1) as u32;
        let height = unsafe { (*raw).height }.max(1) as u32;
        if source.width != width || source.height != height {
            return Err(format!(
                "Composited frame size {}x{} does not match FFmpeg frame size {}x{}",
                source.width, source.height, width, height
            ));
        }

        let source_ptr = source.buffer.cu_deviceptr() as usize as *const u32;
        let y = unsafe { (*raw).data[0] };
        let uv = unsafe { (*raw).data[1] };
        if y.is_null() || uv.is_null() {
            return Err("FFmpeg CUDA frame is missing NV12/P010 planes".to_string());
        }
        let y_pitch = unsafe { (*raw).linesize[0] }.max(0) as usize;
        let uv_pitch = unsafe { (*raw).linesize[1] }.max(0) as usize;
        let luma_count = width as usize * height as usize;
        let chroma_width = width.div_ceil(2).max(1);
        let chroma_height = height.div_ceil(2).max(1);
        let chroma_count = chroma_width as usize * chroma_height as usize;
        let luma_launch =
            u32::try_from(luma_count).map_err(|_| "CUDA export frame is too large".to_string())?;
        let chroma_launch = u32::try_from(chroma_count)
            .map_err(|_| "CUDA export chroma frame is too large".to_string())?;

        bind_context(
            &self.context,
            "bind CUDA context for export frame conversion",
        )?;
        let planes = (
            y as usize as sys::CUdeviceptr,
            uv as usize as sys::CUdeviceptr,
        );
        if !self.verified_export_planes.contains(&planes) {
            require_device_pointer(planes.0, "FFmpeg export luma plane")?;
            require_device_pointer(planes.1, "FFmpeg export chroma plane")?;
            self.verified_export_planes.insert(planes);
        }
        if self.export_module.is_none() {
            let started = std::time::Instant::now();
            self.export_module = Some(
                kernels::load_export(&self.context)
                    .map_err(|error| format!("load sm_86 CUDA export cubin: {error:?}"))?,
            );
            tracing::debug!(
                elapsed_us = started.elapsed().as_micros(),
                "CUDA export cubin loaded"
            );
        }
        let module = self
            .export_module
            .as_ref()
            .expect("CUDA export cubin loaded");
        if self.export_conversion_events.is_none() {
            let flags = Some(sys::CUevent_flags_enum_CU_EVENT_DEFAULT);
            self.export_conversion_events = Some((
                self.context
                    .new_event(flags)
                    .map_err(|error| format!("create CUDA conversion start event: {error:?}"))?,
                self.context
                    .new_event(flags)
                    .map_err(|error| format!("create CUDA conversion end event: {error:?}"))?,
            ));
        }
        let conversion_events = self
            .export_conversion_events
            .as_ref()
            .expect("export conversion events were initialized");
        conversion_events
            .0
            .record(&self.stream)
            .map_err(|error| format!("record CUDA conversion start event: {error:?}"))?;
        unsafe {
            match pixel_format {
                ExportPixelFormat::Nv12 => {
                    module
                        .rgba_to_nv12_luma(
                            &self.stream,
                            LaunchConfig::for_num_elems(luma_launch),
                            (source_ptr, width, height, y, y_pitch),
                        )
                        .map_err(|error| {
                            format!("launch CUDA RGBA-to-NV12 luma kernel: {error:?}")
                        })?;
                    module
                        .rgba_to_nv12_chroma(
                            &self.stream,
                            LaunchConfig::for_num_elems(chroma_launch),
                            (source_ptr, width, height, uv, uv_pitch),
                        )
                        .map_err(|error| {
                            format!("launch CUDA RGBA-to-NV12 chroma kernel: {error:?}")
                        })?;
                }
                ExportPixelFormat::P010 => {
                    module
                        .rgba_to_p010_luma(
                            &self.stream,
                            LaunchConfig::for_num_elems(luma_launch),
                            (source_ptr, width, height, y.cast(), y_pitch),
                        )
                        .map_err(|error| {
                            format!("launch CUDA RGBA-to-P010 luma kernel: {error:?}")
                        })?;
                    module
                        .rgba_to_p010_chroma(
                            &self.stream,
                            LaunchConfig::for_num_elems(chroma_launch),
                            (source_ptr, width, height, uv.cast(), uv_pitch),
                        )
                        .map_err(|error| {
                            format!("launch CUDA RGBA-to-P010 chroma kernel: {error:?}")
                        })?;
                }
            }
        }
        conversion_events
            .1
            .record(&self.stream)
            .map_err(|error| format!("record CUDA conversion end event: {error:?}"))?;
        conversion_events
            .1
            .synchronize()
            .map_err(|error| format!("synchronize CUDA export conversion: {error:?}"))?;
        let render_events = self
            .export_render_events
            .as_ref()
            .ok_or_else(|| "CUDA compositor timing events were not recorded".to_string())?;
        Ok(ExportGpuTiming {
            compositor_ns: crate::math::milliseconds_f32_to_nanoseconds(
                render_events
                    .0
                    .elapsed_ms(&render_events.1)
                    .map_err(|error| format!("time CUDA compositor kernel: {error:?}"))?,
            ),
            conversion_ns: crate::math::milliseconds_f32_to_nanoseconds(
                conversion_events
                    .0
                    .elapsed_ms(&conversion_events.1)
                    .map_err(|error| format!("time CUDA conversion kernels: {error:?}"))?,
            ),
        })
    }

    pub fn copy_to_ffmpeg_hw_frame_and_recycle(
        &mut self,
        source: CompositedVideoFrame,
        destination: &mut ffmpeg_frame::Video,
        pixel_format: ExportPixelFormat,
    ) -> Result<ExportGpuTiming, String> {
        let timing = self.copy_to_ffmpeg_hw_frame(&source, destination, pixel_format)?;
        self.export_output = Some(source.buffer);
        Ok(timing)
    }

    pub fn copy_to_ffmpeg_rgba_frame_and_recycle(
        &mut self,
        source: CompositedVideoFrame,
        destination: &mut ffmpeg_frame::Video,
    ) -> Result<ExportGpuTiming, String> {
        if destination.format() != ffmpeg_next::format::Pixel::RGBA
            || destination.width() != source.width
            || destination.height() != source.height
        {
            return Err("GIF RGBA frame does not match the composited frame".to_string());
        }
        bind_context(&self.context, "bind CUDA context for GIF frame readback")?;
        let started = std::time::Instant::now();
        let pixels = source
            .buffer
            .to_host_vec(&self.stream)
            .map_err(|error| format!("copy GIF frame from CUDA: {error:?}"))?;
        let width = source.width as usize;
        let stride = destination.stride(0);
        for (source_row, destination_row) in pixels
            .chunks_exact(width)
            .zip(destination.data_mut(0).chunks_exact_mut(stride))
        {
            for (pixel, destination) in source_row
                .iter()
                .zip(destination_row.chunks_exact_mut(std::mem::size_of::<u32>()))
            {
                destination.copy_from_slice(&pixel.to_le_bytes());
            }
        }
        let conversion_ns = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
        let render_events = self
            .export_render_events
            .as_ref()
            .ok_or_else(|| "CUDA compositor timing events were not recorded".to_string())?;
        let compositor_ns = crate::math::milliseconds_f32_to_nanoseconds(
            render_events
                .0
                .elapsed_ms(&render_events.1)
                .map_err(|error| format!("time CUDA compositor kernel: {error:?}"))?,
        );
        self.export_output = Some(source.buffer);
        Ok(ExportGpuTiming {
            compositor_ns,
            conversion_ns,
        })
    }

    fn wait_for_frame_stream(
        &self,
        frame_stream: sys::CUstream,
    ) -> Result<StreamWaitEvent, String> {
        bind_context(&self.context, "bind CUDA context for frame stream wait")?;
        let mut event = ptr::null_mut();
        cuda_check(
            unsafe {
                sys::cuEventCreate(&mut event, sys::CUevent_flags_enum_CU_EVENT_DISABLE_TIMING)
            },
            "cuEventCreate",
        )?;
        let wait = StreamWaitEvent {
            event,
            context: self.context.clone(),
        };

        if let Err(error) = cuda_check(
            unsafe { sys::cuEventRecord(wait.event, frame_stream) },
            "cuEventRecord",
        )
        .and_then(|()| {
            cuda_check(
                unsafe {
                    sys::cuStreamWaitEvent(
                        self.stream.cu_stream(),
                        wait.event,
                        sys::CUevent_wait_flags_enum_CU_EVENT_WAIT_DEFAULT,
                    )
                },
                "cuStreamWaitEvent",
            )
        }) {
            drop(wait);
            return Err(error);
        }

        Ok(wait)
    }
}

fn render_with_generated_gpu<T>(
    renderer: &mut Option<generated_gpu::GeneratedGpuRenderer>,
    generation: &mut u64,
    context: &Arc<CudaContext>,
    stream: &CudaStream,
    render_control: Option<&DecodeControl>,
    operation: &str,
    mut render: impl FnMut(&mut generated_gpu::GeneratedGpuRenderer) -> Result<T, String>,
) -> Result<T, String> {
    if render_control.is_some_and(DecodeControl::superseded) {
        return Err(RENDER_SUPERSEDED.to_string());
    }
    if renderer.is_none() {
        *renderer = Some(generated_gpu::GeneratedGpuRenderer::new()?);
    }
    let mut relief_level = 0_u8;
    let error = loop {
        match render(
            renderer
                .as_mut()
                .expect("generated GPU renderer was initialized"),
        ) {
            Ok(output) => return Ok(output),
            Err(error) if is_gpu_oom(&error) && relief_level == 0 => {
                if render_control.is_some_and(DecodeControl::superseded) {
                    return Err(RENDER_SUPERSEDED.to_string());
                }
                let before = gpu_memory_info(context).ok();
                let migrated_bytes = shrimply_gpu_memory::global().relieve_vram_pressure(
                    context,
                    stream,
                    MIGRATE_ALL_REQUIRED_BYTES,
                    false,
                    render_control.map(DecodeControl::generation_check),
                )?;
                if render_control.is_some_and(DecodeControl::superseded) {
                    return Err(RENDER_SUPERSEDED.to_string());
                }
                let released = renderer
                    .as_mut()
                    .expect("generated GPU renderer was initialized")
                    .release_render_surfaces(0)?;
                let after = gpu_memory_info(context).ok();
                let recovered_bytes = before
                    .zip(after)
                    .map_or(0, |((before, _), (after, _))| after.saturating_sub(before));
                let telemetry = shrimply_gpu_memory::global().telemetry();
                let (free, total) = after.unwrap_or_default();
                relief_level = 1;
                tracing::warn!(
                    allocation_description = operation,
                    %error,
                    requested_bytes = 0_u64,
                    allocation_size_known = false,
                    free_vram_bytes = free,
                    total_vram_bytes = total,
                    reserve_bytes = total / DISPLAY_GPU_MEMORY_RESERVE_DIVISOR,
                    managed_bytes = telemetry.managed_bytes,
                    host_reserved_bytes = telemetry.host_reserved_bytes,
                    host_budget_bytes = telemetry.host_budget_bytes,
                    migrated_bytes,
                    recovered_bytes,
                    released,
                    relief_level = "managed migration and render surfaces",
                    "retrying generated GPU operation after pressure relief"
                );
            }
            Err(error) if is_gpu_oom(&error) && relief_level == 1 => {
                let before = gpu_memory_info(context).ok();
                let renderer = renderer
                    .as_mut()
                    .expect("generated GPU renderer was initialized");
                let released_animation = renderer.release_gpu_animation_resources();
                let released_external = renderer.release_external_gpu_resources();
                let after = gpu_memory_info(context).ok();
                let recovered_bytes = before
                    .zip(after)
                    .map_or(0, |((before, _), (after, _))| after.saturating_sub(before));
                shrimply_gpu_memory::global().note_last_resort_cleanup(recovered_bytes);
                let telemetry = shrimply_gpu_memory::global().telemetry();
                let (free, total) = after.unwrap_or_default();
                relief_level = 2;
                tracing::warn!(
                    allocation_description = operation,
                    %error,
                    requested_bytes = 0_u64,
                    allocation_size_known = false,
                    free_vram_bytes = free,
                    total_vram_bytes = total,
                    reserve_bytes = total / DISPLAY_GPU_MEMORY_RESERVE_DIVISOR,
                    managed_bytes = telemetry.managed_bytes,
                    host_reserved_bytes = telemetry.host_reserved_bytes,
                    host_budget_bytes = telemetry.host_budget_bytes,
                    migrated_bytes = 0_u64,
                    recovered_bytes,
                    released_animation,
                    released_external,
                    relief_level = "GPU animation and external resources",
                    "retrying generated GPU operation after last-resort pressure relief"
                );
            }
            Err(error) if !error.contains("ERROR_DEVICE_LOST") => return Err(error),
            Err(error) => break error,
        }
    };
    tracing::warn!(operation, %error, "Vulkan device lost; reinitializing generated GPU renderer");
    renderer.take();
    *generation = generation.wrapping_add(1);
    *renderer = Some(
        generated_gpu::GeneratedGpuRenderer::new().map_err(|reinitialize| {
            format!("{operation} lost the Vulkan device ({error}); reinitialize: {reinitialize}")
        })?,
    );
    render(
        renderer
            .as_mut()
            .expect("generated GPU renderer was reinitialized"),
    )
    .map_err(|retry| format!("{operation} failed after Vulkan reinitialization: {retry}"))
}

fn gpu_memory_info(context: &Arc<CudaContext>) -> Result<(u64, u64), String> {
    bind_context(context, "bind CUDA context for memory query")?;
    let mut free = 0;
    let mut total = 0;
    cuda_check(
        unsafe { sys::cuMemGetInfo_v2(&mut free, &mut total) },
        "cuMemGetInfo",
    )?;
    Ok((free as u64, total as u64))
}

fn is_gpu_oom(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("out of memory")
        || error.contains("out_of_memory")
        || error.contains("out_of_device_memory")
        || error.contains("outofmemory")
}

struct StreamWaitEvent {
    event: sys::CUevent,
    context: Arc<CudaContext>,
}

impl Drop for StreamWaitEvent {
    fn drop(&mut self) {
        if let Err(error) = bind_context(&self.context, "bind CUDA context for event destroy") {
            tracing::warn!("Could not enter CUDA context to destroy event: {error}");
            return;
        }
        if let Err(error) = cuda_check(
            unsafe { sys::cuEventDestroy_v2(self.event) },
            "cuEventDestroy",
        ) {
            tracing::warn!("Could not destroy CUDA event: {error}");
        }
    }
}

fn configure_primary_context_flags() -> Result<(), String> {
    let desired_flags = sys::CUctx_flags_enum_CU_CTX_SCHED_BLOCKING_SYNC;
    unsafe {
        cuda_check(sys::cuInit(0), "cuInit")?;
        let mut device = 0;
        cuda_check(sys::cuDeviceGet(&mut device, 0), "cuDeviceGet")?;

        let mut current_flags = 0;
        let mut active = 0;
        cuda_check(
            sys::cuDevicePrimaryCtxGetState(device, &mut current_flags, &mut active),
            "cuDevicePrimaryCtxGetState",
        )?;
        if active != 0 {
            let current_schedule = current_flags & sys::CUctx_flags_enum_CU_CTX_SCHED_MASK;
            if current_schedule != desired_flags {
                return Err(format!(
                    "CUDA primary context is already active with incompatible flags: {current_flags:#x}"
                ));
            }
            return Ok(());
        }

        cuda_check(
            sys::cuDevicePrimaryCtxSetFlags_v2(device, desired_flags),
            "cuDevicePrimaryCtxSetFlags",
        )
    }
}

fn bind_context(context: &CudaContext, operation: &str) -> Result<(), String> {
    context
        .bind_to_thread()
        .map_err(|error| format!("{operation}: {error:?}"))
}

fn cuda_check(result: sys::CUresult, operation: &str) -> Result<(), String> {
    if result == sys::cudaError_enum_CUDA_SUCCESS {
        return Ok(());
    }

    let mut error_name = ptr::null();
    let mut error_string = ptr::null();
    let name = unsafe {
        (sys::cuGetErrorName(result, &mut error_name) == sys::cudaError_enum_CUDA_SUCCESS
            && !error_name.is_null())
        .then(|| CStr::from_ptr(error_name).to_string_lossy().into_owned())
    };
    let detail = unsafe {
        (sys::cuGetErrorString(result, &mut error_string) == sys::cudaError_enum_CUDA_SUCCESS
            && !error_string.is_null())
        .then(|| CStr::from_ptr(error_string).to_string_lossy().into_owned())
    };

    Err(match (name, detail) {
        (Some(name), Some(detail)) => format!("{operation}: {name}: {detail}"),
        (Some(name), None) => format!("{operation}: {name} ({result})"),
        (None, Some(detail)) => format!("{operation}: {detail} ({result})"),
        (None, None) => format!("{operation}: CUDA error {result}"),
    })
}

fn require_device_pointer(pointer: sys::CUdeviceptr, label: &str) -> Result<(), String> {
    let mut memory_type = 0;
    cuda_check(
        unsafe {
            sys::cuPointerGetAttribute(
                (&mut memory_type as *mut sys::CUmemorytype).cast(),
                sys::CUpointer_attribute_enum_CU_POINTER_ATTRIBUTE_MEMORY_TYPE,
                pointer,
            )
        },
        &format!("inspect {label} memory type"),
    )?;
    if memory_type == sys::CUmemorytype_enum_CU_MEMORYTYPE_DEVICE {
        Ok(())
    } else {
        Err(format!(
            "{label} is not CUDA device memory (memory type {memory_type}); refusing CPU-backed export"
        ))
    }
}
