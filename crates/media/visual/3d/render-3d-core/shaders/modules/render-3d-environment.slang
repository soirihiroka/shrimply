#include "scene-parameter.slang"

module render_3d_environment;

import render_3d_abi;
import render_3d_pbr;
import render_3d_ray;
import render_3d_sampling;

static const float ENVIRONMENT_PI = 3.14159265358979323846;
public static const float SHARED_UNIFORM_ENVIRONMENT_PDF = 1.0 / (4.0 * ENVIRONMENT_PI);

public float2 environment_coordinates(
    float4x4 inverse_rotation,
    float3 direction)
{
    float3 rotated = normalize(mul(
        inverse_rotation,
        float4(direction, 0.0)).xyz);
    float longitude = atan2(rotated.z, rotated.x);
    return float2(
        frac(longitude / (2.0 * ENVIRONMENT_PI) + 0.5),
        acos(clamp(rotated.y, -1.0, 1.0)) / ENVIRONMENT_PI);
}

public interface IPbrEnvironmentTracer
{
    float3 radiance(float3 world_origin, float3 direction, out bool composed_hit);
    float3 visibility(float3 world_position, float3 normal, float3 direction);
}

public float3 integrate_pbr_environment<T : IPbrEnvironmentTracer>(
    SCENE_UNIFORMS scene,
    float3 world_position,
    float3 normal,
    float3 view,
    float3 base_color,
    float metallic,
    float roughness,
    PbrSettings settings,
    inout uint state,
    T tracer)
{
    uint candidate_count = uint(scene.pbr.light_sampling_quality);
    if (scene.pbr.render_quality == RenderQuality::Interactive)
        candidate_count = MINIMUM_VARIANCE_SAMPLES;

    float3 selected_light = float3(0.0);
    float3 selected_contribution = float3(0.0);
    float selected_target = 0.0;
    float reservoir_weight_sum = 0.0;
    for (uint candidate_index = 0; candidate_index < candidate_count; ++candidate_index)
    {
        float3 light = sample_unit_sphere(state);
        bool composed_hit;
        float3 radiance = tracer.radiance(world_position, light, composed_hit);
        if (!composed_hit)
            radiance *= scene.environment.intensity;
        float bsdf_pdf = pbr_reflection_pdf(
            normal,
            view,
            light,
            metallic,
            roughness,
            settings);
        float mis_weight = power_heuristic(SHARED_UNIFORM_ENVIRONMENT_PDF, bsdf_pdf);
        float3 contribution = pbr_light(
            normal,
            view,
            light,
            radiance,
            base_color,
            metallic,
            roughness,
            float3(1.0),
            settings) * mis_weight;
        float target = max(luminance(contribution), 0.0);
        float reservoir_weight = target / SHARED_UNIFORM_ENVIRONMENT_PDF;
        reservoir_weight_sum += reservoir_weight;
        if (reservoir_weight > 0.0
            && random_float(state) * reservoir_weight_sum < reservoir_weight)
        {
            selected_light = light;
            selected_contribution = contribution;
            selected_target = target;
        }
    }
    if (selected_target <= 0.0)
        return float3(0.0);
    return selected_contribution
        * tracer.visibility(world_position, normal, selected_light)
        * reservoir_weight_sum
        / (float(candidate_count) * selected_target);
}
