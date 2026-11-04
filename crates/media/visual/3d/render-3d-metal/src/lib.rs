#![cfg(target_os = "macos")]

use std::{ffi::c_void, mem::size_of, ptr::NonNull};

use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_foundation::{NSArray, NSString};
use objc2_metal::{
    MTLAccelerationStructure, MTLAccelerationStructureCommandEncoder,
    MTLAccelerationStructureInstanceDescriptor, MTLAccelerationStructureInstanceOptions,
    MTLAccelerationStructureTriangleGeometryDescriptor, MTLAttributeFormat, MTLBarrierScope,
    MTLBlitCommandEncoder, MTLBuffer, MTLCommandBuffer, MTLCommandBufferStatus, MTLCommandEncoder,
    MTLCommandQueue, MTLComputeCommandEncoder, MTLComputePipelineState, MTLDevice,
    MTLInstanceAccelerationStructureDescriptor, MTLLibrary, MTLOrigin, MTLPackedFloat3,
    MTLPackedFloat4x3, MTLPixelFormat, MTLPrimitiveAccelerationStructureDescriptor, MTLRegion,
    MTLResource, MTLResourceOptions, MTLResourceUsage, MTLSamplerAddressMode, MTLSamplerDescriptor,
    MTLSamplerMinMagFilter, MTLSamplerMipFilter, MTLSamplerState, MTLSize, MTLStorageMode,
    MTLTexture, MTLTextureDescriptor, MTLTextureType, MTLTextureUsage,
};

include!(concat!(env!("OUT_DIR"), "/obj_metal.rs"));

pub struct Rendered {
    pub buffer: shrimply_render_metal::Buffer,
    pub row_bytes: usize,
}

#[derive(Clone, Copy)]
pub struct CompositeBackground<'a> {
    pub buffer: &'a shrimply_render_metal::Buffer,
    pub width: u32,
    pub height: u32,
}

struct UploadedGeometry {
    identity: shrimply_render_3d_core::GeometryIdentity,
    positions: shrimply_render_metal::Buffer,
    normals: shrimply_render_metal::Buffer,
    tangents: shrimply_render_metal::Buffer,
    tex_coords_0: shrimply_render_metal::Buffer,
    tex_coords_1: shrimply_render_metal::Buffer,
    vertex_colors: shrimply_render_metal::Buffer,
    material_texture: Retained<ProtocolObject<dyn MTLTexture>>,
    blases: Vec<Retained<ProtocolObject<dyn MTLAccelerationStructure>>>,
}

struct UploadedEnvironment {
    identity: shrimply_asset::AssetSnapshot,
    texture: Retained<ProtocolObject<dyn MTLTexture>>,
}

pub struct Renderer {
    _library: Retained<ProtocolObject<dyn MTLLibrary>>,
    pipeline: Retained<ProtocolObject<dyn MTLComputePipelineState>>,
    composite_upload_pipeline: Retained<ProtocolObject<dyn MTLComputePipelineState>>,
    denoise_composite_pipeline: Retained<ProtocolObject<dyn MTLComputePipelineState>>,
    outline_distance_pipeline: Retained<ProtocolObject<dyn MTLComputePipelineState>>,
    outline_pipeline: Retained<ProtocolObject<dyn MTLComputePipelineState>>,
    material_sampler: Retained<ProtocolObject<dyn MTLSamplerState>>,
    environment_sampler: Retained<ProtocolObject<dyn MTLSamplerState>>,
    fallback_environment: Retained<ProtocolObject<dyn MTLTexture>>,
    denoiser: shrimply_denoise_metal::Denoiser,
    environment: Option<UploadedEnvironment>,
    uploaded: Option<UploadedGeometry>,
}

#[repr(C)]
struct OutputSize {
    width: u32,
    height: u32,
    denoising: u32,
    guides: u32,
}

#[repr(C, align(16))]
struct ComputeMaterial {
    base_color: [f32; 4],
    metallic_roughness_normal_alpha: [f32; 4],
    flags: [u32; 4],
    base_color_texture: shrimply_render_3d_core::obj::TextureMapping,
    metallic_roughness_texture: shrimply_render_3d_core::obj::TextureMapping,
    normal_texture: shrimply_render_3d_core::obj::TextureMapping,
    surface: [u32; 4],
    pbr: ComputePbr,
}

#[repr(C)]
struct ComputePbr {
    modes: [u32; 4],
    surface: [f32; 4],
    ior: f32,
    padding: [f32; 3],
}

