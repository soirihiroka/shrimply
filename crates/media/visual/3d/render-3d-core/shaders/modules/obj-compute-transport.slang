static const uint TRANSPORT_SHADE = 0u;
static const uint TRANSPORT_TRACE = 1u;
static const uint TRANSPORT_GROUND = 2u;
static const uint TRANSPORT_REFLECTION = 3u;
static const uint TRANSPORT_MISS = 4u;
static const uint TRANSPORT_UNWIND = 5u;
static const uint TRANSPORT_DONE = 6u;

struct TransportWork
{
    ComputeSurface surface;
    float3 direction;
    float3 throughput;
    float3 radiance;
    float3 background;
    uint transmitted;
    uint path_bounce;
    uint seed;
    float alpha;
    GroundTransportContinuation continuations[FINAL_TRANSPORT_BOUNCES];
    uint continuation_count;
    float4 local;
    TransportResult result;
    PbrEnvironmentSample environment_sample;
    ComputeSurface lighting_surface;
    ToonDirectLighting toon_lighting;
    float3 environment_diffuse;
    float3 environment_reflection;
    uint environment_mode;
    uint lighting_bounce;
    uint local_only;
    uint phase;
    PbrSample sample;
    RayDesc ray;
    float3 ray_origin;
    float4 virtual_color;
    float virtual_distance;
    int virtual_ground;
    GroundReflectionRay reflection;
    float3 ground_position;
    float3 ground_normal;
    uint ground_seed;
    float ground_shadow;
};

void transport_begin(inout TransportWork state,
    ComputeSurface first_surface, float3 first_direction,
    uint first_path_bounce, uint random_seed)
{
    state.surface = first_surface;
    state.direction = first_direction;
    state.throughput = float3(1.0);
    state.radiance = float3(0.0);
    state.background = float3(0.0);
    state.transmitted = 0u;
    state.path_bounce = first_path_bounce;
    state.seed = random_seed;
    state.alpha = materials[first_surface.material_index].flags.x != 0
        ? first_surface.mesh_base_color.a : scene.base_color.a;
    state.continuation_count = 0u;
    state.phase = TRANSPORT_SHADE;
    state.lighting_bounce = 1u;
    state.local_only = 0u;
}



RayDesc transport_object_ray(float3 origin, float3 direction)
{
    RayDesc ray = {mul(scene.model_inverse,float4(origin,1.0)).xyz,RAY_MINIMUM,
        normalize(mul(scene.model_inverse,float4(direction,0.0)).xyz),RAY_MAXIMUM};
    return ray;
}

void transport_scatter(inout TransportWork state)
{
    if (state.phase != TRANSPORT_SHADE) return;
    if (!OBJECT_TRANSPORT)
    {
        state.result.radiance = state.local.rgb;
        state.result.background = float3(0.0);
        state.result.alpha = state.local.a;
            state.phase = TRANSPORT_DONE;
        return;
    }
    state.radiance += state.throughput * state.local.rgb;
    PbrSettings settings = material_pbr(materials[state.surface.material_index]);
    bool path_tracing = scene.pbr.path_tracing != PathTracingMode::Off;
    if ((!path_tracing && settings.transmission <= 0.0)
        || state.path_bounce >= pbr_transport_bounce_limit(scene.pbr))
    {
    state.result.radiance = state.radiance;
    state.result.background = state.background;
    state.result.alpha = state.alpha;
    state.phase = TRANSPORT_UNWIND;
        return;
    }
    float3 normal = state.surface.normal;
    float3 view = -state.direction;
    bool front_face = dot(normal,view) >= 0.0;
    if (!front_face) normal = -normal;
    PbrSample sample = pbr_sample(normal,view,front_face,state.surface.base_color,
        state.surface.metallic,state.surface.roughness,settings,state.seed);
    float survival = 1.0;
    if (sample.valid && state.path_bounce >= 2u)
    {
        survival = clamp(max(sample.weight.r,max(sample.weight.g,sample.weight.b)),0.1,0.95);
        sample.valid = random_float(state.seed) <= survival;
    }
    if (!sample.valid)
    {
    state.result.radiance = state.radiance;
    state.result.background = state.background;
    state.result.alpha = state.alpha;
    state.phase = TRANSPORT_UNWIND;
        return;
    }
    state.throughput *= sample.weight / survival;
    state.transmitted = (state.transmitted != 0u || sample.refracted) ? 1u : 0u;
    float side = dot(sample.direction,normal) >= 0.0 ? 1.0 : -1.0;
    state.ray_origin = state.surface.world_position + normal * (RAY_MINIMUM * side);
    state.ray = transport_object_ray(state.ray_origin,sample.direction);
    state.sample = sample;
    state.phase = TRANSPORT_TRACE;
}

