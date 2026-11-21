fn main() -> Result<(), String> {
    use shrimply_preview_render_core::benchmark;
    use shrimply_visual_cuda::compositor::spawn_worker_with_resources_and_observer;
    #[cfg(target_os = "linux")]
    shrimply_visual_cuda::gpu::configure_gtk_backend()?;
    let mut args = std::env::args().skip(1);
    let path = std::path::PathBuf::from(args.next().ok_or("Pass a project path and frame count")?)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let count = args
        .next()
        .map(|v| v.parse::<u64>().map_err(|e| e.to_string()))
        .transpose()?
        .unwrap_or(120);
    let project = shrimply_project_document::project::load_project_snapshot(&path)?;
    let report = benchmark::run(
        project,
        count,
        |project, observer| {
            spawn_worker_with_resources_and_observer(project, Default::default(), Some(observer))
        },
        |frame| {
            if let Ok(buffer) = frame.buffer.cuda() {
                buffer.context().synchronize().map_err(|e| e.to_string())?;
            }
            Ok(())
        },
    )?;
    print!("{}", report.text);
    Ok(())
}
