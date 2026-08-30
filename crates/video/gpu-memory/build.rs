use std::env;
use std::path::{Path, PathBuf};

const TOOLKIT_ENV_VARS: &[&str] = &["CUDA_TOOLKIT_PATH", "CUDA_HOME"];
const TOOLKIT_TARGET_DIR_ENV: &str = "CUDA_TOOLKIT_TARGET_DIR";

fn main() {
    println!("cargo::rustc-check-cfg=cfg(cuda_uses_mem_location)");
    for variable in TOOLKIT_ENV_VARS {
        println!("cargo:rerun-if-env-changed={variable}");
    }
    println!("cargo:rerun-if-env-changed={TOOLKIT_TARGET_DIR_ENV}");
    let Some(header) = cuda_header() else {
        println!("cargo:warning=CUDA toolkit not found; building gpu-memory with legacy allocation cfg (CUDA acceleration disabled)");
        return;
    };
    println!("cargo:rerun-if-changed={}", header.display());
    let source = std::fs::read_to_string(&header)
        .unwrap_or_else(|error| panic!("read CUDA header {}: {error}", header.display()));
    let version = source
        .lines()
        .find_map(|line| {
            let mut words = line.split_whitespace();
            (words.next() == Some("#define") && words.next() == Some("CUDA_VERSION"))
                .then(|| words.next()?.parse::<u32>().ok())
                .flatten()
        })
        .unwrap_or_else(|| panic!("CUDA_VERSION is missing from {}", header.display()));
    if version >= 13_020 {
        println!("cargo:rustc-cfg=cuda_uses_mem_location");
    }
}

fn cuda_header() -> Option<PathBuf> {
    let toolkit = TOOLKIT_ENV_VARS
        .iter()
        .find_map(|variable| env::var(variable).ok())
        .unwrap_or_else(|| "/usr/local/cuda".to_string());
    let base = Path::new(&toolkit);
    let mut include_dirs = vec![base.join("include")];
    for target in toolkit_target_dirs() {
        include_dirs.push(base.join("targets").join(target).join("include"));
    }
    include_dirs
        .into_iter()
        .map(|directory| directory.join("cuda.h"))
        .find(|header| header.is_file())
}

fn toolkit_target_dirs() -> Vec<String> {
    if let Some(target) = env::var(TOOLKIT_TARGET_DIR_ENV)
        .ok()
        .filter(|target| !target.trim().is_empty())
    {
        return vec![target];
    }
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        return Vec::new();
    }
    match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86_64") => vec!["x86_64-linux".to_string()],
        Ok("aarch64") => vec!["sbsa-linux".to_string(), "aarch64-linux".to_string()],
        _ => Vec::new(),
    }
}
