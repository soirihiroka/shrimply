use shrimply_cuda::LaunchConfig;
use shrimply_render_core::{
    DitheringColorMode as GpuDitheringColorMode, DitheringParams,
    DitheringPattern as GpuDitheringPattern,
};

use super::RasterModifierRuntime;
use crate::gpu::modifiers::{CanvasRgbaFrame, GpuModifier, ModifierContext};
use crate::layer::RasterVisual;
use crate::visual_source::VisualModifierContext;
use shrimply_evaluation::{resolve_color, resolve_scalar};
use shrimply_video_modifiers::dithering::{
    DitheringColorMode, DitheringModifier, DitheringPattern,
};

struct Resolved {
    pattern: GpuDitheringPattern,
    color_mode: GpuDitheringColorMode,
    levels: f32,
    amount: f32,
    palette: Vec<u32>,
}

impl GpuModifier for Resolved {
    fn name(&self) -> &'static str {
        "Dithering"
    }

    fn apply(
        &self,
        context: &mut ModifierContext<'_>,
        input: CanvasRgbaFrame,
    ) -> Result<CanvasRgbaFrame, String> {
        let width = input.width();
        let count = width as usize * input.height() as usize;
        let launch =
            LaunchConfig::for_num_elems(u32::try_from(count).map_err(|_| "canvas is too large")?);
        let mut pass = input.into_pass(context)?;
        let module = context.modifier_module(crate::gpu::modifiers::ModifierModule::General)?;
        let palette = context.upload(&self.palette)?;
        let palette_ptr = palette.cu_deviceptr() as usize as *const u32;
        let palette_len = u32::try_from(self.palette.len()).map_err(|_| "palette is too large")?;
        let params = DitheringParams {
            pattern: self.pattern,
            color_mode: self.color_mode,
            levels: self.levels,
            amount: self.amount,
            palette: palette_ptr,
            palette_len,
            _padding_0: [0; 4],
        };
        unsafe {
            shrimply_cuda::cuda_launch! {
                kernel: dithering,
                stream: context.stream(),
                module: &module,
                config: launch,
                args: [
                    pass.input_ptr(),
                    width,
                    slice_mut(pass.output_buffer()),
                    params
                ]
            }
        }
        .map_err(|error| format!("launch dithering CUDA kernel: {error:?}"))?;
        Ok(pass.finish(context))
    }
}

impl RasterModifierRuntime for DitheringModifier {
    fn apply_raster(
        &self,
        mut input: RasterVisual,
        context: &mut VisualModifierContext<'_>,
    ) -> Result<RasterVisual, String> {
        let mut palette = Vec::with_capacity(self.palette.len());
        for color in &self.palette {
            palette
                .push(resolve_color(color, context.evaluation, context.expressions).to_rgba_u32());
        }
        input.push_pixel(Box::new(Resolved {
            pattern: match self.pattern.value_at(context.evaluation.local_time()) {
                DitheringPattern::Bayer2x2 => GpuDitheringPattern::Bayer2x2,
                DitheringPattern::Bayer4x4 => GpuDitheringPattern::Bayer4x4,
                DitheringPattern::Bayer8x8 => GpuDitheringPattern::Bayer8x8,
            },
            color_mode: match self.color_mode.value_at(context.evaluation.local_time()) {
                DitheringColorMode::Color => GpuDitheringColorMode::Color,
                DitheringColorMode::Grayscale => GpuDitheringColorMode::Grayscale,
                DitheringColorMode::Palette => GpuDitheringColorMode::Palette,
            },
            levels: resolve_scalar(&self.levels, context.evaluation, context.expressions)
                .clamp(2.0, 256.0),
            amount: resolve_scalar(&self.amount, context.evaluation, context.expressions)
                .clamp(0.0, 1.0),
            palette,
        }));
        Ok(input)
    }
}
