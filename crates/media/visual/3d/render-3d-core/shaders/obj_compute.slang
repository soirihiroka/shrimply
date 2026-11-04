import render_3d_abi;
import render_3d_composite;
import render_3d_environment;
import render_3d_ground;
import render_3d_material;
import render_3d_normals;
import render_3d_outline;
import render_3d_pbr;
import render_3d_ray;
import render_3d_sampling;
import render_3d_toon;

struct OutputSize
{
    uint width;
    uint height;
    uint denoising;
    uint guides;
};

struct ComputePbr
{
    uint4 modes;
    float4 surface;
    float ior;
    float3 padding;
};

struct ComputeMaterial
{
    float4 base_color;
    float4 metallic_roughness_normal_alpha;
    uint4 flags;
    TextureMapping base_color_texture;
    TextureMapping metallic_roughness_texture;
    TextureMapping normal_texture;
    uint4 surface;
    ComputePbr pbr;
};

#ifdef SHRIMPLY_SOFTWARE_RAYS
#include "modules/obj-software-resources.slang"
#else
#define OUTPUT_TEXEL(pixel) pixel
#define OBJECT_SHADING_MODEL scene.toon.shading_model
#define OBJECT_TRANSPORT true
#define OBJECT_GROUNDS true
#define OBJECT_GROUND_REFLECTIONS true
ConstantBuffer<SceneUniforms> scene;
ConstantBuffer<OutputSize> output_size;
ConstantBuffer<OutputSize> composite_size;
RaytracingAccelerationStructure scene_acceleration;
StructuredBuffer<float4> positions;
StructuredBuffer<float4> normals;
StructuredBuffer<float4> tangents;
StructuredBuffer<float4> tex_coords_0;
StructuredBuffer<float4> tex_coords_1;
StructuredBuffer<float4> vertex_colors;
StructuredBuffer<ComputeMaterial> materials;
Texture2D<float4> material_texture;
SamplerState material_sampler;
Texture2D<float4> environment_texture;
SamplerState environment_sampler;
Texture2D<float4> composite_texture;
StructuredBuffer<uint> composite_pixels;
RWTexture2D<float4> composite_texture_output;
StructuredBuffer<MeshInstance> mesh_instances;
RWStructuredBuffer<uint> output_pixels;
RWStructuredBuffer<float4> outline_guide;
RWStructuredBuffer<float2> outline_distance;
RWTexture2D<float4> denoiser_beauty;
RWTexture2D<float4> denoiser_background;
RWTexture2D<float4> denoiser_alpha;
RWTexture2D<float4> denoiser_depth_normal;
Texture2D<float4> denoised_texture;
Texture2D<float4> denoised_background_texture;
Texture2D<float4> denoised_alpha_texture;

#endif

PbrSettings material_pbr(ComputeMaterial material)
{
    if (material.flags.w == 0)
        return scene.pbr;
    PbrSettings result;
    result.path_tracing = PathTracingMode(material.pbr.modes.x);
    result.light_sampling_quality = LightSamplingQuality(material.pbr.modes.y);
    result.render_quality = RenderQuality(material.pbr.modes.z);
    result.denoising = material.pbr.modes.w;
    result.subsurface = material.pbr.surface.x;
    result.clearcoat = material.pbr.surface.y;
    result.sheen = material.pbr.surface.z;
    result.transmission = material.pbr.surface.w;
    result.ior = material.pbr.ior;
    result._padding = material.pbr.padding;
    return result;
}


float2 vertex_uv(uint vertex, uint set)
{
    return set == 0 ? tex_coords_0[vertex].xy : tex_coords_1[vertex].xy;
}

float4 sample_material_texture(
    TextureMapping mapping,
    uint first,
    float3 barycentric)
{
    if (mapping.settings.x == 0)
        return float4(1.0);
    float2 atlas_uv = material_texture_coordinates(
        mapping,
        vertex_uv(first, mapping.settings.y),
        vertex_uv(first + 1, mapping.settings.y),
        vertex_uv(first + 2, mapping.settings.y),
        barycentric);
    if (mapping.rotation_filter.y == 0.0)
        return material_texture.SampleLevel(material_sampler, atlas_uv, 0.0);
    uint width;
    uint height;
    material_texture.GetDimensions(width, height);
    int2 texture_pixel = int2(clamp(
        floor(atlas_uv * float2(width, height)),
        float2(0.0),
        float2(width - 1, height - 1)));
    return material_texture.Load(int3(texture_pixel, 0));
}

float4 surface_base_color(
    uint first,
    float3 barycentric,
    ComputeMaterial material)
{
    float4 vertex_color = material_vertex_color(
        vertex_colors[first],
        vertex_colors[first + 1],
        vertex_colors[first + 2],
        barycentric);
    return material_base_color(
        material.base_color,
        vertex_color,
        sample_material_texture(material.base_color_texture, first, barycentric));
}

uint2 hit_offsets(uint instance_index, uint geometry_index)
{
    MeshInstance instance = mesh_instances[instance_index];
    return geometry_index == 0
        ? instance.geometry_offsets.xy
        : instance.geometry_offsets.zw;
}

float candidate_surface_alpha(
    uint instance_index,
    uint geometry_index,
    uint primitive_index,
    float2 coordinates,
    out uint material_index,
    out uint alpha_mode,
    out float alpha_cutoff)
{
    uint2 offsets = hit_offsets(instance_index, geometry_index);
    uint first = offsets.x + primitive_index * 3;
    material_index = offsets.y + primitive_index;
    ComputeMaterial material = materials[material_index];
    alpha_mode = material.flags.x != 0 ? material.flags.y : 0;
    alpha_cutoff = material.metallic_roughness_normal_alpha.w;
    if (material.flags.x == 0)
        return scene.base_color.a;
    float3 barycentric = float3(
        1.0 - coordinates.x - coordinates.y,
        coordinates.x,
        coordinates.y);
    return surface_base_color(first, barycentric, material).a;
}

bool candidate_hit_accepted(
    uint instance_index,
    uint geometry_index,
    uint primitive_index,
    float2 coordinates)
{
    uint material_index;
    uint alpha_mode;
    float alpha_cutoff;
    float alpha = candidate_surface_alpha(
        instance_index,
        geometry_index,
        primitive_index,
        coordinates,
        material_index,
        alpha_mode,
        alpha_cutoff);
    return material_hit_accepted(
        material_index,
        coordinates,
        alpha_mode,
        alpha,
        alpha_cutoff);
}

