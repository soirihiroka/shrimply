import render_3d_abi;
import render_3d_budget;
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

static const float PI = 3.14159265358979323846;
static const float UNIFORM_ENVIRONMENT_PDF = 1.0 / (4.0 * PI);
static const uint DOG_RING_COUNT = 3;

[[vk::binding(0, 0)]] ConstantBuffer<SceneUniforms> scene;
[[vk::binding(1, 0)]] Texture2D<float4> environment_texture;
[[vk::binding(2, 0)]] SamplerState environment_sampler;
[[vk::binding(3, 0)]] RaytracingAccelerationStructure scene_acceleration;
[[vk::binding(4, 0)]] RWTexture2D<float4> output_image;
[[vk::binding(5, 0)]] StructuredBuffer<float4> positions;
[[vk::binding(6, 0)]] StructuredBuffer<float4> normals;
[[vk::binding(7, 0)]] RWTexture2D<float4> denoiser_beauty;
[[vk::binding(8, 0)]] RWTexture2D<float4> denoiser_albedo;
[[vk::binding(9, 0)]] RWTexture2D<float4> denoiser_normal;
[[vk::binding(10, 0)]] StructuredBuffer<float4> tangents;
[[vk::binding(11, 0)]] StructuredBuffer<float4> tex_coords_0;
[[vk::binding(12, 0)]] StructuredBuffer<float4> tex_coords_1;
[[vk::binding(13, 0)]] StructuredBuffer<float4> vertex_colors;
[[vk::binding(14, 0)]] StructuredBuffer<MeshMaterial> mesh_materials;
[[vk::binding(15, 0)]] Texture2D<float4> material_texture;
[[vk::binding(16, 0)]] SamplerState material_sampler;
[[vk::binding(17, 0)]] Texture2D<float4> transmission_background_texture;
[[vk::binding(18, 0)]] RWTexture2D<float4> denoiser_background;
[[vk::binding(19, 0)]] RWTexture2D<float4> outline_guide;
[[vk::binding(20, 0)]] RWTexture2D<float2> outline_distance;
[[vk::binding(21, 0)]] StructuredBuffer<MeshInstance> mesh_instances;

[raypayload]
struct RayPayload
{
    float4 color : read(caller) : write(caller, closesthit, miss);
    float4 background : read(caller) : write(caller, closesthit, miss);
    float3 normal : read(caller) : write(caller, closesthit, miss);
    float depth : read(caller) : write(caller, closesthit, miss);
    float ray_distance : read(caller) : write(caller, closesthit, miss);
    float facing : read(caller) : write(caller, closesthit, miss);
    uint hit : read(caller) : write(caller, closesthit, miss);
    uint probe : read(closesthit) : write(caller);
    uint path_bounce : read(closesthit, miss) : write(caller);
    uint random_seed : read(closesthit) : write(caller);
    uint transmitted : read(closesthit, miss) : write(caller);
    uint material_index : read(caller) : write(caller, closesthit, miss);
};

[raypayload]
struct ShadowPayload
{
    float3 transmittance : read(caller, anyhit) : write(caller, miss, anyhit);
    uint visible : read(caller) : write(caller, miss);
};

float depth_buffer_value(float3 world_position)
{
    float4 clip_position = mul(
        scene.view_projection,
        float4(world_position, 1.0));
    return saturate(clip_position.z / max(abs(clip_position.w), RAY_MINIMUM));
}

#include "modules/obj-material.slang"
#include "modules/obj-background.slang"
#include "modules/obj-lighting.slang"
#include "modules/obj-ground.slang"
#include "modules/obj-sampling.slang"
#include "modules/obj-kuwahara.slang"
#include "modules/obj-outlines.slang"
#include "modules/obj-raygen.slang"

