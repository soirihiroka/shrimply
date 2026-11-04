use std::{
    any::{Any, TypeId},
    sync::Arc,
};

use cached::{Cached, UnboundCache};
use shrimply_gpu_cuda::{CudaContext, CudaStream, DeviceCopy, memory};
use shrimply_gpu_cuda_memory::GpuBuffer as DeviceBuffer;

use super::{CompositedVideoFrame, VisualFrame};

/// A modifier owns its parameters and GPU implementation. The compositor only
/// passes the result of the previous modifier to the next one.
pub(crate) trait GpuModifier {
    fn name(&self) -> &'static str;

    fn apply(
        &self,
        context: &mut ModifierContext<'_>,
        input: CanvasRgbaFrame,
    ) -> Result<CanvasRgbaFrame, String>;
}

#[derive(Clone, Copy)]
pub(crate) enum ModifierModule {
    General,
    Blur,
    Geometry,
    Matte,
    Stabilization,
}

/// A canvas-sized RGBA frame passed between modifiers. The first pass may borrow an existing
/// GPU layer; every modifier result owns its CUDA buffer.
pub(crate) struct CanvasRgbaFrame {
    storage: CanvasRgbaStorage,
    width: u32,
    height: u32,
}

enum CanvasRgbaStorage {
    Owned(DeviceBuffer<u32>),
    Borrowed(*const u32),
}

impl CanvasRgbaFrame {
    pub(crate) fn width(&self) -> u32 {
        self.width
    }

    pub(crate) fn height(&self) -> u32 {
        self.height
    }

    pub(crate) fn device_ptr(&self) -> *const u32 {
        match &self.storage {
            CanvasRgbaStorage::Owned(buffer) => buffer.cu_deviceptr() as usize as *const u32,
            CanvasRgbaStorage::Borrowed(ptr) => *ptr,
        }
    }

    pub(crate) fn into_pass(
        self,
        context: &mut ModifierContext<'_>,
    ) -> Result<ModifierPass, String> {
        let pixel_count = self.width as usize * self.height as usize;
        let output = context.take_buffer(pixel_count)?;
        Ok(ModifierPass {
            input: self,
            output,
        })
    }
}

impl From<CompositedVideoFrame> for CanvasRgbaFrame {
    fn from(frame: CompositedVideoFrame) -> Self {
        Self {
            storage: CanvasRgbaStorage::Owned(
                frame
                    .buffer
                    .into_cuda()
                    .expect("CUDA modifier received an Intel frame"),
            ),
            width: frame.width,
            height: frame.height,
        }
    }
}

/// The input and destination for one modifier. Effect implementations launch
/// their kernels using these pointers, then call `finish`.
pub(crate) struct ModifierPass {
    input: CanvasRgbaFrame,
    output: DeviceBuffer<u32>,
}

impl ModifierPass {
    pub(crate) fn input_ptr(&self) -> *const u32 {
        self.input.device_ptr()
    }

    pub(crate) fn output_buffer(&mut self) -> &mut DeviceBuffer<u32> {
        &mut self.output
    }

    pub(crate) fn finish(self, context: &mut ModifierContext<'_>) -> CanvasRgbaFrame {
        if let CanvasRgbaStorage::Owned(buffer) = self.input.storage {
            context.recycle(buffer);
        }
        CanvasRgbaFrame {
            storage: CanvasRgbaStorage::Owned(self.output),
            width: self.input.width,
            height: self.input.height,
        }
    }
}

/// Narrow access to shared CUDA resources and reusable work buffers. Modifier
/// packages keep their own evaluated parameter types and launch logic.
pub(crate) struct ModifierContext<'a> {
    cuda_context: &'a Arc<CudaContext>,
    stream: &'a Arc<CudaStream>,
    spare: &'a mut Option<DeviceBuffer<u32>>,
    scratch: &'a mut Option<DeviceBuffer<u32>>,
    typed_scratch: &'a mut UnboundCache<TypeId, Box<dyn Any>>,
    modules: &'a mut ModifierModules,
    sam2_mask_upload: &'a mut Option<DeviceBuffer<i8>>,
    transparent_fill_mask_upload: &'a mut Option<DeviceBuffer<u8>>,
    sam2_analysis_target: Option<&'a crate::sam2_analysis::AnalysisTarget>,
    sam2_proxy: &'a mut Option<Vec<u8>>,
}