float3 shadow_visibility(
    float3 world_position,
    float3 world_normal,
    float3 world_direction,
    float maximum)
{
    float3 world_origin = world_position + world_normal * RAY_MINIMUM;
    float3 object_origin = mul(
        scene.model_inverse,
        float4(world_origin, 1.0)).xyz;
    float3 object_target = mul(
        scene.model_inverse,
        float4(world_origin + world_direction * maximum, 1.0)).xyz;
    float3 object_delta = object_target - object_origin;
    RayDesc ray;
    ray.Origin = object_origin;
    ray.Direction = normalize(object_delta);
    ray.TMin = RAY_MINIMUM;
    ray.TMax = max(length(object_delta) - RAY_MINIMUM, RAY_MINIMUM);
    RayQuery<RAY_FLAG_FORCE_NON_OPAQUE | RAY_FLAG_SKIP_PROCEDURAL_PRIMITIVES> query;
    query.TraceRayInline(scene_acceleration, RAY_FLAG_NONE, 0xff, ray);
    float3 visibility = float3(1.0);
    while (query.Proceed())
    {
        if (query.CandidateType() != CANDIDATE_NON_OPAQUE_TRIANGLE)
            continue;
        uint instance_index = query.CandidateInstanceIndex();
        uint geometry_index = query.CandidateGeometryIndex();
        uint primitive_index = query.CandidatePrimitiveIndex();
        float2 coordinates = query.CandidateTriangleBarycentrics();
        uint2 offsets = hit_offsets(instance_index, geometry_index);
        uint first = offsets.x + primitive_index * 3;
        uint material_index = offsets.y + primitive_index;
        ComputeMaterial material = materials[material_index];
        float3 barycentric = float3(
            1.0 - coordinates.x - coordinates.y,
            coordinates.x,
            coordinates.y);
        float4 base_color = material.flags.x != 0
            ? surface_base_color(first, barycentric, material)
            : scene.base_color;
        float coverage = material_shadow_coverage(
            material.flags.x != 0 ? material.flags.y : 0,
            base_color.a,
            material.metallic_roughness_normal_alpha.w);
        float4 metallic_roughness = sample_material_texture(
            material.metallic_roughness_texture,
            first,
            barycentric);
        float metallic = material_metallic(
            material.flags.x != 0,
            material.metallic_roughness_normal_alpha.x,
            metallic_roughness,
            scene.material.x);
        MeshInstance instance = mesh_instances[instance_index];
        float3 object_normal = normalize(cross(
            positions[first + 1].xyz - positions[first].xyz,
            positions[first + 2].xyz - positions[first].xyz));
        float3 world_normal = normalize(mul(
            mul(scene.normal, instance.normal),
            float4(object_normal, 0.0)).xyz);
        visibility *= material_shadow_transmittance(
            base_color,
            coverage,
            metallic,
            dot(world_normal, world_direction),
            material_pbr(material));
        if (max(visibility.r, max(visibility.g, visibility.b)) <= RAY_MINIMUM)
            query.CommitNonOpaqueTriangleHit();
    }
    return query.CommittedStatus() == COMMITTED_NOTHING ? visibility : 0.0;
}

struct GroundShadowTracer : IGroundShadowTracer
{
    float visibility(
        float3 world_position,
        float3 normal,
        float3 direction,
        float maximum)
    {
        return saturate(luminance(shadow_visibility(
            world_position,
            normal,
            direction,
            maximum)));
    }
};

struct ToonShadowTracer : IToonShadowTracer
{
    float visibility(
        float3 world_position,
        float3 normal,
        float3 direction,
        float maximum)
    {
        return saturate(luminance(shadow_visibility(
            world_position,
            normal,
            direction,
            maximum)));
    }
};

float ground_shadow_occlusion(float3 world_position, float3 normal, inout uint seed)
{
    GroundShadowTracer tracer;
    return integrate_ground_shadow(scene, world_position, normal, seed, tracer);
}

struct ComputeCompositeSampler : ICompositeSampler
{
    uint2 composite_dimensions()
    {
        uint width;
        uint height;
        composite_texture.GetDimensions(width, height);
        return uint2(width, height);
    }

    float4 composite_sample(float2 uv, float lod)
    {
        return composite_texture.SampleLevel(environment_sampler, uv, lod);
    }

    float3 environment_sample(float3 direction, float lod)
    {
        return environment_texture.SampleLevel(
            environment_sampler,
            environment_coordinates(scene.environment_inverse_rotation, direction),
            lod).rgb;
    }
};

float3 sample_environment(
    float3 world_origin,
    float3 direction,
    float lod,
    out bool composed_hit)
{
    ComputeCompositeSampler sampler;
    return shared_sample_environment(
        scene,
        sampler,
        world_origin,
        direction,
        lod,
        composed_hit);
}

struct ComputePbrEnvironmentTracer : IPbrEnvironmentTracer
{
    float3 radiance(float3 world_origin, float3 direction, out bool composed_hit)
    {
        return sample_environment(world_origin, direction, 0.0, composed_hit);
    }

    float3 visibility(float3 world_position, float3 normal, float3 direction)
    {
        return shadow_visibility(
            world_position,
            normal,
            direction,
            RAY_MAXIMUM);
    }
};

uint pack_color(float4 color)
{
    uint4 value = uint4(round(saturate(color) * 255.0));
    return value.r | (value.g << 8) | (value.b << 16) | (value.a << 24);
}

float4 unpack_color(uint color)
{
    return float4(
        color & 0xffu,
        (color >> 8) & 0xffu,
        (color >> 16) & 0xffu,
        color >> 24) / 255.0;
}

[shader("compute")]
[numthreads(8, 8, 1)]
void obj_composite_upload_compute(uint3 thread_id : SV_DispatchThreadID)
{
    uint2 pixel = thread_id.xy;
    uint2 dimensions = uint2(composite_size.width, composite_size.height);
    if (pixel.x >= dimensions.x || pixel.y >= dimensions.y)
        return;
    composite_texture_output[OUTPUT_TEXEL(pixel)] = unpack_color(
        composite_pixels[pixel.y * dimensions.x + pixel.x]);
}

struct ComputeSurface
{
    float3 world_position;
    float3 normal;
    float4 mesh_base_color;
    float3 base_color;
    float metallic;
    float roughness;
    ComputeMaterial material;
};

