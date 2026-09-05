#[cfg(not(target_os = "macos"))]
use std::{env, path::PathBuf};

#[cfg(target_os = "macos")]
fn main() {}

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("cargo:rerun-if-changed=bridge.c");
    println!("cargo:rerun-if-env-changed=CUDA_HOME");
    println!("cargo:rerun-if-env-changed=CUDA_TOOLKIT_PATH");
    let cuda = env::var_os("CUDA_TOOLKIT_PATH")
        .or_else(|| env::var_os("CUDA_HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/usr/local/cuda"));
    cc::Build::new()
        .file("bridge.c")
        .include(cuda.join("include"))
        .compile("shrimply_cuda_bridge");
    println!("cargo:rustc-link-lib=dylib=cuda");
}
