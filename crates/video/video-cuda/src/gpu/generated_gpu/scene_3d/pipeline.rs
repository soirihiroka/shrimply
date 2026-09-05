use std::time::Instant;

use super::*;

const RAYGEN_STAGE: u32 = 0;
const OUTLINE_DISTANCE_STAGE: u32 = 1;
const OUTLINE_STAGE: u32 = 2;
const PRIMARY_MISS_STAGE: u32 = 3;
const SHADOW_MISS_STAGE: u32 = 4;
const CLOSEST_HIT_STAGE: u32 = 5;
const ALPHA_ANY_HIT_STAGE: u32 = 6;
const SHADOW_ALPHA_ANY_HIT_STAGE: u32 = 7;

pub(super) struct ScenePipeline {
    vulkan: Arc<VulkanDevice>,
    descriptor_set_layout: vk::DescriptorSetLayout,
    descriptor_pool: vk::DescriptorPool,
    pub(super) descriptor_set: vk::DescriptorSet,
    pub(super) layout: vk::PipelineLayout,
    pub(super) handle: vk::Pipeline,
    shader_binding_table: Option<ShaderBindingTable>,
    pub(super) environment_sampler: vk::Sampler,
    pub(super) material_sampler: vk::Sampler,
}

impl ScenePipeline {
    pub(super) fn new(
        vulkan: Arc<VulkanDevice>,
        physical_device: vk::PhysicalDevice,
        ray_tracing: &ash::khr::ray_tracing_pipeline::Device,
    ) -> Result<Self, String> {
        let mut resources = Self {
            vulkan,
            descriptor_set_layout: vk::DescriptorSetLayout::null(),
            descriptor_pool: vk::DescriptorPool::null(),
            descriptor_set: vk::DescriptorSet::null(),
            layout: vk::PipelineLayout::null(),
            handle: vk::Pipeline::null(),
            shader_binding_table: None,
            environment_sampler: vk::Sampler::null(),
            material_sampler: vk::Sampler::null(),
        };
        let device = &resources.vulkan.device;
        resources.descriptor_set_layout = create_descriptor_set_layout(device)?;
        resources.layout = create_pipeline_layout(device, resources.descriptor_set_layout)?;
        resources.handle = create_ray_tracing_pipeline(
            device,
            ray_tracing,
            resources.vulkan.pipeline_cache,
            resources.layout,
            shrimply_render_3d::obj::SPIRV_BYTES,
        )?;
        resources.shader_binding_table = Some(ShaderBindingTable::new(
            resources.vulkan.clone(),
            physical_device,
            ray_tracing,
            resources.handle,
        )?);
        resources.descriptor_pool = create_descriptor_pool(device)?;
        resources.descriptor_set = allocate_descriptor_set(
            device,
            resources.descriptor_pool,
            resources.descriptor_set_layout,
        )?;
        resources.environment_sampler = create_sampler(
            device,
            vk::SamplerMipmapMode::LINEAR,
            vk::SamplerAddressMode::REPEAT,
            vk::LOD_CLAMP_NONE,
            "environment",
        )?;
        resources.material_sampler = create_sampler(
            device,
            vk::SamplerMipmapMode::NEAREST,
            vk::SamplerAddressMode::CLAMP_TO_EDGE,
            0.0,
            "material",
        )?;
        Ok(resources)
    }

    pub(super) fn shader_binding_table(&self) -> &ShaderBindingTable {
        self.shader_binding_table
            .as_ref()
            .expect("3D shader binding table is initialized")
    }
}

impl Drop for ScenePipeline {
    fn drop(&mut self) {
        unsafe {
            self.vulkan
                .device
                .destroy_sampler(self.material_sampler, None);
            self.vulkan
                .device
                .destroy_sampler(self.environment_sampler, None);
            self.vulkan.device.destroy_pipeline(self.handle, None);
            self.vulkan
                .device
                .destroy_pipeline_layout(self.layout, None);
            self.vulkan
                .device
                .destroy_descriptor_pool(self.descriptor_pool, None);
            self.vulkan
                .device
                .destroy_descriptor_set_layout(self.descriptor_set_layout, None);
        }
    }
}

fn vulkan_descriptor_type(kind: shrimply_render_3d::obj::DescriptorKind) -> vk::DescriptorType {
    use shrimply_render_3d::obj::DescriptorKind;
    match kind {
        DescriptorKind::UniformBuffer => vk::DescriptorType::UNIFORM_BUFFER,
        DescriptorKind::SampledImage => vk::DescriptorType::SAMPLED_IMAGE,
        DescriptorKind::Sampler => vk::DescriptorType::SAMPLER,
        DescriptorKind::AccelerationStructure => vk::DescriptorType::ACCELERATION_STRUCTURE_KHR,
        DescriptorKind::StorageImage => vk::DescriptorType::STORAGE_IMAGE,
        DescriptorKind::StorageBuffer => vk::DescriptorType::STORAGE_BUFFER,
    }
}