ComputeSurface load_surface(
    uint instance_index,
    uint geometry_index,
    uint primitive_index,
    float2 coordinates)
{
    float3 barycentric = float3(
        1.0 - coordinates.x - coordinates.y,
        coordinates.x,
        coordinates.y);
    MeshInstance instance = mesh_instances[instance_index];
    uint2 offsets = hit_offsets(instance_index, geometry_index);
    uint first = offsets.x + primitive_index * 3;
    uint material_index = offsets.y + primitive_index;
    float3 triangle_positions[3] = {
        positions[first].xyz,
        positions[first + 1].xyz,
        positions[first + 2].xyz
    };
    float3 triangle_normals[3] = {
        normals[first].xyz,
        normals[first + 1].xyz,
        normals[first + 2].xyz
    };
    ComputeMaterial material = materials[material_index];
    NormalMode normal_mode = material.flags.w != 0
        ? NormalMode(material.surface.x)
        : scene.normal_mode;
    float3 object_normal = surface_normal(
        normal_mode,
        triangle_positions,
        triangle_normals,
        barycentric);
    float3 normal = normalize(mul(
        mul(scene.normal, instance.normal),
        float4(object_normal, 0.0)).xyz);
    float4 sampled_normal = sample_material_texture(
        material.normal_texture,
        first,
        barycentric);
    if (material.normal_texture.settings.x != 0)
    {
        float4 object_tangent = tangents[first] * barycentric.x
            + tangents[first + 1] * barycentric.y
            + tangents[first + 2] * barycentric.z;
        float3 tangent = normalize(mul(
            mul(scene.model, instance.model),
            float4(object_tangent.xyz, 0.0)).xyz);
        float handedness = object_tangent.w < 0.0 ? -1.0 : 1.0;
        normal = apply_normal_map(
            normal,
            tangent,
            handedness,
            sampled_normal.xyz,
            material.metallic_roughness_normal_alpha.z);
    }
    float3 object_position = triangle_positions[0] * barycentric.x
        + triangle_positions[1] * barycentric.y
        + triangle_positions[2] * barycentric.z;
    float3 scene_position = mul(instance.model, float4(object_position, 1.0)).xyz;
    float4 mesh_base_color = surface_base_color(first, barycentric, material);
    float4 metallic_roughness = sample_material_texture(
        material.metallic_roughness_texture,
        first,
        barycentric);
    ComputeSurface result;
    result.world_position = mul(scene.model, float4(scene_position, 1.0)).xyz;
    result.normal = normal;
    result.mesh_base_color = mesh_base_color;
    result.base_color = material.flags.x != 0
        ? mesh_base_color.rgb
        : scene.base_color.rgb;
    result.metallic = material_metallic(
        material.flags.x != 0,
        material.metallic_roughness_normal_alpha.x,
        metallic_roughness,
        scene.material.x);
    result.roughness = material_roughness(
        material.flags.x != 0,
        material.metallic_roughness_normal_alpha.y,
        metallic_roughness,
        scene.material.y);
    result.material = material;
    return result;
}

OutlineSurface trace_outline_surface(
    float2 position,
    uint2 dimensions,
    uint sample_index)
{
    CameraRay camera = camera_ray(
        scene,
        position,
        dimensions,
        false,
        sample_index);
    if (!camera.valid)
    {
        OutlineSurface invalid = {
            0u,
            float3(0.0),
            RAY_MAXIMUM,
            0.0
        };
        return invalid;
    }
    RayQuery<RAY_FLAG_FORCE_NON_OPAQUE | RAY_FLAG_SKIP_PROCEDURAL_PRIMITIVES> query;
    query.TraceRayInline(scene_acceleration, RAY_FLAG_NONE, 0xff, camera.object_ray);
    while (query.Proceed())
    {
        if (query.CandidateType() == CANDIDATE_NON_OPAQUE_TRIANGLE
            && candidate_hit_accepted(
                query.CandidateInstanceIndex(),
                query.CandidateGeometryIndex(),
                query.CandidatePrimitiveIndex(),
                query.CandidateTriangleBarycentrics()))
            query.CommitNonOpaqueTriangleHit();
    }
    if (query.CommittedStatus() != COMMITTED_TRIANGLE_HIT)
    {
        OutlineSurface miss = {
            0u,
            float3(0.0),
            RAY_MAXIMUM,
            0.0
        };
        return miss;
    }
    ComputeSurface surface = load_surface(
        query.CommittedInstanceIndex(),
        query.CommittedGeometryIndex(),
        query.CommittedPrimitiveIndex(),
        query.CommittedTriangleBarycentrics());
    float3 view = -camera.world_direction;
    if (dot(surface.normal, view) < 0.0)
        surface.normal = -surface.normal;
    OutlineSurface result = {
        1u,
        surface.normal,
        length(scene.camera_position.xyz - surface.world_position),
        1.0 - saturate(dot(surface.normal, view))
    };
    return result;
}

struct ComputeOutlineTracer : IOutlineTracer
{
    OutlineSurface trace(float2 position, uint2 dimensions, uint sample_index)
    {
        return trace_outline_surface(position, dimensions, sample_index);
    }
};

struct ComputeRegionOutlineStorage : IRegionOutlineStorage
{
    float4 guide(float2 position, uint2 dimensions)
    {
        if (any(position < float2(0.0)) || any(position >= float2(dimensions)))
            return float4(0.0, 0.0, 0.0, RAY_MAXIMUM);
        uint2 pixel = uint2(position);
        return outline_guide[pixel.y * dimensions.x + pixel.x];
    }

    float2 distance(float2 position, uint2 dimensions)
    {
        if (any(position < float2(0.0)) || any(position >= float2(dimensions)))
            return float2(RAY_MAXIMUM);
        uint2 pixel = uint2(position);
        return outline_distance[pixel.y * dimensions.x + pixel.x];
    }
};

bool trace_surface_hit(
    RayDesc ray,
    out ComputeSurface surface,
    out float ray_distance)
{
    RayQuery<RAY_FLAG_FORCE_NON_OPAQUE | RAY_FLAG_SKIP_PROCEDURAL_PRIMITIVES> query;
    query.TraceRayInline(scene_acceleration, RAY_FLAG_NONE, 0xff, ray);
    while (query.Proceed())
    {
        if (query.CandidateType() == CANDIDATE_NON_OPAQUE_TRIANGLE
            && candidate_hit_accepted(
                query.CandidateInstanceIndex(),
                query.CandidateGeometryIndex(),
                query.CandidatePrimitiveIndex(),
                query.CandidateTriangleBarycentrics()))
            query.CommitNonOpaqueTriangleHit();
    }
    if (query.CommittedStatus() != COMMITTED_TRIANGLE_HIT)
    {
        ray_distance = RAY_MAXIMUM;
        return false;
    }
    surface = load_surface(
        query.CommittedInstanceIndex(),
        query.CommittedGeometryIndex(),
        query.CommittedPrimitiveIndex(),
        query.CommittedTriangleBarycentrics());
    ray_distance = query.CommittedRayT();
    return true;
}

