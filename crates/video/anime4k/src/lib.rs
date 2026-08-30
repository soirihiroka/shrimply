use hashbrown::HashMap;
use std::sync::Arc;

use cuda_core::{
    CudaContext, CudaModule, CudaStream, DeviceBuffer as CudaDeviceBuffer, DriverError,
    LaunchConfig,
};
use cuda_device::{DisjointSlice, kernel};
use cuda_host::cuda_launch;
use shrimply_gpu_memory::GpuBuffer;

mod types;
use types::{AlphaParams, ConvolutionParams, ConvolutionTerm, ImageDescriptor, MAX_STAGE_INPUTS};

const UPSCALE_CNN_X2_M: &[u8] = include_bytes!("../models/upscale_cnn_x2_m.bin");
const RESTORE_GAN_UUL: &[u8] = include_bytes!("../models/restore_gan_uul.bin");
const UPSCALE_GAN_X4_UUL: &[u8] = include_bytes!("../models/upscale_gan_x4_uul.bin");
#[cfg(target_os = "linux")]
const ANIME4K_CUBIN: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../.oxide-artifacts/cuda/sm_86/anime4k.cubin"
));
#[cfg(not(target_os = "linux"))]
const ANIME4K_CUBIN: &[u8] = &[];

impl ImageDescriptor {
    const EMPTY: Self = Self {
        pixels: std::ptr::null(),
        width: 0,
        height: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Method {
    CnnM,
    SrganUul,
}

#[derive(Clone, Copy)]
pub enum Source {
    Nv12 {
        y_plane: *const u8,
        uv_plane: *const u8,
        y_pitch: usize,
        uv_pitch: usize,
        width: u32,
        height: u32,
    },
    Rgba {
        pixels: *const u8,
        pitch: usize,
        width: u32,
        height: u32,
    },
}

impl Source {
    fn size(self) -> glam::UVec2 {
        match self {
            Self::Nv12 { width, height, .. } | Self::Rgba { width, height, .. } => {
                glam::UVec2::new(width, height)
            }
        }
    }
}

pub struct UpscaledFrame {
    pub pixels: GpuBuffer<u32>,
    pub width: u32,
    pub height: u32,
}

pub struct Workspace {
    module: Arc<CudaModule>,
    models: Option<Models>,
}

impl Workspace {
    pub fn new(context: Arc<CudaContext>) -> Result<Self, String> {
        let module = context
            .load_module_from_image(ANIME4K_CUBIN)
            .map_err(|error| format!("load Anime4K CUDA module: {error:?}"))?;
        Ok(Self {
            module,
            models: None,
        })
    }

    pub fn clear_cached_models(&mut self) -> bool {
        self.models.take().is_some()
    }

    pub fn upscale(
        &mut self,
        stream: &Arc<CudaStream>,
        source: Source,
        method: Method,
        scale: f32,
    ) -> Result<Option<UpscaledFrame>, String> {
        if !scale.is_finite() || scale <= 1.0 {
            return Ok(None);
        }
        let glam::UVec2 {
            x: width,
            y: height,
        } = source.size();
        if width == 0 || height == 0 {
            return Err("Anime4K source dimensions must be non-zero".to_string());
        }
        if self.models.is_none() {
            self.models = Some(Models::load(stream)?);
        }

        let mut image = self.convert_source(stream, source)?;
        let models = self.models.as_ref().expect("Anime4K models loaded");
        let mut accumulated_scale = 1.0;
        match method {
            Method::CnnM => {
                while accumulated_scale < scale {
                    image = self.run_model(stream, &models.cnn_x2, image)?;
                    accumulated_scale *= 2.0;
                }
            }
            Method::SrganUul => {
                image = self.run_model(stream, &models.restore_uul, image)?;
                image = self.run_model(stream, &models.upscale_uul_x4, image)?;
                accumulated_scale = 4.0;
                while accumulated_scale < scale {
                    image = self.run_model(stream, &models.cnn_x2, image)?;
                    accumulated_scale *= 2.0;
                }
            }
        }

        let pixel_count = pixel_count(image.width, image.height)?;
        let launch_count = u32::try_from(pixel_count)
            .map_err(|_| "Anime4K output is too large for a CUDA launch".to_string())?;
        let mut pixels = shrimply_gpu_memory::global().allocate_buffer::<u32>(
            stream,
            pixel_count,
            shrimply_gpu_memory::AllocationClass::Transient,
            "Anime4K RGBA output",
        )?;
        unsafe {
            match source {
                Source::Nv12 { .. } => self
                    .float_to_rgba_opaque(
                        stream,
                        LaunchConfig::for_num_elems(launch_count),
                        image.descriptor().pixels,
                        &mut pixels,
                    )
                    .map_err(|error| format!("launch Anime4K RGBA conversion: {error:?}"))?,
                Source::Rgba {
                    pixels: alpha_source,
                    pitch,
                    width,
                    height,
                } => self
                    .float_to_rgba_alpha(
                        stream,
                        LaunchConfig::for_num_elems(launch_count),
                        (
                            image.descriptor().pixels,
                            alpha_source,
                            pitch,
                            width,
                            height,
                            image.width,
                            image.height,
                        ),
                        &mut pixels,
                    )
                    .map_err(|error| format!("launch Anime4K alpha conversion: {error:?}"))?,
            }
        }
        Ok(Some(UpscaledFrame {
            pixels,
            width: image.width,
            height: image.height,
        }))
    }

    fn convert_source(&self, stream: &Arc<CudaStream>, source: Source) -> Result<Image, String> {
        let glam::UVec2 {
            x: width,
            y: height,
        } = source.size();
        let count = pixel_count(width, height)?;
        let launch_count = u32::try_from(count)
            .map_err(|_| "Anime4K input is too large for a CUDA launch".to_string())?;
        let mut pixels = shrimply_gpu_memory::global().allocate_buffer::<[f32; 4]>(
            stream,
            count,
            shrimply_gpu_memory::AllocationClass::Transient,
            "Anime4K input",
        )?;
        unsafe {
            match source {
                Source::Nv12 {
                    y_plane,
                    uv_plane,
                    y_pitch,
                    uv_pitch,
                    ..
                } => self
                    .nv12_to_float(
                        stream,
                        LaunchConfig::for_num_elems(launch_count),
                        (y_plane, uv_plane, y_pitch, uv_pitch, width, height),
                        &mut pixels,
                    )
                    .map_err(|error| format!("launch Anime4K NV12 conversion: {error:?}"))?,
                Source::Rgba {
                    pixels: source,
                    pitch,
                    ..
                } => self
                    .rgba_to_float(
                        stream,
                        LaunchConfig::for_num_elems(launch_count),
                        (source, pitch, width),
                        &mut pixels,
                    )
                    .map_err(|error| format!("launch Anime4K RGBA input conversion: {error:?}"))?,
            }
        }
        Ok(Image {
            pixels,
            width,
            height,
        })
    }

    fn run_model(
        &self,
        stream: &Arc<CudaStream>,
        model: &Model,
        input: Image,
    ) -> Result<Image, String> {
        let mut images: HashMap<String, Image> = HashMap::from([(String::from("MAIN"), input)]);
        let mut remaining_uses = HashMap::<&str, usize>::new();
        for stage in &model.stages {
            for name in &stage.binds {
                *remaining_uses.entry(name).or_default() += 1;
            }
        }

        for stage in &model.stages {
            let base_width = images.get(&stage.width_source).ok_or_else(|| {
                format!(
                    "Anime4K stage {} is missing width source {}",
                    stage.name, stage.width_source
                )
            })?;
            let width = base_width
                .width
                .checked_mul(stage.width_multiplier)
                .ok_or_else(|| format!("Anime4K stage {} width overflow", stage.name))?;
            let base_height = images.get(&stage.height_source).ok_or_else(|| {
                format!(
                    "Anime4K stage {} is missing height source {}",
                    stage.name, stage.height_source
                )
            })?;
            let height = base_height
                .height
                .checked_mul(stage.height_multiplier)
                .ok_or_else(|| format!("Anime4K stage {} height overflow", stage.name))?;
            let count = pixel_count(width, height)?;
            let launch_count = u32::try_from(count)
                .map_err(|_| format!("Anime4K stage {} is too large", stage.name))?;
            let mut descriptors = [ImageDescriptor::EMPTY; MAX_STAGE_INPUTS];
            for (slot, name) in stage.binds.iter().enumerate() {
                descriptors[slot] = images
                    .get(name)
                    .ok_or_else(|| format!("Anime4K stage {} is missing input {name}", stage.name))?
                    .descriptor();
            }
            let mut output = shrimply_gpu_memory::global().allocate_buffer::<[f32; 4]>(
                stream,
                count,
                shrimply_gpu_memory::AllocationClass::Transient,
                format!("Anime4K stage {}", stage.name),
            )?;
            unsafe {
                match stage.kind {
                    StageKind::Convolution => {
                        let residual = stage
                            .residual
                            .as_ref()
                            .and_then(|name| images.get(name))
                            .map_or(ImageDescriptor::EMPTY, Image::descriptor);
                        self.convolution(
                            stream,
                            LaunchConfig::for_num_elems(launch_count),
                            ConvolutionParams {
                                images: descriptors,
                                bias: stage.bias,
                                residual,
                                result_scale: stage.result_scale,
                                residual_scale: stage.residual_scale,
                                width,
                                height,
                            },
                            &stage.terms,
                            &mut output,
                        )
                        .map_err(|error| {
                            format!("launch Anime4K stage {}: {error:?}", stage.name)
                        })?;
                    }
                    StageKind::DepthToSpaceX2 => {
                        self.depth_to_space_x2(
                            stream,
                            LaunchConfig::for_num_elems(launch_count),
                            descriptors[1],
                            descriptors[0],
                            width,
                            &mut output,
                        )
                        .map_err(|error| {
                            format!("launch Anime4K stage {}: {error:?}", stage.name)
                        })?;
                    }
                }
            }

            for name in &stage.binds {
                let uses = remaining_uses
                    .get_mut(name.as_str())
                    .expect("Anime4K bind use was counted");
                *uses -= 1;
                if *uses == 0 && name != &stage.save {
                    images.remove(name);
                }
            }
            images.insert(
                stage.save.clone(),
                Image {
                    pixels: output,
                    width,
                    height,
                },
            );
        }
        images
            .remove("MAIN")
            .ok_or_else(|| "Anime4K model did not produce MAIN".to_string())
    }
}

struct Image {
    pixels: GpuBuffer<[f32; 4]>,
    width: u32,
    height: u32,
}

impl Image {
    fn descriptor(&self) -> ImageDescriptor {
        ImageDescriptor {
            pixels: self.pixels.cu_deviceptr() as usize as *const [f32; 4],
            width: self.width,
            height: self.height,
        }
    }
}

struct Models {
    cnn_x2: Model,
    restore_uul: Model,
    upscale_uul_x4: Model,
}

impl Models {
    fn load(stream: &Arc<CudaStream>) -> Result<Self, String> {
        Ok(Self {
            cnn_x2: Model::load(stream, UPSCALE_CNN_X2_M)?,
            restore_uul: Model::load(stream, RESTORE_GAN_UUL)?,
            upscale_uul_x4: Model::load(stream, UPSCALE_GAN_X4_UUL)?,
        })
    }
}

struct Model {
    stages: Vec<Stage>,
}

impl Model {
    fn load(stream: &Arc<CudaStream>, source: &[u8]) -> Result<Self, String> {
        let stages = decode_model(source)?
            .into_iter()
            .map(|stage| stage.upload(stream))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { stages })
    }
}

struct Stage {
    name: String,
    binds: Vec<String>,
    save: String,
    width_source: String,
    width_multiplier: u32,
    height_source: String,
    height_multiplier: u32,
    terms: GpuBuffer<ConvolutionTerm>,
    bias: [f32; 4],
    residual: Option<String>,
    result_scale: f32,
    residual_scale: f32,
    kind: StageKind,
}

#[derive(Clone, Copy)]
enum StageKind {
    Convolution,
    DepthToSpaceX2,
}

struct ParsedStage {
    name: String,
    binds: Vec<String>,
    save: String,
    width_source: String,
    width_multiplier: u32,
    height_source: String,
    height_multiplier: u32,
    terms: Vec<ConvolutionTerm>,
    bias: [f32; 4],
    residual: Option<String>,
    result_scale: f32,
    residual_scale: f32,
    kind: StageKind,
}

impl ParsedStage {
    fn upload(self, stream: &Arc<CudaStream>) -> Result<Stage, String> {
        let mut terms = shrimply_gpu_memory::global().allocate_buffer(
            stream,
            self.terms.len(),
            shrimply_gpu_memory::AllocationClass::Persistent,
            format!("Anime4K weights for {}", self.name),
        )?;
        terms
            .copy_from_host(stream, &self.terms)
            .map_err(|error| format!("upload Anime4K weights for {}: {error:?}", self.name))?;
        Ok(Stage {
            name: self.name,
            binds: self.binds,
            save: self.save,
            width_source: self.width_source,
            width_multiplier: self.width_multiplier,
            height_source: self.height_source,
            height_multiplier: self.height_multiplier,
            terms,
            bias: self.bias,
            residual: self.residual,
            result_scale: self.result_scale,
            residual_scale: self.residual_scale,
            kind: self.kind,
        })
    }
}

fn decode_model(source: &[u8]) -> Result<Vec<ParsedStage>, String> {
    let mut decoder = Decoder::new(source);
    if decoder.bytes(4)? != b"A4K1" {
        return Err("invalid Anime4K model header".to_string());
    }
    let stage_count = decoder.u32()? as usize;
    let mut stages = Vec::with_capacity(stage_count);
    for _ in 0..stage_count {
        let name = decoder.string()?;
        let bind_count = decoder.u32()? as usize;
        if bind_count > MAX_STAGE_INPUTS {
            return Err(format!(
                "Anime4K stage {name} binds {bind_count} inputs, maximum is {MAX_STAGE_INPUTS}"
            ));
        }
        let mut binds = Vec::with_capacity(bind_count);
        for _ in 0..bind_count {
            binds.push(decoder.string()?);
        }
        let save = decoder.string()?;
        let width_source = decoder.string()?;
        let width_multiplier = decoder.u32()?;
        let height_source = decoder.string()?;
        let height_multiplier = decoder.u32()?;
        let kind = match decoder.u32()? {
            0 => StageKind::Convolution,
            1 => StageKind::DepthToSpaceX2,
            value => return Err(format!("Anime4K stage {name} has invalid kind {value}")),
        };
        let term_count = decoder.u32()? as usize;
        let mut terms = Vec::with_capacity(term_count);
        for _ in 0..term_count {
            let mut weights = [0.0; 16];
            for value in &mut weights {
                *value = decoder.f32()?;
            }
            let offset_x = decoder.f32()?;
            let offset_y = decoder.f32()?;
            let input = decoder.u32()?;
            let activation = decoder.u32()?;
            if input as usize >= bind_count || activation > 2 {
                return Err(format!(
                    "Anime4K stage {name} has an invalid convolution term"
                ));
            }
            terms.push(ConvolutionTerm {
                weights,
                offset_x,
                offset_y,
                input,
                activation,
            });
        }
        let mut bias = [0.0; 4];
        for value in &mut bias {
            *value = decoder.f32()?;
        }
        let residual = match decoder.string()? {
            value if value.is_empty() => None,
            value => Some(value),
        };
        let result_scale = decoder.f32()?;
        let residual_scale = decoder.f32()?;
        stages.push(ParsedStage {
            name,
            binds,
            save,
            width_source,
            width_multiplier,
            height_source,
            height_multiplier,
            terms,
            bias,
            residual,
            result_scale,
            residual_scale,
            kind,
        });
    }
    if !decoder.is_empty() {
        return Err("Anime4K model has trailing data".to_string());
    }
    Ok(stages)
}

struct Decoder<'a> {
    source: &'a [u8],
    position: usize,
}