impl Renderer {
    pub fn new(compute: &shrimply_render_metal::Renderer) -> Result<Self, String> {
        if !compute.supports_ray_tracing() {
            return Err("OBJ rendering requires Metal ray tracing".to_string());
        }
        let device = compute.device();
        let library = device
            .newLibraryWithSource_options_error(&NSString::from_str(obj_metal::METAL_SOURCE), None)
            .map_err(|error| format!("Compile shared OBJ Metal shader: {error}"))?;
        let pipeline = |entry: &str| {
            let function = library
                .newFunctionWithName(&NSString::from_str(entry))
                .ok_or_else(|| format!("Shared OBJ Metal shader omitted {entry}"))?;
            device
                .newComputePipelineStateWithFunction_error(&function)
                .map_err(|error| format!("Create shared OBJ Metal pipeline {entry}: {error}"))
        };
        let main_pipeline = pipeline(obj_metal::OBJ_COMPUTE_ENTRY_POINT)?;
        let composite_upload_pipeline =
            pipeline(obj_metal::OBJ_COMPOSITE_UPLOAD_COMPUTE_ENTRY_POINT)?;
        let denoise_composite_pipeline =
            pipeline(obj_metal::OBJ_DENOISE_COMPOSITE_COMPUTE_ENTRY_POINT)?;
        let outline_distance_pipeline =
            pipeline(obj_metal::OBJ_OUTLINE_DISTANCE_COMPUTE_ENTRY_POINT)?;
        let outline_pipeline = pipeline(obj_metal::OBJ_OUTLINE_COMPUTE_ENTRY_POINT)?;
        let sampler_descriptor = MTLSamplerDescriptor::new();
        sampler_descriptor.setMinFilter(MTLSamplerMinMagFilter::Linear);
        sampler_descriptor.setMagFilter(MTLSamplerMinMagFilter::Linear);
        let material_sampler = device
            .newSamplerStateWithDescriptor(&sampler_descriptor)
            .ok_or("Could not create OBJ material sampler")?;
        let environment_sampler_descriptor = MTLSamplerDescriptor::new();
        environment_sampler_descriptor.setMinFilter(MTLSamplerMinMagFilter::Linear);
        environment_sampler_descriptor.setMagFilter(MTLSamplerMinMagFilter::Linear);
        environment_sampler_descriptor.setMipFilter(MTLSamplerMipFilter::Linear);
        environment_sampler_descriptor.setSAddressMode(MTLSamplerAddressMode::Repeat);
        environment_sampler_descriptor.setTAddressMode(MTLSamplerAddressMode::ClampToEdge);
        let environment_sampler = device
            .newSamplerStateWithDescriptor(&environment_sampler_descriptor)
            .ok_or("Could not create OBJ environment sampler")?;
        let fallback_environment =
            upload_float_texture(compute, 1, 1, &[[0.0_f32, 0.0_f32, 0.0_f32, 1.0_f32]])?;
        let denoiser = shrimply_denoise_metal::Denoiser::new(device);
        Ok(Self {
            _library: library,
            pipeline: main_pipeline,
            composite_upload_pipeline,
            denoise_composite_pipeline,
            outline_distance_pipeline,
            outline_pipeline,
            material_sampler,
            environment_sampler,
            fallback_environment,
            denoiser,
            environment: None,
            uploaded: None,
        })
    }

