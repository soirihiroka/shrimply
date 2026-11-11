mod agent;
mod catalog;
mod render;
mod render_options;
mod search;

/// Handle commands before GUI startup, leaving ordinary project launching intact.
pub fn run_if_requested() -> Option<u8> {
    let mut arguments = std::env::args_os().skip(1);
    let command = arguments.next()?;
    let name = command.to_str().unwrap_or_default();
    let recognized = name.starts_with('-')
        || matches!(name, "cli" | "schema")
        || shrimply_editing_api::commands::COMMANDS
            .iter()
            .any(|definition| definition.path.split_whitespace().next() == Some(name));
    if !recognized {
        return None;
    }
    #[cfg(windows)]
    if std::env::var_os("CUDA_MODULE_LOADING").is_none() {
        // Before diagnostics, native libraries, or the editing runtime start threads.
        unsafe { std::env::set_var("CUDA_MODULE_LOADING", "EAGER") };
    }
    let mut all = vec![std::ffi::OsString::from("shrimply"), command];
    all.extend(arguments);
    Some(agent::run(all))
}
