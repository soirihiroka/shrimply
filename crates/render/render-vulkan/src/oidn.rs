use crate::{Buffer, Renderer, Submission};
use ash::vk;
use shrimply_denoise_oidn::{Device, Filter, Image, Images, Quality};
use std::sync::Arc;

pub(crate) struct State {
    device: Arc<Device>,
    filters: [Filter; 2],
}

pub(crate) struct Guides<'a> {
    pub colors: [(&'a Buffer, &'a Buffer); 2],
    pub albedo: &'a Buffer,
    pub depth_normal: &'a Buffer,
    pub size: (u32, u32),
    pub quality: Quality,
}

impl Renderer {
    pub(crate) fn denoise_oidn(
        &mut self,
        guides: Guides<'_>,
        submissions: &mut Vec<Submission>,
    ) -> Result<(), String> {
        if self.oidn.is_none() {
            let device = Device::for_uuid(self.device_uuid())?;
            self.oidn = Some(State {
                filters: [Filter::new(device.clone())?, Filter::new(device.clone())?],
                device,
            });
        }
        let resources = guides
            .colors
            .into_iter()
            .flat_map(|(color, output)| [color.clone(), output.clone()])
            .chain([guides.albedo.clone(), guides.depth_normal.clone()])
            .collect::<Vec<_>>();
        let device = self
            .oidn
            .as_ref()
            .expect("initialized OIDN GPU state")
            .device
            .clone();
        let colors = guides
            .colors
            .into_iter()
            .map(|(color, output)| Ok((color.oidn(&device)?, output.oidn(&device)?)))
            .collect::<Result<Vec<_>, String>>()?;
        let albedo = guides.albedo.oidn(&device)?;
        let normal = guides.depth_normal.oidn(&device)?;
        // OIDN's SYCL backend currently lacks external semaphore interop. Fence
        // waits synchronize ownership only; no image pixels leave GPU memory.
        let release = self.external_buffers(&resources, false)?;
        release
            .wait()
            .expect("Vulkan failed while releasing OIDN buffer ownership");
        submissions.push(release);
        let state = self.oidn.as_mut().expect("initialized OIDN GPU state");
        let result =
            state
                .filters
                .iter_mut()
                .zip(colors)
                .try_for_each(|(filter, (color, output))| unsafe {
                    filter.execute(
                        Images {
                            color: Image::Shared {
                                buffer: color,
                                offset: 0,
                            },
                            output: Image::Shared {
                                buffer: output,
                                offset: 0,
                            },
                            albedo: Some(Image::Shared {
                                buffer: albedo,
                                offset: 0,
                            }),
                            normal: Some(Image::Shared {
                                buffer: normal,
                                offset: size_of::<f32>(),
                            }),
                            size: guides.size,
                        },
                        guides.quality,
                    )
                });
        // Always wait before reacquiring or propagating an asynchronous error.
        let completion = state.device.synchronize();
        submissions.push(
            self.external_buffers(&resources, true)
                .expect("Vulkan could not reacquire OIDN buffer ownership"),
        );
        result.and(completion)
    }

    fn external_buffers(&self, resources: &[Buffer], acquire: bool) -> Result<Submission, String> {
        let command = self.command()?;
        let barriers = resources
            .iter()
            .map(|buffer| {
                vk::BufferMemoryBarrier::default()
                    .buffer(buffer.raw())
                    .size(vk::WHOLE_SIZE)
                    .src_queue_family_index(if acquire {
                        vk::QUEUE_FAMILY_EXTERNAL
                    } else {
                        self.device.family
                    })
                    .dst_queue_family_index(if acquire {
                        self.device.family
                    } else {
                        vk::QUEUE_FAMILY_EXTERNAL
                    })
                    .src_access_mask(if acquire {
                        vk::AccessFlags::empty()
                    } else {
                        vk::AccessFlags::MEMORY_WRITE
                    })
                    .dst_access_mask(if acquire {
                        vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE
                    } else {
                        vk::AccessFlags::empty()
                    })
            })
            .collect::<Vec<_>>();
        unsafe {
            self.device.raw.cmd_pipeline_barrier(
                command,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::DependencyFlags::empty(),
                &[],
                &barriers,
                &[],
            );
        }
        self.submit(command, resources.to_vec())
    }
}