    pub fn render(
        &mut self,
        metal: &shrimply_render_metal::Renderer,
        plan: &shrimply_render_3d_core::PreparedFrame,
        composite_background: Option<CompositeBackground<'_>>,
    ) -> Result<Rendered, String> {
        let width = plan.width.max(1);
        let height = plan.height.max(1);
        if !matches!(
            plan.params.denoising,
            shrimply_scene_3d::DenoisingMethod::Off
                | shrimply_scene_3d::DenoisingMethod::Auto
                | shrimply_scene_3d::DenoisingMethod::MetalSvgf
        ) {
            return Err(
                "Selected denoiser is unavailable on Metal; select Auto or Metal SVGF".into(),
            );
        }
        let denoising = plan.params.denoising != shrimply_scene_3d::DenoisingMethod::Off
            && plan.params.shading_model == shrimply_render_3d_core::obj::ShadingModel::Pbr;
        if denoising && plan.uniforms.camera_lens[1] > 0.0 {
            return Err(
                "Metal SVGF denoising does not support stochastic depth of field; disable denoising or set focus distance to 0"
                    .to_string(),
            );
        }
        self.ensure_geometry(metal, &plan.session)?;
        self.ensure_environment(metal, plan)?;
        let row_bytes = usize::try_from(width)
            .map_err(|_| "OBJ width exceeds Metal limits")?
            .checked_mul(size_of::<u32>())
            .ok_or("OBJ output row size overflow")?;
        let output_bytes = row_bytes
            .checked_mul(usize::try_from(height).map_err(|_| "OBJ height exceeds Metal limits")?)
            .ok_or("OBJ output size overflow")?;
        let pixel_count = usize::try_from(u64::from(width) * u64::from(height))
            .map_err(|_| "OBJ output dimensions exceed Metal limits")?;
        let region_boundary = plan.params.toon_outline_mode
            != shrimply_render_3d_core::obj::OutlineMode::Off
            && plan.params.toon_outline_method
                == shrimply_render_3d_core::obj::OutlineMethod::RegionBoundary;
        let outline_elements = if region_boundary { pixel_count } else { 1 };
        let outline_guide = metal.allocate(
            outline_elements
                .checked_mul(size_of::<[f32; 4]>())
                .ok_or("OBJ outline guide size overflow")?,
        )?;
        let outline_distance = metal.allocate(
            outline_elements
                .checked_mul(size_of::<[f32; 2]>())
                .ok_or("OBJ outline distance size overflow")?,
        )?;
        let mut uniforms = plan.uniforms;
        uniforms.transmission_background[1] = u32::from(composite_background.is_some()) as f32;
        let scene = metal.upload(bytes_of(&uniforms))?;
        let output_size = metal.upload(bytes_of(&OutputSize {
            width,
            height,
            denoising: u32::from(denoising),
            guides: 0,
        }))?;
        let (composite_width, composite_height) =
            composite_background.map_or((1, 1), |background| (background.width, background.height));
        let composite_texture =
            allocate_composite_texture(metal.device(), composite_width, composite_height)?;
        let composite_size = composite_background
            .map(|background| {
                let expected =
                    usize::try_from(u64::from(background.width) * u64::from(background.height))
                        .map_err(|_| "OBJ composite background dimensions exceed Metal limits")?
                        .checked_mul(size_of::<u32>())
                        .ok_or("OBJ composite background size overflow")?;
                if background.width == 0
                    || background.height == 0
                    || background.buffer.metal().length() < expected
                {
                    return Err("OBJ composite background buffer is too small".to_string());
                }
                metal.upload(bytes_of(&OutputSize {
                    width: background.width,
                    height: background.height,
                    denoising: 0,
                    guides: 0,
                }))
            })
            .transpose()?;
        let compute_materials = plan
            .session
            .materials()
            .iter()
            .map(|material| ComputeMaterial {
                base_color: material.base_color_factor,
                metallic_roughness_normal_alpha: material.metallic_roughness_normal_alpha,
                flags: material.flags,
                base_color_texture: material.base_color_texture,
                metallic_roughness_texture: material.metallic_roughness_texture,
                normal_texture: material.normal_texture,
                surface: material.surface,
                pbr: ComputePbr {
                    modes: [
                        material.pbr.path_tracing as u32,
                        material.pbr.light_sampling_quality as u32,
                        material.pbr.render_quality as u32,
                        material.pbr.denoising,
                    ],
                    surface: [
                        material.pbr.subsurface,
                        material.pbr.clearcoat,
                        material.pbr.sheen,
                        material.pbr.transmission,
                    ],
                    ior: material.pbr.ior,
                    padding: material.pbr._padding,
                },
            })
            .collect::<Vec<_>>();
        let materials = metal.upload(bytes_slice(&compute_materials))?;
        let mesh_instances = metal.upload(bytes_slice(plan.session.mesh_instances()))?;
        let output = metal.allocate(output_bytes)?;
        let denoiser_width = if denoising { width } else { 1 };
        let denoiser_height = if denoising { height } else { 1 };
        let denoiser_targets = shrimply_denoise_metal::Targets::new(
            metal.device(),
            [denoiser_width, denoiser_height],
        )?;
        let instance_descriptors = plan
            .session
            .acceleration_instances()
            .iter()
            .map(|instance| MTLAccelerationStructureInstanceDescriptor {
                transformationMatrix: packed_transform(instance.transform),
                options: MTLAccelerationStructureInstanceOptions::None,
                mask: u32::from(u8::MAX),
                intersectionFunctionTableOffset: 0,
                accelerationStructureIndex: instance.geometry_index,
            })
            .collect::<Vec<_>>();
        let instance_buffer = metal.upload(bytes_slice(&instance_descriptors))?;
        let uploaded = self.uploaded.as_ref().expect("OBJ geometry uploaded");
        let tlas_descriptor = MTLInstanceAccelerationStructureDescriptor::new();
        tlas_descriptor.setInstanceDescriptorBuffer(Some(instance_buffer.metal()));
        tlas_descriptor.setInstanceCount(instance_descriptors.len());
        let blases = NSArray::from_retained_slice(&uploaded.blases);
        tlas_descriptor.setInstancedAccelerationStructures(Some(&blases));
        let tlas = allocate_acceleration(metal.device(), &tlas_descriptor)?;
        let tlas_scratch = scratch_buffer(metal.device(), tlas.scratch_size)?;
        let command = metal
            .queue()
            .commandBuffer()
            .ok_or("Could not create OBJ Metal command buffer")?;
        let acceleration_encoder = command
            .accelerationStructureCommandEncoder()
            .ok_or("Could not create OBJ Metal acceleration encoder")?;
        acceleration_encoder
            .buildAccelerationStructure_descriptor_scratchBuffer_scratchBufferOffset(
                &tlas.structure,
                &tlas_descriptor,
                &tlas_scratch,
                0,
            );
        acceleration_encoder.endEncoding();
        if let Some(background) = composite_background {
            let composite_size = composite_size
                .as_ref()
                .expect("composite background size was uploaded");
            let composite_encoder = command
                .computeCommandEncoder()
                .ok_or("Could not create OBJ composite upload encoder")?;
            composite_encoder.setComputePipelineState(&self.composite_upload_pipeline);
            unsafe {
                composite_encoder.setBuffer_offset_atIndex(
                    Some(composite_size.metal()),
                    0,
                    obj_metal::COMPOSITE_SIZE_BINDING,
                );
                composite_encoder.setBuffer_offset_atIndex(
                    Some(background.buffer.metal()),
                    0,
                    obj_metal::COMPOSITE_PIXELS_BINDING,
                );
                composite_encoder.setTexture_atIndex(
                    Some(&composite_texture),
                    obj_metal::COMPOSITE_TEXTURE_OUTPUT_BINDING,
                );
            }
            let source_resource: &ProtocolObject<dyn MTLResource> =
                ProtocolObject::from_ref(&**background.buffer.metal());
            composite_encoder.useResource_usage(source_resource, MTLResourceUsage::Read);
            let size_resource: &ProtocolObject<dyn MTLResource> =
                ProtocolObject::from_ref(&**composite_size.metal());
            composite_encoder.useResource_usage(size_resource, MTLResourceUsage::Read);
            let texture_resource: &ProtocolObject<dyn MTLResource> =
                ProtocolObject::from_ref(&*composite_texture);
            composite_encoder.useResource_usage(texture_resource, MTLResourceUsage::Write);
            composite_encoder.dispatchThreads_threadsPerThreadgroup(
                MTLSize {
                    width: background.width as usize,
                    height: background.height as usize,
                    depth: 1,
                },
                MTLSize {
                    width: obj_metal::OBJ_COMPOSITE_UPLOAD_COMPUTE_THREAD_GROUP[0],
                    height: obj_metal::OBJ_COMPOSITE_UPLOAD_COMPUTE_THREAD_GROUP[1],
                    depth: obj_metal::OBJ_COMPOSITE_UPLOAD_COMPUTE_THREAD_GROUP[2],
                },
            );
            composite_encoder.endEncoding();
            let blit = command
                .blitCommandEncoder()
                .ok_or("Could not create OBJ composite mipmap encoder")?;
            blit.generateMipmapsForTexture(&composite_texture);
            blit.endEncoding();
        }
        let encoder = command
            .computeCommandEncoder()
            .ok_or("Could not create OBJ Metal compute encoder")?;
        encoder.setComputePipelineState(&self.pipeline);
        unsafe {
            encoder.setBuffer_offset_atIndex(Some(scene.metal()), 0, obj_metal::SCENE_BINDING);
            encoder.setBuffer_offset_atIndex(
                Some(output_size.metal()),
                0,
                obj_metal::OUTPUT_SIZE_BINDING,
            );
            encoder.setAccelerationStructure_atBufferIndex(
                Some(&tlas.structure),
                obj_metal::SCENE_ACCELERATION_BINDING,
            );
            encoder.setBuffer_offset_atIndex(
                Some(uploaded.positions.metal()),
                0,
                obj_metal::POSITIONS_BINDING,
            );
            encoder.setBuffer_offset_atIndex(
                Some(uploaded.normals.metal()),
                0,
                obj_metal::NORMALS_BINDING,
            );
            encoder.setBuffer_offset_atIndex(
                Some(uploaded.tangents.metal()),
                0,
                obj_metal::TANGENTS_BINDING,
            );
            encoder.setBuffer_offset_atIndex(
                Some(uploaded.tex_coords_0.metal()),
                0,
                obj_metal::TEX_COORDS_0_BINDING,
            );
            encoder.setBuffer_offset_atIndex(
                Some(uploaded.tex_coords_1.metal()),
                0,
                obj_metal::TEX_COORDS_1_BINDING,
            );
            encoder.setBuffer_offset_atIndex(
                Some(uploaded.vertex_colors.metal()),
                0,
                obj_metal::VERTEX_COLORS_BINDING,
            );
            encoder.setBuffer_offset_atIndex(
                Some(materials.metal()),
                0,
                obj_metal::MATERIALS_BINDING,
            );
            encoder.setTexture_atIndex(
                Some(&uploaded.material_texture),
                obj_metal::MATERIAL_TEXTURE_BINDING,
            );
            encoder.setSamplerState_atIndex(
                Some(&self.material_sampler),
                obj_metal::MATERIAL_SAMPLER_BINDING,
            );
            encoder.setTexture_atIndex(
                Some(
                    self.environment
                        .as_ref()
                        .map_or(self.fallback_environment.as_ref(), |environment| {
                            environment.texture.as_ref()
                        }),
                ),
                obj_metal::ENVIRONMENT_TEXTURE_BINDING,
            );
            encoder.setSamplerState_atIndex(
                Some(&self.environment_sampler),
                obj_metal::ENVIRONMENT_SAMPLER_BINDING,
            );
            encoder.setTexture_atIndex(
                Some(&composite_texture),
                obj_metal::COMPOSITE_TEXTURE_BINDING,
            );
            encoder.setBuffer_offset_atIndex(
                Some(mesh_instances.metal()),
                0,
                obj_metal::MESH_INSTANCES_BINDING,
            );
            encoder.setBuffer_offset_atIndex(
                Some(output.metal()),
                0,
                obj_metal::OUTPUT_PIXELS_BINDING,
            );
            encoder.setBuffer_offset_atIndex(
                Some(outline_guide.metal()),
                0,
                obj_metal::OUTLINE_GUIDE_BINDING,
            );
            encoder.setBuffer_offset_atIndex(
                Some(outline_distance.metal()),
                0,
                obj_metal::OUTLINE_DISTANCE_BINDING,
            );
            encoder.setTexture_atIndex(
                Some(&denoiser_targets.beauty),
                obj_metal::DENOISER_BEAUTY_BINDING,
            );
            encoder.setTexture_atIndex(
                Some(&denoiser_targets.background),
                obj_metal::DENOISER_BACKGROUND_BINDING,
            );
            encoder.setTexture_atIndex(
                Some(&denoiser_targets.alpha),
                obj_metal::DENOISER_ALPHA_BINDING,
            );
            encoder.setTexture_atIndex(
                Some(&denoiser_targets.depth_normal),
                obj_metal::DENOISER_DEPTH_NORMAL_BINDING,
            );
        }
        for buffer in [
            &scene,
            &output_size,
            &uploaded.positions,
            &uploaded.normals,
            &uploaded.tangents,
            &uploaded.tex_coords_0,
            &uploaded.tex_coords_1,
            &uploaded.vertex_colors,
            &materials,
            &mesh_instances,
            &output,
            &outline_guide,
            &outline_distance,
        ] {
            let resource: &ProtocolObject<dyn MTLResource> =
                ProtocolObject::from_ref(&**buffer.metal());
            encoder.useResource_usage(resource, MTLResourceUsage::Read | MTLResourceUsage::Write);
        }
        let material_texture_resource: &ProtocolObject<dyn MTLResource> =
            ProtocolObject::from_ref(&*uploaded.material_texture);
        encoder.useResource_usage(material_texture_resource, MTLResourceUsage::Read);
        let environment_texture: &ProtocolObject<dyn MTLTexture> = self
            .environment
            .as_ref()
            .map_or(&*self.fallback_environment, |environment| {
                &*environment.texture
            });
        let environment_texture_resource: &ProtocolObject<dyn MTLResource> =
            ProtocolObject::from_ref(environment_texture);
        encoder.useResource_usage(environment_texture_resource, MTLResourceUsage::Read);
        let composite_texture_resource: &ProtocolObject<dyn MTLResource> =
            ProtocolObject::from_ref(&*composite_texture);
        encoder.useResource_usage(composite_texture_resource, MTLResourceUsage::Read);
        let denoiser_beauty_resource: &ProtocolObject<dyn MTLResource> =
            ProtocolObject::from_ref(&*denoiser_targets.beauty);
        encoder.useResource_usage(
            denoiser_beauty_resource,
            MTLResourceUsage::Read | MTLResourceUsage::Write,
        );
        let denoiser_background_resource: &ProtocolObject<dyn MTLResource> =
            ProtocolObject::from_ref(&*denoiser_targets.background);
        encoder.useResource_usage(
            denoiser_background_resource,
            MTLResourceUsage::Read | MTLResourceUsage::Write,
        );
        let denoiser_alpha_resource: &ProtocolObject<dyn MTLResource> =
            ProtocolObject::from_ref(&*denoiser_targets.alpha);
        encoder.useResource_usage(
            denoiser_alpha_resource,
            MTLResourceUsage::Read | MTLResourceUsage::Write,
        );
        let denoiser_depth_normal_resource: &ProtocolObject<dyn MTLResource> =
            ProtocolObject::from_ref(&*denoiser_targets.depth_normal);
        encoder.useResource_usage(
            denoiser_depth_normal_resource,
            MTLResourceUsage::Read | MTLResourceUsage::Write,
        );
        let tlas_resource: &ProtocolObject<dyn MTLResource> =
            ProtocolObject::from_ref(&*tlas.structure);
        encoder.useResource_usage(tlas_resource, MTLResourceUsage::Read);
        for blas in &uploaded.blases {
            let resource: &ProtocolObject<dyn MTLResource> = ProtocolObject::from_ref(&**blas);
            encoder.useResource_usage(resource, MTLResourceUsage::Read);
        }
        encoder.dispatchThreads_threadsPerThreadgroup(
            MTLSize {
                width: width as usize,
                height: height as usize,
                depth: 1,
            },
            MTLSize {
                width: obj_metal::OBJ_COMPUTE_THREAD_GROUP[0],
                height: obj_metal::OBJ_COMPUTE_THREAD_GROUP[1],
                depth: obj_metal::OBJ_COMPUTE_THREAD_GROUP[2],
            },
        );
        if region_boundary {
            encoder.memoryBarrierWithScope(MTLBarrierScope::Buffers | MTLBarrierScope::Textures);
            encoder.setComputePipelineState(&self.outline_distance_pipeline);
            encoder.dispatchThreads_threadsPerThreadgroup(
                MTLSize {
                    width: width as usize,
                    height: height as usize,
                    depth: 1,
                },
                MTLSize {
                    width: obj_metal::OBJ_OUTLINE_DISTANCE_COMPUTE_THREAD_GROUP[0],
                    height: obj_metal::OBJ_OUTLINE_DISTANCE_COMPUTE_THREAD_GROUP[1],
                    depth: obj_metal::OBJ_OUTLINE_DISTANCE_COMPUTE_THREAD_GROUP[2],
                },
            );
            encoder.memoryBarrierWithScope(MTLBarrierScope::Buffers | MTLBarrierScope::Textures);
            encoder.setComputePipelineState(&self.outline_pipeline);
            encoder.dispatchThreads_threadsPerThreadgroup(
                MTLSize {
                    width: width as usize,
                    height: height as usize,
                    depth: 1,
                },
                MTLSize {
                    width: obj_metal::OBJ_OUTLINE_COMPUTE_THREAD_GROUP[0],
                    height: obj_metal::OBJ_OUTLINE_COMPUTE_THREAD_GROUP[1],
                    depth: obj_metal::OBJ_OUTLINE_COMPUTE_THREAD_GROUP[2],
                },
            );
        }
        encoder.endEncoding();
        if denoising {
            let shrimply_denoise_metal::Output {
                beauty: denoised,
                background: denoised_background,
                alpha: denoised_alpha,
            } = self.denoiser.encode(&command, &denoiser_targets);
            let composite_encoder = command
                .computeCommandEncoder()
                .ok_or("Could not create OBJ MPS denoiser output encoder")?;
            composite_encoder.setComputePipelineState(&self.denoise_composite_pipeline);
            unsafe {
                composite_encoder.setBuffer_offset_atIndex(
                    Some(output_size.metal()),
                    0,
                    obj_metal::OUTPUT_SIZE_BINDING,
                );
                composite_encoder.setBuffer_offset_atIndex(
                    Some(output.metal()),
                    0,
                    obj_metal::OUTPUT_PIXELS_BINDING,
                );
                composite_encoder.setTexture_atIndex(
                    Some(&denoiser_targets.beauty),
                    obj_metal::DENOISER_BEAUTY_BINDING,
                );
                composite_encoder
                    .setTexture_atIndex(Some(&denoised), obj_metal::DENOISED_TEXTURE_BINDING);
                composite_encoder.setTexture_atIndex(
                    Some(&denoised_background),
                    obj_metal::DENOISED_BACKGROUND_TEXTURE_BINDING,
                );
                composite_encoder.setTexture_atIndex(
                    Some(&denoised_alpha),
                    obj_metal::DENOISED_ALPHA_TEXTURE_BINDING,
                );
            }
            let output_resource: &ProtocolObject<dyn MTLResource> =
                ProtocolObject::from_ref(&**output.metal());
            composite_encoder.useResource_usage(output_resource, MTLResourceUsage::Write);
            composite_encoder.useResource_usage(denoiser_beauty_resource, MTLResourceUsage::Read);
            let denoised_resource: &ProtocolObject<dyn MTLResource> =
                ProtocolObject::from_ref(&*denoised);
            composite_encoder.useResource_usage(denoised_resource, MTLResourceUsage::Read);
            let denoised_background_resource: &ProtocolObject<dyn MTLResource> =
                ProtocolObject::from_ref(&*denoised_background);
            composite_encoder
                .useResource_usage(denoised_background_resource, MTLResourceUsage::Read);
            let denoised_alpha_resource: &ProtocolObject<dyn MTLResource> =
                ProtocolObject::from_ref(&*denoised_alpha);
            composite_encoder.useResource_usage(denoised_alpha_resource, MTLResourceUsage::Read);
            composite_encoder.dispatchThreads_threadsPerThreadgroup(
                MTLSize {
                    width: width as usize,
                    height: height as usize,
                    depth: 1,
                },
                MTLSize {
                    width: obj_metal::OBJ_DENOISE_COMPOSITE_COMPUTE_THREAD_GROUP[0],
                    height: obj_metal::OBJ_DENOISE_COMPOSITE_COMPUTE_THREAD_GROUP[1],
                    depth: obj_metal::OBJ_DENOISE_COMPOSITE_COMPUTE_THREAD_GROUP[2],
                },
            );
            composite_encoder.endEncoding();
        }
        command.commit();
        command.waitUntilCompleted();
        if command.status() == MTLCommandBufferStatus::Error {
            return Err(command.error().map_or_else(
                || "OBJ Metal command failed".to_string(),
                |error| format!("OBJ Metal command failed: {error}"),
            ));
        }
        Ok(Rendered {
            buffer: output,
            row_bytes,
        })
    }

