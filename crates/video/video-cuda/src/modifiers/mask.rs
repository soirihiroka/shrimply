use std::rc::Rc;

use shrimply_cuda::LaunchConfig;

use super::RasterModifierRuntime;
use crate::gpu::VisualFrame;
use crate::gpu::modifiers::{CanvasRgbaFrame, GpuModifier, ModifierContext};
use crate::layer::{PreservingRasterModifier, RasterVisual, VisualState};
use crate::visual_source::VisualModifierContext;
use shrimply_render_core::MaskParams;
use shrimply_video_modifiers::mask::{MaskMode, MaskModifier};

struct Resolved {
    mask: Option<Rc<VisualFrame>>,
    transform: glam::Mat3,
    luminance: bool,
    invert: bool,
}

struct Pending {
    mask: Option<Rc<VisualFrame>>,
    luminance: bool,
    invert: bool,
}

impl PreservingRasterModifier for Pending {
    fn resolve(&self, state: VisualState) -> Box<dyn GpuModifier> {
        Box::new(Resolved {
            mask: self.mask.clone(),
            transform: state.transform.matrix,
            luminance: self.luminance,
            invert: self.invert,
        })
    }
}

impl GpuModifier for Resolved {
    fn name(&self) -> &'static str {
        "Mask"
    }

    fn apply(
        &self,
        context: &mut ModifierContext<'_>,
        input: CanvasRgbaFrame,
    ) -> Result<CanvasRgbaFrame, String> {
        let width = input.width();
        let height = input.height();
        let count = width as usize * height as usize;
        let (mask, mask_width, mask_height) =
            self.mask.as_ref().map_or((std::ptr::null(), 1, 1), |mask| {
                let plane = mask.plane(0).expect("RGBA mask has no plane");
                (
                    plane.device_ptr as usize as *const u32,
                    mask.width().max(1),
                    mask.height().max(1),
                )
            });
        let mut pass = input.into_pass(context)?;
        let module = context.modifier_module(crate::gpu::modifiers::ModifierModule::Matte)?;
        unsafe {
            shrimply_cuda::cuda_launch! {
                kernel: mask,
                stream: context.stream(),
                module: &module,
                config: LaunchConfig::for_num_elems(
                    u32::try_from(count).map_err(|_| "canvas is too large")?
                ),
                args: [
                    pass.input_ptr(),
                    MaskParams {
                        mask,
                        input_width: width,
                        mask_width,
                        mask_height,
                        transform: self.transform,
                        luminance: self.luminance,
                        invert: self.invert,
                        _padding_0: [0; 6],
                    },
                    slice_mut(pass.output_buffer())
                ]
            }
        }
        .map_err(|error| format!("launch mask CUDA kernel: {error:?}"))?;
        Ok(pass.finish(context))
    }
}

impl RasterModifierRuntime for MaskModifier {
    fn apply_raster(
        &self,
        mut input: RasterVisual,
        context: &mut VisualModifierContext<'_>,
    ) -> Result<RasterVisual, String> {
        input.push_preserving_pixel(Box::new(Pending {
            mask: context.mask_source.clone(),
            luminance: self.mode.value_at(context.evaluation.local_time()) == MaskMode::Luminance,
            invert: self.invert,
        }));
        Ok(input)
    }
}
