#![cfg(target_os = "macos")]

mod skia;

use objc2::{msg_send, rc::Retained, runtime::ProtocolObject};
use objc2_foundation::NSString;
use objc2_metal::{
    MTLBinding, MTLBindingType, MTLBlitCommandEncoder, MTLBuffer, MTLCommandBuffer,
    MTLCommandEncoder, MTLCommandQueue, MTLComputeCommandEncoder, MTLComputePipelineState,
    MTLDevice, MTLLibrary, MTLOrigin, MTLPipelineOption, MTLPixelFormat, MTLResource,
    MTLResourceUsage, MTLSize, MTLStorageMode, MTLTexture, MTLTextureDescriptor, MTLTextureUsage,
};
use shrimply_render_core::Nv12LayerParams;
use std::collections::HashMap;

const MESH_FLOW_THREAD_WIDTH: usize = 16;
const MESH_FLOW_THREAD_HEIGHT: usize = 16;

struct Field {
    name: &'static str,
    offset: usize,
    size: usize,
}
struct KernelLayout {
    name: &'static str,
    group: [usize; 3],
    fields: &'static [Field],
}
struct Module {
    name: &'static str,
    source: &'static str,
    kernels: &'static [KernelLayout],
}
include!(concat!(env!("OUT_DIR"), "/kernels.rs"));

struct Kernel {
    pipeline: Retained<ProtocolObject<dyn MTLComputePipelineState>>,
    layout: &'static KernelLayout,
    binding: usize,
    argument_size: usize,
}

/// Named values are packed using Slang's Metal reflection, never CUDA argument offsets.
pub struct Arguments {
    layout: &'static KernelLayout,
    bytes: Vec<u8>,
}

impl Arguments {
    pub fn set_matrix3(
        &mut self,
        name: &str,
        matrix: shrimply_render_core::math::Mat3,
    ) -> Result<&mut Self, String> {
        // Uniform Mat3 columns use float3's 16-byte alignment on Metal. Pointer
        // pointees, including the motion buffer, use their own packed layout.
        for (axis, column) in [
            ("x_axis", matrix.x_axis),
            ("y_axis", matrix.y_axis),
            ("z_axis", matrix.z_axis),
        ] {
            self.set(
                &format!("{name}.{axis}"),
                column
                    .extend(0.0)
                    .to_array()
                    .map(f32::to_ne_bytes)
                    .as_flattened(),
            )?;
        }
        Ok(self)
    }

    pub fn set(&mut self, name: &str, bytes: &[u8]) -> Result<&mut Self, String> {
        let field = self
            .layout
            .fields
            .iter()
            .find(|field| field.name == name)
            .ok_or_else(|| format!("Unknown {} argument {name}", self.layout.name))?;
        if field.size != bytes.len() {
            return Err(format!(
                "{} argument {name} requires {} bytes, received {}",
                self.layout.name,
                field.size,
                bytes.len()
            ));
        }
        self.bytes[field.offset..field.offset + field.size].copy_from_slice(bytes);
        Ok(self)
    }
}

pub use shrimply_gpu_metal::{Buffer, Submission};

pub struct Frame {
    submission: Submission,
    output: Buffer,
}
impl Frame {
    pub fn into_parts(self) -> (Buffer, Submission) {
        (self.output, self.submission)
    }

    pub fn completed(&self) -> Result<bool, String> {
        self.submission.completed()
    }

    pub fn pixels(&self) -> Result<Option<&[u8]>, String> {
        if !self.submission.completed()? {
            return Ok(None);
        }
        // Shared storage is CPU coherent after the command reports completion.
        Ok(Some(unsafe {
            std::slice::from_raw_parts(
                self.output.metal().contents().as_ptr().cast(),
                self.output.metal().length(),
            )
        }))
    }
}