    fn ensure_geometry(
        &mut self,
        metal: &shrimply_render_metal::Renderer,
        session: &shrimply_render_3d_core::ObjRenderSession,
    ) -> Result<(), String> {
        if self
            .uploaded
            .as_ref()
            .is_some_and(|uploaded| &uploaded.identity == session.geometry_identity())
        {
            return Ok(());
        }
        let positions = metal.upload(bytes_slice(session.positions()))?;
        let normals = metal.upload(bytes_slice(session.normals()))?;
        let tangents = metal.upload(bytes_slice(session.tangents()))?;
        let tex_coords_0 = metal.upload(bytes_slice(session.tex_coords_0()))?;
        let tex_coords_1 = metal.upload(bytes_slice(session.tex_coords_1()))?;
        let vertex_colors = session
            .colors()
            .iter()
            .map(|color| color.to_array())
            .collect::<Vec<_>>();
        let vertex_colors = metal.upload(bytes_slice(&vertex_colors))?;
        let material_texture = upload_material_texture(metal.device(), session.texture_atlas())?;
        let mut descriptors = Vec::with_capacity(session.geometries().len());
        for geometry in session.geometries() {
            let mut triangles = Vec::with_capacity(geometry.geometry_count as usize);
            for slot in 0..geometry.geometry_count as usize {
                let descriptor = MTLAccelerationStructureTriangleGeometryDescriptor::new();
                descriptor.setVertexBuffer(Some(positions.metal()));
                unsafe {
                    descriptor.setVertexBufferOffset(
                        geometry.vertex_offsets[slot] as usize * size_of::<[f32; 4]>(),
                    );
                }
                descriptor.setVertexStride(size_of::<[f32; 4]>());
                descriptor.setVertexFormat(MTLAttributeFormat::Float3);
                descriptor.setTriangleCount(geometry.primitive_counts[slot] as usize);
                descriptor.setOpaque(geometry.opaque[slot]);
                triangles.push(descriptor.into_super());
            }
            let descriptor = MTLPrimitiveAccelerationStructureDescriptor::new();
            descriptor.setGeometryDescriptors(Some(&NSArray::from_retained_slice(&triangles)));
            descriptors.push(descriptor);
        }
        let command = metal
            .queue()
            .commandBuffer()
            .ok_or("Could not create OBJ Metal BLAS command buffer")?;
        let encoder = command
            .accelerationStructureCommandEncoder()
            .ok_or("Could not create OBJ Metal BLAS encoder")?;
        let mut blases = Vec::with_capacity(descriptors.len());
        let mut scratch = Vec::with_capacity(descriptors.len());
        for descriptor in &descriptors {
            let allocation = allocate_acceleration(metal.device(), descriptor)?;
            let buffer = scratch_buffer(metal.device(), allocation.scratch_size)?;
            encoder.buildAccelerationStructure_descriptor_scratchBuffer_scratchBufferOffset(
                &allocation.structure,
                descriptor,
                &buffer,
                0,
            );
            blases.push(allocation.structure);
            scratch.push(buffer);
        }
        encoder.endEncoding();
        command.commit();
        command.waitUntilCompleted();
        if command.status() == MTLCommandBufferStatus::Error {
            return Err(command.error().map_or_else(
                || "OBJ Metal BLAS build failed".to_string(),
                |error| format!("OBJ Metal BLAS build failed: {error}"),
            ));
        }
        self.uploaded = Some(UploadedGeometry {
            identity: session.geometry_identity().clone(),
            positions,
            normals,
            tangents,
            tex_coords_0,
            tex_coords_1,
            vertex_colors,
            material_texture,
            blases,
        });
        Ok(())
    }

