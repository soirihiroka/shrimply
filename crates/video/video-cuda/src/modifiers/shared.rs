use crate::gpu::modifiers::{CanvasRgbaFrame, GpuModifier, ModifierContext, ModifierModule};
use shrimply_render_core::effects::{BufferSlot, Module, PixelEffect};

impl GpuModifier for PixelEffect {
    fn name(&self) -> &'static str {
        PixelEffect::name(self)
    }

    fn apply(
        &self,
        context: &mut ModifierContext<'_>,
        input: CanvasRgbaFrame,
    ) -> Result<CanvasRgbaFrame, String> {
        let (width, height) = (input.width(), input.height());
        let count = width as usize * height as usize;
        let launch = shrimply_cuda::LaunchConfig::for_num_elems(
            u32::try_from(count).map_err(|_| "canvas is too large")?,
        );
        let mut pass = input.into_pass(context)?;
        let scratch_count = count
            .checked_mul(self.scratch_words_per_pixel())
            .ok_or("effect scratch size overflow")?;
        let scratch = (scratch_count != 0)
            .then(|| context.take_scratch(scratch_count))
            .transpose()?;
        let input_address = pass.input_ptr() as u64;
        let output_address = pass.output_buffer().cu_deviceptr();
        for kernel in self.passes(width, height) {
            let module = context.modifier_module(match kernel.module {
                Module::General => ModifierModule::General,
                Module::Geometry => ModifierModule::Geometry,
                Module::Matte => ModifierModule::Matte,
                Module::Blur => ModifierModule::Blur,
            })?;
            let mut storage: Vec<_> = kernel
                .arguments
                .iter()
                .map(|(_, value)| {
                    value.bytes(|slot| match slot {
                        BufferSlot::Input => input_address,
                        BufferSlot::Output => output_address,
                        BufferSlot::Scratch => scratch
                            .as_ref()
                            .expect("effect requires scratch")
                            .cu_deviceptr(),
                    })
                })
                .collect();
            let arguments = storage
                .iter_mut()
                .map(|bytes| bytes.as_mut_ptr().cast())
                .collect();
            // Shared passes follow the Slang declaration order and scalar ABI. Input,
            // output and scratch allocations remain live through submission; stream
            // ordering protects their subsequent recycling, as in the original adapters.
            unsafe {
                shrimply_cuda::launch_raw(
                    &module,
                    context.stream(),
                    launch,
                    kernel.kernel,
                    arguments,
                )
            }
            .map_err(|error| format!("launch {} CUDA kernel: {error:?}", kernel.kernel))?;
        }
        if let Some(scratch) = scratch {
            context.recycle_scratch(scratch);
        }
        Ok(pass.finish(context))
    }
}
