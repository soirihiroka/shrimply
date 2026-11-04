use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use shrimply_gpu_cuda::{CudaContext, CudaStream, sys};
use shrimply_gpu_cuda_memory::AllocationClass;
use shrimply_gpu_cuda_memory::GpuBuffer;
pub use shrimply_gpu_cuda_memory::MemoryKind;

mod ffmpeg;

pub const GPU_FRAME_ALLOCATION_EXHAUSTED: &str =
    "allocate visual frame plane: CUDA ran out of memory";

static GPU_FRAME_COUNT: AtomicU64 = AtomicU64::new(0);
static GPU_FRAME_BYTES: AtomicU64 = AtomicU64::new(0);
static GPU_OOM_GENERATION: AtomicU64 = AtomicU64::new(0);

pub use ffmpeg::ffmpeg_cuda_context;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualFormat {
    Rgba8,
    Nv12,
    P010,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Device {
    Cpu,
    Cuda(usize),
}

#[derive(Clone, Copy)]
pub struct VisualPlane {
    pub device_ptr: sys::CUdeviceptr,
    pub pitch_bytes: usize,
    pub width_bytes: usize,
    pub height: usize,
}

#[derive(Clone)]
pub struct VisualFrame {
    inner: Arc<FrameStorage>,
    format: VisualFormat,
    width: u32,
    height: u32,
}

enum FrameStorage {
    Gpu(GpuStorage),
    Cpu(Vec<CpuPlane>),
}

struct GpuStorage {
    context: Arc<CudaContext>,
    device_index: usize,
    planes: Vec<GpuPlane>,
    allocation_bytes: u64,
    _owner: GpuOwner,
}

enum GpuOwner {
    Oxide { buffers: Vec<GpuBuffer<u8>> },
    External { _owner: Box<dyn Any + Send + Sync> },
}

struct GpuPlane {
    device_ptr: sys::CUdeviceptr,
    pitch_bytes: usize,
    width_bytes: usize,
    height: usize,
    memory_kind: MemoryKind,
}

struct CpuPlane {
    bytes: Vec<u8>,
    width_bytes: usize,
    height: usize,
}

impl GpuStorage {
    fn new(context: Arc<CudaContext>, planes: Vec<GpuPlane>, owner: GpuOwner) -> Self {
        let allocation_bytes = planes
            .iter()
            .try_fold(0_u64, |bytes, plane| {
                let plane_bytes = plane
                    .pitch_bytes
                    .checked_mul(plane.height)
                    .and_then(|bytes| u64::try_from(bytes).ok())?;
                bytes.checked_add(plane_bytes)
            })
            .expect("GPU visual frame byte accounting overflowed");
        GPU_FRAME_BYTES
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |bytes| {
                bytes.checked_add(allocation_bytes)
            })
            .expect("GPU visual frame byte counter overflowed");
        GPU_FRAME_COUNT
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                count.checked_add(1)
            })
            .expect("GPU visual frame counter overflowed");
        Self {
            device_index: context.ordinal(),
            context,
            planes,
            allocation_bytes,
            _owner: owner,
        }
    }
}

impl Drop for GpuStorage {
    fn drop(&mut self) {
        GPU_FRAME_COUNT
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                count.checked_sub(1)
            })
            .expect("GPU visual frame counter underflowed");
        GPU_FRAME_BYTES
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |bytes| {
                bytes.checked_sub(self.allocation_bytes)
            })
            .expect("GPU visual frame byte counter underflowed");
    }
}

pub fn gpu_allocation_stats() -> (u64, u64) {
    (
        GPU_FRAME_COUNT.load(Ordering::Acquire),
        GPU_FRAME_BYTES.load(Ordering::Acquire),
    )
}

pub fn gpu_oom_generation() -> u64 {
    GPU_OOM_GENERATION.load(Ordering::Acquire)
}

impl VisualFrame {
    pub fn allocate(
        context: Arc<CudaContext>,
        format: VisualFormat,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        Self::allocate_on(context, format, width, height, AllocationClass::Transient)
    }

    pub fn allocate_persistent(
        context: Arc<CudaContext>,
        format: VisualFormat,
        width: u32,
        height: u32,
        description: &str,
    ) -> Result<Self, String> {
        Self::allocate_on_with_description(
            context,
            format,
            width,
            height,
            AllocationClass::Persistent,
            description,
        )
    }