struct ComputeToonTextureTracer : IToonTextureTracer
{
    ToonTextureSample sample(float2 position, uint2 dimensions)
    {
        CameraRay camera = camera_ray(scene, position, dimensions, false, 0u);
        ToonTextureSample result;
        result.hit = false;
        result.color = float3(0.0);
        result.normal = float3(0.0);
        result.depth = RAY_MAXIMUM;
        if (!camera.valid)
            return result;
        ComputeSurface surface;
        float ray_distance;
        if (!trace_surface_hit(camera.object_ray, surface, ray_distance))
            return result;
        float3 view = -camera.world_direction;
        if (dot(surface.normal, view) < 0.0)
            surface.normal = -surface.normal;
        result.hit = true;
        result.color = surface.base_color;
        result.normal = surface.normal;
        result.depth = length(scene.camera_position.xyz - surface.world_position);
        return result;
    }
};

void sample_surface_environment(
    float3 world_position,
    float3 normal,
    float3 view,
    float roughness,
    out float3 diffuse,
    out float3 reflection)
{
    uint width;
    uint height;
    uint levels;
    if (scene.environment.source == EnvironmentSource::Composite)
        composite_texture.GetDimensions(0, width, height, levels);
    else
        environment_texture.GetDimensions(0, width, height, levels);
    float max_lod = float(max(levels, 1u) - 1u);
    bool diffuse_composed_hit;
    diffuse = sample_environment(
        world_position,
        normal,
        max_lod,
        diffuse_composed_hit);
    diffuse *= diffuse_composed_hit ? 1.0 : scene.environment.intensity;
    bool reflection_composed_hit;
    reflection = sample_environment(
        world_position,
        reflect(-view, normal),
        roughness * max_lod,
        reflection_composed_hit);
    reflection *= reflection_composed_hit ? 1.0 : scene.environment.intensity;
}

float4 shade_surface(
    ComputeSurface surface,
    float3 world_direction,
    uint2 pixel,
    uint path_bounce,
    inout uint seed)
{
    ComputeMaterial material = surface.material;
    float3 world_position = surface.world_position;
    float3 normal = surface.normal;
    float4 mesh_base_color = surface.mesh_base_color;
    float3 base_color = surface.base_color;
    float metallic = surface.metallic;
    float roughness = surface.roughness;
    float3 view = -world_direction;
    if (dot(normal, view) < 0.0)
        normal = -normal;
    if (OBJECT_SHADING_MODEL == ShadingModel::Toon
        && scene.toon.texture_filter == ToonTextureFilter::Kuwahara
        && material.flags.x != 0
        && material.base_color_texture.settings.x != 0)
    {
        ComputeToonTextureTracer tracer;
        base_color = toon_screen_space_kuwahara(
            base_color,
            normal,
            length(scene.camera_position.xyz - world_position),
            float2(pixel) + 0.5,
            uint2(output_size.width, output_size.height),
            scene.toon,
            tracer);
    }
    PbrSettings surface_pbr = material_pbr(material);
    bool path_tracing = scene.pbr.path_tracing != PathTracingMode::Off;
    bool stochastic_transport = OBJECT_TRANSPORT && (path_tracing || surface_pbr.transmission > 0.0);
    if (OBJECT_SHADING_MODEL == ShadingModel::Depth)
    {
        float4 clip_position = mul(
            scene.view_projection,
            float4(world_position, 1.0));
        float depth = saturate(
            clip_position.z / max(abs(clip_position.w), RAY_MINIMUM));
        return float4(
            float3(depth),
            material.flags.x != 0 ? mesh_base_color.a : scene.base_color.a);
    }

    float3 shaded;
    if (OBJECT_SHADING_MODEL == ShadingModel::Toon)
    {
        float3 environment_diffuse;
        float3 environment_reflection;
        sample_surface_environment(
            world_position,
            normal,
            view,
            roughness,
            environment_diffuse,
            environment_reflection);
        ToonShadowTracer tracer;
        ToonDirectLighting lighting = integrate_toon_direct_lights(
            scene,
            world_position,
            normal,
            view,
            scene.toon,
            tracer);
        shaded = toon_surface_color(
            base_color,
            normal,
            view,
            float2(pixel) + 0.5,
            environment_diffuse * 0.25,
            environment_reflection * metallic,
            scene.toon,
            lighting);
    }
    else
    {
        shaded = path_tracing ? float3(0.0) : base_color * 0.02;
        uint light_samples = stochastic_transport
            ? 1u
            : uint(scene.pbr.light_sampling_quality);
        for (uint light_index = 0; light_index < scene.sun_light_count; ++light_index)
        {
            SunLight light = scene.sun_lights[light_index];
            float3 light_color = float3(0.0);
            for (uint light_sample = 0; light_sample < light_samples; ++light_sample)
            {
                float3 world_light = sample_cone(
                    normalize(-light.direction_angular_radius.xyz),
                    light.direction_angular_radius.w,
                    seed);
                float3 visibility = shadow_visibility(
                    world_position,
                    normal,
                    world_light,
                    RAY_MAXIMUM);
                light_color += pbr_light(
                    normal,
                    view,
                    world_light,
                    light.color_intensity.rgb * light.color_intensity.w * visibility,
                    base_color,
                    metallic,
                    roughness,
                    float3(1.0),
                    surface_pbr);
            }
            shaded += light_color / float(light_samples);
        }
        for (uint light_index = 0; light_index < scene.point_light_count; ++light_index)
        {
            PointLight light = scene.point_lights[light_index];
            float3 light_color = float3(0.0);
            for (uint light_sample = 0; light_sample < light_samples; ++light_sample)
            {
                float3 sampled_position = light.position_radius.xyz
                    + sample_unit_sphere(seed) * light.position_radius.w;
                float3 delta = sampled_position - world_position;
                float distance_squared = dot(delta, delta);
                float distance = sqrt(distance_squared);
                if (distance >= light.range_padding.x)
                    continue;
                float3 visibility = shadow_visibility(
                    world_position,
                    normal,
                    delta / max(distance, RAY_MINIMUM),
                    distance);
                float falloff = pow(1.0 - distance / light.range_padding.x, 2.0)
                    / max(distance_squared, 0.01);
                light_color += pbr_light(
                    normal,
                    view,
                    delta / max(distance, RAY_MINIMUM),
                    light.color_intensity.rgb * light.color_intensity.w
                        * falloff * visibility,
                    base_color,
                    metallic,
                    roughness,
                    float3(1.0),
                    surface_pbr);
            }
            shaded += light_color / float(light_samples);
        }
        if (stochastic_transport && shared_environment_enabled(scene))
        {
            ComputePbrEnvironmentTracer tracer;
            shaded += integrate_pbr_environment(
                scene,
                world_position,
                normal,
                view,
                base_color,
                metallic,
                roughness,
                surface_pbr,
                seed,
                tracer);
        }
        else
        {
            float3 environment_diffuse;
            float3 environment_reflection;
            sample_surface_environment(
                world_position,
                normal,
                view,
                roughness,
                environment_diffuse,
                environment_reflection);
            float3 fresnel = pbr_fresnel(
                base_color,
                metallic,
                saturate(dot(normal, view)),
                surface_pbr.ior);
            shaded += environment_diffuse * base_color * (1.0 - metallic)
                + environment_reflection * fresnel;
        }
    }
    if (path_bounce == 0)
    {
        shaded *= scene.material.z;
        if (scene.pbr.denoising == 0)
        {
            shaded = float3(
                linear_to_srgb(aces_tone_map(shaded.r)),
                linear_to_srgb(aces_tone_map(shaded.g)),
                linear_to_srgb(aces_tone_map(shaded.b)));
        }
    }
    return float4(
        shaded,
        material.flags.x != 0 ? mesh_base_color.a : scene.base_color.a);
}

