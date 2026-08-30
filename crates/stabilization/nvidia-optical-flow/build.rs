use std::env;
use std::path::Path;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=CUDA_HOME");
    println!("cargo:rerun-if-env-changed=CUDA_PATH");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        println!("cargo:warning=NVIDIA optical flow bridge is only built for Linux; CUDA acceleration is disabled on this target");
        return;
    }

    let cuda_include = env::var_os("CUDA_HOME")
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
    println!("cargo:rustc-link-lib=dylib=dl");
    println!("cargo:rerun-if-changed=src/bridge.cpp");
    println!("cargo:rerun-if-changed=include/nvOpticalFlowCommon.h");
    println!("cargo:rerun-if-changed=include/nvOpticalFlowCuda.h");
}
