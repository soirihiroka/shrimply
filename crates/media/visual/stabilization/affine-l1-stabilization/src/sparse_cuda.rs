use shrimply_gpu_cuda::{CudaContext, CudaModule, CudaStream, LaunchConfig, cuda_launch};
use shrimply_gpu_cuda_memory::{AllocationClass, GpuBuffer};
use shrimply_vision::gpu::TrackingJob;
use std::sync::Arc;

pub(super) struct Device {
    stream: Arc<CudaStream>,
    module: Arc<CudaModule>,
}

impl Device {
    pub(super) fn new() -> Result<Self, String> {
        let context = CudaContext::new(shrimply_gpu_cuda::selected_device())
            .map_err(|error| error.to_string())?;
        let stream = context.new_stream().map_err(|error| error.to_string())?;
        let module = context
            .load_module_from_image(shrimply_render_kernels_cuda::TRACKING)
            .map_err(|error| error.to_string())?;
        Ok(Self { stream, module })
    }

    pub(super) fn track(&mut self, job: &TrackingJob) -> Result<Vec<u8>, String> {
        self.stream
            .context()
            .bind_to_thread()
            .map_err(|error| error.to_string())?;
        let upload = |bytes: &[u8]| -> Result<GpuBuffer<u8>, String> {
            let mut buffer = shrimply_gpu_cuda_memory::global().allocate_buffer(
                &self.stream,
                bytes.len(),
                AllocationClass::Transient,
                "sparse tracking upload",
            )?;
            buffer
                .copy_from_host(&self.stream, bytes)
                .map_err(|error| error.to_string())?;
            Ok(buffer)
        };
        let allocate = |length| {
            shrimply_gpu_cuda_memory::global().allocate_buffer::<u8>(
                &self.stream,
                length,
                AllocationClass::Transient,
                "sparse tracking scratch",
            )
        };
        let previous = upload(&job.previous)?;
        let current = upload(&job.current)?;
        let derivatives = allocate(job.derivative_bytes)?;
        let levels = upload(&job.levels)?;
        let points = upload(&job.points)?;
        let output = allocate(job.output_bytes())?;
        for step in &job.preparation {
            use shrimply_vision::gpu::{PREPARATION_GROUP_SIZE, PreparationStep};
            let threads = match *step {
                PreparationStep::Pyramid { threads, .. }
                | PreparationStep::Derivatives { threads, .. } => threads as u32,
            };
            let config = LaunchConfig {
                grid_dim: (threads.div_ceil(PREPARATION_GROUP_SIZE), 1, 1),
                block_dim: (PREPARATION_GROUP_SIZE, 1, 1),
                shared_mem_bytes: 0,
            };
            // Each step uses this same stream, so image writes precede dependent reads.
            unsafe {
                match *step {
                    PreparationStep::Pyramid { previous: early, level, .. } => {
                        let image = if early { &previous } else { &current };
                        cuda_launch! { kernel: vision_pyr_down, stream: &self.stream, module: &self.module,
                            config: config, args: [image.cu_deviceptr(), levels.cu_deviceptr(), level] }
                    }
                    PreparationStep::Derivatives { level, .. } => {
                        cuda_launch! { kernel: vision_scharr, stream: &self.stream, module: &self.module,
                            config: config, args: [previous.cu_deviceptr(), derivatives.cu_deviceptr(),
                                levels.cu_deviceptr(), level] }
                    }
                }
            }.map_err(|error| error.to_string())?;
        }
        // The Slang kernel uses raw pointers (not Rust slice pointer/length pairs).
        // All allocations remain alive until the synchronized result download.
        unsafe {
            cuda_launch! {
                kernel: vision_sparse_lk, stream: &self.stream, module: &self.module,
                config: LaunchConfig {
                    grid_dim: (job.count, 1, 1),
                    block_dim: (shrimply_vision::gpu::WORKGROUP_SIZE, 1, 1),
                    shared_mem_bytes: 0,
                },
                args: [previous.cu_deviceptr(), current.cu_deviceptr(), derivatives.cu_deviceptr(),
                    levels.cu_deviceptr(), points.cu_deviceptr(), output.cu_deviceptr(),
                    job.count, job.level_count,
                    job.window, job.iterations, job.epsilon_squared, job.minimum_eigenvalue]
            }
        }
        .map_err(|error| error.to_string())?;
        output
            .to_host_vec(&self.stream)
            .map_err(|error| error.to_string())
    }
}
