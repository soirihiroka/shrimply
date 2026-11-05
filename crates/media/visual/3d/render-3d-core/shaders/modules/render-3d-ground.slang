#include "scene-parameter.slang"

module render_3d_ground;

import render_3d_abi;
import render_3d_pbr;
import render_3d_ray;
import render_3d_sampling;
import render_3d_toon;

public uint ground_light_sample_count(PbrSettings settings)
{
    uint requested = max(uint(settings.light_sampling_quality), 1u);
    return settings.render_quality == RenderQuality::Interactive
        ? min(requested, MINIMUM_VARIANCE_SAMPLES)
        : requested;
}

public bool intersect_ground(
    Ground ground,
    float3 world_origin,
    float3 world_direction,
    out float distance,
    out float3 position,
    out float3 normal)
{
    normal = normalize(ground.normal_composite.xyz);
    float denominator = dot(world_direction, normal);
    distance = RAY_MAXIMUM;
    position = float3(0.0);
    if (abs(denominator) <= RAY_MINIMUM)
        return false;
    distance = dot(ground.position_opacity.xyz - world_origin, normal) / denominator;
    if (distance <= RAY_MINIMUM)
        return false;
    position = world_origin + world_direction * distance;
    if (ground.settings.w != 0.0)
    {
        float3 local = position - ground.position_opacity.xyz;
        float half_size = ground.basis_x_size.w * 0.5;
        if (abs(dot(local, normalize(ground.basis_x_size.xyz))) > half_size
            || abs(dot(local, normalize(ground.basis_z_intensity.xyz))) > half_size)
            return false;
    }
    return true;
}

public float ground_shadow_ratio(float blocked_illuminance, float total_illuminance)
{
    return total_illuminance <= RAY_MINIMUM
        ? 0.0
        : saturate(blocked_illuminance / total_illuminance);
}

public float ground_sun_illuminance(SunLight light, float3 normal, float3 direction)
{
    return luminance(light.color_intensity.rgb)
        * light.color_intensity.w
        * max(dot(normal, direction), 0.0);
}

public bool ground_point_illuminance(
    PointLight light,
    float3 sampled_position,
    float3 world_position,
    float3 normal,
    out float3 direction,
    out float distance,
    out float illuminance)
{
    float3 delta = sampled_position - world_position;
    float distance_squared = dot(delta, delta);
    distance = sqrt(distance_squared);
    direction = delta / max(distance, RAY_MINIMUM);
    illuminance = 0.0;
    if (distance >= light.range_padding.x)
        return false;
    float falloff = pow(1.0 - distance / light.range_padding.x, 2.0)
        / max(distance_squared, 0.01);
    illuminance = luminance(light.color_intensity.rgb)
        * light.color_intensity.w
        * falloff
        * max(dot(normal, direction), 0.0);
    return true;
}

public interface IGroundShadowTracer
{
    float visibility(
        float3 world_position,
        float3 normal,
        float3 direction,
        float maximum);
}

public interface IGroundReflectionTracer
{
    float4 radiance(
        float3 world_origin,
        float3 world_direction,
        uint path_bounce,
        uint random_seed);
}

public struct GroundReflectionRay
{
    public bool valid;
    public float3 origin;
    public float3 direction;
    public uint path_bounce;
    public uint random_seed;
    public float opacity;
    public bool tone_map;
};

public GroundReflectionRay sample_ground_reflection(
    SCENE_UNIFORMS scene,
    Ground ground,
    float3 world_position,
    float3 normal,
    float3 world_direction,
    uint path_bounce,
    inout uint random_seed)
{
    GroundReflectionRay result;
    result.valid = false;
    result.origin = float3(0.0);
    result.direction = float3(0.0);
    result.path_bounce = path_bounce + 1u;
    result.random_seed = random_seed;
    result.opacity = ground.position_opacity.w * ground.settings.x;
    result.tone_map = path_bounce == 0u && scene.pbr.denoising == 0u;
    if (ground.settings.x <= 0.0 || ground.position_opacity.w <= 0.0)
        return result;

    float3 reflection_normal = ground.settings.z > 0.0
        ? sample_ggx_half_vector(normal, ground.settings.z, random_seed)
        : normal;
    result.direction = reflect(world_direction, reflection_normal);
    if (dot(result.direction, normal) <= RAY_MINIMUM)
        return result;
    result.valid = true;
    result.origin = world_position + result.direction * RAY_MINIMUM;
    result.random_seed = random_seed;
    return result;
}

