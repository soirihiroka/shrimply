use std::env;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/bridge.c");
    println!("cargo:rerun-if-env-changed=OPTIX_ROOT");
    println!("cargo:rerun-if-env-changed=CUDA_HOME");
    println!("cargo:rerun-if-env-changed=CUDA_TOOLKIT_PATH");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        println!("cargo:warning=OptiX denoiser bridge is only built for Linux; OptiX acceleration is disabled on this target");
        return;
    }

    let optix_root = env::var_os("OPTIX_ROOT")
        .expect("OPTIX_ROOT must point to an NVIDIA optix-dev checkout");
    let optix = std::path::PathBuf::from(optix_root);
    let optix_include = optix.join("include");
    require_header(&optix_include, "optix.h", "OPTIX_ROOT");

    let cuda = env::var_os("CUDA_HOME")
        .or_else(|| env::var_os("CUDA_TOOLKIT_PATH"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/usr/local/cuda"));
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
    println!("cargo:rustc-link-lib=dylib=dl");
}

fn require_header(directory: &std::path::Path, name: &str, variable: &str) {
    let header = directory.join(name);
    assert!(
        header.is_file(),
        "{variable} does not contain include/{name}: {}",
        header.display()
    );
}
