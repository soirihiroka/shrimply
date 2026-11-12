fn main() -> Result<(), String> {
    use shrimply_math_core::time_from_frame;
    use shrimply_project_document::project;
    use shrimply_visual_cuda::compositor::{
        CompositeAccuracy, VideoCommand, VideoEvent, spawn_worker,
    };
    use std::{
        path::PathBuf,
        time::{Duration, Instant},
    };

    // First use can compile native GPU pipelines before the first frame arrives.
    const WORKER_TIMEOUT: Duration = Duration::from_secs(180);

    #[cfg(target_os = "linux")]
    shrimply_visual_cuda::gpu::configure_gtk_backend()?;
    let mut arguments = std::env::args().skip(1);
    let path = PathBuf::from(
        arguments
            .next()
            .ok_or("Pass a native project path and frame count")?,
    )
    .canonicalize()
    .map_err(|error| error.to_string())?;
    let count = arguments
        .next()
        .map(|value| value.parse::<u64>().map_err(|e| e.to_string()))
        .transpose()?
        .unwrap_or(120);
    let accuracy_name = arguments.next().unwrap_or_else(|| "playback".into());
    let accuracy = match accuracy_name.as_str() {
        "playback" => CompositeAccuracy::CONTINUOUS_TIME_ACCURATE,
        "accurate" => CompositeAccuracy::FULLY_ACCURATE,
        _ => return Err("Accuracy must be playback or accurate".into()),
    };
    if count == 0 {
        return Err("Frame count must be positive".into());
    }
    let project = project::load_project_snapshot(&path)?;
    let fps = project.fps;
    let last = time_from_frame(count - 1, fps).ok_or("Invalid playback frame position")?;
    if last >= project.duration() {
        return Err("Benchmark frame count exceeds the project duration".into());
    }
    let (commands, events) = spawn_worker(project);
    for pass in ["Cold", "Warm"] {
        shrimply_profiling::clear();
        let started = Instant::now();
        let mut first_frame = Duration::ZERO;
        for index in 0..count {
            let position = time_from_frame(index, fps).ok_or("Invalid playback frame position")?;
            commands
                .send(VideoCommand::Render {
                    position,
                    accuracy: if index == 0 {
                        CompositeAccuracy::FULLY_ACCURATE
                    } else {
                        accuracy
                    },
                })
                .map_err(|e| e.to_string())?;
            loop {
                match events
                    .recv_timeout(WORKER_TIMEOUT)
                    .map_err(|e| e.to_string())?
                {
                    VideoEvent::Error(error) => return Err(error),
                    VideoEvent::Frame {
                        position: rendered,
                        frame,
                        settled: true,
                        ..
                    } if rendered == position => {
                        // Measure completed CUDA work, and never count a loading screen as an animation frame.
                        if let Ok(buffer) = frame.buffer.cuda() {
                            buffer.context().synchronize().map_err(|error| error.to_string())?;
                        }
                        let _measurement = shrimply_profiling::measure("Playback / Release frame");
                        drop(frame);
                        break;
                    }
                    VideoEvent::Clear {
                        position: rendered, ..
                    } if rendered == position => {
                        return Err(format!("No visible video at {}", position.as_label()));
                    }
                    _ => {}
                }
            }
            if index == 0 {
                first_frame = started.elapsed();
            }
        }
        let elapsed = started.elapsed();
        println!(
            "{pass}: {count} preview frames in {elapsed:?}; {} fps; first frame {first_frame:?}; mode={accuracy_name}; no pixel readbacks",
            u128::from(count) * 1_000_000_000 / elapsed.as_nanos().max(1)
        );
        println!("{pass} profile: {}", shrimply_profiling::report_json());
    }
    let shutdown = Instant::now();
    commands
        .send(VideoCommand::Stop)
        .map_err(|e| e.to_string())?;
    // Drain until the worker releases its native GPU contexts before process exit.
    loop {
        match events.recv_timeout(WORKER_TIMEOUT) {
            Ok(VideoEvent::Error(error)) => return Err(error),
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(error) => return Err(error.to_string()),
        }
    }
    println!("GPU worker shutdown: {:?}", shutdown.elapsed());
    Ok(())
}
