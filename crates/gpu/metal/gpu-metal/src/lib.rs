#![cfg(target_os = "macos")]

use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_metal::{
    MTLBuffer, MTLCommandBuffer, MTLCommandBufferStatus, MTLCommandQueue,
    MTLCreateSystemDefaultDevice, MTLDevice, MTLResourceOptions,
};
use std::ptr::NonNull;

#[derive(Clone)]
pub struct Buffer(Retained<ProtocolObject<dyn MTLBuffer>>);
impl Buffer {
    pub fn metal(&self) -> &Retained<ProtocolObject<dyn MTLBuffer>> {
        &self.0
    }
    pub fn address(&self) -> u64 {
        self.0.gpuAddress()
    }

    /// Copy shared-storage contents after every submission writing this buffer
    /// has reported completion.
    pub fn copy_bytes(&self) -> Vec<u8> {
        unsafe {
            std::slice::from_raw_parts(self.0.contents().as_ptr().cast(), self.0.length()).to_vec()
        }
    }

    pub fn write_bytes(&self, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() > self.0.length() {
            return Err("Metal buffer write exceeds its allocation".into());
        }
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                self.0.contents().as_ptr().cast::<u8>(),
                bytes.len(),
            );
        }
        Ok(())
    }
}

pub struct Submission {
    command: Retained<ProtocolObject<dyn MTLCommandBuffer>>,
    // Every pointer reachable from the argument block remains valid until completion.
    _resources: Vec<Buffer>,
}
impl Submission {
    pub fn wait(&self) -> Result<(), String> {
        self.command.waitUntilCompleted();
        if self.completed()? {
            Ok(())
        } else {
            Err("Metal command did not finish after waiting".into())
        }
    }

    pub fn new(
        command: Retained<ProtocolObject<dyn MTLCommandBuffer>>,
        resources: Vec<Buffer>,
    ) -> Self {
        Self {
            command,
            _resources: resources,
        }
    }

    pub fn completed(&self) -> Result<bool, String> {
        match self.command.status() {
            MTLCommandBufferStatus::Completed => Ok(true),
            MTLCommandBufferStatus::Error => Err(self.command.error().map_or_else(
                || "Metal compute command failed".to_string(),
                |error| error.to_string(),
            )),
            _ => Ok(false),
        }
    }
}

pub struct Context {
    pub device: Retained<ProtocolObject<dyn MTLDevice>>,
    pub queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
}

impl Context {
    pub fn new() -> Result<Self, String> {
        let device = MTLCreateSystemDefaultDevice().ok_or("Metal device unavailable")?;
        let queue = device
            .newCommandQueue()
            .ok_or("Could not create Metal compute queue")?;
        Ok(Self { device, queue })
    }

    pub fn upload(&self, bytes: &[u8]) -> Result<Buffer, String> {
        if bytes.is_empty() {
            return Err("Cannot upload an empty Metal buffer".into());
        }
        // Metal copies the borrowed bytes before returning.
        let buffer = unsafe {
            self.device.newBufferWithBytes_length_options(
                NonNull::new(bytes.as_ptr().cast_mut().cast()).expect("slice pointer"),
                bytes.len(),
                MTLResourceOptions::StorageModeShared,
            )
        }
        .ok_or("Could not allocate Metal input buffer")?;
        Ok(Buffer(buffer))
    }

    pub fn allocate(&self, length: usize) -> Result<Buffer, String> {
        if length == 0 {
            return Err("Cannot allocate an empty Metal buffer".into());
        }
        self.device
            .newBufferWithLength_options(length, MTLResourceOptions::StorageModeShared)
            .map(Buffer)
            .ok_or_else(|| "Could not allocate Metal output buffer".into())
    }
}