struct TransportResult
{
    float3 radiance;
    float3 background;
    float alpha;
};

struct GroundTransportContinuation
{
    GroundReflectionRay reflection;
    float4 virtual_color;
    float3 radiance;
    float3 background;
    float3 throughput;
    float alpha;
    float shadow_occlusion;
    float receiver_strength;
    uint transmitted;
};

TransportResult shade_transport_path(
    ComputeSurface first_surface,
    float3 first_direction,
    uint2 pixel,
    uint first_path_bounce,
    uint random_seed)
{
    if (!OBJECT_TRANSPORT)
    {
        float4 local = shade_surface(first_surface, first_direction, pixel, 1u, random_seed);
        TransportResult result;
        result.radiance = local.rgb;
        result.background = float3(0.0);
        result.alpha = local.a;
        return result;
    }
    ComputeSurface surface = first_surface;
    float3 direction = first_direction;
    float3 throughput = float3(1.0);
    float3 radiance = float3(0.0);
    float3 background = float3(0.0);
    bool transmitted = false;
    uint path_bounce = first_path_bounce;
    uint seed = random_seed;
    float alpha = surface.material.flags.x != 0
        ? surface.mesh_base_color.a
        : scene.base_color.a;
    uint bounce_limit = pbr_transport_bounce_limit(scene.pbr);
    bool path_tracing = scene.pbr.path_tracing != PathTracingMode::Off;
    GroundTransportContinuation continuations[FINAL_TRANSPORT_BOUNCES];
    uint continuation_count = 0u;

    while (true)
    {
        TransportResult completed;
        bool path_complete = false;
        float4 local = shade_surface(surface, direction, pixel, 1u, seed);
        radiance += throughput * local.rgb;
        PbrSettings settings = material_pbr(surface.material);
        if ((!path_tracing && settings.transmission <= 0.0)
            || path_bounce >= bounce_limit)
        {
            completed.radiance = radiance;
            completed.background = background;
            completed.alpha = alpha;
            path_complete = true;
        }
        else
        {
            float3 normal = surface.normal;
            float3 view = -direction;
            bool front_face = dot(normal, view) >= 0.0;
            if (!front_face)
                normal = -normal;
            PbrSample sample = pbr_sample(
                normal,
                view,
                front_face,
                surface.base_color,
                surface.metallic,
                surface.roughness,
                settings,
                seed);
            float survival = 1.0;
            if (sample.valid && path_bounce >= 2u)
            {
                survival = clamp(
                    max(sample.weight.r, max(sample.weight.g, sample.weight.b)),
                    0.1,
                    0.95);
                sample.valid = random_float(seed) <= survival;
            }
            if (!sample.valid)
            {
                completed.radiance = radiance;
                completed.background = background;
                completed.alpha = alpha;
                path_complete = true;
            }
            else
            {
                throughput *= sample.weight / survival;
                transmitted = transmitted || sample.refracted;
                float side = dot(sample.direction, normal) >= 0.0 ? 1.0 : -1.0;
                float3 origin = surface.world_position + normal * (RAY_MINIMUM * side);
                RayDesc ray = {
                    mul(scene.model_inverse, float4(origin, 1.0)).xyz,
                    RAY_MINIMUM,
                    normalize(mul(
                        scene.model_inverse,
                        float4(sample.direction, 0.0)).xyz),
                    RAY_MAXIMUM
                };
                ComputeSurface next_surface;
                float query_distance;
                bool geometry_hit = trace_surface_hit(ray, next_surface, query_distance);
                float geometry_distance = geometry_hit
                    ? length(next_surface.world_position - origin)
                    : RAY_MAXIMUM;

                ComputeCompositeSampler composite_sampler;
                float4 virtual_color;
                float virtual_distance;
                int virtual_ground;
                bool virtual_hit = shared_sample_composed_planes(
                    scene,
                    composite_sampler,
                    origin,
                    sample.direction,
                    0.0,
                    virtual_color,
                    virtual_distance,
                    virtual_ground)
                    && virtual_distance < geometry_distance;
                if (OBJECT_GROUNDS && virtual_hit && virtual_ground >= 0)
                {
                    Ground ground = scene.grounds[virtual_ground];
                    float3 ground_position = origin + sample.direction * virtual_distance;
                    float3 ground_normal = normalize(ground.normal_composite.xyz);
                    if (scene.ground_shading_model == ShadingModel::Depth)
                    {
                        float4 clip_position = mul(
                            scene.view_projection,
                            float4(ground_position, 1.0));
                        float depth = saturate(
                            clip_position.z / max(abs(clip_position.w), RAY_MINIMUM));
                        radiance += throughput * float3(depth);
                        completed.radiance = radiance;
                        completed.background = background;
                        completed.alpha = alpha;
                        path_complete = true;
                    }
                    else
                    {
                        uint ground_seed = seed;
                        float shadow_occlusion = ground_shadow_occlusion(
                            ground_position,
                            ground_normal,
                            ground_seed);
                        GroundReflectionRay reflection = sample_ground_reflection(
                            scene,
                            ground,
                            ground_position,
                            ground_normal,
                            sample.direction,
                            path_bounce + 1u,
                            ground_seed);
                        GroundTransportContinuation continuation;
                        continuation.reflection = reflection;
                        continuation.virtual_color = virtual_color;
                        continuation.radiance = radiance;
                        continuation.background = background;
                        continuation.throughput = throughput;
                        continuation.alpha = alpha;
                        continuation.shadow_occlusion = shadow_occlusion;
                        continuation.receiver_strength =
                            ground.settings.y * ground.position_opacity.w;
                        continuation.transmitted = transmitted ? 1u : 0u;
                        continuations[continuation_count] = continuation;
                        continuation_count += 1u;

                        ComputeSurface reflected_surface;
                        float reflected_distance;
                        bool reflected_hit = false;
                        if (OBJECT_GROUND_REFLECTIONS && reflection.valid)
                        {
                            RayDesc reflection_ray = {
                                mul(scene.model_inverse, float4(reflection.origin, 1.0)).xyz,
                                RAY_MINIMUM,
                                normalize(mul(
                                    scene.model_inverse,
                                    float4(reflection.direction, 0.0)).xyz),
                                RAY_MAXIMUM
                            };
                            reflected_hit = trace_surface_hit(
                                reflection_ray,
                                reflected_surface,
                                reflected_distance);
                        }
                        if (reflected_hit)
                        {
                            surface = reflected_surface;
                            direction = reflection.direction;
                            throughput = float3(1.0);
                            radiance = float3(0.0);
                            background = float3(0.0);
                            transmitted = false;
                            path_bounce = reflection.path_bounce;
                            seed = reflection.random_seed;
                            alpha = surface.material.flags.x != 0
                                ? surface.mesh_base_color.a
                                : scene.base_color.a;
                            continue;
                        }
                        completed.radiance = float3(0.0);
                        completed.background = float3(0.0);
                        completed.alpha = 0.0;
                        path_complete = true;
                    }
                }
                else if (virtual_hit)
                {
                    if (transmitted)
                        background += throughput * virtual_color.rgb;
                    else
                        radiance += throughput * virtual_color.rgb;
                    completed.radiance = radiance;
                    completed.background = background;
                    completed.alpha = alpha;
                    path_complete = true;
                }
                else if (!geometry_hit)
                {
                    float4 transmitted_background;
                    bool background_hit = transmitted
                        && shared_sample_transmission_background(
                            scene,
                            composite_sampler,
                            origin,
                            sample.direction,
                            transmitted_background);
                    if (background_hit)
                        background += throughput * transmitted_background.rgb;
                    else
                    {
                        bool composed_hit;
                        float3 environment = sample_environment(
                            origin,
                            sample.direction,
                            0.0,
                            composed_hit);
                        if (!composed_hit)
                            environment *= scene.environment.intensity;
                        float environment_mis = shared_environment_enabled(scene) && !sample.delta
                            ? power_heuristic(sample.pdf, SHARED_UNIFORM_ENVIRONMENT_PDF)
                            : 1.0;
                        radiance += throughput * environment * environment_mis;
                    }
                    completed.radiance = radiance;
                    completed.background = background;
                    completed.alpha = alpha;
                    path_complete = true;
                }
                else
                {
                    surface = next_surface;
                    direction = sample.direction;
                    path_bounce += 1u;
                    continue;
                }
            }
        }

        if (!path_complete)
            continue;
        while (OBJECT_GROUNDS && continuation_count > 0u)
        {
            continuation_count -= 1u;
            GroundTransportContinuation continuation =
                continuations[continuation_count];
            float4 reflection = continuation.reflection.valid
                ? finish_ground_reflection(
                    continuation.reflection,
                    float4(completed.radiance, completed.alpha))
                : float4(0.0);
            float4 virtual_color = continuation.virtual_color;
            composite_over(virtual_color, reflection);
            float3 parent_radiance = continuation.radiance;
            float3 parent_background = continuation.background;
            if (scene.ground_shading_model == ShadingModel::Toon)
            {
                float4 toon_shadow = toon_ground_shadow(
                    scene,
                    continuation.shadow_occlusion,
                    continuation.receiver_strength,
                    float2(pixel) + 0.5);
                if (continuation.transmitted != 0u)
                {
                    parent_radiance += continuation.throughput
                        * toon_shadow.rgb * toon_shadow.a;
                    virtual_color.rgb = lerp(
                        virtual_color.rgb,
                        toon_shadow.rgb,
                        toon_shadow.a);
                }
                else
                    composite_over(virtual_color, toon_shadow);
            }
            else if (continuation.transmitted != 0u)
            {
                virtual_color.rgb *= 1.0
                    - continuation.shadow_occlusion * continuation.receiver_strength;
            }
            else
            {
                composite_ground_shadow(
                    virtual_color,
                    continuation.shadow_occlusion * continuation.receiver_strength);
            }
            if (continuation.transmitted != 0u)
                parent_background += continuation.throughput * virtual_color.rgb;
            else
                parent_radiance += continuation.throughput * virtual_color.rgb;
            completed.radiance = parent_radiance;
            completed.background = parent_background;
            completed.alpha = continuation.alpha;
        }
        return completed;
    }
}

