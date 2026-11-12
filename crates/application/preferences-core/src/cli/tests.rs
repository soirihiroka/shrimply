use super::flatpak_launcher;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "shrimply-cli-install-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let fixture = Self(directory.canonicalize().unwrap());
        fs::write(
            fixture.0.join("flatpak"),
            "#!/bin/sh\nset -eu\nprintf '%s\\0' \"$PWD\" \"$@\" > \"$SHRIMPLY_TEST_ARGUMENTS\"\ncat \"$SHRIMPLY_TEST_DEPLOYMENT\"\nexit \"${SHRIMPLY_TEST_EXIT_CODE:-0}\"\n",
        )
        .unwrap();
        fs::set_permissions(fixture.0.join("flatpak"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(fixture.0.join("deployment"), "first-deployment\n").unwrap();
        fixture
    }
    fn run(&self, launcher: &str, directory: &Path, arguments: &[&str], status: u8) -> Output {
        Command::new("/bin/sh")
            .args(["-c", launcher, "shrimply"])
            .args(arguments)
            .current_dir(directory)
            .env("PATH", format!("{}:/usr/bin:/bin", self.0.display()))
            .env("SHRIMPLY_TEST_ARGUMENTS", self.0.join("arguments"))
            .env("SHRIMPLY_TEST_DEPLOYMENT", self.0.join("deployment"))
            .env("SHRIMPLY_TEST_EXIT_CODE", status.to_string())
            .output()
            .unwrap()
    }
    fn arguments(&self) -> Vec<String> {
        String::from_utf8(fs::read(self.0.join("arguments")).unwrap())
            .unwrap()
            .split_terminator('\0')
            .map(str::to_owned)
            .collect()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn installed_flatpak_launcher_resolves_updated_deployment_without_reinstallation() {
    let fixture = Fixture::new();
    let reference = "dev.shrimply.Shrimply/x86_64/master";
    let launcher = flatpak_launcher("--user", reference);
    let first = fixture.run(&launcher, &fixture.0, &["--help"], 0);
    assert!(first.status.success());
    assert_eq!(first.stdout, b"first-deployment\n");
    fs::write(fixture.0.join("deployment"), "updated-deployment\n").unwrap();
    let updated = fixture.run(&launcher, &fixture.0, &["--help"], 0);
    assert!(updated.status.success());
    assert_eq!(updated.stdout, b"updated-deployment\n");
    assert!(fixture.arguments().iter().any(|arg| arg == reference));
}

#[test]
fn installed_flatpak_launcher_preserves_cwd_arguments_and_failure_status() {
    let fixture = Fixture::new();
    let directory = fixture.0.join("directory with spaces and 'quotes'");
    fs::create_dir(&directory).unwrap();
    let project = "CUDA Name 'quoted' $(must-not-run) `literal`.shrimp";
    let launcher = flatpak_launcher(
        "--installation=custom scope",
        "dev.shrimply.Shrimply/x86_64/master",
    );
    let result = fixture.run(
        &launcher,
        &directory,
        &["project", "get", "--project", project],
        37,
    );
    assert_eq!(result.status.code(), Some(37));
    assert_eq!(
        fixture.arguments(),
        [
            directory.display().to_string(),
            "run".into(),
            "--installation=custom scope".into(),
            "--command=shrimply".into(),
            format!("--cwd={}", directory.display()),
            "--filesystem=/tmp".into(),
            format!("--filesystem={}:rw", directory.display()),
            "dev.shrimply.Shrimply/x86_64/master".into(),
            "project".into(),
            "get".into(),
            "--project".into(),
            project.into(),
        ]
    );
}

#[test]
fn installed_flatpak_launcher_supports_root_working_directory() {
    let fixture = Fixture::new();
    let result = fixture.run(
        &flatpak_launcher("--system", "dev.shrimply.Shrimply/x86_64/master"),
        Path::new("/"),
        &["--help"],
        0,
    );
    assert!(result.status.success());
    assert!(
        fixture
            .arguments()
            .iter()
            .any(|arg| arg == "--filesystem=host")
    );
    assert!(fixture.arguments().iter().any(|arg| arg == "--cwd=/"));
}
