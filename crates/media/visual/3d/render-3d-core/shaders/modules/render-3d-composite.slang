#include "scene-parameter.slang"

module render_3d_composite;

import render_3d_abi;
import render_3d_ground;
import render_3d_pbr;
import render_3d_ray;

static const float COMPOSITE_PI = 3.14159265358979323846;

public float2 shared_composite_world_uv(SCENE_UNIFORMS scene, float3 direction)
{
    float3 rotated = normalize(mul(
        scene.environment_inverse_rotation,
        float4(direction, 0.0)).xyz);
    float3 right = normalize(mul(
        scene.camera_world,
        float4(1.0, 0.0, 0.0, 0.0)).xyz);
    float3 up = normalize(mul(
        scene.camera_world,
        float4(0.0, 1.0, 0.0, 0.0)).xyz);
    float3 forward = normalize(mul(
        scene.camera_world,
        float4(0.0, 0.0, -1.0, 0.0)).xyz);
    float3 local = float3(
        dot(rotated, right),
        dot(rotated, up),
        dot(rotated, forward));
    float2 hemisphere = float2(
        atan2(local.x, abs(local.z)) * 2.0 / COMPOSITE_PI,
        atan2(local.y, length(local.xz)) * 2.0 / COMPOSITE_PI);
    return float2(
        hemisphere.x * 0.5 + 0.5,
        0.5 - hemisphere.y * 0.5);
}

