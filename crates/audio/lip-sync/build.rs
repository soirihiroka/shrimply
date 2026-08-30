use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(unix)]
use std::os::unix::fs::symlink;

const ORIGINAL_MODEL_DIRECTORY: &str = r#"const path& getSphinxModelDirectory() {
	static path sphinxModelDirectory(getBinDirectory() / "res" / "sphinx");
	return sphinxModelDirectory;
}"#;

const EMBEDDED_MODEL_DIRECTORY: &str = r#"const path& shrimplySphinxModelDirectory();

const path& getSphinxModelDirectory() {
	return shrimplySphinxModelDirectory();
}"#;

fn run(command: &mut Command) {
    let description = format!("{command:?}");
    let status = command
        .status()
        .unwrap_or_else(|error| panic!("failed to run {description}: {error}"));
    assert!(status.success(), "{description} exited with {status}");
}

fn write_patched_recognizer(source: &Path, destination: &Path) {
    let source = fs::read_to_string(source)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", source.display()));
    assert!(
        source.contains(ORIGINAL_MODEL_DIRECTORY),
        "Rhubarb model-directory code changed; update the lip-sync build integration"
    );
    fs::write(
        destination,
        source.replacen(ORIGINAL_MODEL_DIRECTORY, EMBEDDED_MODEL_DIRECTORY, 1),
    )
    .unwrap_or_else(|error| panic!("failed to write {}: {error}", destination.display()));
}

#[cfg(unix)]
fn stage_models(rhubarb: &Path, output: &Path) {
    fs::create_dir_all(output)
        .unwrap_or_else(|error| panic!("failed to create {}: {error}", output.display()));
    let language = rhubarb.join("rhubarb/lib/pocketsphinx-rev13216/model/en-us");
    for entry in fs::read_dir(&language)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", language.display()))
    {
        let entry = entry.expect("failed to read Rhubarb model entry");
        let destination = output.join(entry.file_name());
        if !destination.exists() {
            symlink(entry.path(), &destination).unwrap_or_else(|error| {
                panic!("failed to link {}: {error}", destination.display())
            });
        }
    }
    let acoustic_model = output.join("acoustic-model");
    if !acoustic_model.exists() {
        symlink(
            rhubarb.join("rhubarb/lib/cmusphinx-en-us-5.2"),
            &acoustic_model,
        )
        .unwrap_or_else(|error| panic!("failed to link {}: {error}", acoustic_model.display()));
    }
}

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("manifest path"));
    let rhubarb = manifest.join("../../../external/rhubarb-lip-sync");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("build output path"));
    let native = out.join("native");
    let models = out.join("res/sphinx");
    let patched_recognizer = out.join("pocketSphinxTools.cpp");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        println!("cargo:warning=Rhubarb lip-sync native bridge is only built for Linux; lip-sync is disabled on this target");
        return;
    }

    assert!(
        rhubarb.join("rhubarb/CMakeLists.txt").is_file(),
        "Rhubarb submodule is missing; run `git submodule update --init --recursive`"
    );
    write_patched_recognizer(
        &rhubarb.join("rhubarb/src/recognition/pocketSphinxTools.cpp"),
        &patched_recognizer,
    );
    #[cfg(unix)]
    stage_models(&rhubarb, &models);

    let mut configure = Command::new("cmake");
    configure
        .arg("-S")
        .arg(manifest.join("native"))
        .arg("-B")
        .arg(&native)
        .arg(format!("-DRHUBARB_SOURCE={}", rhubarb.display()))
        .arg(format!(
            "-DPATCHED_POCKET_SPHINX_TOOLS={}",
            patched_recognizer.display()
        ))
        .arg("-DCMAKE_BUILD_TYPE=Release");
    run(&mut configure);
    run(Command::new("cmake")
        .arg("--build")
        .arg(&native)
        .arg("--target")
        .arg("shrimply-rhubarb")
        .arg("--parallel")
        .arg(
            std::thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(1)
                .to_string(),
        ));

    let library = native.join("libshrimply-rhubarb.so");
    let profile = out
        .ancestors()
        .nth(3)
        .expect("Cargo build output must be inside a target profile directory");
    fs::copy(&library, profile.join("libshrimply-rhubarb.so")).unwrap_or_else(|error| {
        panic!(
            "failed to stage {} in {}: {error}",
            library.display(),
            profile.display()
        )
    });
    println!(
        "cargo:rustc-env=SHRIMPLY_BUILD_RHUBARB_LIBRARY={}",
        library.display()
    );
    println!(
        "cargo:rustc-env=SHRIMPLY_BUILD_RHUBARB_RESOURCES={}",
        models.display()
    );
    println!("cargo:rerun-if-changed=../../../external/rhubarb-lip-sync/rhubarb");
    println!("cargo:rerun-if-changed=native/CMakeLists.txt");
    println!("cargo:rerun-if-changed=native/shim.cpp");
    println!("cargo:rerun-if-changed=native/shim.h");
    println!("cargo:rerun-if-changed=native/gtest/CMakeLists.txt");
    println!("cargo:rerun-if-changed=native/gtest/dummy.cpp");
}
