#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
fn main() -> std::process::ExitCode {
    shrimply_process_reporting::crash::install();
    let mut args = std::env::args_os().skip(1);
    if let Some(path) = args.next() {
        if args.next().is_some() {
            eprintln!("usage: shrimply-appkit [PROJECT]");
            return std::process::ExitCode::FAILURE;
        }
        return shrimply_editor_appkit::run(Some(std::path::Path::new(&path)));
    }
    macos::run();
    std::process::ExitCode::SUCCESS
}

#[cfg(not(target_os = "macos"))]
fn main() {
    panic!("shrimply-appkit requires macOS");
}
