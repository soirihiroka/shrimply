use std::{env, fs, path::PathBuf, process::Command};

const VERSION: &str = "2026.17";

fn main() {
    for variable in ["SLANG_LIBRARY_DIR", "SLANG_INCLUDE_DIR"] {
        println!("cargo:rerun-if-env-changed={variable}");
    }
    println!("cargo:rerun-if-changed=compiler.cpp");
    let (library_dir, include_dir) = match (
        env::var_os("SLANG_LIBRARY_DIR"),
        env::var_os("SLANG_INCLUDE_DIR"),
    ) {
        (Some(library), Some(include)) => (PathBuf::from(library), PathBuf::from(include)),
        (None, None) => {
            let (platform, checksum) = match (env::consts::OS, env::consts::ARCH) {
                ("macos", "aarch64") => (
                    "macos-aarch64",
                    "9c374eccbf268768d68c1374e3519771c4d4a0992a2680e901bee72d5f000dd5",
                ),
                ("macos", "x86_64") => (
                    "macos-x86_64",
                    "ce3d677aefc69aa693cd3a0cf0a2212fdfcf4f7db11a6698d51f4d4f7397bf56",
                ),
                ("linux", "aarch64") => (
                    "linux-aarch64-glibc-2.28",
                    "1873b032fd9e44fa91cba567287304b8886e3e01a4ebe4f26dd20af5f8050582",
                ),
                ("linux", "x86_64") => (
                    "linux-x86_64-glibc-2.28",
                    "a5a48530e7218d79e10b633c216ef04cbe778450b8c0a7579125e630c088ca75",
                ),
                ("windows", "x86_64") => (
                    "windows-x86_64",
                    "9ef90a87d6836d88f83ddaa0bd7fded28a36a3d994bccd8f4634e999f17461c5",
                ),
                host => panic!("no prebuilt Slang release for {host:?}"),
            };
            let output = PathBuf::from(env::var_os("OUT_DIR").expect("Slang build output"));
            // Share the download across crate fingerprints within this Cargo profile.
            let profile = output.ancestors().nth(3).expect("Cargo profile directory");
            let cache = profile.join(format!("slang-{VERSION}-{platform}"));
            let lock = fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(profile.join("slang-download.lock"))
                .expect("open Slang download lock");
            lock.lock().expect("lock Slang download");
            let marker = cache.join(".shrimply-prebuilt");
            if fs::read_to_string(&marker).ok().as_deref() != Some(checksum) {
                let staging = profile.join(format!("slang-{VERSION}-{platform}.tmp"));
                if staging.exists() {
                    fs::remove_dir_all(&staging).expect("remove interrupted Slang download");
                }
                fs::create_dir(&staging).expect("create Slang download directory");
                let archive = staging.join("slang.tar.gz");
                println!("cargo:warning=Downloading Slang {VERSION} for {platform}");
                let status = Command::new("curl")
                    .args(["--fail", "--location", "--silent", "--show-error", "--retry", "3"])
                    .arg(format!("https://github.com/shader-slang/slang/releases/download/v{VERSION}/slang-{VERSION}-{platform}.tar.gz"))
                    .arg("--output")
                    .arg(&archive)
                    .status()
                    .expect("download Slang with curl");
                assert!(status.success(), "download Slang: {status}");
                let mut hash = if cfg!(target_os = "macos") {
                    let mut command = Command::new("shasum");
                    command.args(["-a", "256"]);
                    command
                } else {
                    Command::new("sha256sum")
                };
                let hash = hash.arg(&archive).output().expect("hash Slang archive");
                assert!(hash.status.success(), "hash Slang archive: {}", hash.status);
                assert_eq!(
                    String::from_utf8_lossy(&hash.stdout)
                        .split_whitespace()
                        .next(),
                    Some(checksum),
                    "Slang archive checksum mismatch"
                );
                let status = Command::new("tar")
                    .arg("-xzf")
                    .arg(&archive)
                    .arg("-C")
                    .arg(&staging)
                    .status()
                    .expect("extract Slang archive");
                assert!(status.success(), "extract Slang archive: {status}");
                let runtime = if cfg!(windows) {
                    staging.join("bin/slang.dll")
                } else {
                    staging.join(format!("lib/libslang.{}", env::consts::DLL_EXTENSION))
                };
                assert!(runtime.is_file(), "Slang archive is missing its library");
                assert!(
                    staging.join("include/slang.h").is_file(),
                    "Slang archive is missing its headers"
                );
                fs::remove_file(archive).expect("remove extracted Slang archive");
                fs::write(staging.join(".shrimply-prebuilt"), checksum)
                    .expect("mark verified Slang cache");
                if cache.exists() {
                    fs::remove_dir_all(&cache).expect("remove outdated Slang cache");
                }
                fs::rename(staging, &cache).expect("publish verified Slang cache");
            }
            (cache.join("lib"), cache.join("include"))
        }
        _ => panic!(
            "set both SLANG_LIBRARY_DIR and SLANG_INCLUDE_DIR to use an existing Slang distribution"
        ),
    };
    let runtime_dir = if cfg!(windows) {
        library_dir
            .parent()
            .expect("Slang library directory has an archive root")
            .join("bin")
    } else {
        library_dir.clone()
    };
    let library = if cfg!(windows) {
        runtime_dir.join("slang.dll")
    } else {
        library_dir.join(format!("libslang.{}", env::consts::DLL_EXTENSION))
    };
    assert!(
        library.is_file(),
        "missing prebuilt Slang library: {}",
        library.display()
    );
    assert!(
        include_dir.join("slang.h").is_file(),
        "missing prebuilt Slang headers: {}",
        include_dir.display()
    );
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Slang build output"));
    let library_dir = library_dir
        .canonicalize()
        .expect("resolve Slang library directory");
    let runtime_dir = runtime_dir
        .canonicalize()
        .expect("resolve Slang runtime directory");
    println!("cargo:rerun-if-changed={}", library_dir.display());
    println!("cargo:rerun-if-changed={}", include_dir.display());
    println!(
        "cargo:rustc-env=SHRIMPLY_SLANG_LIBRARY_DIR={}",
        runtime_dir.display()
    );
    let bridge = output.join(format!(
        "libshrimply_slang_api.{}",
        env::consts::DLL_EXTENSION
    ));
    let compiler = cc::Build::new()
        .cpp(true)
        .std("c++17")
        .pic(true)
        .cargo_metadata(false)
        .get_compiler();
    let mut command = compiler.to_command();
    if compiler.is_like_msvc() {
        command
            .arg("/LD")
            .arg("/EHsc")
            .arg("compiler.cpp")
            .arg(format!("/I{}", include_dir.display()))
            .arg("/link")
            .arg(format!("/LIBPATH:{}", library_dir.display()))
            .arg("slang.lib")
            .arg(format!("/OUT:{}", bridge.display()));
    } else {
        command
            .arg(if cfg!(target_os = "macos") {
                "-dynamiclib"
            } else {
                "-shared"
            })
            .arg("compiler.cpp")
            .arg("-I")
            .arg(include_dir)
            .arg("-L")
            .arg(&library_dir)
            .arg(format!("-Wl,-rpath,{}", library_dir.display()))
            .arg("-lslang")
            .arg("-o")
            .arg(&bridge);
    }
    let status = command.status().expect("build Slang C++ API bridge");
    assert!(status.success(), "build Slang C++ API bridge: {status}");
    println!("cargo:rustc-env=SHRIMPLY_SLANG_API={}", bridge.display());
}