impl<'a> Decoder<'a> {
    fn new(source: &'a [u8]) -> Self {
        Self {
            source,
            position: 0,
        }
    }

    fn bytes(&mut self, count: usize) -> Result<&'a [u8], String> {
        let end = self
            .position
            .checked_add(count)
            .filter(|end| *end <= self.source.len())
            .ok_or_else(|| "truncated Anime4K model".to_string())?;
        let value = &self.source[self.position..end];
        self.position = end;
        Ok(value)
    }

    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(
            self.bytes(4)?.try_into().expect("read four bytes"),
        ))
    }

    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_le_bytes(
            self.bytes(4)?.try_into().expect("read four bytes"),
        ))
    }

    fn string(&mut self) -> Result<String, String> {
        let length = self.u32()? as usize;
        std::str::from_utf8(self.bytes(length)?)
            .map(str::to_owned)
            .map_err(|_| "Anime4K model contains invalid text".to_string())
    }

    fn is_empty(&self) -> bool {
        self.position == self.source.len()
    }
}

fn pixel_count(width: u32, height: u32) -> Result<usize, String> {
    (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| "Anime4K image dimensions overflow addressable memory".to_string())
}

#[kernel]
fn rgba_to_float(_: *const u8, _: usize, _: u32, _: DisjointSlice<[f32; 4]>) {}
#[kernel]
fn nv12_to_float(
    _: *const u8,
    _: *const u8,
    _: usize,
    _: usize,
    _: u32,
    _: u32,
    _: DisjointSlice<[f32; 4]>,
) {
}
#[kernel]
fn convolution(_: ConvolutionParams, _: &[ConvolutionTerm], _: DisjointSlice<[f32; 4]>) {}
#[kernel]
fn depth_to_space_x2(_: ImageDescriptor, _: ImageDescriptor, _: u32, _: DisjointSlice<[f32; 4]>) {}
#[kernel]
fn float_to_rgba_opaque(_: *const [f32; 4], _: DisjointSlice<u32>) {}
#[kernel]
fn float_to_rgba_alpha(_: AlphaParams, _: DisjointSlice<u32>) {}