void transport_trace(inout TransportWork state)
{
    if (state.phase == TRANSPORT_DONE) return;
    if (state.phase == TRANSPORT_REFLECTION)
    {
        GroundReflectionRay reflection = state.reflection;
        RayDesc ray = transport_object_ray(reflection.origin,reflection.direction);
        ComputeSurface surface;
        float distance;
        if (trace_surface_hit(ray,surface,distance))
        {
            state.surface = surface;
            state.direction = reflection.direction;
            state.throughput = float3(1.0);
            state.radiance = float3(0.0);
            state.background = float3(0.0);
            state.transmitted = 0u;
            state.path_bounce = reflection.path_bounce;
            state.seed = reflection.random_seed;
            state.alpha = materials[surface.material_index].flags.x != 0
                ? surface.mesh_base_color.a : scene.base_color.a;
            state.phase = TRANSPORT_SHADE;
        }
        else
        {
            state.result.radiance = float3(0.0);
            state.result.background = float3(0.0);
            state.result.alpha = 0.0;
            state.phase = TRANSPORT_UNWIND;
        }
        return;
    }
    if (state.phase != TRANSPORT_TRACE) return;
    ComputeSurface next_surface;
    float query_distance;
    bool geometry_hit = trace_surface_hit(state.ray,next_surface,query_distance);
    float geometry_distance = geometry_hit ? length(next_surface.world_position-state.ray_origin) : RAY_MAXIMUM;
    ComputeCompositeSampler sampler;
    float4 virtual_color;
    float virtual_distance;
    int virtual_ground;
    bool virtual_hit = shared_sample_composed_planes(scene,sampler,state.ray_origin,
        state.sample.direction,0.0,virtual_color,virtual_distance,virtual_ground)
        && virtual_distance < geometry_distance;
    if (OBJECT_GROUNDS && virtual_hit && virtual_ground >= 0)
    {
        state.virtual_color = virtual_color;
        state.virtual_distance = virtual_distance;
        state.virtual_ground = virtual_ground;
        state.phase = TRANSPORT_GROUND;
    }
    else if (virtual_hit)
    {
        if (state.transmitted != 0u) state.background += state.throughput*virtual_color.rgb;
        else state.radiance += state.throughput*virtual_color.rgb;
    state.result.radiance = state.radiance;
    state.result.background = state.background;
    state.result.alpha = state.alpha;
    state.phase = TRANSPORT_UNWIND;
    }
    else if (!geometry_hit) state.phase = TRANSPORT_MISS;
    else
    {
        state.surface = next_surface;
        state.direction = state.sample.direction;
        state.path_bounce += 1u;
        state.phase = TRANSPORT_SHADE;
    }
}

void transport_ground_prepare(inout TransportWork state)
{
    SCENE_ARRAY_VIEW(Ground, scene, grounds);

    if (state.phase != TRANSPORT_GROUND) return;
    Ground ground = SCENE_ARRAY_AT(scene, grounds, state.virtual_ground);
    float3 position = state.ray_origin + state.sample.direction*state.virtual_distance;
    float3 normal = normalize(ground.normal_composite.xyz);
    if (scene.ground_shading_model == ShadingModel::Depth)
    {
        float4 clip = mul(scene.view_projection,float4(position,1.0));
        float depth = saturate(clip.z/max(abs(clip.w),RAY_MINIMUM));
        state.radiance += state.throughput*float3(depth);
    state.result.radiance = state.radiance;
    state.result.background = state.background;
    state.result.alpha = state.alpha;
    state.phase = TRANSPORT_UNWIND;
        return;
    }
    state.ground_position = position;
    state.ground_normal = normal;
    state.ground_seed = state.seed;
}