impl ModifierContext<'_> {
    pub(crate) fn stream(&self) -> &Arc<CudaStream> {
        self.stream
    }

    pub(crate) fn modifier_module(
        &mut self,
        kind: ModifierModule,
    ) -> Result<Arc<shrimply_gpu_cuda::CudaModule>, String> {
        let module = self.modules.slot(kind);
        if module.is_none() {
            let started = std::time::Instant::now();
            *module = Some(
                self.cuda_context
                    .load_module_from_image(kind.image())
                    .map_err(|error| format!("load CUDA {} module: {error:?}", kind.label()))?,
            );
            tracing::debug!(
                elapsed_us = started.elapsed().as_micros(),
                "CUDA modifier module loaded"
            );
        }
        Ok(module
            .as_ref()
            .expect("CUDA modifier module loaded")
            .clone())
    }

    fn allocate<T: DeviceCopy>(
        &mut self,
        len: usize,
        description: &str,
    ) -> Result<DeviceBuffer<T>, String> {
        let error = match shrimply_gpu_cuda_memory::global().allocate_buffer(
            self.stream,
            len,
            shrimply_gpu_cuda_memory::AllocationClass::Transient,
            description,
        ) {
            Ok(buffer) => return Ok(buffer),
            Err(error) => error,
        };
        *self.spare = None;
        *self.scratch = None;
        self.typed_scratch.cache_clear();
        *self.sam2_mask_upload = None;
        *self.transparent_fill_mask_upload = None;
        shrimply_gpu_cuda_memory::global()
            .allocate_buffer(
                self.stream,
                len,
                shrimply_gpu_cuda_memory::AllocationClass::Transient,
                description,
            )
            .map_err(|retry| {
            format!(
                "allocate {description} after clearing modifier GPU caches: {retry}; initial error: {error}"
            )
        })
    }

    /// Borrow an additional canvas buffer for multi-pass effects such as a
    /// separable convolution. The effect returns it after its final launch.
    pub(crate) fn take_scratch(&mut self, pixel_count: usize) -> Result<DeviceBuffer<u32>, String> {
        if self
            .scratch
            .as_ref()
            .is_some_and(|buffer| buffer.len() == pixel_count)
        {
            return Ok(self
                .scratch
                .take()
                .expect("checked modifier scratch buffer"));
        }
        *self.scratch = None;
        self.allocate(pixel_count, "CUDA modifier scratch frame")
    }

    pub(crate) fn recycle_scratch(&mut self, buffer: DeviceBuffer<u32>) {
        *self.scratch = Some(buffer);
    }

    pub(crate) fn upload<T: DeviceCopy>(
        &mut self,
        values: &[T],
    ) -> Result<DeviceBuffer<T>, String> {
        let mut buffer = self.allocate(values.len(), "CUDA modifier data")?;
        if values.is_empty() {
            return Ok(buffer);
        }
        buffer
            .copy_from_host(self.stream, values)
            .map_err(|error| format!("upload CUDA modifier data: {error:?}"))?;
        Ok(buffer)
    }

    pub(crate) fn upload_sam2_mask(&mut self, mask: &[i8]) -> Result<*const i8, String> {
        let upload = self.upload(mask)?;
        let pointer = upload.cu_deviceptr() as usize as *const i8;
        *self.sam2_mask_upload = Some(upload);
        Ok(pointer)
    }

    pub(crate) fn upload_transparent_fill_mask(
        &mut self,
        mask: &[u8],
    ) -> Result<*const u8, String> {
        let upload = self.upload(mask)?;
        let pointer = upload.cu_deviceptr() as usize as *const u8;
        *self.transparent_fill_mask_upload = Some(upload);
        Ok(pointer)
    }

    pub(crate) fn capture_sam2(
        &mut self,
        target: &crate::sam2_analysis::AnalysisTarget,
        input: &CanvasRgbaFrame,
    ) -> Result<bool, String> {
        if self.sam2_analysis_target != Some(target) {
            return Ok(false);
        }
        let pixels = crate::modifiers::sam2::MODEL_SIZE as usize
            * crate::modifiers::sam2::MODEL_SIZE as usize;
        let proxy: DeviceBuffer<u32> = self.allocate(pixels, "SAM2 proxy frame")?;
        let module = self.modifier_module(ModifierModule::Matte)?;
        unsafe {
            shrimply_gpu_cuda::cuda_launch! {
                kernel: crate::modifiers::sam2::sam2_proxy,
                stream: self.stream, module: &module,
                config: shrimply_gpu_cuda::LaunchConfig::for_num_elems(crate::modifiers::sam2::MODEL_SIZE.pow(2)),
                args: [input.device_ptr(), proxy.cu_deviceptr() as usize as *mut u32, shrimply_render_core::Sam2ProxyParams {
                    input_width: input.width(),
                    input_height: input.height(),
                    model_size: crate::modifiers::sam2::MODEL_SIZE,
                }]
            }
        }
        .map_err(|error| format!("launch SAM2 proxy kernel: {error:?}"))?;
        let mut host = vec![0_u32; pixels];
        unsafe {
            memory::memcpy_dtoh_async(
                host.as_mut_ptr(),
                proxy.cu_deviceptr(),
                std::mem::size_of_val(host.as_slice()),
                self.stream.cu_stream(),
            )
        }
        .map_err(|error| format!("download SAM2 proxy frame: {error:?}"))?;
        self.stream
            .synchronize()
            .map_err(|error| format!("synchronize SAM2 proxy frame: {error:?}"))?;
        let bytes = unsafe {
            std::slice::from_raw_parts(
                host.as_ptr().cast::<u8>(),
                std::mem::size_of_val(host.as_slice()),
            )
        };
        *self.sam2_proxy = Some(bytes.to_vec());
        Ok(true)
    }

    fn take_buffer(&mut self, pixel_count: usize) -> Result<DeviceBuffer<u32>, String> {
        if self
            .spare
            .as_ref()
            .is_some_and(|buffer| buffer.len() == pixel_count)
        {
            return Ok(self.spare.take().expect("checked modifier spare buffer"));
        }
        *self.spare = None;
        self.allocate(pixel_count, "CUDA modifier frame")
    }

    fn recycle(&mut self, buffer: DeviceBuffer<u32>) {
        *self.spare = Some(buffer);
    }
}

