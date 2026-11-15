#include "scene-parameter.slang"
module render_3d_budget;
import render_3d_abi;
import render_3d_sampling;
import render_3d_ground;
public struct SamplingBudget
{
    public uint minimum_samples;
    public uint maximum_samples;
    public uint anti_aliasing_samples;
    public float relative_error;
    public uint adaptive;
};
public SamplingBudget sampling_budget(SCENE_UNIFORMS scene)
{
    SamplingBudget result;
    result.minimum_samples = 1u;
    result.maximum_samples = 1u;
    result.anti_aliasing_samples = anti_aliasing_sample_count(scene.toon.anti_aliasing);
    result.relative_error = 0.0;
    result.adaptive = scene.pbr.path_tracing != PathTracingMode::Off ? 1u : 0u;
    if (result.adaptive != 0u)
        adaptive_path_budget(scene.pbr,result.minimum_samples,result.maximum_samples,result.relative_error);
    uint depth_of_field_samples = scene.camera_lens.y > 0.0
        ? (scene.pbr.render_quality == RenderQuality::Interactive ? 2u : 8u) : 1u;
    uint transmission_samples = scene.pbr.transmission > 0.0 ? ground_light_sample_count(scene.pbr) : 1u;
    result.minimum_samples = max(result.minimum_samples,max(result.anti_aliasing_samples,max(transmission_samples,depth_of_field_samples)));
    result.maximum_samples = max(result.maximum_samples,result.minimum_samples);
    return result;
}
public struct SampleMoments
{
    public float4 color;
    public float4 background;
    public float4 squared_error;
    public uint count;
};
public bool accumulate_sample(inout SampleMoments state, SamplingBudget budget, float4 color, float4 background)
{
    state.count += 1u;
    float4 delta = color-state.color;
    state.color += delta/float(state.count);
    state.background += (background-state.background)/float(state.count);
    state.squared_error += delta*(color-state.color);
    return state.count >= budget.maximum_samples || (budget.adaptive != 0u
        && state.count >= budget.minimum_samples
        && adaptive_converged(state.color,state.squared_error,state.count,budget.relative_error));
}