float4 shade_stochastic_path(
    ComputeSurface first_surface,
    float3 first_direction,
    uint2 pixel,
    uint random_seed,
    out float4 background)
{
    TransportResult result = shade_transport_path(
        first_surface,
        first_direction,
        pixel,
        0u,
        random_seed);
    float3 radiance = result.radiance;
    radiance *= scene.material.z;
    background = float4(result.background, 0.0);
    if (scene.pbr.denoising == 0)
    {
        radiance = float3(
            aces_tone_map(radiance.r),
            aces_tone_map(radiance.g),
            aces_tone_map(radiance.b));
        radiance = saturate(radiance + result.background);
        radiance = float3(
            linear_to_srgb(radiance.r),
            linear_to_srgb(radiance.g),
            linear_to_srgb(radiance.b));
        background = float4(0.0);
    }
    return float4(radiance, result.alpha);
}


float4 render_sample(
    float2 sample_position,
    uint2 pixel,
    uint2 dimensions,
    uint seed,
    out float4 background)
{
    background = float4(0.0);
    CameraRay camera = camera_ray(
        scene,
        sample_position,
        dimensions,
        true,
        seed);
    if (!camera.valid)
        return float4(0.0);
    uint ground_seed = seed;
    RayDesc ray = camera.object_ray;

    ComputeSurface surface;
    float ray_distance;
    float4 color = float4(0.0);
    float visible_distance = RAY_MAXIMUM;
    if (trace_surface_hit(ray, surface, ray_distance))
    {
        visible_distance = length(surface.world_position - camera.world_origin);
        color = OBJECT_SHADING_MODEL == ShadingModel::Pbr
                && OBJECT_TRANSPORT
                && (scene.pbr.path_tracing != PathTracingMode::Off
                    || material_pbr(surface.material).transmission > 0.0)
            ? shade_stochastic_path(
                surface,
                camera.world_direction,
                pixel,
                seed,
                background)
            : shade_surface(surface, camera.world_direction, pixel, 0u, seed);
    }
    if (!OBJECT_GROUNDS) return color;
    int ground_index = -1;
    float ground_distance = visible_distance + RAY_MINIMUM;
    float3 ground_position = float3(0.0);
    float3 ground_normal = float3(0.0);
    for (uint index = 0; index < scene.ground_count; ++index)
    {
        float candidate_distance;
        float3 candidate_position;
        float3 candidate_normal;
        if (intersect_ground(
            scene.grounds[index],
            camera.ground_origin,
            camera.world_direction,
            candidate_distance,
            candidate_position,
            candidate_normal)
            && candidate_distance < ground_distance)
        {
            ground_index = int(index);
            ground_distance = candidate_distance;
            ground_position = candidate_position;
            ground_normal = candidate_normal;
        }
    }
    if (ground_index < 0 || ground_distance >= visible_distance + RAY_MINIMUM)
        return color;
    Ground ground = scene.grounds[ground_index];
    if (scene.ground_shading_model == ShadingModel::Depth)
    {
        float4 clip_position = mul(scene.view_projection, float4(ground_position, 1.0));
        float depth = saturate(clip_position.z / max(abs(clip_position.w), RAY_MINIMUM));
        return float4(float3(depth), ground.position_opacity.w);
    }
    float shadow_occlusion = ground_shadow_occlusion(
        ground_position,
        ground_normal,
        ground_seed);
    float receiver_strength = ground.settings.y * ground.position_opacity.w;
    GroundReflectionRay reflection_ray = sample_ground_reflection(
        scene,
        ground,
        ground_position,
        ground_normal,
        camera.world_direction,
        0u,
        ground_seed);
    float4 reflection = float4(0.0);
    if (OBJECT_GROUND_REFLECTIONS && reflection_ray.valid)
    {
        RayDesc reflected_ray = {
            mul(scene.model_inverse, float4(reflection_ray.origin, 1.0)).xyz,
            RAY_MINIMUM,
            normalize(mul(
                scene.model_inverse,
                float4(reflection_ray.direction, 0.0)).xyz),
            RAY_MAXIMUM
        };
        ComputeSurface reflected_surface;
        float reflected_distance;
        if (trace_surface_hit(reflected_ray, reflected_surface, reflected_distance))
        {
            TransportResult reflected = shade_transport_path(
                reflected_surface,
                reflection_ray.direction,
                pixel,
                reflection_ray.path_bounce,
                reflection_ray.random_seed);
            reflection = finish_ground_reflection(
                reflection_ray,
                float4(reflected.radiance, reflected.alpha));
        }
    }
    composite_over(color, reflection);
    if (scene.ground_shading_model == ShadingModel::Toon)
    {
        composite_over(
            color,
            toon_ground_shadow(
                scene,
                shadow_occlusion,
                receiver_strength,
                float2(pixel) + 0.5));
        return color;
    }
    composite_ground_shadow(color, shadow_occlusion * receiver_strength);
    return color;
}