impl Workspace {
    unsafe fn rgba_to_float(
        &self,
        stream: &Arc<CudaStream>,
        config: LaunchConfig,
        args: (*const u8, usize, u32),
        mut output: &mut CudaDeviceBuffer<[f32; 4]>,
    ) -> Result<(), DriverError> {
        let (source, pitch, width) = args;
        unsafe {
            cuda_launch! {
                kernel: rgba_to_float,
                stream: stream,
                module: &self.module,
                config: config,
                args: [source, pitch, width, slice_mut(output)]
            }
        }
    }

    unsafe fn nv12_to_float(
        &self,
        stream: &Arc<CudaStream>,
        config: LaunchConfig,
        args: (*const u8, *const u8, usize, usize, u32, u32),
        mut output: &mut CudaDeviceBuffer<[f32; 4]>,
    ) -> Result<(), DriverError> {
        let (y, uv, y_pitch, uv_pitch, width, height) = args;
        unsafe {
            cuda_launch! {
                kernel: nv12_to_float,
                stream: stream,
                module: &self.module,
                config: config,
                args: [y, uv, y_pitch, uv_pitch, width, height, slice_mut(output)]
            }
        }
    }

    unsafe fn convolution(
        &self,
        stream: &Arc<CudaStream>,
        config: LaunchConfig,
        params: ConvolutionParams,
        terms: &CudaDeviceBuffer<ConvolutionTerm>,
        mut output: &mut CudaDeviceBuffer<[f32; 4]>,
    ) -> Result<(), DriverError> {
        unsafe {
            cuda_launch! {
                kernel: convolution,
                stream: stream,
                module: &self.module,
                config: config,
                args: [params, slice(terms), slice_mut(output)]
            }
        }
    }