impl Renderer {
    /// Queue a GPU-only copy of a composite's straight RGBA buffer.
    /// This renderer's queue orders the copy after the frame's compute work.
    pub fn presentation_texture(
        &self,
        frame: &Frame,
        size: (u32, u32),
    ) -> Result<(Retained<ProtocolObject<dyn MTLTexture>>, Submission), String> {
        let row_bytes = (size.0 as usize)
            .checked_mul(size_of::<u32>())
            .ok_or("Presentation row size overflow")?;
        let length = row_bytes
            .checked_mul(size.1 as usize)
            .ok_or("Presentation image size overflow")?;
        if size.0 == 0 || size.1 == 0 || length != frame.output.metal().length() {
            return Err("Presentation dimensions do not match the composite buffer".into());
        }
        let descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                MTLPixelFormat::RGBA8Unorm,
                size.0 as usize,
                size.1 as usize,
                false,
            )
        };
        descriptor.setStorageMode(MTLStorageMode::Private);
        descriptor.setUsage(MTLTextureUsage::ShaderRead);
        let texture = self
            .context
            .device
            .newTextureWithDescriptor(&descriptor)
            .ok_or("Could not allocate the presentation texture")?;
        let command = self
            .context
            .queue
            .commandBuffer()
            .ok_or("Could not create the presentation command")?;
        let blit = command
            .blitCommandEncoder()
            .ok_or("Could not create the presentation blit encoder")?;
        // The command retains the texture and Submission retains the source
        // buffer. Neither resource is modified after this copy is submitted.
        unsafe {
            blit.copyFromBuffer_sourceOffset_sourceBytesPerRow_sourceBytesPerImage_sourceSize_toTexture_destinationSlice_destinationLevel_destinationOrigin(
                frame.output.metal(),
                0,
                row_bytes,
                length,
                MTLSize { width: size.0 as usize, height: size.1 as usize, depth: 1 },
                &texture,
                0,
                0,
                MTLOrigin { x: 0, y: 0, z: 0 },
            );
        }
        blit.endEncoding();
        command.commit();
        Ok((
            texture,
            Submission::new(command, vec![frame.output.clone()]),
        ))
    }
}

/// Metal resource management and dispatch for the shared Slang rendering modules.
pub struct Renderer {
    skia: Option<skia_safe::gpu::DirectContext>,
    context: shrimply_gpu_metal::Context,
    libraries: HashMap<&'static str, Retained<ProtocolObject<dyn MTLLibrary>>>,
    kernels: HashMap<&'static str, Kernel>,
    direct_kernels: HashMap<&'static str, Retained<ProtocolObject<dyn MTLComputePipelineState>>>,
}

impl Renderer {
    pub fn supports_ray_tracing(&self) -> bool {
        self.context.device.supportsRaytracing()
    }

    pub fn mesh_flow(
        &mut self,
        input: Buffer,
        width: u32,
        height: u32,
        grid_width: u32,
        grid_height: u32,
        offsets: &[[f32; 2]],
    ) -> Result<(Buffer, Submission), String> {
        let pixel_count = width
            .checked_mul(height)
            .and_then(|count| usize::try_from(count).ok())
            .ok_or("MeshFlow image size overflow")?;
        let grid_count = grid_width
            .checked_mul(grid_height)
            .and_then(|count| usize::try_from(count).ok())
            .ok_or("MeshFlow grid size overflow")?;
        if width < 2
            || height < 2
            || grid_width < 2
            || grid_height < 2
            || offsets.len() != grid_count
            || offsets.iter().flatten().any(|value| !value.is_finite())
        {
            return Err("Invalid Metal MeshFlow field".into());
        }
        let offset_bytes = offsets
            .iter()
            .flat_map(|offset| offset.iter().flat_map(|value| value.to_ne_bytes()))
            .collect::<Vec<_>>();
        let offsets = self.upload(&offset_bytes)?;
        let output = self.allocate(
            pixel_count
                .checked_mul(size_of::<u32>())
                .ok_or("MeshFlow output size overflow")?,
        )?;
        let mut uniform_bytes = Vec::with_capacity(4 * size_of::<u32>());
        for value in [width, height, grid_width, grid_height] {
            uniform_bytes.extend(value.to_ne_bytes());
        }
        let uniforms = self.upload(&uniform_bytes)?;
        self.mesh_flow_pipeline()?;
        let command = self
            .context
            .queue
            .commandBuffer()
            .ok_or("Could not create Metal MeshFlow command")?;
        let encoder = command
            .computeCommandEncoder()
            .ok_or("Could not create Metal MeshFlow encoder")?;
        encoder.setComputePipelineState(&self.direct_kernels["mesh_flow"]);
        unsafe {
            encoder.setBuffer_offset_atIndex(Some(input.metal()), 0, 0);
            encoder.setBuffer_offset_atIndex(Some(offsets.metal()), 0, 1);
            encoder.setBuffer_offset_atIndex(Some(output.metal()), 0, 2);
            encoder.setBuffer_offset_atIndex(Some(uniforms.metal()), 0, 3);
        }
        let resources = vec![input, offsets, output.clone(), uniforms];
        for buffer in &resources {
            let resource: &ProtocolObject<dyn MTLResource> =
                ProtocolObject::from_ref(&**buffer.metal());
            encoder.useResource_usage(resource, MTLResourceUsage::Read | MTLResourceUsage::Write);
        }
        encoder.dispatchThreads_threadsPerThreadgroup(
            MTLSize {
                width: width as usize,
                height: height as usize,
                depth: 1,
            },
            MTLSize {
                width: MESH_FLOW_THREAD_WIDTH,
                height: MESH_FLOW_THREAD_HEIGHT,
                depth: 1,
            },
        );
        encoder.endEncoding();
        command.commit();
        let submission = Submission::new(command, resources);
        Ok((output, submission))
    }