    pub fn allocate_cached(
        context: Arc<CudaContext>,
        format: VisualFormat,
        width: u32,
        height: u32,
        description: &str,
    ) -> Result<Self, String> {
        Self::allocate_on_with_description(
            context,
            format,
            width,
            height,
            AllocationClass::Cached,
            description,
        )
    }

    fn allocate_on(
        context: Arc<CudaContext>,
        format: VisualFormat,
        width: u32,
        height: u32,
        allocation_class: AllocationClass,
    ) -> Result<Self, String> {
        Self::allocate_on_with_description(
            context,
            format,
            width,
            height,
            allocation_class,
            "CUDA visual plane",
        )
    }

    fn allocate_on_with_description(
        context: Arc<CudaContext>,
        format: VisualFormat,
        width: u32,
        height: u32,
        allocation_class: AllocationClass,
        description: &str,
    ) -> Result<Self, String> {
        let layout = plane_layout(format, width, height)?;
        context
            .bind_to_thread()
            .map_err(|error| format!("bind CUDA context for visual frame allocation: {error}"))?;
        let mut buffers = Vec::with_capacity(layout.len());
        let mut planes = Vec::with_capacity(layout.len());
        for (width_bytes, height) in layout {
            let length = width_bytes
                .checked_mul(height)
                .ok_or("visual frame allocation size overflow")?;
            let buffer = shrimply_gpu_cuda_memory::global()
                .allocate_buffer::<u8>(
                    context.default_stream().as_ref(),
                    length,
                    allocation_class,
                    description,
                )
                .map_err(|error| {
                    if error.contains("out of memory") || error.contains("OUT_OF_MEMORY") {
                        GPU_OOM_GENERATION.fetch_add(1, Ordering::AcqRel);
                        format!("{GPU_FRAME_ALLOCATION_EXHAUSTED}; {error}")
                    } else {
                        error
                    }
                })?;
            let device_ptr = buffer.cu_deviceptr();
            let memory_kind = buffer.memory_kind();
            buffers.push(buffer);
            planes.push(GpuPlane {
                device_ptr,
                pitch_bytes: width_bytes,
                width_bytes,
                height,
                memory_kind,
            });
        }
        context
            .default_stream()
            .synchronize()
            .map_err(|error| format!("finish CUDA visual plane allocation: {error}"))?;
        Ok(Self {
            inner: Arc::new(FrameStorage::Gpu(GpuStorage::new(
                context,
                planes,
                GpuOwner::Oxide { buffers },
            ))),
            format,
            width,
            height,
        })
    }

    /// # Safety
    ///
    /// Every plane must remain valid for the lifetime of `owner`, belong to `context`, and match
    /// the declared format and dimensions. Dropping `owner` must release the external storage.
    pub unsafe fn from_external_gpu(
        context: Arc<CudaContext>,
        format: VisualFormat,
        width: u32,
        height: u32,
        planes: &[VisualPlane],
        owner: Box<dyn Any + Send + Sync>,
    ) -> Result<Self, String> {
        unsafe {
            Self::from_external_gpu_with_memory_kinds(
                context,
                format,
                width,
                height,
                planes,
                &vec![MemoryKind::Device; planes.len()],
                owner,
            )
        }
    }

    /// # Safety
    ///
    /// The `memory_kinds` must describe the allocation backing each plane. All other ownership
    /// requirements are the same as [`Self::from_external_gpu`].
    pub unsafe fn from_external_gpu_with_memory_kinds(
        context: Arc<CudaContext>,
        format: VisualFormat,
        width: u32,
        height: u32,
        planes: &[VisualPlane],
        memory_kinds: &[MemoryKind],
        owner: Box<dyn Any + Send + Sync>,
    ) -> Result<Self, String> {
        Self::from_gpu_planes(
            context,
            format,
            width,
            height,
            planes,
            memory_kinds,
            GpuOwner::External { _owner: owner },
        )
    }

