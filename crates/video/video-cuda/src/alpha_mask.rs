use std::rc::Rc;

use shrimply_cuda::LaunchConfig;

use crate::gpu::VisualFrame;
use crate::gpu::modifiers::{CanvasRgbaFrame, GpuModifier, ModifierContext};
use crate::layer::{PreservingRasterModifier, Visual, VisualState};
use shrimply_render_core::{AlphaMaskParams, ShapeAlphaMaskParams};

pub(crate) use shrimply_video_core::alpha_mask::ResolvedShapeAlphaMask;

struct Resolved {
    mask: Option<Rc<VisualFrame>>,
}

struct Pending {
    mask: Option<Rc<VisualFrame>>,
}

struct PendingShape {
    mask: ResolvedShapeAlphaMask,
}

struct ResolvedShape {
    mask: ResolvedShapeAlphaMask,
    base: Option<Rc<VisualFrame>>,
    canvas_to_local: glam::Mat3,
    local_size: Option<glam::Vec2>,
}

impl PreservingRasterModifier for Pending {
    fn resolve(&self, _: VisualState) -> Box<dyn GpuModifier> {
        Box::new(Resolved {
            mask: self.mask.clone(),
        })
    }
}

impl PreservingRasterModifier for PendingShape {
    fn resolve(&self, _: VisualState) -> Box<dyn GpuModifier> {
        Box::new(ResolvedShape {
            mask: self.mask.clone(),
            base: None,
            canvas_to_local: glam::Mat3::IDENTITY,
            local_size: None,
        })
    }
}

impl GpuModifier for Resolved {
    fn name(&self) -> &'static str {
        "Alpha mask video stream"
    }

    fn apply(
        &self,
        context: &mut ModifierContext<'_>,
        input: CanvasRgbaFrame,
    ) -> Result<CanvasRgbaFrame, String> {
        let input_width = input.width().max(1);
        let input_height = input.height().max(1);
        let count = input_width as usize * input_height as usize;
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
                kernel: alpha_mask,
                stream: context.stream(),
                module: &module,
                config: LaunchConfig::for_num_elems(
                    u32::try_from(count).map_err(|_| "video frame is too large")?
                ),
                args: [
                    pass.input_ptr(),
                    AlphaMaskParams {
                        mask,
                        input_width,
                        input_height,
                        mask_width,
                        mask_height,
                    },
                    slice_mut(pass.output_buffer())
                ]
            }
        }
        .map_err(|error| format!("launch alpha mask CUDA kernel: {error:?}"))?;
        Ok(pass.finish(context))
    }
}

impl GpuModifier for ResolvedShape {
    fn name(&self) -> &'static str {
        "Shape alpha mask"
    }

    fn apply(
        &self,
        context: &mut ModifierContext<'_>,
        input: CanvasRgbaFrame,
    ) -> Result<CanvasRgbaFrame, String> {
        let input_width = input.width().max(1);
        let input_height = input.height().max(1);
        let count = input_width as usize * input_height as usize;
        let base = self.base.as_ref().map_or(std::ptr::null(), |base| {
            assert_eq!(base.width(), input_width, "alpha mask branch width changed");
            assert_eq!(
                base.height(),
                input_height,
                "alpha mask branch height changed"
            );
            base.plane(0)
                .expect("RGBA mask branch has no plane")
                .device_ptr as usize as *const u32
        });
        let local_size = self
            .local_size
            .unwrap_or(glam::Vec2::new(input_width as f32, input_height as f32));
        let vertex_data: Vec<_> = self
            .mask
            .vertices
            .iter()
            .map(|point| point.to_array())
            .collect();
        let vertices = (!vertex_data.is_empty())
            .then(|| context.upload(&vertex_data))
            .transpose()?;
        let vertex_count = u32::try_from(self.mask.vertices.len())
            .map_err(|_| "alpha mask has too many polygon vertices")?;
        let mut pass = input.into_pass(context)?;
        let module = context.modifier_module(crate::gpu::modifiers::ModifierModule::Matte)?;
        unsafe {
            shrimply_cuda::cuda_launch! {
                kernel: shape_alpha_mask,
                stream: context.stream(),
                module: &module,
                config: LaunchConfig::for_num_elems(
                    u32::try_from(count).map_err(|_| "video frame is too large")?
                ),
                args: [
                    pass.input_ptr(),
                    ShapeAlphaMaskParams {
                        base,
                        input_width,
                        canvas_to_local: self.canvas_to_local,
                        local_width: local_size.x.max(1.0),
                        local_height: local_size.y.max(1.0),
                        center: self.mask.center,
                        size: self.mask.size,
                        rotation_degrees: self.mask.rotation_degrees,
                        feather: self.mask.feather,
                        rounding: self.mask.rounding,
                        shape: self.mask.shape,
                        vertices: vertices.as_ref().map_or(std::ptr::null(), |vertices| {
                            vertices.cu_deviceptr() as usize as *const glam::Vec2
                        }),
                        vertex_count,
                        invert: self.mask.invert,
                        _padding_0: [0; 3],
                    },
                    slice_mut(pass.output_buffer())
                ]
            }
        }
        .map_err(|error| format!("launch shape alpha mask CUDA kernel: {error:?}"))?;
        Ok(pass.finish(context))
    }
}

pub(crate) fn apply(input: Visual, mask: Option<Rc<VisualFrame>>) -> Result<Visual, String> {
    let Visual::Raster(mut input) = input else {
        return Err("alpha mask video stream requires a raster video".to_string());
    };
    input.push_preserving_pixel(Box::new(Pending { mask }));
    Ok(Visual::Raster(input))
}

pub(crate) fn pending_shape(mask: ResolvedShapeAlphaMask) -> Box<dyn PreservingRasterModifier> {
    Box::new(PendingShape { mask })
}

pub(crate) fn combine_shape(
    compositor: &mut crate::gpu::CudaVideoCompositor,
    affected: &VisualFrame,
    base: Rc<VisualFrame>,
    canvas_to_local: glam::Mat3,
    local_size: glam::Vec2,
    mask: ResolvedShapeAlphaMask,
) -> Result<VisualFrame, String> {
    compositor.apply_rgba_modifier(
        affected,
        &ResolvedShape {
            mask,
            base: Some(base),
            canvas_to_local,
            local_size: Some(local_size),
        },
    )
}