    fn ensure_environment(
        &mut self,
        metal: &shrimply_render_metal::Renderer,
        plan: &shrimply_render_3d_core::PreparedFrame,
    ) -> Result<(), String> {
        if plan.params.environment_source != shrimply_scene_3d::EnvironmentSource::Image {
            self.environment = None;
            return Ok(());
        }
        let identity = plan
            .environment
            .as_ref()
            .ok_or("Image OBJ environment has no configured file")?;
        if self
            .environment
            .as_ref()
            .is_some_and(|environment| &environment.identity == identity)
        {
            return Ok(());
        }
        let decoded = shrimply_render_3d_core::load_environment(identity.path())
            .map_err(|error| error.to_string())?;
        identity.verify_current()?;
        let pixels = decoded
            .pixels
            .iter()
            .map(|color| color.to_array())
            .collect::<Vec<_>>();
        self.environment = Some(UploadedEnvironment {
            identity: identity.clone(),
            texture: upload_float_texture(metal, decoded.width, decoded.height, &pixels)?,
        });
        Ok(())
    }
}

fn upload_material_texture(
    device: &ProtocolObject<dyn MTLDevice>,
    atlas: &shrimply_scene_3d::TextureAtlas,
) -> Result<Retained<ProtocolObject<dyn MTLTexture>>, String> {
    let pixels = atlas
        .pixels
        .iter()
        .map(|color| color.to_array())
        .collect::<Vec<_>>();
    let expected = usize::try_from(u64::from(atlas.width) * u64::from(atlas.height))
        .map_err(|_| "OBJ material atlas dimensions exceed Metal limits")?;
    if atlas.width == 0 || atlas.height == 0 || pixels.len() != expected {
        return Err("OBJ material atlas dimensions do not match its pixels".into());
    }
    let descriptor = MTLTextureDescriptor::new();
    descriptor.setTextureType(MTLTextureType::Type2D);
    descriptor.setPixelFormat(MTLPixelFormat::RGBA8Unorm);
    unsafe {
        descriptor.setWidth(atlas.width as usize);
        descriptor.setHeight(atlas.height as usize);
    }
    descriptor.setStorageMode(MTLStorageMode::Shared);
    descriptor.setUsage(MTLTextureUsage::ShaderRead);
    let texture = device
        .newTextureWithDescriptor(&descriptor)
        .ok_or("Could not allocate OBJ material texture")?;
    let bytes_per_row = usize::try_from(atlas.width)
        .map_err(|_| "OBJ material atlas width exceeds Metal limits")?
        .checked_mul(size_of::<[u8; 4]>())
        .ok_or("OBJ material atlas row size overflow")?;
    let bytes = pixels.as_ptr().cast_mut().cast::<c_void>();
    unsafe {
        texture.replaceRegion_mipmapLevel_withBytes_bytesPerRow(
            MTLRegion {
                origin: MTLOrigin { x: 0, y: 0, z: 0 },
                size: MTLSize {
                    width: atlas.width as usize,
                    height: atlas.height as usize,
                    depth: 1,
                },
            },
            0,
            NonNull::new(bytes).ok_or("OBJ material atlas has no pixels")?,
            bytes_per_row,
        );
    }
    Ok(texture)
}