    /// # Safety
    ///
    /// Each plane must describe the corresponding owned buffer and match the declared format and
    /// dimensions.
    pub unsafe fn from_owned_gpu_buffers(
        context: Arc<CudaContext>,
        format: VisualFormat,
        width: u32,
        height: u32,
        planes: &[VisualPlane],
        buffers: Vec<GpuBuffer<u8>>,
    ) -> Result<Self, String> {
        if buffers.len() != planes.len()
            || buffers.iter().zip(planes).any(|(buffer, plane)| {
                buffer.cu_deviceptr() != plane.device_ptr
                    || plane
                        .pitch_bytes
                        .checked_mul(plane.height)
                        .is_none_or(|bytes| bytes > buffer.len())
            })
        {
            return Err("owned visual frame planes do not match their CUDA buffers".to_string());
        }
        let memory_kinds = buffers
            .iter()
            .map(GpuBuffer::memory_kind)
            .collect::<Vec<_>>();
        Self::from_gpu_planes(
            context,
            format,
            width,
            height,
            planes,
            &memory_kinds,
            GpuOwner::Oxide { buffers },
        )
    }

    fn from_gpu_planes(
        context: Arc<CudaContext>,
        format: VisualFormat,
        width: u32,
        height: u32,
        planes: &[VisualPlane],
        memory_kinds: &[MemoryKind],
        owner: GpuOwner,
    ) -> Result<Self, String> {
        let layout = plane_layout(format, width, height)?;
        if planes.len() != layout.len()
            || memory_kinds.len() != layout.len()
            || planes
                .iter()
                .zip(layout)
                .any(|(plane, (width_bytes, height))| {
                    plane.device_ptr == 0
                        || plane.pitch_bytes < width_bytes
                        || plane.width_bytes != width_bytes
                        || plane.height != height
                })
        {
            return Err("external visual frame planes do not match its format".to_string());
        }
        Ok(Self {
            inner: Arc::new(FrameStorage::Gpu(GpuStorage::new(
                context,
                planes
                    .iter()
                    .zip(memory_kinds)
                    .map(|(plane, &memory_kind)| GpuPlane {
                        device_ptr: plane.device_ptr,
                        pitch_bytes: plane.pitch_bytes,
                        width_bytes: plane.width_bytes,
                        height: plane.height,
                        memory_kind,
                    })
                    .collect(),
                owner,
            ))),
            format,
            width,
            height,
        })
    }

    pub fn from_rgba_bytes(width: u32, height: u32, bytes: Vec<u8>) -> Result<Self, String> {
        let [(width_bytes, height_rows)] = plane_layout(VisualFormat::Rgba8, width, height)?
            .try_into()
            .expect("RGBA layout must contain exactly one plane");
        let expected = width_bytes
            .checked_mul(height_rows)
            .ok_or("RGBA frame size overflow")?;
        if bytes.len() != expected {
            return Err(format!(
                "RGBA frame has {} bytes, expected {expected}",
                bytes.len()
            ));
        }
        Ok(Self {
            inner: Arc::new(FrameStorage::Cpu(vec![CpuPlane {
                bytes,
                width_bytes,
                height: height_rows,
            }])),
            format: VisualFormat::Rgba8,
            width,
            height,
        })
    }