pub(crate) struct ModifierWorkspace {
    spare: Option<DeviceBuffer<u32>>,
    scratch: Option<DeviceBuffer<u32>>,
    typed_scratch: UnboundCache<TypeId, Box<dyn Any>>,
    modules: ModifierModules,
    sam2_mask_upload: Option<DeviceBuffer<i8>>,
    transparent_fill_mask_upload: Option<DeviceBuffer<u8>>,
    sam2_analysis_target: Option<crate::sam2_analysis::AnalysisTarget>,
    sam2_proxy: Option<Vec<u8>>,
}

#[derive(Default)]
struct ModifierModules {
    general: Option<Arc<shrimply_gpu_cuda::CudaModule>>,
    blur: Option<Arc<shrimply_gpu_cuda::CudaModule>>,
    geometry: Option<Arc<shrimply_gpu_cuda::CudaModule>>,
    matte: Option<Arc<shrimply_gpu_cuda::CudaModule>>,
    stabilization: Option<Arc<shrimply_gpu_cuda::CudaModule>>,
}

impl ModifierModules {
    fn slot(&mut self, kind: ModifierModule) -> &mut Option<Arc<shrimply_gpu_cuda::CudaModule>> {
        match kind {
            ModifierModule::General => &mut self.general,
            ModifierModule::Blur => &mut self.blur,
            ModifierModule::Geometry => &mut self.geometry,
            ModifierModule::Matte => &mut self.matte,
            ModifierModule::Stabilization => &mut self.stabilization,
        }
    }
}

impl ModifierModule {
    pub(crate) fn image(self) -> &'static [u8] {
        match self {
            Self::General => shrimply_render_kernels_cuda::MODIFIERS,
            Self::Blur => shrimply_render_kernels_cuda::MODIFIERS_BLUR,
            Self::Geometry => shrimply_render_kernels_cuda::MODIFIERS_GEOMETRY,
            Self::Matte => shrimply_render_kernels_cuda::MODIFIERS_MATTE,
            Self::Stabilization => shrimply_render_kernels_cuda::STABILIZATION,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::General => "modifier",
            Self::Blur => "blur modifier",
            Self::Geometry => "geometry modifier",
            Self::Matte => "matte modifier",
            Self::Stabilization => "stabilization modifier",
        }
    }
}

