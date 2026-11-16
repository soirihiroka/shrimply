//! Actual-example setup shared by manual voxel render benchmarks.
use sanim_lang::{Engine, FileSources, Prepared};
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Instant,
};

pub fn valley() -> (tempfile::TempDir, Prepared) {
    let temporary = tempfile::tempdir().unwrap();
    let file = if let Some(source) = std::env::var_os("SANIM_VOXEL_PROFILE_SOURCE") {
        PathBuf::from(source)
    } else {
        let generator = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/voxel_pack.py");
        assert!(
            Command::new("uv")
                .args(["run", "--no-project", "python"])
                .arg(generator)
                .arg(temporary.path())
                .status()
                .unwrap()
                .success()
        );
        temporary.path().join("voxel_valley.sanim")
    };
    let sources = FileSources::new(file.parent().unwrap()).unwrap();
    let start = Instant::now();
    let prepared = Engine::default()
        .prepare(
            file.file_name().unwrap().to_str().unwrap(),
            &sources,
            &Default::default(),
        )
        .unwrap();
    eprintln!("actual-valley prepare_us={}", start.elapsed().as_micros());
    (temporary, prepared)
}
