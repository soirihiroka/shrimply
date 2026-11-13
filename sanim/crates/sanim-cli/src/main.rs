fn main() -> std::process::ExitCode {
    std::process::ExitCode::from(sanim_cli::run(std::env::args_os().skip(1).collect()))
}