public bool shared_composed_background_uv(
    SCENE_UNIFORMS scene,
    float3 world_position,
    uint2 texture_dimensions,
    out float2 uv)
{
    uv = float2(0.0);
    float3 direction = normalize(world_position - scene.camera_position.xyz);
    float3 right = normalize(mul(
        scene.camera_world,
        float4(1.0, 0.0, 0.0, 0.0)).xyz);
    float3 up = normalize(mul(
        scene.camera_world,
        float4(0.0, 1.0, 0.0, 0.0)).xyz);
    float3 forward = normalize(mul(
        scene.camera_world,
        float4(0.0, 0.0, -1.0, 0.0)).xyz);
    float3 camera_direction = float3(
        dot(direction, right),
        dot(direction, up),
        dot(direction, forward));
    float2 ndc;
    CameraProjection projection = CameraProjection(uint(scene.camera_lens.w));
    switch (projection)
    {
    case CameraProjection::Equirectangular:
        {
            float longitude = atan2(camera_direction.x, camera_direction.z);
            float latitude = asin(clamp(camera_direction.y, -1.0, 1.0));
            ndc = float2(
                longitude / COMPOSITE_PI,
                latitude * 2.0 / COMPOSITE_PI);
            break;
        }
    case CameraProjection::Cylindrical:
        {
            float horizontal = length(camera_direction.xz);
            if (horizontal <= RAY_MINIMUM)
                return false;
            float longitude = atan2(camera_direction.x, camera_direction.z);
            ndc = float2(
                longitude / COMPOSITE_PI,
                camera_direction.y / (horizontal * scene.camera_lens.z));
            break;
        }
    case CameraProjection::Fisheye:
        {
            float angle = acos(clamp(camera_direction.z, -1.0, 1.0));
            float lens_radius = angle / scene.camera_lens.z;
            if (lens_radius > 1.0)
                return false;
            float transverse = length(camera_direction.xy);
            float2 lens_direction = transverse > RAY_MINIMUM
                ? camera_direction.xy / transverse
                : float2(0.0);
            float minimum_dimension = float(min(
                texture_dimensions.x,
                texture_dimensions.y));
            ndc = lens_direction * lens_radius * minimum_dimension
                / float2(texture_dimensions);
            break;
        }
    case CameraProjection::Perspective:
    case CameraProjection::Orthographic:
    default:
        {
            float4 clip = mul(
                scene.view_projection,
                float4(world_position, 1.0));
            if (clip.w <= RAY_MINIMUM)
                return false;
            ndc = clip.xy / clip.w;
            break;
        }
    }

    uv = float2(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    uint address_mode = uint(scene.transmission_background.z);
    if (address_mode == 0)
        uv = saturate(uv);
    else if (address_mode == 1)
        uv = frac(uv);
    else
        uv = 1.0 - abs(frac(uv * 0.5) * 2.0 - 1.0);
    return true;
}

public interface ICompositeSampler
{
    uint2 composite_dimensions();
    float4 composite_sample(float2 uv, float lod);
    float3 environment_sample(float3 direction, float lod);
}

public float4 shared_sample_composite<T : ICompositeSampler>(
    T sampler,
    float2 uv,
    float lod)
{
    float4 sampled = sampler.composite_sample(uv, lod);
    return float4(
        float3(
            srgb_to_linear(sampled.r),
            srgb_to_linear(sampled.g),
            srgb_to_linear(sampled.b)) * sampled.a,
        sampled.a);
}

public bool shared_environment_enabled(SCENE_UNIFORMS scene)
{
    return scene.environment.source == EnvironmentSource::Black
        || scene.environment.source == EnvironmentSource::Image
        || (scene.environment.source == EnvironmentSource::Composite
            && scene.transmission_background.y != 0.0)
        || (scene.transmission_background.y != 0.0
            && (scene.transmission_background.w != 0.0
                || scene.ground_count != 0));
}

public bool shared_sample_transmission_background_at_lod<T : ICompositeSampler>(
    SCENE_UNIFORMS scene,
    T sampler,
    float3 world_position,
    float lod,
    out float4 background)
{
    background = float4(0.0);
    if (scene.transmission_background.y == 0.0)
        return false;
    float2 uv;
    if (!shared_composed_background_uv(
        scene,
        world_position,
        sampler.composite_dimensions(),
        uv))
        return false;
    background = shared_sample_composite(sampler, uv, lod);
    return true;
}

public bool shared_sample_composed_planes<T : ICompositeSampler>(
    SCENE_UNIFORMS scene,
    T sampler,
    float3 world_origin,
    float3 world_direction,
    float lod,
    out float4 color,
    out float distance,
    out int ground_index)
{
    color = float4(0.0);
    distance = RAY_MAXIMUM;
    ground_index = -1;
    if (scene.transmission_background.y == 0.0)
        return false;

    for (uint index = 0; index < scene.ground_count; ++index)
    {
        Ground ground = scene.grounds[index];
        if (ground.normal_composite.w == 0.0)
            continue;
        float ground_distance;
        float3 ground_position;
        float3 ground_normal;
        if (intersect_ground(
            ground,
            world_origin,
            world_direction,
            ground_distance,
            ground_position,
            ground_normal)
            && ground_distance < distance)
        {
            float4 ground_color;
            if (shared_sample_transmission_background_at_lod(
                scene,
                sampler,
                ground_position,
                lod,
                ground_color))
            {
                ground_color.rgb *= ground.basis_z_intensity.w;
                color = ground_color;
                distance = ground_distance;
                ground_index = int(index);
            }
        }
    }

    if (scene.transmission_background.w != 0.0)
    {
        float3 camera_forward = normalize(mul(
            scene.camera_world,
            float4(0.0, 0.0, -1.0, 0.0)).xyz);
        float denominator = dot(world_direction, camera_forward);
        if (abs(denominator) > RAY_MINIMUM)
        {
            float3 center = scene.camera_position.xyz
                + camera_forward * scene.transmission_background.x;
            float candidate_distance = dot(center - world_origin, camera_forward)
                / denominator;
            float4 background;
            if (candidate_distance > RAY_MINIMUM
                && candidate_distance < distance
                && shared_sample_transmission_background_at_lod(
                    scene,
                    sampler,
                    world_origin + world_direction * candidate_distance,
                    lod,
                    background))
            {
                background.rgb *= scene.composed_plane_intensity.y;
                color = background;
                distance = candidate_distance;
                ground_index = -1;
            }
        }
    }
    return distance < RAY_MAXIMUM;
}

public float3 shared_sample_environment<T : ICompositeSampler>(
    SCENE_UNIFORMS scene,
    T sampler,
    float3 world_origin,
    float3 direction,
    float lod,
    out bool composed_hit)
{
    float4 composed_color;
    float composed_distance;
    int ground_index;
    composed_hit = shared_sample_composed_planes(
        scene,
        sampler,
        world_origin,
        direction,
        lod,
        composed_color,
        composed_distance,
        ground_index);
    if (composed_hit)
        return composed_color.rgb;
    if (scene.environment.source == EnvironmentSource::Image)
        return sampler.environment_sample(direction, lod);
    if (scene.environment.source == EnvironmentSource::Composite
        && scene.transmission_background.y != 0.0)
        return shared_sample_composite(
            sampler,
            shared_composite_world_uv(scene, direction),
            lod).rgb;
    if (scene.environment.source == EnvironmentSource::Black)
        return scene.environment_color.rgb;
    return float3(0.0);
}

public bool shared_sample_transmission_background<T : ICompositeSampler>(
    SCENE_UNIFORMS scene,
    T sampler,
    float3 world_origin,
    float3 world_direction,
    out float4 background)
{
    background = float4(0.0);
    if (scene.transmission_background.w == 0.0)
        return false;
    float3 camera_forward = normalize(mul(
        scene.camera_world,
        float4(0.0, 0.0, -1.0, 0.0)).xyz);
    float3 plane_center = scene.camera_position.xyz
        + camera_forward * scene.transmission_background.x;
    float denominator = dot(world_direction, camera_forward);
    if (abs(denominator) <= RAY_MINIMUM)
        return false;
    float distance = dot(plane_center - world_origin, camera_forward) / denominator;
    if (distance <= RAY_MINIMUM
        || !shared_sample_transmission_background_at_lod(
            scene,
            sampler,
            world_origin + world_direction * distance,
            0.0,
            background))
        return false;
    background.rgb *= scene.composed_plane_intensity.y;
    return true;
}