[shader("compute")]
[numthreads(8, 8, 1)]
void obj_compute(uint3 thread_id : SV_DispatchThreadID)
{
    uint2 pixel = thread_id.xy;
    uint2 dimensions = uint2(output_size.width, output_size.height);
    if (pixel.x >= dimensions.x || pixel.y >= dimensions.y)
        return;

    AntiAliasing anti_aliasing = scene.toon.anti_aliasing;
    uint anti_aliasing_samples = anti_aliasing_sample_count(anti_aliasing);
    uint minimum_samples = 1u;
    uint maximum_samples = 1u;
    float relative_error = 0.0;
    bool adaptive = scene.pbr.path_tracing != PathTracingMode::Off;
    if (adaptive)
    {
        adaptive_path_budget(
            scene.pbr,
            minimum_samples,
            maximum_samples,
            relative_error);
    }
    uint depth_of_field_samples = scene.camera_lens.y > 0.0
        ? (scene.pbr.render_quality == RenderQuality::Interactive ? 2u : 8u)
        : 1u;
    uint transmission_samples = scene.pbr.transmission > 0.0
        ? ground_light_sample_count(scene.pbr)
        : 1u;
    minimum_samples = max(
        minimum_samples,
        max(anti_aliasing_samples, max(transmission_samples, depth_of_field_samples)));
    maximum_samples = max(maximum_samples, minimum_samples);
    float4 color = float4(0.0);
    float4 background = float4(0.0);
    float4 squared_error = float4(0.0);
    uint sample_count = 0u;
    for (uint sample_index = 0; sample_index < maximum_samples; ++sample_index)
    {
        float2 offset = maximum_samples == anti_aliasing_samples
            ? anti_aliasing_offset(sample_index, anti_aliasing, pixel)
            : stochastic_pixel_offset(sample_index, maximum_samples, pixel);
        float4 sample_background;
        float4 sample_color = render_sample(
            float2(pixel) + 0.5 + offset,
            pixel,
            dimensions,
            random_seed(pixel, sample_index),
            sample_background);
        sample_count += 1u;
        float4 delta = sample_color - color;
        color += delta / float(sample_count);
        background += (sample_background - background) / float(sample_count);
        squared_error += delta * (sample_color - color);
        if (adaptive
            && sample_count >= minimum_samples
            && adaptive_converged(color, squared_error, sample_count, relative_error))
            break;
    }
    if (scene.toon.outline_mode != OutlineMode::Off
        && scene.toon.outline_method != OutlineMethod::RegionBoundary)
    {
        float2 center_position = float2(pixel) + 0.5;
        float2 outline_offset;
        float width_variation;
        outline_variation(
            center_position,
            scene.toon,
            outline_offset,
            width_variation);
        float2 outline_position = center_position + outline_offset;
        OutlineSurface surface = trace_outline_surface(
            outline_position,
            dimensions,
            0u);
        float width = varied_outline_width(width_variation, scene.toon);
        uint probe_count = uint(scene.toon.outline_quality);
        ComputeOutlineTracer tracer;
        float coverage = 0.0;
        switch (scene.toon.outline_method)
        {
        case OutlineMethod::RayTraced:
            coverage = radial_outline(
                outline_position,
                dimensions,
                surface,
                scene.toon.outline_mode,
                width,
                probe_count,
                scene.toon,
                tracer);
            break;
        case OutlineMethod::Fresnel:
            break;
        case OutlineMethod::Hybrid:
            coverage = radial_outline(
                outline_position,
                dimensions,
                surface,
                scene.toon.outline_mode,
                width,
                probe_count,
                scene.toon,
                tracer);
            break;
        case OutlineMethod::Sobel:
            coverage = sobel_outline(
                outline_position,
                dimensions,
                surface,
                scene.toon.outline_mode,
                width,
                scene.toon,
                tracer);
            break;
        case OutlineMethod::RobertsCross:
            coverage = roberts_cross_outline(
                outline_position,
                dimensions,
                surface,
                scene.toon.outline_mode,
                width,
                scene.toon,
                tracer);
            break;
        case OutlineMethod::DifferenceOfGaussians:
            coverage = difference_of_gaussians_outline(
                outline_position,
                dimensions,
                surface,
                scene.toon.outline_mode,
                outline_clamp_width(
                    scene.toon.dog_inner_radius + width_variation),
                probe_count,
                scene.toon,
                tracer);
            break;
        default:
            break;
        }
        if ((scene.toon.outline_method == OutlineMethod::Fresnel
                || scene.toon.outline_method == OutlineMethod::Hybrid)
            && surface.hit != 0)
            coverage = max(
                coverage,
                toon_outline_fresnel(surface.facing, width));
        color = composite_outline(
            color,
            outline_binary_coverage(coverage, scene.toon),
            scene.toon);
    }
    if (OBJECT_SHADING_MODEL == ShadingModel::Toon)
        color.rgb = toon_palette_quantize(color.rgb, scene.toon.color_levels);
    uint pixel_index = pixel.y * dimensions.x + pixel.x;
    if (output_size.denoising != 0)
    {
        denoiser_beauty[OUTPUT_TEXEL(pixel)] = color;
        denoiser_background[OUTPUT_TEXEL(pixel)] = background;
        denoiser_alpha[OUTPUT_TEXEL(pixel)] = color.aaaa;
#ifdef SHRIMPLY_SOFTWARE_RAYS
        CameraRay camera = camera_ray(scene, float2(pixel) + 0.5, dimensions, false, 0u);
        ComputeSurface guide;
        float distance;
        bool hit = camera.valid && trace_surface_hit(camera.object_ray, guide, distance);
        if (hit && dot(guide.normal, -camera.world_direction) < 0.0)
            guide.normal = -guide.normal;
        denoiser_depth_normal[OUTPUT_TEXEL(pixel)] = hit
            ? float4(length(scene.camera_position.xyz - guide.world_position), guide.normal)
            : float4(RAY_MAXIMUM, 0.0, 0.0, 0.0);
        if (output_size.guides != 0)
            denoiser_albedo[OUTPUT_TEXEL(pixel)] = hit ? float4(guide.base_color, 1.0) : float4(0.0);
#else
        OutlineSurface guide = trace_outline_surface(float2(pixel) + 0.5, dimensions, 0u);
        denoiser_depth_normal[OUTPUT_TEXEL(pixel)] = guide.hit != 0
            ? float4(guide.depth, guide.normal)
            : float4(RAY_MAXIMUM, 0.0, 0.0, 0.0);
#endif
    }
    else
        output_pixels[pixel_index] = pack_color(color);
    if (scene.toon.outline_mode != OutlineMode::Off
        && scene.toon.outline_method == OutlineMethod::RegionBoundary)
    {
        OutlineSurface guide = trace_outline_surface(
            float2(pixel) + 0.5,
            dimensions,
            0u);
        outline_guide[pixel_index] = guide.hit != 0
            ? float4(guide.normal, guide.depth)
            : float4(0.0, 0.0, 0.0, RAY_MAXIMUM);
    }
}

