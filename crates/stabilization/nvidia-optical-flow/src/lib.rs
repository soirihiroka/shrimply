use std::ffi::c_void;
use std::ptr::NonNull;

#[cfg(target_os = "linux")]
use std::ffi::{CStr, c_char, c_int};

use cuda_core::{CudaContext, CudaStream, sys};

const ERROR_CAPACITY: usize = 512;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Quality {
    Quality = 5,
    Balanced = 10,
    Fast = 20,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum OutputGrid {
    OneByOne = 1,
    TwoByTwo = 2,
    FourByFour = 4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Settings {
    pub quality: Quality,
    pub output_grid: OutputGrid,
    pub temporal_hints: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            quality: Quality::Quality,
            output_grid: OutputGrid::TwoByTwo,
            temporal_hints: true,
        }
    }
}

#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct FlowVector {
    pub x: i16,
    pub y: i16,
}

pub struct FlowField {
    pub forward: Vec<FlowVector>,
    pub backward: Vec<FlowVector>,
    pub forward_cost: Vec<u8>,
    pub backward_cost: Vec<u8>,
    pub width: usize,
    pub height: usize,
    pub grid_size: u32,
}

pub struct OpticalFlow {
    raw: NonNull<c_void>,
    width: u32,
    height: u32,
    settings: Settings,
}

// SAFETY: the driver session is only accessed through `&mut self`, and every bridge call binds
// the CUDA context stored by the session before touching its stream or buffers.
unsafe impl Send for OpticalFlow {}

impl OpticalFlow {
    pub fn new(
        context: &CudaContext,
        stream: &CudaStream,
        width: u32,
        height: u32,
        settings: Settings,
    ) -> Result<Self, String> {
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (context, stream, width, height, settings);
            return Err("NVIDIA optical flow requires Linux with an NVIDIA CUDA driver".to_string());
        }
        #[cfg(target_os = "linux")]
        {
            context
                .bind_to_thread()
                .map_err(|error| format!("bind CUDA context for NVIDIA optical flow: {error:?}"))?;
            let mut error = [0; ERROR_CAPACITY];
            let raw = unsafe {
                shrimply_nvof_create(
                    context.cu_ctx(),
                    stream.cu_stream(),
                    width,
                    height,
                    settings.quality as u32,
                    settings.output_grid as u32,
                    error.as_mut_ptr(),
                    error.len(),
                )
            };
            Ok(Self {
                raw: NonNull::new(raw).ok_or_else(|| bridge_error(&error))?,
                width,
                height,
                settings,
            })
        }
    }

    pub fn matches(&self, width: u32, height: u32, settings: Settings) -> bool {
        self.width == width && self.height == height && self.settings == settings
    }

    pub fn estimate(
        &mut self,
        input: sys::CUdeviceptr,
        reference: sys::CUdeviceptr,
        reset_temporal_hints: bool,
    ) -> Result<FlowField, String> {
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (self, input, reference, reset_temporal_hints);
            return Err("NVIDIA optical flow requires Linux with an NVIDIA CUDA driver".to_string());
        }
        #[cfg(target_os = "linux")]
        {
            let grid_size = self.settings.output_grid as u32;
            let width = self.width.div_ceil(grid_size) as usize;
            let height = self.height.div_ceil(grid_size) as usize;
            let len = width
                .checked_mul(height)
                .ok_or_else(|| "NVIDIA optical flow dimensions overflow".to_string())?;
            let mut field = FlowField {
                forward: vec![FlowVector::default(); len],
                backward: vec![FlowVector::default(); len],
                forward_cost: vec![0; len],
                backward_cost: vec![0; len],
                width,
                height,
                grid_size,
            };
            let mut error = [0; ERROR_CAPACITY];
            let result = unsafe {
                shrimply_nvof_estimate(
                    self.raw.as_ptr(),
                    input,
                    reference,
                    c_int::from(self.settings.temporal_hints),
                    c_int::from(reset_temporal_hints),
                    field.forward.as_mut_ptr(),
                    field.backward.as_mut_ptr(),
                    field.forward_cost.as_mut_ptr(),
                    field.backward_cost.as_mut_ptr(),
                    error.as_mut_ptr(),
                    error.len(),
                )
            };
            if result == 0 {
                Ok(field)
            } else {
                Err(bridge_error(&error))
            }
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for OpticalFlow {
    fn drop(&mut self) {
        unsafe { shrimply_nvof_destroy(self.raw.as_ptr()) };
    }
}

#[cfg(target_os = "linux")]
fn bridge_error(error: &[c_char]) -> String {
    unsafe { CStr::from_ptr(error.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn shrimply_nvof_create(
        context: sys::CUcontext,
        stream: sys::CUstream,
        width: u32,
        height: u32,
        quality: u32,
        output_grid: u32,
        error: *mut c_char,
        error_size: usize,
    ) -> *mut c_void;

    fn shrimply_nvof_estimate(
        context: *mut c_void,
        input: sys::CUdeviceptr,
        reference: sys::CUdeviceptr,
        use_temporal_hints: c_int,
        disable_temporal_hints: c_int,
        forward: *mut FlowVector,
        backward: *mut FlowVector,
        forward_cost: *mut u8,
        backward_cost: *mut u8,
        error: *mut c_char,
        error_size: usize,
    ) -> c_int;

    fn shrimply_nvof_destroy(context: *mut c_void);
}
