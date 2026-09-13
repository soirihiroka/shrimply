//! CUDA kernel images compiled from the shared Slang shaders.
#![cfg(any(target_os = "linux", windows))]
include!(concat!(env!("OUT_DIR"), "/kernels.rs"));
