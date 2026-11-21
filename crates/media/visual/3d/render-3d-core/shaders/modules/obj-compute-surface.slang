void surface_prepare(inout TransportWork state, uint2 pixel)
{
    if (state.phase != TRANSPORT_SHADE) return;
    ComputeSurface surface = prepare_lighting_surface(state.surface,state.direction,pixel);
    state.lighting_surface = surface;
    ComputeMaterial material = materials[surface.material_index];
    state.local = float4(0.0,0.0,0.0,material.flags.x != 0u ? surface.mesh_base_color.a : scene.base_color.a);
    state.environment_mode = 0u;
    if (OBJECT_SHADING_MODEL == ShadingModel::Depth)
    {
        float4 clip = mul(scene.view_projection,float4(surface.world_position,1.0));
        state.local.rgb = saturate(clip.z/max(abs(clip.w),RAY_MINIMUM));
        state.environment_mode = 2u;
    }
}

void surface_direct(inout TransportWork state, uint2 pixel)
{
    if (state.phase != TRANSPORT_SHADE || state.environment_mode == 2u) return;
    ComputeSurface surface = state.lighting_surface;
    if (OBJECT_SHADING_MODEL == ShadingModel::Toon) state.toon_lighting = surface_toon_direct(surface,state.direction);
    else
    {
        uint seed = state.seed;
        state.local.rgb = pbr_direct_lighting(surface,state.direction,seed);
        state.seed = seed;
    }
}

void surface_environment(inout TransportWork state)
{
    if (state.phase != TRANSPORT_SHADE || state.environment_mode == 2u) return;
    ComputeSurface surface = state.lighting_surface;
    PbrSettings settings = material_pbr(materials[surface.material_index]);
    bool stochastic = OBJECT_TRANSPORT && (scene.pbr.path_tracing != PathTracingMode::Off || settings.transmission > 0.0);
    state.environment_mode = OBJECT_SHADING_MODEL == ShadingModel::Pbr && stochastic && shared_environment_enabled(scene) ? 1u : 0u;
    if (state.environment_mode != 0u) return;
    float3 diffuse;
    float3 reflection;
    sample_surface_environment(surface.world_position,surface.normal,-state.direction,surface.roughness,diffuse,reflection);
    state.environment_diffuse = diffuse;
    state.environment_reflection = reflection;
    if (OBJECT_SHADING_MODEL == ShadingModel::Pbr)
    {
        state.local.rgb += surface_static_environment(surface,state.direction,settings,diffuse,reflection);
    }
}

void surface_reservoir(inout TransportWork state)
{
    if (state.phase != TRANSPORT_SHADE || state.environment_mode != 1u) return;
    ComputeSurface surface = state.lighting_surface;
    PbrSettings settings = material_pbr(materials[surface.material_index]);
    uint seed = state.seed;
    ComputePbrEnvironmentTracer tracer;
    state.environment_sample = sample_pbr_environment(scene,surface.world_position,surface.normal,-state.direction,
        surface.base_color,surface.metallic,surface.roughness,settings,seed,tracer);
    state.seed = seed;
}

void surface_visibility(inout TransportWork state)
{
    if (state.phase != TRANSPORT_SHADE || state.environment_mode != 1u) return;
    PbrEnvironmentSample sample = state.environment_sample;
    ComputeSurface surface = state.lighting_surface;
    float3 visibility = sample.target > 0.0 ? shadow_visibility(surface.world_position,surface.normal,sample.direction,RAY_MAXIMUM) : float3(0.0);
    state.local.rgb += finish_pbr_environment(sample,visibility);
}

void surface_toon_finish(inout TransportWork state, uint2 pixel)
{
    if (state.phase != TRANSPORT_SHADE || OBJECT_SHADING_MODEL != ShadingModel::Toon) return;
    ComputeSurface surface = state.lighting_surface;
    float3 direction = state.direction;
    float3 diffuse = state.environment_diffuse;
    float3 reflection = state.environment_reflection;
    ToonDirectLighting lighting = state.toon_lighting;
    float3 color = surface_toon_color(surface,direction,pixel,diffuse,reflection,lighting);
    state.local.rgb = color;
}

void surface_finish(inout TransportWork state, uint2 pixel)
{
    if (state.phase != TRANSPORT_SHADE) return;
    if (OBJECT_SHADING_MODEL != ShadingModel::Depth)
    {
        state.local.rgb = surface_display_color(state.local.rgb,state.lighting_bounce);
    }
    if (state.local_only != 0u)
    {
        state.result.radiance = state.local.rgb;
        state.result.background = float3(0.0);
        state.result.alpha = state.local.a;
            state.phase = TRANSPORT_DONE;
    }
}