fn allocate_composite_texture(
    device: &ProtocolObject<dyn MTLDevice>,
    width: u32,
    height: u32,
) -> Result<Retained<ProtocolObject<dyn MTLTexture>>, String> {
    if width == 0 || height == 0 {
        return Err("OBJ composite texture dimensions must be nonzero".into());
    }
    let descriptor = MTLTextureDescriptor::new();
    descriptor.setTextureType(MTLTextureType::Type2D);
    descriptor.setPixelFormat(MTLPixelFormat::RGBA8Unorm);
    unsafe {
        descriptor.setWidth(width as usize);
        descriptor.setHeight(height as usize);
        descriptor.setMipmapLevelCount(
            usize::try_from(u32::BITS - width.max(height).leading_zeros())
                .map_err(|_| "OBJ composite mip count exceeds Metal limits")?,
        );
    }
    descriptor.setStorageMode(MTLStorageMode::Private);
    descriptor.setUsage(MTLTextureUsage::ShaderRead | MTLTextureUsage::ShaderWrite);
    device
        .newTextureWithDescriptor(&descriptor)
        .ok_or_else(|| "Could not allocate OBJ composite texture".into())
}

fn upload_float_texture(
    metal: &shrimply_render_metal::Renderer,
    width: u32,
    height: u32,
    pixels: &[[f32; 4]],
) -> Result<Retained<ProtocolObject<dyn MTLTexture>>, String> {
    let expected = usize::try_from(u64::from(width) * u64::from(height))
        .map_err(|_| "OBJ environment dimensions exceed Metal limits")?;
    if width == 0 || height == 0 || pixels.len() != expected {
        return Err("OBJ environment dimensions do not match its pixels".into());
    }
    let descriptor = MTLTextureDescriptor::new();
    descriptor.setTextureType(MTLTextureType::Type2D);
    descriptor.setPixelFormat(MTLPixelFormat::RGBA32Float);
    unsafe {
        descriptor.setWidth(width as usize);
        descriptor.setHeight(height as usize);
        descriptor.setMipmapLevelCount(
            usize::try_from(u32::BITS - width.max(height).leading_zeros())
                .map_err(|_| "OBJ environment mip count exceeds Metal limits")?,
        );
    }
    descriptor.setStorageMode(MTLStorageMode::Shared);
    descriptor.setUsage(MTLTextureUsage::ShaderRead);
    let texture = metal
        .device()
        .newTextureWithDescriptor(&descriptor)
        .ok_or("Could not allocate OBJ environment texture")?;
    let bytes_per_row = usize::try_from(width)
        .map_err(|_| "OBJ environment width exceeds Metal limits")?
        .checked_mul(size_of::<[f32; 4]>())
        .ok_or("OBJ environment row size overflow")?;
    unsafe {
        texture.replaceRegion_mipmapLevel_withBytes_bytesPerRow(
            MTLRegion {
                origin: MTLOrigin { x: 0, y: 0, z: 0 },
                size: MTLSize {
                    width: width as usize,
                    height: height as usize,
                    depth: 1,
                },
            },
            0,
            NonNull::new(pixels.as_ptr().cast_mut().cast::<c_void>())
                .ok_or("OBJ environment has no pixels")?,
            bytes_per_row,
        );
    }
    if texture.mipmapLevelCount() > 1 {
        let command = metal
            .queue()
            .commandBuffer()
            .ok_or("Could not create OBJ environment mipmap command")?;
        let encoder = command
            .blitCommandEncoder()
            .ok_or("Could not create OBJ environment mipmap encoder")?;
        encoder.generateMipmapsForTexture(&texture);
        encoder.endEncoding();
        command.commit();
        command.waitUntilCompleted();
        if command.status() == MTLCommandBufferStatus::Error {
            return Err(command.error().map_or_else(
                || "OBJ environment mipmap generation failed".to_string(),
                |error| format!("OBJ environment mipmap generation failed: {error}"),
            ));
        }
    }
    Ok(texture)
}

