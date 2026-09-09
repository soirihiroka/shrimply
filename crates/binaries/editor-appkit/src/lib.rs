#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
pub fn run(project: Option<&std::path::Path>) -> std::process::ExitCode {
    shrimply_process_reporting::diagnostics::init();
    if let Some(path) = project {
        assert!(path.is_file(), "project does not exist: {}", path.display());
    }
    match macos::run(project) {
        Ok(true) => std::process::ExitCode::SUCCESS,
        Ok(false) => std::process::ExitCode::from(
            shrimply_cross_ui_core::launcher::EDITOR_OPEN_CANCELED_EXIT_CODE,
        ),
        Err(()) => std::process::ExitCode::FAILURE,
    }
}

#[cfg(not(target_os = "macos"))]
pub fn run(_project: Option<&std::path::Path>) -> std::process::ExitCode {
    panic!("shrimply-editor-appkit requires macOS");
}