fn create_descriptor_set_layout(device: &ash::Device) -> Result<vk::DescriptorSetLayout, String> {
    let ray_stages = vk::ShaderStageFlags::RAYGEN_KHR
        | vk::ShaderStageFlags::MISS_KHR
        | vk::ShaderStageFlags::CLOSEST_HIT_KHR
        | vk::ShaderStageFlags::ANY_HIT_KHR;
    let bindings: Vec<_> = shrimply_render_3d::obj::DESCRIPTORS
        .iter()
        .map(|descriptor| {
            vk::DescriptorSetLayoutBinding::default()
                .binding(descriptor.binding)
                .descriptor_type(vulkan_descriptor_type(descriptor.kind))
                .descriptor_count(1)
                .stage_flags(ray_stages)
        })
        .collect();
    let info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
    unsafe { device.create_descriptor_set_layout(&info, None) }
        .map_err(|error| format!("create Vulkan 3D descriptor layout: {error:?}"))
}

fn create_pipeline_layout(
    device: &ash::Device,
    descriptor: vk::DescriptorSetLayout,
) -> Result<vk::PipelineLayout, String> {
    let sets = [descriptor];
    let info = vk::PipelineLayoutCreateInfo::default().set_layouts(&sets);
    unsafe { device.create_pipeline_layout(&info, None) }
        .map_err(|error| format!("create Vulkan 3D pipeline layout: {error:?}"))
}

fn general_group(shader: u32) -> vk::RayTracingShaderGroupCreateInfoKHR<'static> {
    vk::RayTracingShaderGroupCreateInfoKHR::default()
        .ty(vk::RayTracingShaderGroupTypeKHR::GENERAL)
        .general_shader(shader)
        .closest_hit_shader(vk::SHADER_UNUSED_KHR)
        .any_hit_shader(vk::SHADER_UNUSED_KHR)
        .intersection_shader(vk::SHADER_UNUSED_KHR)
}

fn triangle_group(
    closest_hit: u32,
    any_hit: u32,
) -> vk::RayTracingShaderGroupCreateInfoKHR<'static> {
    vk::RayTracingShaderGroupCreateInfoKHR::default()
        .ty(vk::RayTracingShaderGroupTypeKHR::TRIANGLES_HIT_GROUP)
        .general_shader(vk::SHADER_UNUSED_KHR)
        .closest_hit_shader(closest_hit)
        .any_hit_shader(any_hit)
        .intersection_shader(vk::SHADER_UNUSED_KHR)
}