struct AccelerationAllocation {
    structure: Retained<ProtocolObject<dyn MTLAccelerationStructure>>,
    scratch_size: usize,
}

fn allocate_acceleration(
    device: &ProtocolObject<dyn MTLDevice>,
    descriptor: &objc2_metal::MTLAccelerationStructureDescriptor,
) -> Result<AccelerationAllocation, String> {
    let sizes = device.accelerationStructureSizesWithDescriptor(descriptor);
    let structure = device
        .newAccelerationStructureWithSize(sizes.accelerationStructureSize)
        .ok_or("Could not allocate OBJ Metal acceleration structure")?;
    Ok(AccelerationAllocation {
        structure,
        scratch_size: sizes.buildScratchBufferSize,
    })
}

fn scratch_buffer(
    device: &ProtocolObject<dyn MTLDevice>,
    length: usize,
) -> Result<Retained<ProtocolObject<dyn objc2_metal::MTLBuffer>>, String> {
    device
        .newBufferWithLength_options(length, MTLResourceOptions::StorageModePrivate)
        .ok_or_else(|| "Could not allocate OBJ Metal acceleration scratch buffer".into())
}

fn packed_transform(matrix: [f32; 16]) -> MTLPackedFloat4x3 {
    MTLPackedFloat4x3 {
        columns: std::array::from_fn(|column| MTLPackedFloat3 {
            x: matrix[column * 4],
            y: matrix[column * 4 + 1],
            z: matrix[column * 4 + 2],
        }),
    }
}

fn bytes_of<T>(value: &T) -> &[u8] {
    unsafe { std::slice::from_raw_parts(std::ptr::from_ref(value).cast(), size_of::<T>()) }
}

fn bytes_slice<T>(values: &[T]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(values.as_ptr().cast(), std::mem::size_of_val(values)) }
}
