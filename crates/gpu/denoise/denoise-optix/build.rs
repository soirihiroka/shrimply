use std::env;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/bridge.c");
    println!("cargo:rerun-if-env-changed=OPTIX_ROOT");
    println!("cargo:rerun-if-env-changed=CUDA_HOME");
    println!("cargo:rerun-if-env-changed=CUDA_TOOLKIT_PATH");
    println!("cargo:rerun-if-env-changed=CUDA_PATH");

    let optix = env::var_os("OPTIX_ROOT")
        .map(PathBuf::from)
        .expect("OPTIX_ROOT must point to an NVIDIA optix-dev checkout");
    let optix_include = optix.join("include");
    require_header(&optix_include, "optix.h", "OPTIX_ROOT");

    let cuda = env::var_os("CUDA_HOME")
        .or_else(|| env::var_os("CUDA_TOOLKIT_PATH"))
        .or_else(|| env::var_os("CUDA_PATH"))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            #[cfg(target_os = "linux")]
            return PathBuf::from("/usr/local/cuda");
            #[cfg(windows)]
            panic!("CUDA_PATH, CUDA_HOME, or CUDA_TOOLKIT_PATH must locate the CUDA toolkit");
        });
    let cuda_include = cuda.join("include");
    require_header(&cuda_include, "cuda.h", "CUDA_HOME");

    println!(
        "building OptiX 9 denoiser bridge from {}",
        optix_include.display()
    );
    cc::Build::new()
        .file("src/bridge.c")
        .include(optix_include)
        .include(cuda_include)
        .warnings(true)
        .extra_warnings(true)
        .compile("shrimply_optix_denoiser_bridge");
    println!("cargo:rustc-link-lib=dylib=cuda");
    #[cfg(windows)]
    println!(
        "cargo:rustc-link-search=native={}",
        cuda.join("lib/x64").display()
    );
    #[cfg(unix)]
    println!("cargo:rustc-link-lib=dylib=dl");
}

fn require_header(directory: &Path, name: &str, variable: &str) {
    let header = directory.join(name);
    assert!(
        header.is_file(),
        "{variable} does not contain include/{name}: {}",
        header.display()
    );
}
