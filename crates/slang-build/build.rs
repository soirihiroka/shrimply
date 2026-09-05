use std::{
    env,
    fs::{self, OpenOptions},
    path::PathBuf,
    process::Command,
};

const CONFIGURATION: &str = "Release";

fn main() {
    for variable in ["SLANG_SOURCE_DIR", "SLANG_BUILD_DIR"] {
        println!("cargo:rerun-if-env-changed={variable}");
    }
    println!("cargo:rerun-if-changed=compiler.cpp");
    let source = env::var_os("SLANG_SOURCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../external/slang"));
    let build = env::var_os("SLANG_BUILD_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| source.join("build"));
    assert!(
        source.join("CMakeLists.txt").is_file(),
        "missing Slang source: {}",
        source.display()
    );
    fs::create_dir_all(&build).expect("create Slang build directory");
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(build.join("shrimply.lock"))
        .expect("open Slang build lock");
    lock.lock().expect("lock Slang build");
    let library_dir = build.join(CONFIGURATION).join("lib");
    let library = library_dir.join(format!("libslang.{}", env::consts::DLL_EXTENSION));
    if !library.is_file() {
        let configured = Command::new("cmake")
            .arg("-S")
            .arg(&source)
            .arg("-B")
            .arg(&build)
            .args([
                "-G",
                "Ninja Multi-Config",
                "-DSLANG_ENABLE_SLANGC=OFF",
                "-DSLANG_ENABLE_SLANG_RHI=OFF",
                "-DSLANG_ENABLE_GFX=OFF",
                "-DSLANG_ENABLE_TESTS=OFF",
                "-DSLANG_ENABLE_EXAMPLES=OFF",
                "-DSLANG_ENABLE_SLANGD=OFF",
                "-DSLANG_ENABLE_SLANGI=OFF",
                "-DSLANG_ENABLE_SLANGRT=OFF",
                "-DSLANG_ENABLE_SPLIT_DEBUG_INFO=OFF",
                "-DSLANG_ENABLE_SLANG_GLSLANG=ON",
                "-DSLANG_ENABLE_REPLAYER=OFF",
                "-DSLANG_SLANG_LLVM_FLAVOR=DISABLE",
                "-DSLANG_ENABLE_DXIL=OFF",
            ])
            .status()
            .expect("configure Slang library");
        assert!(
            configured.success(),
            "configure Slang library: {configured}"
        );
        let compiled = Command::new("cmake")
            .arg("--build")
            .arg(&build)
            .args([
                "--config",
                CONFIGURATION,
                "--target",
                "slang",
                "slang-glslang",
            ])
            .status()
            .expect("build Slang library");
        assert!(compiled.success(), "build Slang library: {compiled}");
    }
    drop(lock);
    println!("cargo:rerun-if-changed={}", library.display());
    println!(
        "cargo:rerun-if-changed={}",
        source.join("include").display()
    );
    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .file("compiler.cpp")
        .include(source.join("include"))
        .compile("shrimply_slang_api");
    println!("cargo:rustc-link-search=native={}", library_dir.display());
    println!("cargo:rustc-link-lib=dylib=slang");
}