    pub fn format(&self) -> VisualFormat {
        self.format
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn context(&self) -> Option<&Arc<CudaContext>> {
        match &*self.inner {
            FrameStorage::Gpu(storage) => Some(&storage.context),
            FrameStorage::Cpu(_) => None,
        }
    }

    pub fn plane(&self, index: usize) -> Option<VisualPlane> {
        let FrameStorage::Gpu(storage) = &*self.inner else {
            return None;
        };
        let plane = storage.planes.get(index)?;
        if let GpuOwner::Oxide { buffers } = &storage._owner {
            buffers.get(index)?.cu_deviceptr();
        }
        Some(VisualPlane {
            device_ptr: plane.device_ptr,
            pitch_bytes: plane.pitch_bytes,
            width_bytes: plane.width_bytes,
            height: plane.height,
        })
    }

    pub fn memory_kind(&self, index: usize) -> Option<MemoryKind> {
        let FrameStorage::Gpu(storage) = &*self.inner else {
            return None;
        };
        storage.planes.get(index).map(|plane| plane.memory_kind)
    }

    pub fn is_managed(&self) -> bool {
        let FrameStorage::Gpu(storage) = &*self.inner else {
            return false;
        };
        !storage.planes.is_empty()
            && storage
                .planes
                .iter()
                .all(|plane| plane.memory_kind == MemoryKind::Managed)
    }

    pub fn is_cached(&self) -> bool {
        let FrameStorage::Gpu(storage) = &*self.inner else {
            return false;
        };
        let GpuOwner::Oxide { buffers } = &storage._owner else {
            return false;
        };
        !buffers.is_empty()
            && buffers
                .iter()
                .all(|buffer| buffer.allocation_class() == AllocationClass::Cached)
    }

    pub fn prefetch_to_device(&self, stream: &CudaStream) -> Result<(), String> {
        let FrameStorage::Gpu(storage) = &*self.inner else {
            return Ok(());
        };
        let GpuOwner::Oxide { buffers } = &storage._owner else {
            return Ok(());
        };
        for buffer in buffers {
            buffer.prefetch_to_device(stream)?;
        }
        Ok(())
    }

    pub fn plane_count(&self) -> usize {
        match &*self.inner {
            FrameStorage::Gpu(storage) => storage.planes.len(),
            FrameStorage::Cpu(planes) => planes.len(),
        }
    }

    pub fn is_gpu(&self) -> bool {
        matches!(&*self.inner, FrameStorage::Gpu(_))
    }

    pub fn device(&self) -> Device {
        match &*self.inner {
            FrameStorage::Gpu(storage) => Device::Cuda(storage.device_index),
            FrameStorage::Cpu(_) => Device::Cpu,
        }
    }

    pub fn bytes(&self) -> u64 {
        match &*self.inner {
            FrameStorage::Gpu(storage) => storage.allocation_bytes,
            FrameStorage::Cpu(planes) => planes.iter().map(|plane| plane.bytes.len() as u64).sum(),
        }
    }

    pub fn copy_to(&self, device: Device) -> Result<Self, String> {
        if self.device() == device {
            return Ok(self.clone());
        }
        match device {
            Device::Cpu => self.copy_to_cpu(),
            Device::Cuda(index) => {
                if self.is_gpu() {
                    self.copy_to_cpu()?.copy_to_cuda(index)
                } else {
                    self.copy_to_cuda(index)
                }
            }
        }
    }

    pub fn copy_to_cached(
        &self,
        context: Arc<CudaContext>,
        stream: &CudaStream,
        description: &str,
    ) -> Result<Self, String> {
        let FrameStorage::Gpu(storage) = &*self.inner else {
            return Err("copy a CPU visual frame to cached GPU storage".to_string());
        };
        if storage.device_index != context.ordinal() {
            return Err("copy a visual frame between different CUDA devices".to_string());
        }
        let frame = Self::allocate_cached(
            context.clone(),
            self.format,
            self.width,
            self.height,
            description,
        )?;
        context
            .bind_to_thread()
            .map_err(|error| format!("bind CUDA context for cached frame copy: {error}"))?;
        for (index, source) in storage.planes.iter().enumerate() {
            let destination = frame
                .plane(index)
                .expect("cached visual frame lost a format plane");
            let mut copy: sys::CUDA_MEMCPY2D = unsafe { std::mem::zeroed() };
            copy.srcMemoryType = memory_type(source.memory_kind);
            copy.srcDevice = source.device_ptr;
            copy.srcPitch = source.pitch_bytes;
            copy.dstMemoryType = memory_type(
                frame
                    .memory_kind(index)
                    .expect("cached visual frame lost allocation metadata"),
            );
            copy.dstDevice = destination.device_ptr;
            copy.dstPitch = destination.pitch_bytes;
            copy.WidthInBytes = source.width_bytes;
            copy.Height = source.height;
            let result = unsafe { sys::cuMemcpy2DAsync_v2(&copy, stream.cu_stream()) };
            if result != sys::cudaError_enum_CUDA_SUCCESS {
                return Err(format!("copy cached visual frame plane: {result:?}"));
            }
        }
        Ok(frame)
    }

    pub fn copy_plane_to_vec(&self, index: usize) -> Result<Vec<u8>, String> {
        let cpu = self.copy_to_cpu()?;
        let FrameStorage::Cpu(planes) = &*cpu.inner else {
            unreachable!("copying a visual frame to the CPU returned GPU storage");
        };
        planes
            .get(index)
            .map(|plane| plane.bytes.clone())
            .ok_or_else(|| format!("visual frame has no plane {index}"))
    }

    fn copy_to_cpu(&self) -> Result<Self, String> {
        let FrameStorage::Gpu(storage) = &*self.inner else {
            return Ok(self.clone());
        };
        storage
            .context
            .bind_to_thread()
            .map_err(|error| format!("bind CUDA context for visual frame download: {error}"))?;
        let stream = storage.context.default_stream();
        let mut planes = Vec::with_capacity(storage.planes.len());
        for source in &storage.planes {
            let length = source
                .width_bytes
                .checked_mul(source.height)
                .ok_or("CPU visual frame size overflow")?;
            let mut bytes = vec![0_u8; length];
            let mut copy: sys::CUDA_MEMCPY2D = unsafe { std::mem::zeroed() };
            copy.srcMemoryType = memory_type(source.memory_kind);
            copy.srcDevice = source.device_ptr;
            copy.srcPitch = source.pitch_bytes;
            copy.dstMemoryType = sys::CUmemorytype_enum_CU_MEMORYTYPE_HOST;
            copy.dstHost = bytes.as_mut_ptr().cast();
            copy.dstPitch = source.width_bytes;
            copy.WidthInBytes = source.width_bytes;
            copy.Height = source.height;
            let result = unsafe { sys::cuMemcpy2DAsync_v2(&copy, stream.cu_stream()) };
            if result != sys::cudaError_enum_CUDA_SUCCESS {
                return Err(format!("download visual frame plane: {result:?}"));
            }
            planes.push(CpuPlane {
                bytes,
                width_bytes: source.width_bytes,
                height: source.height,
            });
        }
        stream
            .synchronize()
            .map_err(|error| format!("finish visual frame download: {error}"))?;
        Ok(Self {
            inner: Arc::new(FrameStorage::Cpu(planes)),
            format: self.format,
            width: self.width,
            height: self.height,
        })
    }

    fn copy_to_cuda(&self, device_ordinal: usize) -> Result<Self, String> {
        let FrameStorage::Cpu(planes) = &*self.inner else {
            return Ok(self.clone());
        };
        let context = CudaContext::new(device_ordinal)
            .map_err(|error| format!("create CUDA context for visual frame upload: {error}"))?;
        let frame = Self::allocate_on(
            context.clone(),
            self.format,
            self.width,
            self.height,
            AllocationClass::Persistent,
        )?;
        let stream = context.default_stream();
        for (index, source) in planes.iter().enumerate() {
            let destination = frame
                .plane(index)
                .expect("uploaded visual frame lost a format plane");
            let mut copy: sys::CUDA_MEMCPY2D = unsafe { std::mem::zeroed() };
            copy.srcMemoryType = sys::CUmemorytype_enum_CU_MEMORYTYPE_HOST;
            copy.srcHost = source.bytes.as_ptr().cast();
            copy.srcPitch = source.width_bytes;
            copy.dstMemoryType = memory_type(
                frame
                    .memory_kind(index)
                    .expect("uploaded visual frame lost allocation metadata"),
            );
            copy.dstDevice = destination.device_ptr;
            copy.dstPitch = destination.pitch_bytes;
            copy.WidthInBytes = source.width_bytes;
            copy.Height = source.height;
            let result = unsafe { sys::cuMemcpy2DAsync_v2(&copy, stream.cu_stream()) };
            if result != sys::cudaError_enum_CUDA_SUCCESS {
                return Err(format!("upload visual frame plane: {result:?}"));
            }
        }
        stream
            .synchronize()
            .map_err(|error| format!("finish visual frame upload: {error}"))?;
        Ok(frame)
    }
}

fn memory_type(kind: MemoryKind) -> sys::CUmemorytype {
    match kind {
        MemoryKind::Device => sys::CUmemorytype_enum_CU_MEMORYTYPE_DEVICE,
        MemoryKind::Managed => sys::CUmemorytype_enum_CU_MEMORYTYPE_UNIFIED,
    }
}

fn plane_layout(
    format: VisualFormat,
    width: u32,
    height: u32,
) -> Result<Vec<(usize, usize)>, String> {
    if width == 0 || height == 0 {
        return Err("a visual frame cannot be empty".to_string());
    }
    let width = width as usize;
    let height = height as usize;
    Ok(match format {
        VisualFormat::Rgba8 => vec![(
            width.checked_mul(4).ok_or("RGBA frame row size overflow")?,
            height,
        )],
        VisualFormat::Nv12 => vec![(width, height), (width, height.div_ceil(2))],
        VisualFormat::P010 => {
            let row = width.checked_mul(2).ok_or("P010 frame row size overflow")?;
            vec![(row, height), (row, height.div_ceil(2))]
        }
    })
}
