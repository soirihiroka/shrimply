use shrimply_cuda::sys;
use shrimply_gpu_memory::GpuBuffer as DeviceBuffer;

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct CompositedFrameStorageKey {
    serial: u64,
    device_ptr: sys::CUdeviceptr,
    width: u32,
    height: u32,
}

pub struct CompositedVideoFrame {
    pub buffer: DeviceBuffer<u32>,
    pub width: u32,
    pub height: u32,
    pub storage_key: CompositedFrameStorageKey,
}

impl CompositedVideoFrame {
    pub(super) fn new(buffer: DeviceBuffer<u32>, width: u32, height: u32, serial: u64) -> Self {
        let storage_key = CompositedFrameStorageKey {
            serial,
            device_ptr: buffer.cu_deviceptr(),
            width,
            height,
        };
        Self {
            buffer,
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
