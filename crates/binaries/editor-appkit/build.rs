use std::{env, path::PathBuf, process::Command};

fn xcrun(args: &[&str]) -> String {
    let output = Command::new("xcrun")
        .args(args)
        .output()
        .expect("run xcrun");
    assert!(
        output.status.success(),
        "xcrun {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("Xcode path")
        .trim()
        .to_owned()
}

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    const SOURCE: &str = "src/macos/settings_form.swift";
    println!("cargo:rerun-if-changed={SOURCE}");
    println!("cargo:rerun-if-env-changed=DEVELOPER_DIR");
    println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");
    let sdk = PathBuf::from(xcrun(&["--sdk", "macosx", "--show-sdk-path"]));
    let swift = PathBuf::from(xcrun(&["--find", "swiftc"]));
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok("x86_64") => "x86_64",
        other => panic!("unsupported macOS architecture: {other:?}"),
    };
    let version = env::var("MACOSX_DEPLOYMENT_TARGET").unwrap_or_else(|_| "15.0".to_owned());
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory"));
    let built = Command::new(&swift)
        .args([
            "-parse-as-library",
            "-emit-library",
            "-static",
            "-O",
            "-module-name",
            "ShrimplySettings",
            "-target",
        ])
        .arg(format!("{arch}-apple-macosx{version}"))
        .arg("-sdk")
        .arg(&sdk)
        .arg(SOURCE)
        .arg("-o")
        .arg(out.join("libshrimply_settings.a"))
        .status()
        .expect("run swiftc");
    assert!(built.success(), "SwiftUI Settings form failed to compile");

    let toolchain = swift
        .parent()
        .and_then(|parent| parent.parent())
        .expect("Xcode toolchain");
    for path in [
        &out,
        &sdk.join("usr/lib/swift"),
        &toolchain.join("lib/swift/macosx"),
    ] {
        println!("cargo:rustc-link-search=native={}", path.display());
    }
    println!("cargo:rustc-link-lib=static=shrimply_settings");
    println!("cargo:rustc-link-lib=framework=SwiftUI");
    println!("cargo:rustc-link-lib=dylib=swiftCore");
}
