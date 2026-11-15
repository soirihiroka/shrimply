//! Shared source-faithful sparse tracking, with native dispatch of the same Slang kernel.
#[cfg(any(
    feature = "nvidia",
    target_os = "linux",
    target_os = "android",
    target_os = "macos"
))]
use shrimply_vision::gpu::TrackingJob;
use shrimply_vision::{FlowOptions, GrayImage, Track};

#[cfg(feature = "nvidia")]
#[path = "sparse_cuda.rs"]
mod cuda;
#[cfg(feature = "nvidia")]
use cuda::Device;
#[cfg(all(not(feature = "nvidia"), target_os = "macos"))]
use shrimply_render_metal::Renderer as Device;
#[cfg(all(
    not(feature = "nvidia"),
    any(target_os = "linux", target_os = "android")
))]
use shrimply_render_vulkan::Renderer as Device;

/// Reuses the compute context across all pairs in an analysis chunk.
#[cfg_attr(
    not(any(
        feature = "nvidia",
        target_os = "linux",
        target_os = "android",
        target_os = "macos"
    )),
    derive(Default)
)]
pub struct Tracker {
    #[cfg(any(
        feature = "nvidia",
        target_os = "linux",
        target_os = "android",
        target_os = "macos"
    ))]
    device: Option<Device>,
    #[cfg(any(
        feature = "nvidia",
        target_os = "linux",
        target_os = "android",
        target_os = "macos"
    ))]
    calibrated: bool,
}

#[cfg(any(
    feature = "nvidia",
    target_os = "linux",
    target_os = "android",
    target_os = "macos"
))]
impl Default for Tracker {
    fn default() -> Self {
        Self {
            device: match Device::new() {
                Ok(device) => Some(device),
                Err(error) => {
                    tracing::info!(%error, "GPU sparse tracking unavailable; using scalar reference");
                    None
                }
            },
            calibrated: false,
        }
    }
}

impl Tracker {
    pub fn track(
        &mut self,
        previous: &GrayImage,
        current: &GrayImage,
        points: &[[f32; 2]],
        options: FlowOptions,
    ) -> Result<Vec<Track>, String> {
        if points.is_empty() || options.window > shrimply_vision::gpu::MAX_WINDOW {
            return shrimply_vision::track(previous, current, points, options);
        }
        #[cfg(any(
            feature = "nvidia",
            target_os = "linux",
            target_os = "android",
            target_os = "macos"
        ))]
        if let Some(device) = self.device.as_mut() {
            let packing_started = std::time::Instant::now();
            let job = TrackingJob::new(previous, current, points, options)?;
            let packing_time = packing_started.elapsed();
            let calibrate = !self.calibrated;
            self.calibrated = true;
            let mut execute = || dispatch(device, &job).and_then(|output| job.finish(&output));
            let result: Result<_, String> = (|| {
                let cpu = if calibrate {
                    // Warm both allocation paths and GPU pipelines before measuring steady work.
                    execute()?;
                    shrimply_vision::track(previous, current, points, options)?;
                    let started = std::time::Instant::now();
                    let tracks = shrimply_vision::track(previous, current, points, options)?;
                    Some((tracks, started.elapsed()))
                } else {
                    None
                };
                let started = std::time::Instant::now();
                let tracks = execute()?;
                let gpu_time = started.elapsed() + packing_time;
                if let Some((cpu_tracks, cpu_time)) = cpu {
                    let keep_gpu = gpu_time < cpu_time;
                    tracing::debug!(
                        ?cpu_time,
                        ?gpu_time,
                        keep_gpu,
                        "Selected sparse tracking backend for analysis chunk"
                    );
                    if !keep_gpu {
                        return Ok((cpu_tracks, false));
                    }
                }
                Ok((tracks, true))
            })();
            match result {
                Ok((tracks, keep_gpu)) => {
                    if !keep_gpu {
                        self.device = None;
                    }
                    return Ok(tracks);
                }
                Err(error) => {
                    tracing::warn!(%error, "GPU sparse tracking failed; using scalar reference");
                    self.device = None;
                }
            }
        }
        shrimply_vision::track(previous, current, points, options)
    }
}

#[cfg(feature = "nvidia")]
fn dispatch(device: &mut Device, job: &TrackingJob) -> Result<Vec<u8>, String> {
    device.track(job)
}

#[cfg(all(
    not(feature = "nvidia"),
    any(target_os = "linux", target_os = "android", target_os = "macos")
))]
fn dispatch(device: &mut Device, job: &TrackingJob) -> Result<Vec<u8>, String> {
    let previous = device.upload(&job.previous)?;
    let current = device.upload(&job.current)?;
    let derivatives = device.allocate(job.derivative_bytes)?;
    let levels = device.upload(&job.levels)?;
    let points = device.upload(&job.points)?;
    let output = device.allocate(job.output_bytes())?;
    let mut preparation = Vec::with_capacity(job.preparation.len());
    for step in &job.preparation {
        use shrimply_vision::gpu::PreparationStep;
        let (name, image, level, threads) = match *step {
            PreparationStep::Pyramid {
                previous: early,
                level,
                threads,
            } => (
                "vision_pyr_down",
                if early { &previous } else { &current },
                level,
                threads,
            ),
            PreparationStep::Derivatives { level, threads } => {
                ("vision_scharr", &previous, level, threads)
            }
        };
        let mut arguments = device.arguments(name)?;
        arguments
            .set("image", &image.address().to_ne_bytes())?
            .set("levels", &levels.address().to_ne_bytes())?
            .set("level", &level.to_ne_bytes())?;
        if matches!(step, PreparationStep::Derivatives { .. }) {
            arguments.set("derivatives", &derivatives.address().to_ne_bytes())?;
        }
        // The shared plan orders dependencies; queue submissions retain their buffers.
        preparation.push(unsafe {
            device.dispatch(
                arguments,
                vec![image.clone(), levels.clone(), derivatives.clone()],
                [threads, 1, 1],
            )
        }?);
    }
    let mut arguments = device.arguments("vision_sparse_lk")?;
    for (name, buffer) in [
        ("previous", &previous),
        ("current", &current),
        ("derivatives", &derivatives),
        ("levels", &levels),
        ("points", &points),
        ("output", &output),
    ] {
        arguments.set(name, &buffer.address().to_ne_bytes())?;
    }
    for (name, value) in [
        ("count", job.count),
        ("level_count", job.level_count),
        ("window", job.window),
        ("iterations", job.iterations),
    ] {
        arguments.set(name, &value.to_ne_bytes())?;
    }
    arguments
        .set("epsilon_squared", &job.epsilon_squared.to_ne_bytes())?
        .set("minimum_eigenvalue", &job.minimum_eigenvalue.to_ne_bytes())?;
    // Every pointer is backed by a retained allocation with job-validated bounds.
    unsafe {
        device.dispatch(
            arguments,
            vec![
                previous,
                current,
                derivatives,
                levels,
                points,
                output.clone(),
            ],
            job.dispatch_extent(),
        )
    }?
    .wait()?;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    return device.readback(&output);
    #[cfg(target_os = "macos")]
    Ok(output.copy_bytes())
}