[shader("miss")]
void primary_miss(inout RayPayload payload)
{
    if (payload.path_bounce == 0)
    {
        payload.color = float4(0.0);
        payload.background = float4(0.0);
        payload.normal = float3(0.0);
        payload.depth = RAY_MAXIMUM;
        payload.ray_distance = RAY_MAXIMUM;
        payload.facing = 0.0;
        payload.hit = 0;
        payload.material_index = 0xffffffffu;
        return;
    }

    if (payload.transmitted != 0
        && sample_transmission_background(payload.background))
    {
        payload.color = float4(0.0);
        payload.normal = float3(0.0);
        payload.depth = RAY_MAXIMUM;
        payload.ray_distance = RAY_MAXIMUM;
        payload.facing = 0.0;
        payload.hit = 0;
        payload.material_index = 0xffffffffu;
        return;
    }

    float3 world_direction = normalize(mul(
        scene.model,
        float4(WorldRayDirection(), 0.0)).xyz);
    float3 world_origin = mul(
        scene.model,
        float4(WorldRayOrigin(), 1.0)).xyz;
    bool has_environment = environment_enabled();
    bool composed_hit = false;
    float3 environment = has_environment
        ? sample_environment(
            world_origin,
            world_direction,
            0.0,
            composed_hit)
        : float3(0.0);
    if (!composed_hit)
        environment *= scene.environment.intensity;
    payload.color = float4(
        environment,
        has_environment ? 1.0 : 0.0);
    payload.background = float4(0.0);
    payload.normal = float3(0.0);
    payload.depth = RAY_MAXIMUM;
    payload.ray_distance = RAY_MAXIMUM;
    payload.facing = 0.0;
    payload.hit = 0;
    payload.material_index = 0xffffffffu;
}

[shader("miss")]
void shadow_miss(inout ShadowPayload payload)
{
    payload.visible = 1;
}

[shader("anyhit")]
void alpha_any_hit(
    inout RayPayload _payload,
    in BuiltInTriangleIntersectionAttributes attributes)
{
    MeshInstance instance = mesh_instances[InstanceID()];
    uint2 offsets = GeometryIndex() == 0
        ? instance.geometry_offsets.xy
        : instance.geometry_offsets.zw;
    uint first = offsets.x + PrimitiveIndex() * 3;
    uint material_index = offsets.y + PrimitiveIndex();
    float3 barycentric = float3(
        1.0 - attributes.barycentrics.x - attributes.barycentrics.y,
        attributes.barycentrics.x,
        attributes.barycentrics.y);
    MeshMaterial material = mesh_materials[material_index];
    float4 base_color = material.flags.x != 0
        ? surface_base_color(first, barycentric, material)
        : scene.base_color;
    if (material.flags.x == 0 || material.flags.y == 0)
        return;
    if (!material_hit_accepted(
            material_index,
            attributes.barycentrics,
            material.flags.y,
            base_color.a,
            material.metallic_roughness_normal_alpha.w))
        IgnoreHit();
}

[shader("anyhit")]
void shadow_alpha_any_hit(
    inout ShadowPayload payload,
    in BuiltInTriangleIntersectionAttributes attributes)
{
    MeshInstance instance = mesh_instances[InstanceID()];
    uint2 offsets = GeometryIndex() == 0
        ? instance.geometry_offsets.xy
        : instance.geometry_offsets.zw;
    uint first = offsets.x + PrimitiveIndex() * 3;
    uint material_index = offsets.y + PrimitiveIndex();
    float3 barycentric = float3(
        1.0 - attributes.barycentrics.x - attributes.barycentrics.y,
        attributes.barycentrics.x,
        attributes.barycentrics.y);
    MeshMaterial material = mesh_materials[material_index];
    PbrSettings surface_pbr = material.flags.w != 0 ? material.pbr : scene.pbr;
    float4 base_color = material.flags.x != 0
        ? surface_base_color(first, barycentric, material)
        : scene.base_color;
    float coverage = 1.0;
    if (material.flags.x != 0 && material.flags.y == 1)
    {
        if (base_color.a < material.metallic_roughness_normal_alpha.w)
        {
            IgnoreHit();
            return;
        }
    }
    else if (material.flags.x != 0 && material.flags.y != 0)
        coverage = saturate(base_color.a);

    float4 metallic_roughness = sample_material_texture(
        material.metallic_roughness_texture,
        first,
        barycentric);
    float metallic = material_metallic(
        material.flags.x != 0,
        material.metallic_roughness_normal_alpha.x,
        metallic_roughness,
        scene.material.x);

    float3 object_normal = normalize(cross(
        positions[first + 1].xyz - positions[first].xyz,
        positions[first + 2].xyz - positions[first].xyz));
    float3 world_normal = normalize(mul(
        mul(scene.normal, instance.normal),
        float4(object_normal, 0.0)).xyz);
    float3 world_direction = normalize(mul(
        scene.model,
        float4(WorldRayDirection(), 0.0)).xyz);
    payload.transmittance *= material_shadow_transmittance(
        base_color,
        coverage,
        metallic,
        dot(world_normal, world_direction),
        surface_pbr);
    if (max(
            payload.transmittance.r,
            max(payload.transmittance.g, payload.transmittance.b))
        <= RAY_MINIMUM)
    {
        payload.transmittance = float3(0.0);
        return;
    }
    IgnoreHit();
}

