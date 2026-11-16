struct SceneLight {
    position_kind: vec4<f32>,
    radiance_radius: vec4<f32>,
    tangent_cosine: vec4<f32>,
    bitangent_pdf: vec4<f32>,
    cumulative: vec4<f32>,
};
struct SceneLights { counts: vec4<u32>, items: array<SceneLight, MAX_LIGHTS> };
@group(0) @binding(6) var<uniform> scene_lights: SceneLights;
struct LightSample { direction: vec3<f32>, radiance: vec3<f32>, limit: f32 };

fn sample_light(point: vec3<f32>, state: ptr<function, u32>) -> LightSample {
    let count = scene_lights.counts.x;
    if count == 0u { return LightSample(vec3<f32>(0.0), vec3<f32>(0.0), 0.0); }
    var index = 0u;
    if count > 1u {
        let selected = random(state);
        var lower = 0u;
        var upper = count;
        for (var step = 0u; step < LIGHT_SEARCH_STEPS; step += 1u) {
            if lower >= upper { break; }
            let middle = lower + (upper - lower) / 2u;
            if selected < scene_lights.items[middle].cumulative.x { upper = middle; }
            else { lower = middle + 1u; }
        }
        index = min(lower, count - 1u);
    }
    let light = scene_lights.items[index];
    return evaluate_light(light, point, vec2<f32>(random(state), random(state)), 1.0 / light.bitangent_pdf.w);
}

fn evaluate_light(light: SceneLight, point: vec3<f32>, coordinate: vec2<f32>, importance: f32) -> LightSample {
    let weighted = light.radiance_radius.rgb * importance;
    if light.position_kind.w < 0.5 {
        var direction = light.position_kind.xyz;
        if params.quality.x == 2u && light.tangent_cosine.w < 1.0 {
            let cosine = mix(1.0, light.tangent_cosine.w, coordinate.x);
            let sine = sqrt(max(0.0, 1.0 - cosine * cosine));
            let angle = coordinate.y * TAU;
            direction = direction * cosine + light.tangent_cosine.xyz * (sine * cos(angle)) + light.bitangent_pdf.xyz * (sine * sin(angle));
        }
        return LightSample(direction, weighted, UNBOUNDED_RAY);
    }
    let delta = light.position_kind.xyz - point;
    let squared = dot(delta, delta);
    let distance = sqrt(squared);
    if distance < MIN_LIGHT_DISTANCE { return LightSample(vec3<f32>(0.0), vec3<f32>(0.0), 0.0); }
    let center = delta / distance;
    let radius = light.radiance_radius.w;
    if params.quality.x != 2u || radius <= MIN_LIGHT_DISTANCE {
        return LightSample(center, weighted / max(squared, MIN_LIGHT_DISTANCE * MIN_LIGHT_DISTANCE), distance);
    }
    var cosine = 1.0 - 2.0 * coordinate.x;
    var normalization = 4.0 / (radius * radius);
    if distance > radius {
        let cos_max = sqrt(max(0.0, 1.0 - radius * radius / squared));
        // Stable solid angle and far-field inverse-square limit for tiny lights.
        let one_minus_cos = radius * radius / (squared * (1.0 + cos_max));
        cosine = 1.0 - one_minus_cos * coordinate.x;
        normalization = 2.0 / (squared * (1.0 + cos_max));
    }
    let sine = sqrt(max(0.0, 1.0 - cosine * cosine));
    let angle = coordinate.y * TAU;
    let frame = normal_frame(center);
    let direction = to_world(frame, vec3<f32>(sine * cos(angle), sine * sin(angle), cosine));
    let projection = dot(delta, direction);
    let root = sqrt(max(0.0, radius * radius - squared + projection * projection));
    let limit = select(projection + root, projection - root, distance > radius);
    return LightSample(direction, weighted * normalization, max(limit, MIN_LIGHT_DISTANCE));
}

fn light_visibility(hit: Hit, light: LightSample) -> vec3<f32> {
    if params.quality.x == 0u { return vec3<f32>(1.0); }
    let origin = offset_surface(hit.point, hit.normal);
    let limit = max(0.0, light.limit - dot(origin - hit.point, light.direction));
    var visibility = vec3<f32>(1.0);
    if TRANSMISSIVE { visibility = shadow_transmittance(origin, light.direction, limit, hit.from_medium); }
    else {
        let blocker = trace(origin, light.direction, limit);
        visibility = vec3<f32>(select(1.0, 0.0, blocker.material != 0u));
    }
    return mix(vec3<f32>(1.0), visibility, params.artistic.x);
}

fn lambert_irradiance(hit: Hit, light: LightSample) -> vec3<f32> {
    let cosine = max(0.0, dot(hit.normal, light.direction));
    if cosine <= 0.0 || !any(light.radiance > vec3<f32>(0.0)) { return vec3<f32>(0.0); }
    return light.radiance * cosine * light_visibility(hit, light);
}

fn direct_light(hit: Hit, state: ptr<function, u32>) -> vec3<f32> {
    var radiance = vec3<f32>(0.0);
    let samples = select(params.quality.y, 1u, params.quality.x == 0u);
    if !PATH_TRACED {
        // Preview always sums every source. Only soft visibility is stochastic:
        // never amplify a randomly selected point light by its selection PDF.
        let group_seed = *state;
        let advanced = random(state);
        for (var index = 0u; index < scene_lights.counts.x; index += 1u) {
            for (var sample = 0u; sample < samples; sample += 1u) {
                let coordinate = sample_square(sample, samples, group_seed ^ scramble(index));
                let light = evaluate_light(scene_lights.items[index], hit.point, coordinate, 1.0);
                radiance += lambert_irradiance(hit, light);
            }
        }
    } else {
        // Monte Carlo final rendering samples one source with its actual PDF.
        for (var sample = 0u; sample < samples; sample += 1u) {
            radiance += lambert_irradiance(hit, sample_light(hit.point, state));
        }
    }
    return radiance / f32(samples);
}

// Delta reflection sees the actual finite sun disk. Non-delta paths use NEE,
// avoiding a second copy of that energy in the environment estimator.
fn sun_radiance(direction: vec3<f32>) -> vec3<f32> {
    var radiance = vec3<f32>(0.0);
    for (var index = 0u; index < scene_lights.counts.x; index += 1u) {
        let light = scene_lights.items[index];
        if light.position_kind.w < 0.5 && light.tangent_cosine.w < 1.0 && dot(direction, light.position_kind.xyz) >= light.tangent_cosine.w {
            radiance += light.radiance_radius.rgb / (2.0 * (1.0 - light.tangent_cosine.w));
        }
    }
    return radiance;
}
