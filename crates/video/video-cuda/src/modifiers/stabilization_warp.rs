use shrimply_cuda::LaunchConfig;
use shrimply_render_core::AffineStabilizationParams;

use crate::gpu::modifiers::{CanvasRgbaFrame, GpuModifier, ModifierContext};
use crate::layer::{PreservingRasterModifier, VisualState};

pub(super) struct Source {
    pub source_transform: glam::Mat3,
}

struct Resolved {
    pub source_transform: glam::Mat3,
}

impl PreservingRasterModifier for Source {
    fn resolve(&self, _: VisualState) -> Box<dyn GpuModifier> {
        Box::new(Resolved {
            source_transform: self.source_transform,
        })
    }
}

impl GpuModifier for Resolved {
    fn name(&self) -> &'static str {
        "video stabilization"
    }

    fn apply(
        &self,
        context: &mut ModifierContext<'_>,
        input: CanvasRgbaFrame,
    ) -> Result<CanvasRgbaFrame, String> {
        let width = input.width();
        let height = input.height();
        let pixel_count = width as usize * height as usize;
        let mut pass = input.into_pass(context)?;
        let module =
            context.modifier_module(crate::gpu::modifiers::ModifierModule::Stabilization)?;
        unsafe {
            shrimply_cuda::cuda_launch! {
                kernel: affine_stabilization,
                stream: context.stream(), module: &module,
                config: LaunchConfig::for_num_elems(u32::try_from(pixel_count).map_err(|_| "canvas is too large")?),
                args: [AffineStabilizationParams {
                    input: pass.input_ptr(),
                    width,
                    height,
                    source_transform: self.source_transform,
                    _padding_0: [0; 4],
                }, slice_mut(pass.output_buffer())]
            }
        }
        .map_err(|error| format!("launch CUDA affine stabilization kernel: {error:?}"))?;
        Ok(pass.finish(context))
    }
}