[shader("closesthit")]
void closest_hit(
    inout RayPayload payload,
    in BuiltInTriangleIntersectionAttributes attributes)
{
    MeshInstance instance = mesh_instances[InstanceID()];
    uint2 offsets = GeometryIndex() == 0
        ? instance.geometry_offsets.xy
        : instance.geometry_offsets.zw;
    uint first = offsets.x + PrimitiveIndex() * 3;
    uint material_index = offsets.y + PrimitiveIndex();
    float3 barycentric = float3(
        1.0 - attributes.barycentrics.x - attributes.barycentrics.y,
        attributes.barycentrics.x,
        attributes.barycentrics.y);
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
    float3 object_position = triangle_positions[0] * barycentric.x
        + triangle_positions[1] * barycentric.y
        + triangle_positions[2] * barycentric.z;
    MeshMaterial mesh_material = mesh_materials[material_index];
    NormalMode normal_mode = mesh_material.flags.w != 0
        ? NormalMode(mesh_material.surface.x)
        : scene.normal_mode;
    float3 object_normal = surface_normal(
        normal_mode,
        triangle_positions,
        triangle_normals,
        barycentric);

    PbrSettings surface_pbr = mesh_material.flags.w != 0
        ? mesh_material.pbr
        : scene.pbr;
    ToonSettings surface_toon = scene.toon;
    float3 world_position = mul(
        mul(scene.model, instance.model),
        float4(object_position, 1.0)).xyz;
    float3 normal = normalize(mul(
        mul(scene.normal, instance.normal),
        float4(object_normal, 0.0)).xyz);
    float4 sampled_normal = sample_material_texture(
        mesh_material.normal_texture,
        first,
        barycentric);
    if (mesh_material.normal_texture.settings.x != 0)
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
            mesh_material.metallic_roughness_normal_alpha.z);
    }
    float3 world_ray_direction = normalize(mul(
        scene.model,
        float4(WorldRayDirection(), 0.0)).xyz);
    float3 view = -world_ray_direction;
    bool front_face = dot(normal, view) >= 0.0;
    if (!front_face)
        normal = -normal;
    payload.normal = normal;
    payload.depth = length(scene.camera_position.xyz - world_position);
    float3 world_ray_origin = mul(
        scene.model,
        float4(WorldRayOrigin(), 1.0)).xyz;
    payload.ray_distance = length(world_position - world_ray_origin);
    payload.facing = 1.0 - saturate(dot(normal, view));
    payload.hit = 1;
    payload.material_index = material_index;
    float4 mesh_base_color = surface_base_color(first, barycentric, mesh_material);
    if (payload.probe != 0)
    {
        payload.color = float4(
            mesh_material.flags.x != 0
                ? mesh_base_color.rgb
                : scene.base_color.rgb,
            mesh_base_color.a);
        return;
    }

    if (surface_toon.shading_model == ShadingModel::Depth)
    {
        float depth = depth_buffer_value(world_position);
        payload.color = float4(
            float3(depth),
            mesh_material.flags.x != 0 ? mesh_base_color.a : scene.base_color.a);
        return;
    }

    if (surface_toon.shading_model == ShadingModel::Toon
        && surface_toon.texture_filter == ToonTextureFilter::Kuwahara
        && mesh_material.flags.x != 0
        && mesh_material.base_color_texture.settings.x != 0)
        mesh_base_color.rgb = screen_space_kuwahara(
            mesh_base_color.rgb,
            normal,
            payload.depth,
            surface_toon);

    float4 metallic_roughness = sample_material_texture(
        mesh_material.metallic_roughness_texture,
        first,
        barycentric);
    float3 base_color = mesh_material.flags.x != 0
        ? mesh_base_color.rgb
        : scene.base_color.rgb;
    float metallic = material_metallic(
        mesh_material.flags.x != 0,
        mesh_material.metallic_roughness_normal_alpha.x,
        metallic_roughness,
        scene.material.x);
    float roughness = material_roughness(
        mesh_material.flags.x != 0,
        mesh_material.metallic_roughness_normal_alpha.y,
        metallic_roughness,
        scene.material.y);
    bool toon = surface_toon.shading_model == ShadingModel::Toon;
    bool path_tracing = scene.pbr.path_tracing != PathTracingMode::Off;
    bool stochastic_transport = path_tracing || surface_pbr.transmission > 0.0;
    uint state = payload.random_seed;
    uint light_samples = stochastic_transport ? 1u : uint(scene.pbr.light_sampling_quality);
    ToonDirectLighting toon_lighting;
    if (toon)
    {
        ToonShadowTracer tracer;
        toon_lighting = integrate_toon_direct_lights(
            scene,
            world_position,
            normal,
            view,
            surface_toon,
            tracer);
    }
    float3 toon_environment_light = float3(0.0);
    float3 toon_environment_reflection = float3(0.0);
    float3 color = toon
        ? float3(0.0)
        : (path_tracing ? float3(0.0) : base_color * 0.02);
    float4 background = float4(0.0);

    if (!toon)
    {
        for (uint light_index = 0; light_index < scene.sun_light_count; ++light_index)
        {
            color += sample_pbr_sun(
                scene.sun_lights[light_index],
                world_position,
                normal,
                view,
                base_color,
                metallic,
                roughness,
                surface_pbr,
                light_samples,
                state);
        }
        for (uint light_index = 0; light_index < scene.point_light_count; ++light_index)
        {
            color += sample_pbr_point(
                scene.point_lights[light_index],
                world_position,
                normal,
                view,
                base_color,
                metallic,
                roughness,
                surface_pbr,
                light_samples,
                state);
        }
    }

    if (!toon && stochastic_transport && environment_enabled())
        color += sample_pbr_environment(
            world_position,
            normal,
            view,
            base_color,
            metallic,
            roughness,
            surface_pbr,
            state);

    if (environment_enabled() && (!stochastic_transport || toon))
    {
        uint environment_width;
        uint environment_height;
        uint environment_levels;
        if (scene.environment.source == EnvironmentSource::Composite)
            transmission_background_texture.GetDimensions(
                0,
                environment_width,
                environment_height,
                environment_levels);
        else
            environment_texture.GetDimensions(
                0,
                environment_width,
                environment_height,
                environment_levels);
        float max_lod = float(max(environment_levels, 1u) - 1u);
        float n_dot_v = saturate(dot(normal, view));
        float3 reflection = reflect(-view, normal);
        bool reflection_composed_hit;
        float3 environment = sample_environment(
            world_position,
            reflection,
            roughness * max_lod,
            reflection_composed_hit);
        if (toon)
        {
            bool diffuse_composed_hit;
            float3 diffuse_environment = sample_environment(
                world_position,
                normal,
                max_lod,
                diffuse_composed_hit);
            float diffuse_scale =
                (diffuse_composed_hit ? 1.0 : scene.environment.intensity) * 0.25;
            toon_environment_light = diffuse_environment * diffuse_scale;
            float reflection_scale =
                (reflection_composed_hit ? 1.0 : scene.environment.intensity)
                * metallic;
            toon_environment_reflection = environment * reflection_scale;
        }
        else
        {
            float3 fresnel = pbr_fresnel(base_color, metallic, n_dot_v, surface_pbr.ior);
            bool diffuse_composed_hit;
            float3 diffuse_environment = sample_environment(
                world_position,
                normal,
                max_lod,
                diffuse_composed_hit);
            color += diffuse_environment * base_color * (1.0 - metallic)
                    * (diffuse_composed_hit ? 1.0 : scene.environment.intensity)
                + environment * fresnel
                    * (reflection_composed_hit ? 1.0 : scene.environment.intensity);
        }
    }

    uint bounce_limit = pbr_transport_bounce_limit(scene.pbr);
    if (!toon
        && stochastic_transport
        && payload.path_bounce < bounce_limit)
    {
        PbrSample sample = pbr_sample(
            normal,
            view,
            front_face,
            base_color,
            metallic,
            roughness,
            surface_pbr,
            state);
        if (sample.valid)
        {
            float survival = 1.0;
            if (payload.path_bounce >= 2)
            {
                survival = clamp(max(sample.weight.r, max(sample.weight.g, sample.weight.b)), 0.1, 0.95);
                sample.valid = random_float(state) <= survival;
            }
            if (sample.valid)
            {
                float side = dot(sample.direction, normal) >= 0.0 ? 1.0 : -1.0;
                float3 origin = world_position + normal * (RAY_MINIMUM * side);
                float3 object_origin = mul(scene.model_inverse, float4(origin, 1.0)).xyz;
                float3 object_direction = normalize(mul(
                    scene.model_inverse,
                    float4(sample.direction, 0.0)).xyz);
                RayDesc bounce_ray = {
                    object_origin,
                    RAY_MINIMUM,
                    object_direction,
                    RAY_MAXIMUM
                };
                RayPayload bounce = {
                    float4(0.0),
                    float4(0.0),
                    float3(0.0),
                    RAY_MAXIMUM,
                    RAY_MAXIMUM,
                    0.0,
                    0,
                    0,
                    payload.path_bounce + 1,
                    state,
                    payload.transmitted != 0 || sample.refracted ? 1u : 0u,
                    0xffffffffu
                };
                TraceRay(
                    scene_acceleration,
                    RAY_FLAG_SKIP_PROCEDURAL_PRIMITIVES,
                    0xff,
                    0,
                    0,
                    0,
                    bounce_ray,
                    bounce);
                composite_virtual_surfaces(
                    bounce,
                    origin,
                    sample.direction,
                    state);
                float environment_mis = bounce.hit == 0
                    && environment_enabled()
                    && !sample.delta
                    ? power_heuristic(sample.pdf, UNIFORM_ENVIRONMENT_PDF)
                    : 1.0;
                color += bounce.color.rgb * sample.weight * environment_mis / survival;
                background.rgb += bounce.background.rgb * sample.weight / survival;
                background.a = saturate(background.a
                    + bounce.background.a
                        * max(sample.weight.r, max(sample.weight.g, sample.weight.b))
                        / survival);
            }
        }
    }

    if (toon)
        color = toon_surface_color(
            base_color,
            normal,
            view,
            float2(DispatchRaysIndex().xy) + 0.5,
            toon_environment_light,
            toon_environment_reflection,
            surface_toon,
            toon_lighting);

    if (payload.path_bounce == 0)
        color *= scene.material.z;
    if (payload.path_bounce == 0 && scene.pbr.denoising == 0)
        color = float3(
            aces_tone_map(color.r),
            aces_tone_map(color.g),
            aces_tone_map(color.b));
    if (payload.path_bounce == 0 && scene.pbr.denoising == 0)
    {
        color = saturate(color + background.rgb);
        color = float3(
            linear_to_srgb(color.r),
            linear_to_srgb(color.g),
            linear_to_srgb(color.b));
    }
    payload.color = float4(
        color,
        mesh_material.flags.x != 0 ? mesh_base_color.a : scene.base_color.a);
    payload.background = background;
}
