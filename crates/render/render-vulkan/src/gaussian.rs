use crate::{Buffer, Renderer};
use ash::vk;

impl Renderer {
    pub fn gaussian(
        &mut self,
        session: &shrimply_3dgs_core::RenderSession,
        size: (u32, u32),
        parameters: &shrimply_3dgs_core::RenderParams,
    ) -> Result<Buffer, String> {
        let length = (size.0 as usize)
            .checked_mul(size.1 as usize)
            .and_then(|count| count.checked_mul(4))
            .ok_or("Gaussian output size overflow")?;
        let output = self.allocate(length)?;
        let _guard = self.device.queue_lock();
        if self.gaussian.is_none() {
            self.gaussian = Some(shrimply_3dgs_vulkan::Renderer::new(
                shrimply_3dgs_vulkan::RenderContext {
                    device: self.device.raw.clone(),
                    memory_properties: self.device.memory,
                    queue: self.device.queue,
                    command_pool: self.pool.raw,
                    pipeline_cache: vk::PipelineCache::null(),
                },
            )?);
        }
        self.gaussian
            .as_mut()
            .expect("initialized Gaussian Vulkan renderer")
            .render_to_buffer(session, size.0, size.1, parameters, output.raw())?;
        Ok(output)
    }
}