    unsafe fn depth_to_space_x2(
        &self,
        stream: &Arc<CudaStream>,
        config: LaunchConfig,
        convolution_image: ImageDescriptor,
        residual: ImageDescriptor,
        width: u32,
        mut output: &mut CudaDeviceBuffer<[f32; 4]>,
    ) -> Result<(), DriverError> {
        unsafe {
            cuda_launch! {
                kernel: depth_to_space_x2,
                stream: stream,
                module: &self.module,
                config: config,
                args: [convolution_image, residual, width, slice_mut(output)]
            }
        }
    }

    unsafe fn float_to_rgba_opaque(
        &self,
        stream: &Arc<CudaStream>,
        config: LaunchConfig,
        source: *const [f32; 4],
        mut output: &mut CudaDeviceBuffer<u32>,
    ) -> Result<(), DriverError> {
        unsafe {
            cuda_launch! {
                kernel: float_to_rgba_opaque,
                stream: stream,
                module: &self.module,
                config: config,
                args: [source, slice_mut(output)]
            }
        }
    }

    unsafe fn float_to_rgba_alpha(
        &self,
        stream: &Arc<CudaStream>,
        config: LaunchConfig,
        args: (*const [f32; 4], *const u8, usize, u32, u32, u32, u32),
        mut output: &mut CudaDeviceBuffer<u32>,
    ) -> Result<(), DriverError> {
        let (source, alpha_source, alpha_pitch, alpha_width, alpha_height, width, height) = args;
        let params = AlphaParams {
            source,
            alpha_source,
            alpha_pitch,
            alpha_width,
            alpha_height,
            width,
            height,
        };
        unsafe {
            cuda_launch! {
                kernel: float_to_rgba_alpha,
                stream: stream,
                module: &self.module,
                config: config,
                args: [params, slice_mut(output)]
            }
        }
    }
}
