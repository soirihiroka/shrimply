use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::Arc;

#[cfg(target_os = "linux")]
use std::ffi::{c_char, c_int};
#[cfg(target_os = "linux")]
use std::ptr;

use cuda_core::{CudaContext, CudaStream};

const ERROR_CAPACITY: usize = 1024;

enum NativeDenoiser {}

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn shrimply_optix_denoiser_create(
        context: *mut c_void,
        stream: *mut c_void,
        width: u32,
        height: u32,
        output: *mut *mut NativeDenoiser,
        error: *mut c_char,
        error_capacity: usize,
    ) -> c_int;
    fn shrimply_optix_denoiser_invoke(
        denoiser: *mut NativeDenoiser,
        stream: *mut c_void,
        beauty: u64,
        refraction: u64,
        albedo: u64,
        normal: u64,
        error: *mut c_char,
        error_capacity: usize,
    ) -> c_int;
    fn shrimply_optix_denoiser_destroy(
        denoiser: *mut NativeDenoiser,
        error: *mut c_char,
        error_capacity: usize,
    ) -> c_int;
}

pub struct OptixDenoiser {
    native: NonNull<NativeDenoiser>,
    context: Arc<CudaContext>,
    width: u32,
    height: u32,
}

pub struct DenoiseInputs {
    pub beauty: u64,
    pub refraction: u64,
    pub albedo: u64,
    pub normal: u64,
}

impl OptixDenoiser {
    pub fn new(
        context: Arc<CudaContext>,
        stream: &CudaStream,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (context, stream, width, height);
            return Err("OptiX denoiser requires Linux with an NVIDIA CUDA driver".to_string());
        }
        #[cfg(target_os = "linux")]
        {
            if width == 0 || height == 0 {
                return Err("OptiX denoiser dimensions must be nonzero".to_string());
            }
            context
                .bind_to_thread()
                .map_err(|error| format!("bind CUDA context for OptiX: {error:?}"))?;
            let mut native = ptr::null_mut();
            ffi_result(|error, capacity| unsafe {
                shrimply_optix_denoiser_create(
                    context.cu_ctx().cast(),
                    stream.cu_stream().cast(),
                    width,
                    height,
                    &mut native,
                    error,
                    capacity,
                )
            })?;
            Ok(Self {
                native: NonNull::new(native).expect("successful OptiX creation returns a handle"),
                context,
                width,
                height,
            })
        }
    }

    pub fn denoise(&mut self, stream: &CudaStream, inputs: DenoiseInputs) -> Result<(), String> {
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (stream, inputs);
            return Err("OptiX denoiser requires Linux with an NVIDIA CUDA driver".to_string());
        }
        #[cfg(target_os = "linux")]
        {
            self.context
                .bind_to_thread()
                .map_err(|error| format!("bind CUDA context for OptiX: {error:?}"))?;
            ffi_result(|error, capacity| unsafe {
                shrimply_optix_denoiser_invoke(
                    self.native.as_ptr(),
                    stream.cu_stream().cast(),
                    inputs.beauty,
                    inputs.refraction,
                    inputs.albedo,
                    inputs.normal,
                    error,
                    capacity,
                )
            })
        }
    }

    pub fn matches(&self, width: u32, height: u32) -> bool {
        self.width == width && self.height == height
    }
}

#[cfg(target_os = "linux")]
impl Drop for OptixDenoiser {
    fn drop(&mut self) {
        if self.context.bind_to_thread().is_err()
            || ffi_result(|error, capacity| unsafe {
                shrimply_optix_denoiser_destroy(self.native.as_ptr(), error, capacity)
            })
            .is_err()
        {
            std::process::abort();
        }
    }
}

#[cfg(target_os = "linux")]
fn ffi_result(call: impl FnOnce(*mut c_char, usize) -> c_int) -> Result<(), String> {
    let mut error = [0_u8; ERROR_CAPACITY];
    if call(error.as_mut_ptr().cast(), error.len()) == 0 {
        return Ok(());
    }
    let end = error
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(error.len());
    Err(String::from_utf8_lossy(&error[..end]).into_owned())
}
