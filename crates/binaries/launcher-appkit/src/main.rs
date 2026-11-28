#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
fn main() -> std::process::ExitCode {
    if let Some(status) = shrimply_cli::run_if_requested() {
        return std::process::ExitCode::from(status);
    }
    shrimply_process_reporting::crash::install();
    match shrimply_cross_ui_core::launcher::project_path_argument() {
        Ok(Some(path)) => return shrimply_editor_appkit::run(Some(&path)),
        Ok(None) => (),
        Err(error) => {
            eprintln!("{error}");
            return std::process::ExitCode::FAILURE;
        }
    }
    macos::run();
    std::process::ExitCode::SUCCESS
}

#[cfg(not(target_os = "macos"))]
fn main() {
    panic!("shrimply requires macOS");
}