    /// Compile the complete built-in pipeline set on the renderer's owning worker.
    pub fn warmup(&mut self) -> Result<(), String> {
        for module in MODULES {
            if module.name == "mesh_flow" {
                self.mesh_flow_pipeline()?;
            } else {
                for kernel in module.kernels {
                    self.kernel(kernel.name)?;
                }
            }
        }
        Ok(())
    }

    fn mesh_flow_pipeline(&mut self) -> Result<(), String> {
        let module = MODULES
            .iter()
            .find(|module| module.name == "mesh_flow")
            .ok_or("Shared MeshFlow Metal module is missing")?;
        if !self.libraries.contains_key(module.name) {
            let library = self
                .context
                .device
                .newLibraryWithSource_options_error(&NSString::from_str(module.source), None)
                .map_err(|error| format!("Compile Metal module mesh_flow: {error}"))?;
            self.libraries.insert(module.name, library);
        }
        if !self.direct_kernels.contains_key("mesh_flow") {
            let function = self.libraries[module.name]
                .newFunctionWithName(&NSString::from_str("main_0"))
                .ok_or("Shared MeshFlow Metal module has no compute entry")?;
            let pipeline = self
                .context
                .device
                .newComputePipelineStateWithFunction_error(&function)
                .map_err(|error| format!("Create Metal MeshFlow kernel: {error}"))?;
            self.direct_kernels.insert("mesh_flow", pipeline);
        }
        Ok(())
    }

    pub fn background(
        &mut self,
        uniforms: &shrimply_render_core::background_spirv::BackgroundUniforms,
    ) -> Result<(Buffer, Submission), String> {
        let width = uniforms.common.width as usize;
        let height = uniforms.common.height as usize;
        let length = width
            .checked_mul(height)
            .and_then(|count| count.checked_mul(size_of::<u32>()))
            .ok_or("Background image size overflow")?;
        let output = self.allocate(length)?;
        let mut arguments = self.arguments("render_background")?;
        arguments.set("output", &output.address().to_ne_bytes())?;
        shrimply_render_core::background_arguments(uniforms, |name, bytes| {
            arguments.set(name, bytes).map(|_| ())
        })?;
        // Output allocation matches the kernel bounds and is retained until completion.
        let submission =
            unsafe { self.dispatch(arguments, vec![output.clone()], [width, height, 1]) }?;
        Ok((output, submission))
    }

    pub fn new() -> Result<Self, String> {
        Ok(Self {
            skia: None,
            context: shrimply_gpu_metal::Context::new()?,
            libraries: HashMap::new(),
            kernels: HashMap::new(),
            direct_kernels: HashMap::new(),
        })
    }

    pub fn device(&self) -> &Retained<ProtocolObject<dyn MTLDevice>> {
        &self.context.device
    }

    pub fn queue(&self) -> &Retained<ProtocolObject<dyn MTLCommandQueue>> {
        &self.context.queue
    }