public float4 finish_ground_reflection(
    GroundReflectionRay ray,
    float4 radiance)
{
    radiance.a *= ray.opacity;
    if (ray.tone_map)
    {
        radiance.rgb = float3(
            linear_to_srgb(aces_tone_map(radiance.r)),
            linear_to_srgb(aces_tone_map(radiance.g)),
            linear_to_srgb(aces_tone_map(radiance.b)));
    }
    return radiance;
}

public float4 ground_reflection<T : IGroundReflectionTracer>(
    SCENE_UNIFORMS scene,
    Ground ground,
    float3 world_position,
    float3 normal,
    float3 world_direction,
    uint path_bounce,
    inout uint random_seed,
    T tracer)
{
    GroundReflectionRay ray = sample_ground_reflection(
        scene,
        ground,
        world_position,
        normal,
        world_direction,
        path_bounce,
        random_seed);
    if (!ray.valid)
        return float4(0.0);
    float4 result = tracer.radiance(
        ray.origin,
        ray.direction,
        ray.path_bounce,
        ray.random_seed);
    return finish_ground_reflection(ray, result);
}

public float integrate_ground_shadow<T : IGroundShadowTracer>(
    SCENE_UNIFORMS scene,
    float3 world_position,
    float3 normal,
    inout uint state,
    T tracer)
{
    uint sample_count = ground_light_sample_count(scene.pbr);
    float total_illuminance = 0.0;
    float blocked_illuminance = 0.0;
    for (uint light_index = 0; light_index < scene.sun_light_count; ++light_index)
    {
        SunLight light = scene.sun_lights[light_index];
        for (uint sample_index = 0; sample_index < sample_count; ++sample_index)
        {
            float3 direction = sample_cone(
                normalize(-light.direction_angular_radius.xyz),
                light.direction_angular_radius.w,
                state);
            float illuminance = ground_sun_illuminance(light, normal, direction);
            total_illuminance += illuminance;
            blocked_illuminance += illuminance * (
                1.0 - tracer.visibility(
                    world_position,
                    normal,
                    direction,
                    RAY_MAXIMUM));
        }
    }
    for (uint light_index = 0; light_index < scene.point_light_count; ++light_index)
    {
        PointLight light = scene.point_lights[light_index];
        for (uint sample_index = 0; sample_index < sample_count; ++sample_index)
        {
            float3 sampled_position = light.position_radius.xyz
                + sample_unit_sphere(state) * light.position_radius.w;
            float3 direction;
            float distance;
            float illuminance;
            if (!ground_point_illuminance(
                light,
                sampled_position,
                world_position,
                normal,
                direction,
                distance,
                illuminance))
                continue;
            total_illuminance += illuminance;
            blocked_illuminance += illuminance * (
                1.0 - tracer.visibility(
                    world_position,
                    normal,
                    direction,
                    distance));
        }
    }
    return ground_shadow_ratio(blocked_illuminance, total_illuminance);
}

public void composite_over(inout float4 destination, float4 source)
{
    float destination_weight = destination.a * (1.0 - source.a);
    float output_alpha = source.a + destination_weight;
    destination.rgb = output_alpha > 0.0
        ? (source.rgb * source.a + destination.rgb * destination_weight) / output_alpha
        : float3(0.0);
    destination.a = output_alpha;
}

public void composite_ground_shadow(inout float4 destination, float shadow_alpha)
{
    if (shadow_alpha <= 0.0)
        return;
    float destination_weight = destination.a * (1.0 - shadow_alpha);
    float output_alpha = shadow_alpha + destination_weight;
    destination.rgb = output_alpha > 0.0
        ? destination.rgb * destination_weight / output_alpha
        : float3(0.0);
    destination.a = output_alpha;
}

public float4 toon_ground_shadow(
    SCENE_UNIFORMS scene,
    float shadow_occlusion,
    float receiver_strength,
    float2 pixel_position)
{
    ToonSettings settings = scene.toon;
    settings.shading_model = scene.ground_shading_model;
    settings.shadow_kind = scene.ground_toon_shadow_kind;
    settings.shadow = scene.ground_toon_shadow;
    float3 color = settings.shadow.rgb * scene.material.z;
    if (scene.pbr.denoising == 0)
    {
        color = float3(
            linear_to_srgb(aces_tone_map(color.r)),
            linear_to_srgb(aces_tone_map(color.g)),
            linear_to_srgb(aces_tone_map(color.b)));
    }
    float tone_index = toon_tone_index(1.0 - shadow_occlusion, settings);
    float alpha = receiver_strength * settings.shadow.w
        * toon_shadow_pattern(pixel_position, tone_index, settings);
    return float4(color, alpha);
}
