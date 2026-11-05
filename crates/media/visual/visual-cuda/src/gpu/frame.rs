use std::sync::Arc;

use shrimply_gpu_cuda_memory::GpuBuffer as DeviceBuffer;

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct CompositedFrameStorageKey {
    serial: u64,
    device_ptr: u64,
    width: u32,
    height: u32,
}

pub struct CompositedVideoFrame {
    pub buffer: FrameBuffer,
    pub width: u32,
    pub height: u32,
    pub storage_key: CompositedFrameStorageKey,
}

pub enum FrameBuffer {
    Cuda(Arc<DeviceBuffer<u32>>),
    #[cfg(target_os = "linux")]
    Intel {
        display: shrimply_render_vulkan::Texture,
        rendered: shrimply_render_vulkan::Buffer,
    },
}

impl FrameBuffer {
    pub fn cuda(&self) -> Result<&Arc<DeviceBuffer<u32>>, String> {
        match self {
            Self::Cuda(buffer) => Ok(buffer),
            #[cfg(target_os = "linux")]
            Self::Intel { .. } => Err("Intel frame passed to a CUDA operation".into()),
        }
    }
    pub fn into_cuda(self) -> Result<DeviceBuffer<u32>, String> {
        match self {
            Self::Cuda(buffer) => Arc::try_unwrap(buffer)
                .map_err(|_| "Cannot consume a shared CUDA preview buffer".into()),
            #[cfg(target_os = "linux")]
            Self::Intel { .. } => Err("Intel frame passed to a CUDA encoder".into()),
        }
    }
}

impl CompositedVideoFrame {
    /// Capturing a frame explicitly requests CPU pixels. Playback uses native GPU imports.
    pub fn rgba_bytes(&self) -> Result<Vec<u8>, String> {
        match &self.buffer {
            FrameBuffer::Cuda(buffer) => {
                buffer
                    .context()
                    .synchronize()
                    .map_err(|error| format!("Synchronize preview capture: {error}"))?;
                let pixels = buffer
                    .to_host_vec(&buffer.context().default_stream())
                    .map_err(|error| format!("Read preview capture: {error}"))?;
                Ok(pixels.into_iter().flat_map(u32::to_le_bytes).collect())
            }
            #[cfg(target_os = "linux")]
            FrameBuffer::Intel { rendered, .. } => {
                shrimply_render_vulkan::Renderer::new()?.readback(rendered)
            }
        }
    }
    pub(super) fn new(buffer: DeviceBuffer<u32>, width: u32, height: u32, serial: u64) -> Self {
        let storage_key = CompositedFrameStorageKey {
            serial,
            device_ptr: buffer.cu_deviceptr(),
            width,
            height,
        };
        Self {
            buffer: FrameBuffer::Cuda(Arc::new(buffer)),
            width,
            height,
            storage_key,
        }
    }

    #[cfg(target_os = "linux")]
    pub fn intel(
        display: shrimply_render_vulkan::Texture,
        rendered: shrimply_render_vulkan::Buffer,
        width: u32,
        height: u32,
        serial: u64,
    ) -> Self {
        let storage_key = CompositedFrameStorageKey {
            serial,
            device_ptr: display.id(),
            width,
            height,
        };
        Self {
            buffer: FrameBuffer::Intel { display, rendered },
            width,
            height,
            storage_key,
        }
    }

    pub fn debug_label(&self) -> String {
        format!(
            "{}x{} serial={} device_ptr=0x{:x}",
            self.width, self.height, self.storage_key.serial, self.storage_key.device_ptr
        )
    }
}
