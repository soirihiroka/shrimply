#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
pub use macos::{about::show as show_about, settings::PreferencesWindow};

#[cfg(target_os = "macos")]
pub fn run(project: &std::path::Path) -> std::process::ExitCode {
    shrimply_process_reporting::diagnostics::init();
    match macos::run(project) {
        Ok(true) => std::process::ExitCode::SUCCESS,
        Ok(false) => std::process::ExitCode::from(
            shrimply_cross_ui_core::launcher::EDITOR_OPEN_CANCELED_EXIT_CODE,
        ),
        Err(()) => std::process::ExitCode::FAILURE,
    }
}

#[cfg(not(target_os = "macos"))]
pub fn run(_project: &std::path::Path) -> std::process::ExitCode {
    panic!("shrimply-editor-appkit requires macOS");
}
