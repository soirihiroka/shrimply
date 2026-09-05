use std::{env, fs, path::PathBuf};

use serde_json::Value;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=shaders");
    println!("cargo:rerun-if-changed=../slang-build/compiler.cpp");
    println!("cargo:rerun-if-env-changed=SLANG_SOURCE_DIR");
    println!("cargo:rerun-if-env-changed=SLANG_BUILD_DIR");

    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let shader_directory = manifest.join("shaders");
    let source = shader_directory.join("reflection.slang");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let compiler = shrimply_slang_build::Compiler::new(&shader_directory, &output);
    let artifacts = compiler.compile(
        &source,
        shrimply_slang_build::Target::Host,
        &["reflect_abi"],
    );
    let reflection: Value = serde_json::from_slice(&artifacts.reflection)
        .unwrap_or_else(|error| panic!("parse compositor host reflection: {error}"));
    fs::write(
        output.join("abi.rs"),
        shrimply_slang_build::generate_abi(&reflection, &artifacts.abi),
    )
    .expect("write reflected compositor host ABI");
}
