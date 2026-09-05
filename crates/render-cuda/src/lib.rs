//! CUDA kernel images compiled from the shared Slang shaders.
#![cfg(target_os = "linux")]
include!(concat!(env!("OUT_DIR"), "/kernels.rs"));
