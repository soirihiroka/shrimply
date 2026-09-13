#[cfg(any(target_os = "linux", windows))]
use std::{env, path::PathBuf};

#[cfg(target_os = "macos")]
fn main() {}

#[cfg(any(target_os = "linux", windows))]
fn main() {
    println!("cargo:rerun-if-changed=bridge.c");
    println!("cargo:rerun-if-env-changed=CUDA_HOME");
    println!("cargo:rerun-if-env-changed=CUDA_TOOLKIT_PATH");
    let cuda = env::var_os("CUDA_TOOLKIT_PATH")
        .or_else(|| env::var_os("CUDA_HOME"))
        .or_else(|| env::var_os("CUDA_PATH"))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            #[cfg(target_os = "linux")]
            return PathBuf::from("/usr/local/cuda");
            #[cfg(windows)]
            panic!("CUDA_PATH, CUDA_HOME, or CUDA_TOOLKIT_PATH must locate the CUDA toolkit");
        });
    cc::Build::new()
        .file("bridge.c")
        .include(cuda.join("include"))
        .compile("shrimply_cuda_bridge");
    #[cfg(windows)]
    println!(
        "cargo:rustc-link-search=native={}",
        cuda.join("lib/x64").display()
    );
    println!("cargo:rustc-link-lib=dylib=cuda");
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn main() {}
