#[cfg(target_os = "linux")]
use std::{env, fs, path::PathBuf, process::Command};

#[cfg(target_os = "linux")]
use shrimply_slang_build::{Compiler, Target};

#[cfg(target_os = "linux")]
const CUDA_TARGET: &str = "sm_86";
#[cfg(target_os = "linux")]
const MODULES: &str = include_str!("../render-core/shaders/kernels.txt");

#[cfg(not(target_os = "linux"))]
fn main() {}

#[cfg(target_os = "linux")]
fn main() {
    for variable in [
        "CUDA_TARGET",
        "CUDA_HOST_CXX",
        "CUDA_HOME",
        "CUDA_TOOLKIT_PATH",
    ] {
        println!("cargo:rerun-if-env-changed={variable}");
    }
    let target = env::var("CUDA_TARGET").unwrap_or_else(|_| CUDA_TARGET.to_owned());
    assert_eq!(target, CUDA_TARGET, "unsupported CUDA kernel target");
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let shaders = manifest.join("../render-core/shaders");
    let output = manifest
        .join("../../.slang-artifacts/cuda")
        .join(CUDA_TARGET);
    fs::create_dir_all(&output).expect("create CUDA artifact directory");
    let compiler = Compiler::new(&shaders, &output);
    let toolkit = env::var_os("CUDA_TOOLKIT_PATH")
        .or_else(|| env::var_os("CUDA_HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/usr/local/cuda"));
    let host = env::var("CUDA_HOST_CXX").unwrap_or_else(|_| "g++-15".to_owned());
    let mut bindings = String::new();
    for module in MODULES.lines() {
        let source = shaders.join(format!("{module}.slang"));
        let artifact = compiler.compile(&source, Target::Cuda, &[]);
        let image = output.join(format!("{module}.cubin"));
        let status = Command::new(toolkit.join("bin/nvcc"))
            .arg(format!("--compiler-bindir={host}"))
            .args(["--cubin", "-O2", "-w"])
            .arg(format!("--gpu-architecture={CUDA_TARGET}"))
            .arg(output.join(artifact.filename))
            .arg("-o")
            .arg(&image)
            .status()
            .expect("compile generated CUDA source with NVCC");
        assert!(
            status.success(),
            "compile CUDA kernel module {module}: {status}"
        );
        bindings.push_str(&format!(
            "pub const {}: &[u8] = include_bytes!({:?});\n",
            module.to_uppercase(),
            image
        ));
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("CUDA build output"));
    fs::write(out.join("kernels.rs"), bindings).expect("write CUDA kernel bindings");
}