void transport_ground_shadow(inout TransportWork state)
{
    if (state.phase != TRANSPORT_GROUND) return;
    uint seed = state.ground_seed;
    float shadow = ground_shadow_occlusion(state.ground_position,state.ground_normal,seed);
    state.ground_seed = seed;
    state.ground_shadow = shadow;
}

void transport_ground_push(inout TransportWork state)
{
    SCENE_ARRAY_VIEW(Ground, scene, grounds);

    if (state.phase != TRANSPORT_GROUND) return;
    Ground ground = SCENE_ARRAY_AT(scene, grounds, state.virtual_ground);
    GroundReflectionRay reflection = sample_ground_reflection(scene,ground,state.ground_position,state.ground_normal,
        state.sample.direction,state.path_bounce+1u,state.ground_seed);
    GroundTransportContinuation continuation;
    continuation.reflection = reflection;
    continuation.virtual_color = state.virtual_color;
    continuation.radiance = state.radiance;
    continuation.background = state.background;
    continuation.throughput = state.throughput;
    continuation.alpha = state.alpha;
    continuation.shadow_occlusion = state.ground_shadow;
    continuation.receiver_strength = ground.settings.y*ground.position_opacity.w;
    continuation.transmitted = state.transmitted;
    state.continuations[state.continuation_count] = continuation;
    state.continuation_count += 1u;
    state.reflection = reflection;
    if (OBJECT_GROUND_REFLECTIONS && reflection.valid) state.phase = TRANSPORT_REFLECTION;
    else
    {
        state.result.radiance = float3(0.0);
        state.result.background = float3(0.0);
        state.result.alpha = 0.0;
        state.phase = TRANSPORT_UNWIND;
    }
}

void transport_ground(inout TransportWork state)
{
    transport_ground_prepare(state);
    transport_ground_shadow(state);
    transport_ground_push(state);
}

void transport_miss(inout TransportWork state)
{
    if (state.phase != TRANSPORT_MISS) return;
    ComputeCompositeSampler sampler;
    float4 transmitted_background;
    bool hit = state.transmitted != 0u && shared_sample_transmission_background(scene,sampler,
        state.ray_origin,state.sample.direction,transmitted_background);
    if (hit) state.background += state.throughput*transmitted_background.rgb;
    else
    {
        bool composed_hit;
        float3 environment = sample_environment(state.ray_origin,state.sample.direction,0.0,composed_hit);
        if (!composed_hit) environment *= scene.environment.intensity;
        float mis = shared_environment_enabled(scene) && !state.sample.delta
            ? power_heuristic(state.sample.pdf,SHARED_UNIFORM_ENVIRONMENT_PDF) : 1.0;
        state.radiance += state.throughput*environment*mis;
    }
    state.result.radiance = state.radiance;
    state.result.background = state.background;
    state.result.alpha = state.alpha;
    state.phase = TRANSPORT_UNWIND;
}

void transport_unwind(inout TransportWork state, uint2 pixel)
{
    if (state.phase != TRANSPORT_UNWIND) return;
    TransportResult completed = state.result;
    while (OBJECT_GROUNDS && state.continuation_count > 0u)
    {
        state.continuation_count -= 1u;
        GroundTransportContinuation continuation =
            state.continuations[state.continuation_count];
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

    state.result = completed;
    state.phase = TRANSPORT_DONE;
}

void transport_advance(inout TransportWork state, uint2 pixel)
{
    transport_scatter(state);
    transport_trace(state);
    transport_ground(state);
    transport_trace(state);
    transport_miss(state);
    transport_unwind(state,pixel);
}
