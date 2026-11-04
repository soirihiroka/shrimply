use std::{env, path::Path, process::Command};

fn git(arguments: &[&str]) -> Option<String> {
    let output = Command::new("git").args(arguments).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn main() {
    for name in ["SHRIMPLY_BUILD_GIT_HASH", "GITHUB_SHA"] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    for path in [
        Some("HEAD".to_owned()),
        git(&["symbolic-ref", "-q", "HEAD"]),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(path) = git(&["rev-parse", "--git-path", &path]) {
            if let Ok(path) = Path::new(&path).canonicalize() {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }
    let hash = env::var("SHRIMPLY_BUILD_GIT_HASH")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| git(&["rev-parse", "HEAD"]))
        .or_else(|| env::var("GITHUB_SHA").ok());
    if let Some(hash) = hash {
        let hash = hash.strip_suffix("-dirty").unwrap_or(&hash);
        assert!(
            hash.len() == 40 && hash.bytes().all(|value| value.is_ascii_hexdigit()),
            "Build Git hash must be a full commit SHA"
        );
        println!("cargo:rustc-env=SHRIMPLY_BUILD_GIT_HASH={hash}");
    }
}
