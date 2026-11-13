fn main() -> std::process::ExitCode {
    match sanim_lsp::run(std::io::stdin().lock(), std::io::stdout().lock()) {
        Ok(status) => std::process::ExitCode::from(status),
        Err(error) => {
            eprintln!("sanim-lsp: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
