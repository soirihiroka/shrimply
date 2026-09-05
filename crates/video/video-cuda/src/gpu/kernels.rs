use std::sync::Arc;

use shrimply_cuda::cuda_launch;
use shrimply_cuda::{CudaContext, CudaModule, CudaStream, DeviceBuffer, DriverError, LaunchConfig};
use shrimply_math_color::Color;
use shrimply_project::project;
use shrimply_render_core::LayerCompositeParams;

pub(crate) use shrimply_render_core::{LayerKind, Nv12LayerParams};

const PREVIEW_CUBIN: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../.slang-artifacts/cuda/sm_86/preview.cubin"
));
const EXPORT_CUBIN: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../.slang-artifacts/cuda/sm_86/export.cubin"
));

pub(crate) struct PreviewModule(Arc<CudaModule>);
pub(crate) struct ExportModule(Arc<CudaModule>);

pub(crate) fn load_preview(context: &Arc<CudaContext>) -> Result<PreviewModule, DriverError> {
    context
        .load_module_from_image(PREVIEW_CUBIN)
        .map(PreviewModule)
}

pub(crate) fn load_export(context: &Arc<CudaContext>) -> Result<ExportModule, DriverError> {
    context
        .load_module_from_image(EXPORT_CUBIN)
        .map(ExportModule)
}

impl PreviewModule {
    pub(crate) unsafe fn tone_map_hdr(
        &self,
        stream: &Arc<CudaStream>,
        config: LaunchConfig,
        input: *const Color,
        background: *const Color,
        output: &mut DeviceBuffer<u32>,
        toon_color_levels: f32,
    ) -> Result<(), DriverError> {
        unsafe {
            cuda_launch! {
                kernel: tone_map_hdr,
                stream: stream,
                module: &self.0,
                config: config,
                args: [input, background, slice_mut(output), toon_color_levels]
            }
        }
    }

    pub(crate) unsafe fn composite_nv12_layers(
        &self,
        stream: &Arc<CudaStream>,
        config: LaunchConfig,
        layers: &DeviceBuffer<Nv12LayerParams>,
        motion_transforms: &DeviceBuffer<glam::Mat3>,
        output: &mut DeviceBuffer<u32>,
        background: u32,
    ) -> Result<(), DriverError> {
        unsafe {
            cuda_launch! {
                kernel: composite_nv12_layers,
                stream: stream,
                module: &self.0,
                config: config,
                args: [slice(layers), slice(motion_transforms), slice_mut(output), background]
            }
        }
    }

    pub(crate) unsafe fn composite_layered_image_layer(
        &self,
        stream: &Arc<CudaStream>,
        config: LaunchConfig,
        params: LayerCompositeParams,
        output: &mut DeviceBuffer<u32>,
    ) -> Result<(), DriverError> {
        unsafe {
            cuda_launch! {
                kernel: composite_layered_image_layer,
                stream: stream,
                module: &self.0,
                config: config,
                args: [params, slice_mut(output)]
            }
        }
    }
}

macro_rules! export_kernel {
    ($name:ident, $plane:ty) => {
        pub(crate) unsafe fn $name(
            &self,
            stream: &Arc<CudaStream>,
            config: LaunchConfig,
            args: (*const u32, u32, u32, *mut $plane, usize),
        ) -> Result<(), DriverError> {
            let (rgba, width, height, plane, pitch) = args;
            unsafe {
                cuda_launch! {
                    kernel: $name,
                    stream: stream,
                    module: &self.0,
                    config: config,
                    args: [rgba, width, height, plane, pitch]
                }
            }
        }
    };
}

impl ExportModule {
    export_kernel!(rgba_to_nv12_luma, u8);
    export_kernel!(rgba_to_nv12_chroma, u8);
    export_kernel!(rgba_to_p010_luma, u16);
    export_kernel!(rgba_to_p010_chroma, u16);
}

pub(crate) fn sample_method(
    value: project::VideoSampleMethod,
) -> shrimply_render_core::VideoSampleMethod {
    value
}

pub(crate) fn address_mode(
    value: project::TextureAddressMode,
) -> shrimply_render_core::TextureAddressMode {
    match value {
        project::TextureAddressMode::Transparent => {
            shrimply_render_core::TextureAddressMode::Transparent
        }
        project::TextureAddressMode::ClampToEdge => {
            shrimply_render_core::TextureAddressMode::ClampToEdge
        }
        project::TextureAddressMode::Repeat => shrimply_render_core::TextureAddressMode::Repeat,
        project::TextureAddressMode::MirrorRepeat => {
            shrimply_render_core::TextureAddressMode::MirrorRepeat
        }
        project::TextureAddressMode::BlurredMirror => {
            shrimply_render_core::TextureAddressMode::BlurredMirror
        }
        project::TextureAddressMode::Stochastic => {
            shrimply_render_core::TextureAddressMode::Stochastic
        }
    }
}
