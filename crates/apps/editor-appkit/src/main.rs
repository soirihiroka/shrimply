#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
fn main() {
    shrimply_support::crash::install();
    shrimply_support::diagnostics::init();
    let mut args = std::env::args_os().skip(1);
    let project = args.next().map(std::path::PathBuf::from);
    assert!(
        args.next().is_none(),
        "usage: shrimply-editor-appkit [PROJECT]"
    );
    if let Some(path) = &project {
        assert!(path.is_file(), "project does not exist: {}", path.display());
    }
    macos::run(project.as_deref());
}

#[cfg(not(target_os = "macos"))]
fn main() {
    panic!("shrimply-editor-appkit requires macOS");
}