    fn kernel(&mut self, name: &str) -> Result<&Kernel, String> {
        if !self.kernels.contains_key(name) {
            let (module, layout) = MODULES
                .iter()
                .find_map(|module| {
                    module
                        .kernels
                        .iter()
                        .find(|kernel| kernel.name == name)
                        .map(|kernel| (module, kernel))
                })
                .ok_or_else(|| format!("Unknown shared Slang kernel {name}"))?;
            if !self.libraries.contains_key(module.name) {
                let started = std::time::Instant::now();
                tracing::info!(
                    module = module.name,
                    kernel = name,
                    "Compiling shared Slang Metal module"
                );
                let library = self
                    .context
                    .device
                    .newLibraryWithSource_options_error(&NSString::from_str(module.source), None)
                    .map_err(|error| format!("Compile Metal module {}: {error}", module.name))?;
                self.libraries.insert(module.name, library);
                tracing::info!(
                    module = module.name,
                    elapsed_ms = started.elapsed().as_millis(),
                    "Compiled shared Slang Metal module"
                );
            }
            let library = &self.libraries[module.name];
            let function = library
                .newFunctionWithName(&NSString::from_str(name))
                .ok_or_else(|| format!("Metal module {} has no kernel {name}", module.name))?;
            let mut reflection = None;
            // The entry was generated and validated by Slang and Metal at build time.
            let pipeline_started = std::time::Instant::now();
            tracing::info!(kernel = name, "Creating shared Slang Metal pipeline");
            let pipeline = unsafe {
                self.context
                    .device
                    .newComputePipelineStateWithFunction_options_reflection_error(
                        &function,
                        MTLPipelineOption::BindingInfo,
                        Some(&mut reflection),
                    )
            }
            .map_err(|error| format!("Create Metal kernel {name}: {error}"))?;
            tracing::info!(
                kernel = name,
                elapsed_ms = pipeline_started.elapsed().as_millis(),
                "Created shared Slang Metal pipeline"
            );
            let bindings = reflection
                .ok_or("Metal omitted pipeline reflection")?
                .bindings();
            let buffers: Vec<_> = bindings
                .iter()
                .filter(|binding| binding.r#type() == MTLBindingType::Buffer)
                .collect();
            if buffers.len() != 1 {
                return Err(format!(
                    "Metal kernel {name} requires one reflected argument buffer"
                ));
            }
            let binding = &buffers[0];
            // MTLBindingType::Buffer guarantees the MTLBufferBinding protocol.
            let argument_size: usize = unsafe { msg_send![&**binding, bufferDataSize] };
            if layout
                .fields
                .iter()
                .any(|field| field.offset + field.size > argument_size)
            {
                return Err(format!(
                    "Slang and Metal disagree about {name} argument size"
                ));
            }
            self.kernels.insert(
                layout.name,
                Kernel {
                    pipeline,
                    layout,
                    binding: binding.index(),
                    argument_size,
                },
            );
        }
        Ok(self.kernels.get(name).expect("inserted Metal kernel"))
    }

    pub fn arguments(&mut self, name: &str) -> Result<Arguments, String> {
        let kernel = self.kernel(name)?;
        Ok(Arguments {
            layout: kernel.layout,
            bytes: vec![0; kernel.argument_size],
        })
    }

    pub fn upload(&self, bytes: &[u8]) -> Result<Buffer, String> {
        self.context.upload(bytes)
    }

    pub fn allocate(&self, length: usize) -> Result<Buffer, String> {
        self.context.allocate(length)
    }

    /// # Safety
    /// Every GPU pointer in `arguments` must refer to a retained entry in
    /// `resources`, with enough initialized storage for the kernel's accesses.
    pub unsafe fn dispatch(
        &mut self,
        arguments: Arguments,
        resources: Vec<Buffer>,
        extent: [usize; 3],
    ) -> Result<Submission, String> {
        if extent.contains(&0) {
            return Err("Metal dispatch dimensions must be nonzero".into());
        }
        let argument_buffer = self.upload(&arguments.bytes)?;
        let command = self
            .context
            .queue
            .commandBuffer()
            .ok_or("Could not create Metal compute command")?;
        let encoder = command
            .computeCommandEncoder()
            .ok_or("Could not create Metal compute encoder")?;
        let kernel = self.kernel(arguments.layout.name)?;
        encoder.setComputePipelineState(&kernel.pipeline);
        unsafe {
            encoder.setBuffer_offset_atIndex(Some(argument_buffer.metal()), 0, kernel.binding);
        }
        let mut resources = resources;
        resources.push(argument_buffer);
        for buffer in &resources {
            let resource: &ProtocolObject<dyn MTLResource> =
                ProtocolObject::from_ref(&**buffer.metal());
            encoder.useResource_usage(resource, MTLResourceUsage::Read | MTLResourceUsage::Write);
        }
        let [x, y, z] = kernel.layout.group;
        encoder.dispatchThreads_threadsPerThreadgroup(
            MTLSize {
                width: extent[0],
                height: extent[1],
                depth: extent[2],
            },
            MTLSize {
                width: x,
                height: y,
                depth: z,
            },
        );
        encoder.endEncoding();
        command.commit();
        Ok(Submission::new(command, resources))
    }

    /// Pixel operations are performed by render-core/shaders/preview.slang.
    /// Parameters use the shared device-buffer ABI; this method only installs GPU addresses.
    pub fn composite(
        &mut self,
        layers: &[(Nv12LayerParams, &[u8])],
        width: u32,
        height: u32,
        background: u32,
    ) -> Result<Frame, String> {
        let layers = layers
            .iter()
            .map(|(parameters, pixels)| self.upload(pixels).map(|buffer| (*parameters, buffer)))
            .collect::<Result<Vec<_>, _>>()?;
        self.composite_buffers(&layers, width, height, background)
    }

    /// Dispatch with resident source buffers, preserving cached decoded frames during scrubbing.
    pub fn composite_buffers(
        &mut self,
        layers: &[(Nv12LayerParams, Buffer)],
        width: u32,
        height: u32,
        background: u32,
    ) -> Result<Frame, String> {
        self.composite_buffers_with_transforms(layers, width, height, background, &[])
    }

    pub fn composite_buffers_with_transforms(
        &mut self,
        layers: &[(Nv12LayerParams, Buffer)],
        width: u32,
        height: u32,
        background: u32,
        transforms: &[shrimply_render_core::math::Mat3],
    ) -> Result<Frame, String> {
        u32::try_from(transforms.len())
            .map_err(|_| "Motion transform count exceeds the shared ABI")?;
        if transforms.iter().any(|transform| !transform.is_finite()) {
            return Err("Motion transforms must be finite".into());
        }
        let count = (width as usize)
            .checked_mul(height as usize)
            .ok_or("Canvas size overflow")?;
        u32::try_from(count).map_err(|_| "Canvas exceeds the shared kernels' pixel index range")?;
        let output = self.allocate(
            count
                .checked_mul(size_of::<u32>())
                .ok_or("Canvas byte size overflow")?,
        )?;
        let mut resources = vec![output.clone()];
        let mut parameters = Vec::with_capacity(layers.len());
        for (params, buffer) in layers {
            if params.opacity.clamp(0.0, 1.0) <= 0.0 {
                continue;
            }
            if params.motion_sample_count > 0 && params.motion_transform_count == 0 {
                continue;
            }
            // Reject only when sampling is actually used. Earlier ordered
            // modifiers can replace it, and scrubbing uses the shared fallback.
            if matches!(
                params.sample_method,
                shrimply_render_core::VideoSampleMethod::Anime4k
                    | shrimply_render_core::VideoSampleMethod::Anime4kSrgan
            ) {
                return Err("Anime4K sampling is not yet connected to Metal".into());
            }
            let row_bytes = (params.source_width as usize)
                .checked_mul(size_of::<u32>())
                .ok_or("Source row size overflow")?;
            let required = params
                .rgba_pitch
                .checked_mul(params.source_height as usize)
                .ok_or("Source buffer size overflow")?;
            if !matches!(params.kind, shrimply_render_core::LayerKind::Rgba)
                || params.source_width == 0
                || params.source_height == 0
                || params.rgba_pitch < row_bytes
                || params.rgba_pitch % size_of::<u32>() != 0
                || required > buffer.metal().length()
                || params.motion_transform_count > params.motion_sample_count
                || params
                    .motion_transform_offset
                    .checked_add(params.motion_transform_count)
                    .is_none_or(|end| end as usize > transforms.len())
            {
                return Err("Invalid RGBA layer buffer for Metal composition".into());
            }
            let mut params = *params;
            params.rgba = buffer.address() as usize as *const u32;
            params.y_plane = std::ptr::null();
            params.uv_plane = std::ptr::null();
            params.canvas_width = width;
            parameters.push(params);
            resources.push(buffer.clone());
        }
        let layers_address = if parameters.is_empty() {
            0
        } else {
            // The shared ABI generator emits explicit padding, initialized by the caller.
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    parameters.as_ptr().cast(),
                    std::mem::size_of_val(parameters.as_slice()),
                )
            };
            let buffer = self.upload(bytes)?;
            let address = buffer.address();
            resources.push(buffer);
            address
        };
        let transforms_address = if transforms.is_empty() {
            0
        } else {
            // Slang's pointer-pointee Mat3 is three packed_float3 columns on Metal,
            // matching the 36-byte column-major host representation.
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    transforms.as_ptr().cast(),
                    std::mem::size_of_val(transforms),
                )
            };
            let buffer = self.upload(bytes)?;
            let address = buffer.address();
            resources.push(buffer);
            address
        };
        let mut arguments = self.arguments("composite_nv12_layers")?;
        arguments
            .set("layers", &layers_address.to_ne_bytes())?
            .set("layer_count", &(parameters.len() as u64).to_ne_bytes())?
            .set("transforms", &transforms_address.to_ne_bytes())?
            .set("transform_count", &(transforms.len() as u64).to_ne_bytes())?
            .set("output", &output.address().to_ne_bytes())?
            .set("output_count", &(count as u64).to_ne_bytes())?
            .set("background", &background.to_ne_bytes())?;
        // All indirect addresses refer to the validated buffers retained above.
        let submission = unsafe { self.dispatch(arguments, resources, [count, 1, 1])? };
        Ok(Frame { submission, output })
    }
}