impl ModifierWorkspace {
    pub(crate) fn new() -> Self {
        Self {
            spare: None,
            scratch: None,
            typed_scratch: UnboundCache::builder()
                .build()
                .expect("valid CUDA typed scratch cache"),
            modules: ModifierModules::default(),
            sam2_mask_upload: None,
            transparent_fill_mask_upload: None,
            sam2_analysis_target: None,
            sam2_proxy: None,
        }
    }

    pub(crate) fn begin_sam2_analysis(&mut self, target: crate::sam2_analysis::AnalysisTarget) {
        self.sam2_analysis_target = Some(target);
        self.sam2_proxy = None;
    }

    pub(crate) fn end_sam2_analysis(&mut self) {
        self.sam2_analysis_target = None;
        self.sam2_proxy = None;
    }

    pub(crate) fn take_sam2_proxy(&mut self) -> Option<Vec<u8>> {
        self.sam2_proxy.take()
    }

    pub(crate) fn clear_cached_gpu_resources(&mut self) -> bool {
        let released = self.spare.is_some()
            || self.scratch.is_some()
            || self.typed_scratch.cache_size() != 0
            || self.sam2_mask_upload.is_some()
            || self.transparent_fill_mask_upload.is_some()
            || self.modules.general.is_some()
            || self.modules.blur.is_some()
            || self.modules.geometry.is_some()
            || self.modules.matte.is_some()
            || self.modules.stabilization.is_some();
        self.spare = None;
        self.scratch = None;
        self.typed_scratch.cache_clear();
        self.sam2_mask_upload = None;
        self.transparent_fill_mask_upload = None;
        self.modules = ModifierModules::default();
        released
    }

    pub(crate) fn apply(
        &mut self,
        cuda_context: &Arc<CudaContext>,
        stream: &Arc<CudaStream>,
        frame: CompositedVideoFrame,
        modifiers: &[&dyn GpuModifier],
        serial: u64,
    ) -> Result<CompositedVideoFrame, String> {
        self.apply_frame(
            cuda_context,
            stream,
            CanvasRgbaFrame::from(frame),
            modifiers,
            serial,
        )
    }

    pub(crate) fn apply_layer(
        &mut self,
        cuda_context: &Arc<CudaContext>,
        stream: &Arc<CudaStream>,
        layer: &VisualFrame,
        modifier: &dyn GpuModifier,
        serial: u64,
    ) -> Result<CompositedVideoFrame, String> {
        let plane = layer.plane(0).expect("RGBA layer has no plane");
        self.apply_frame(
            cuda_context,
            stream,
            CanvasRgbaFrame {
                storage: CanvasRgbaStorage::Borrowed(plane.device_ptr as usize as *const u32),
                width: layer.width(),
                height: layer.height(),
            },
            std::slice::from_ref(&modifier),
            serial,
        )
    }

    fn apply_frame(
        &mut self,
        cuda_context: &Arc<CudaContext>,
        stream: &Arc<CudaStream>,
        mut frame: CanvasRgbaFrame,
        modifiers: &[&dyn GpuModifier],
        serial: u64,
    ) -> Result<CompositedVideoFrame, String> {
        let mut context = ModifierContext {
            cuda_context,
            stream,
            spare: &mut self.spare,
            scratch: &mut self.scratch,
            typed_scratch: &mut self.typed_scratch,
            modules: &mut self.modules,
            sam2_mask_upload: &mut self.sam2_mask_upload,
            transparent_fill_mask_upload: &mut self.transparent_fill_mask_upload,
            sam2_analysis_target: self.sam2_analysis_target.as_ref(),
            sam2_proxy: &mut self.sam2_proxy,
        };
        for modifier in modifiers {
            frame = modifier
                .apply(&mut context, frame)
                .map_err(|error| format!("{} modifier: {error}", modifier.name()))?;
        }
        cuda_context
            .bind_to_thread()
            .map_err(|error| format!("bind CUDA context after modifiers: {error:?}"))?;
        stream
            .synchronize()
            .map_err(|error| format!("synchronize CUDA modifiers: {error:?}"))?;
        if self.sam2_analysis_target.is_none() {
            self.sam2_mask_upload = None;
        }
        self.transparent_fill_mask_upload = None;
        let CanvasRgbaStorage::Owned(buffer) = frame.storage else {
            return Err("CUDA modifier did not produce an owned frame".to_string());
        };
        Ok(CompositedVideoFrame::new(
            buffer,
            frame.width,
            frame.height,
            serial,
        ))
    }
}
