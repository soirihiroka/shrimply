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
            #[cfg(any(
                feature = "nvidia",
                target_os = "linux",
                target_os = "android",
                target_os = "macos"
            ))]
            device: match Device::new() {
                Ok(device) => Some(device),
                Err(error) => {
                    tracing::info!(%error, "GPU sparse tracking unavailable; using scalar reference");
                    None
                }
            },
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
        if points.is_empty() {
            return shrimply_vision::track(previous, current, points, options);
        }
        #[cfg(any(
            feature = "nvidia",
            target_os = "linux",
            target_os = "android",
            target_os = "macos"
        ))]
        if let Some(device) = self.device.as_mut() {
            let job = TrackingJob::new(previous, current, points, options)?;
            match dispatch(device, &job).and_then(|output| job.finish(&output)) {
                Ok(tracks) => return Ok(tracks),
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
    let derivatives = device.upload(&job.derivatives)?;
    let levels = device.upload(&job.levels)?;
    let points = device.upload(&job.points)?;
    let output = device.allocate(job.output_bytes())?;
    let patch = device.allocate(job.patch_bytes())?;
    let gradients = device.allocate(job.gradient_bytes())?;
    let mut arguments = device.arguments("vision_sparse_lk")?;
    for (name, buffer) in [
        ("previous", &previous),
        ("current", &current),
        ("derivatives", &derivatives),
        ("levels", &levels),
        ("points", &points),
        ("output", &output),
        ("patch", &patch),
        ("gradients", &gradients),
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
                patch,
                gradients,
            ],
            [job.count as usize, 1, 1],
        )
    }?
    .wait()?;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    return device.readback(&output);
    #[cfg(target_os = "macos")]
    Ok(output.copy_bytes())
}
