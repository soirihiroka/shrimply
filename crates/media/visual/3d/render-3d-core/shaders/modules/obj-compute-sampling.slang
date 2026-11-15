float4 display_transport_result(TransportResult result, out float4 background)
{
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


static const uint SAMPLE_PRIMARY = 0u;
static const uint SAMPLE_GROUND = 1u;
static const uint SAMPLE_REFLECTION = 2u;
static const uint SAMPLE_COMPLETE = 3u;
struct SampleRayWork
{
    float3 world_direction;
    float3 ground_origin;
    float4 color;
    float4 background;
    float visible_distance;
    uint seed;
    uint stochastic;
    uint phase;
    int ground_index;
    float shadow_occlusion;
    float receiver_strength;
    GroundReflectionRay reflection;
};
struct PixelAccumulator
{
    SamplingBudget budget;
    SampleMoments moments;
    uint done;
};
void sample_begin(inout SampleRayWork sample, inout TransportWork state,
    float2 position, uint2 pixel, uint2 dimensions, uint seed)
{
    CameraRay camera = camera_ray(scene,position,dimensions,true,seed);
    sample.world_direction = camera.world_direction;
    sample.ground_origin = camera.ground_origin;
    sample.color = float4(0.0);
    sample.background = float4(0.0);
    sample.visible_distance = RAY_MAXIMUM;
    sample.seed = seed;
    sample.stochastic = 0u;
    sample.phase = SAMPLE_PRIMARY;
    sample.ground_index = -1;
    state.continuation_count = 0u;
    state.result.radiance = float3(0.0);
    state.result.background = float3(0.0);
    state.result.alpha = 0.0;
    state.phase = TRANSPORT_DONE;
    if (!camera.valid) {sample.phase=SAMPLE_COMPLETE;return;}
    ComputeSurface surface;
    float distance;
    if (!trace_surface_hit(camera.object_ray,surface,distance)) return;
    sample.visible_distance = length(surface.world_position-camera.world_origin);
    sample.stochastic = OBJECT_SHADING_MODEL == ShadingModel::Pbr && OBJECT_TRANSPORT
        && (scene.pbr.path_tracing != PathTracingMode::Off
            || material_pbr(materials[surface.material_index]).transmission > 0.0) ? 1u : 0u;
    transport_begin(state,surface,sample.world_direction,0u,seed);
    state.local_only = sample.stochastic != 0u ? 0u : 1u;
    state.lighting_bounce = sample.stochastic != 0u ? 1u : 0u;
}
void sample_primary_finish(inout SampleRayWork sample, inout TransportWork state)
{
    if (sample.phase != SAMPLE_PRIMARY || state.phase != TRANSPORT_DONE) return;
    if (sample.stochastic != 0u) sample.color = display_transport_result(state.result,sample.background);
    else sample.color = float4(state.result.radiance,state.result.alpha);
    sample.phase = scene.ground_count == 0u ? SAMPLE_COMPLETE : SAMPLE_GROUND;
}
void sample_ground_begin(inout SampleRayWork sample, inout TransportWork state)
{
    SCENE_ARRAY_VIEW(Ground, scene, grounds);

    if (sample.phase != SAMPLE_GROUND) return;
    sample.phase = SAMPLE_COMPLETE;
    if (!OBJECT_GROUNDS) return;
    int ground_index = -1;
    float ground_distance = sample.visible_distance+RAY_MINIMUM;
    float3 position = float3(0.0);
    float3 normal = float3(0.0);
    for (uint index=0u; index<scene.ground_count; ++index)
    {
        float distance;
        float3 hit_position;
        float3 hit_normal;
        if (intersect_ground(SCENE_ARRAY_AT(scene, grounds, index),sample.ground_origin,
            sample.world_direction,distance,hit_position,hit_normal) && distance<ground_distance)
        {
            ground_index=int(index);ground_distance=distance;
            position=hit_position;normal=hit_normal;
        }
    }
    if (ground_index<0 || ground_distance>=sample.visible_distance+RAY_MINIMUM) return;
    Ground ground = SCENE_ARRAY_AT(scene, grounds, ground_index);
    if (scene.ground_shading_model == ShadingModel::Depth)
    {
        float4 clip=mul(scene.view_projection,float4(position,1.0));
        float depth=saturate(clip.z/max(abs(clip.w),RAY_MINIMUM));
        sample.color=float4(float3(depth),ground.position_opacity.w);
        return;
    }
    sample.ground_index=ground_index;
    sample.phase=SAMPLE_REFLECTION;
    state.ground_position=position;
    state.ground_normal=normal;
    state.ground_seed=sample.seed;

    state.phase=TRANSPORT_GROUND;
}
void sample_reflection_begin(inout SampleRayWork sample, inout TransportWork state)
{
    SCENE_ARRAY_VIEW(Ground, scene, grounds);

    if (sample.phase != SAMPLE_REFLECTION || state.phase != TRANSPORT_GROUND) return;
    Ground ground=SCENE_ARRAY_AT(scene, grounds, sample.ground_index);
    sample.shadow_occlusion=state.ground_shadow;
    sample.receiver_strength=ground.settings.y*ground.position_opacity.w;
    sample.reflection=sample_ground_reflection(scene,ground,state.ground_position,state.ground_normal,
        sample.world_direction,0u,state.ground_seed);
    state.result.radiance=float3(0.0);
    state.result.background=float3(0.0);
    state.result.alpha=0.0;
    state.local_only=0u;
    state.lighting_bounce=1u;
    if (OBJECT_GROUND_REFLECTIONS && sample.reflection.valid)
    {
        state.reflection=sample.reflection;
        state.phase=TRANSPORT_REFLECTION;
    }
    else {state.phase=TRANSPORT_DONE;}
}
void sample_ground_finish(inout SampleRayWork sample, inout TransportWork state, uint2 pixel)
{
    if (sample.phase != SAMPLE_REFLECTION || state.phase != TRANSPORT_DONE) return;
    float4 reflection=OBJECT_GROUND_REFLECTIONS && sample.reflection.valid
        ? finish_ground_reflection(sample.reflection,float4(state.result.radiance,state.result.alpha)) : float4(0.0);
    composite_over(sample.color,reflection);
    if (scene.ground_shading_model == ShadingModel::Toon)
        composite_over(sample.color,toon_ground_shadow(scene,sample.shadow_occlusion,
            sample.receiver_strength,float2(pixel)+0.5));
    else composite_ground_shadow(sample.color,sample.shadow_occlusion*sample.receiver_strength);
    sample.phase=SAMPLE_COMPLETE;
}
void complete_surface_transport(inout TransportWork state,uint2 pixel)
{
    while (state.phase != TRANSPORT_DONE)
    {
        surface_prepare(state,pixel);
        surface_direct(state,pixel);
        surface_environment(state);
        surface_reservoir(state);
        surface_visibility(state);
        surface_toon_finish(state,pixel);
        surface_finish(state,pixel);
        transport_advance(state,pixel);
    }
}
float4 render_sample(float2 position,uint2 pixel,uint2 dimensions,uint seed,out float4 background)
{
    SampleRayWork sample;
    TransportWork state;
    sample_begin(sample,state,position,pixel,dimensions,seed);
    complete_surface_transport(state,pixel);
    sample_primary_finish(sample,state);
    sample_ground_begin(sample,state);
    if (sample.phase == SAMPLE_REFLECTION)
    {
        transport_ground_shadow(state);
        sample_reflection_begin(sample,state);
        transport_trace(state);
        complete_surface_transport(state,pixel);
        sample_ground_finish(sample,state,pixel);
    }
    background=sample.background;
    return sample.color;
}
