use std::env;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=native/nvdec_scrub.c");
    println!("cargo:rerun-if-env-changed=FFMPEG_DIR");
    println!("cargo:rerun-if-env-changed=SHRIMPLY_FFMPEG_PREFIX");
    println!("cargo:rerun-if-env-changed=FFNVCODEC_INCLUDE");

    if env::var_os("CARGO_FEATURE_CUDA").is_none() {
        return;
    }

    let mut build = cc::Build::new();
    build
        .file("native/nvdec_scrub.c")
        .warnings(true)
        .extra_warnings(true);

    if let Some(ffmpeg_dir) = env::var_os("FFMPEG_DIR")
        .or_else(|| env::var_os("SHRIMPLY_FFMPEG_PREFIX"))
        .map(PathBuf::from)
    {
        let include = ffmpeg_dir.join("include");
        if include.is_dir() {
            build.include(include);
        }
    }

    if let Some(custom) = env::var_os("FFNVCODEC_INCLUDE").map(PathBuf::from)
        && custom.is_dir()
    {
        build.include(custom);
    } else if let Some(pkg_inc) = std::process::Command::new("pkg-config")
        .args(["--variable=includedir", "ffnvcodec"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| PathBuf::from(s.trim()))
        .filter(|p| p.join("ffnvcodec").is_dir())
    {
        build.include(pkg_inc);
    } else if let Some(fallback) = ["/usr/include", "/usr/local/include"]
        .into_iter()
        .find(|dir| Path::new(dir).join("ffnvcodec").is_dir())
    {
        build.include(fallback);
    }

    build.compile("shrimply_nvdec_scrub");

    if env::var_os("CARGO_CFG_UNIX").is_some() {
        println!("cargo:rustc-link-lib=dylib=dl");
    }
}
