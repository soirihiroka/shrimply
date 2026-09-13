use std::env;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=CUDA_HOME");
    println!("cargo:rerun-if-env-changed=CUDA_PATH");

    let cuda_include = env::var_os("CUDA_HOME")
        .or_else(|| env::var_os("CUDA_TOOLKIT_PATH"))
        .or_else(|| env::var_os("CUDA_PATH"))
        .map(PathBuf::from)
        .map(|path| path.join("include"))
        .filter(|path| path.join("cuda.h").is_file())
        .unwrap_or_else(|| Path::new("/usr/local/cuda/include").to_path_buf());

    cc::Build::new()
        .cpp(true)
        .file("src/bridge.cpp")
        .include("include")
        .include(cuda_include)
        .flag_if_supported("-std=c++17")
        .warnings(true)
        .compile("shrimply_nvidia_optical_flow_bridge");

    println!("cargo:rustc-link-lib=dylib=cuda");
    #[cfg(windows)]
    if let Some(cuda) = env::var_os("CUDA_HOME")
        .or_else(|| env::var_os("CUDA_TOOLKIT_PATH"))
        .or_else(|| env::var_os("CUDA_PATH"))
    {
        println!(
            "cargo:rustc-link-search=native={}",
            PathBuf::from(cuda).join("lib/x64").display()
        );
    }
    #[cfg(unix)]
    println!("cargo:rustc-link-lib=dylib=dl");
    println!("cargo:rerun-if-changed=src/bridge.cpp");
    println!("cargo:rerun-if-changed=include/nvOpticalFlowCommon.h");
    println!("cargo:rerun-if-changed=include/nvOpticalFlowCuda.h");
}