[shader("compute")]
[numthreads(8, 8, 1)]
void obj_denoise_composite_compute(uint3 thread_id : SV_DispatchThreadID)
{
    uint2 pixel = thread_id.xy;
    uint2 dimensions = uint2(output_size.width, output_size.height);
    if (pixel.x >= dimensions.x || pixel.y >= dimensions.y)
        return;
    float4 beauty = denoised_texture[OUTPUT_TEXEL(pixel)];
    float4 background = denoised_background_texture[OUTPUT_TEXEL(pixel)];
    float3 color = float3(
        aces_tone_map(beauty.r),
        aces_tone_map(beauty.g),
        aces_tone_map(beauty.b));
    color = saturate(color + background.rgb);
    color = float3(
        linear_to_srgb(color.r),
        linear_to_srgb(color.g),
        linear_to_srgb(color.b));
    output_pixels[pixel.y * dimensions.x + pixel.x]
        = pack_color(float4(color, denoised_alpha_texture[OUTPUT_TEXEL(pixel)].r));
}

[shader("compute")]
[numthreads(8, 8, 1)]
void obj_outline_distance_compute(uint3 thread_id : SV_DispatchThreadID)
{
    uint2 pixel = thread_id.xy;
    uint2 dimensions = uint2(output_size.width, output_size.height);
    if (pixel.x >= dimensions.x || pixel.y >= dimensions.y)
        return;
    ComputeRegionOutlineStorage storage;
    outline_distance[pixel.y * dimensions.x + pixel.x]
        = region_horizontal_outline_distances(
            float2(pixel) + 0.5,
            dimensions,
            scene.toon.outline_mode,
            scene.toon,
            storage);
}

[shader("compute")]
[numthreads(8, 8, 1)]
void obj_outline_compute(uint3 thread_id : SV_DispatchThreadID)
{
    uint2 pixel = thread_id.xy;
    uint2 dimensions = uint2(output_size.width, output_size.height);
    if (pixel.x >= dimensions.x || pixel.y >= dimensions.y)
        return;
    float2 center_position = float2(pixel) + 0.5;
    float2 outline_offset;
    float width_variation;
    outline_variation(
        center_position,
        scene.toon,
        outline_offset,
        width_variation);
    ComputeRegionOutlineStorage storage;
    float coverage = region_boundary_outline(
        center_position + outline_offset,
        dimensions,
        scene.toon.outline_mode,
        varied_outline_width(width_variation, scene.toon),
        storage);
    if (coverage == 0.0)
        return;
    uint pixel_index = pixel.y * dimensions.x + pixel.x;
    float4 color = composite_outline(
        output_size.denoising != 0
            ? denoiser_beauty[OUTPUT_TEXEL(pixel)]
            : unpack_color(output_pixels[pixel_index]),
        coverage,
        scene.toon);
    if (OBJECT_SHADING_MODEL == ShadingModel::Toon)
        color.rgb = toon_palette_quantize(color.rgb, scene.toon.color_levels);
    if (output_size.denoising != 0)
    {
        denoiser_beauty[OUTPUT_TEXEL(pixel)] = color;
        denoiser_alpha[OUTPUT_TEXEL(pixel)] = color.aaaa;
    }
    else
        output_pixels[pixel_index] = pack_color(color);
}