fn create_ray_tracing_pipeline(
    device: &ash::Device,
    ray_tracing: &ash::khr::ray_tracing_pipeline::Device,
    pipeline_cache: vk::PipelineCache,
    layout: vk::PipelineLayout,
    spirv: &[u8],
) -> Result<vk::Pipeline, String> {
    let spirv = ash::util::read_spv(&mut std::io::Cursor::new(spirv))
        .map_err(|error| format!("decode 3D ray-tracing SPIR-V: {error}"))?;
    let module = unsafe {
        device.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&spirv), None)
    }
    .map_err(|error| format!("create Slang ray-tracing shader module: {error:?}"))?;
    let stages = [
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::RAYGEN_KHR)
            .module(module)
            .name(shrimply_render_3d::obj::RAYGEN_MAIN_ENTRY_POINT),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::RAYGEN_KHR)
            .module(module)
            .name(shrimply_render_3d::obj::OUTLINE_DISTANCE_MAIN_ENTRY_POINT),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::RAYGEN_KHR)
            .module(module)
            .name(shrimply_render_3d::obj::OUTLINE_MAIN_ENTRY_POINT),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::MISS_KHR)
            .module(module)
            .name(shrimply_render_3d::obj::PRIMARY_MISS_ENTRY_POINT),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::MISS_KHR)
            .module(module)
            .name(shrimply_render_3d::obj::SHADOW_MISS_ENTRY_POINT),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::CLOSEST_HIT_KHR)
            .module(module)
            .name(shrimply_render_3d::obj::CLOSEST_HIT_ENTRY_POINT),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::ANY_HIT_KHR)
            .module(module)
            .name(shrimply_render_3d::obj::ALPHA_ANY_HIT_ENTRY_POINT),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::ANY_HIT_KHR)
            .module(module)
            .name(shrimply_render_3d::obj::SHADOW_ALPHA_ANY_HIT_ENTRY_POINT),
    ];
    let groups = [
        general_group(RAYGEN_STAGE),
        general_group(OUTLINE_DISTANCE_STAGE),
        general_group(OUTLINE_STAGE),
        general_group(PRIMARY_MISS_STAGE),
        general_group(SHADOW_MISS_STAGE),
        triangle_group(CLOSEST_HIT_STAGE, ALPHA_ANY_HIT_STAGE),
        triangle_group(vk::SHADER_UNUSED_KHR, SHADOW_ALPHA_ANY_HIT_STAGE),
    ];
    let info = vk::RayTracingPipelineCreateInfoKHR::default()
        .flags(
            vk::PipelineCreateFlags::RAY_TRACING_SKIP_AABBS_KHR
                | vk::PipelineCreateFlags::RAY_TRACING_NO_NULL_ANY_HIT_SHADERS_KHR
                | vk::PipelineCreateFlags::RAY_TRACING_NO_NULL_MISS_SHADERS_KHR,
        )
        .stages(&stages)
        .groups(&groups)
        .max_pipeline_ray_recursion_depth(PATH_TRACING_RECURSION_DEPTH)
        .layout(layout);
    tracing::debug!(
        stages = stages.len(),
        groups = groups.len(),
        "Creating Vulkan 3D ray-tracing pipeline from SPIR-V"
    );
    let started = Instant::now();
    let pipeline = unsafe {
        ray_tracing.create_ray_tracing_pipelines(
            vk::DeferredOperationKHR::null(),
            pipeline_cache,
            &[info],
            None,
        )
    }
    .map(|pipelines| pipelines[0])
    .map_err(|error| format!("create Vulkan 3D ray-tracing pipeline: {error:?}"));
    tracing::debug!(
        elapsed_us = started.elapsed().as_micros(),
        success = pipeline.is_ok(),
        "Finished Vulkan 3D ray-tracing pipeline creation"
    );
    unsafe { device.destroy_shader_module(module, None) };
    pipeline
}

fn create_descriptor_pool(device: &ash::Device) -> Result<vk::DescriptorPool, String> {
    use shrimply_render_3d::obj::DescriptorKind;
    let kinds = [
        DescriptorKind::UniformBuffer,
        DescriptorKind::SampledImage,
        DescriptorKind::Sampler,
        DescriptorKind::AccelerationStructure,
        DescriptorKind::StorageImage,
        DescriptorKind::StorageBuffer,
    ];
    let sizes: Vec<_> = kinds
        .into_iter()
        .filter_map(|kind| {
            let descriptor_count = shrimply_render_3d::obj::DESCRIPTORS
                .iter()
                .filter(|descriptor| descriptor.kind == kind)
                .count()
                .try_into()
                .ok()?;
            (descriptor_count > 0).then_some(vk::DescriptorPoolSize {
                ty: vulkan_descriptor_type(kind),
                descriptor_count,
            })
        })
        .collect();
    let info = vk::DescriptorPoolCreateInfo::default()
        .max_sets(1)
        .pool_sizes(&sizes);
    unsafe { device.create_descriptor_pool(&info, None) }
        .map_err(|error| format!("create Vulkan 3D descriptor pool: {error:?}"))
}

fn allocate_descriptor_set(
    device: &ash::Device,
    pool: vk::DescriptorPool,
    layout: vk::DescriptorSetLayout,
) -> Result<vk::DescriptorSet, String> {
    let layouts = [layout];
    let info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(pool)
        .set_layouts(&layouts);
    unsafe { device.allocate_descriptor_sets(&info) }
        .map(|sets| sets[0])
        .map_err(|error| format!("allocate Vulkan 3D descriptor set: {error:?}"))
}

fn create_sampler(
    device: &ash::Device,
    mipmap_mode: vk::SamplerMipmapMode,
    address_mode_u: vk::SamplerAddressMode,
    max_lod: f32,
    label: &str,
) -> Result<vk::Sampler, String> {
    let info = vk::SamplerCreateInfo::default()
        .mag_filter(vk::Filter::LINEAR)
        .min_filter(vk::Filter::LINEAR)
        .mipmap_mode(mipmap_mode)
        .address_mode_u(address_mode_u)
        .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .max_lod(max_lod);
    unsafe { device.create_sampler(&info, None) }
        .map_err(|error| format!("create Vulkan {label} sampler: {error:?}"))
}
