#[cfg(target_os = "macos")]
fn main() -> std::process::ExitCode {
    shrimply_process_reporting::crash::install();
    let mut args = std::env::args_os().skip(1);
    let project = args.next().map(std::path::PathBuf::from);
    assert!(
        args.next().is_none(),
        "usage: shrimply-editor-appkit [PROJECT]"
    );
    shrimply_editor_appkit::run(project.as_deref())
}

#[cfg(not(target_os = "macos"))]
fn main() {
    panic!("shrimply-editor-appkit requires macOS");
}
