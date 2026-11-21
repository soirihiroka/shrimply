// Dielectrics share voxel.wgsl's traversal and exact geometric entry planes.
// Optical tint is Beer–Lambert transmittance; surface color remains diffuse albedo.
const MAX_MICROFACET_GAIN: f32 = 1e12;
fn optical_properties(material: u32) -> vec4<f32> {
    if material == 0u { return vec4<f32>(0.0, 1.0, 0.0, 0.0); }
    let base = material * MATERIAL_WORDS + 8u;
    return vec4<f32>(bitcast<f32>(materials[base]), bitcast<f32>(materials[base + 1u]), bitcast<f32>(materials[base + 2u]), 0.0);
}
fn is_transmissive(material: u32) -> bool {
    return material != 0u && optical_properties(material).x > 0.0;
}
fn medium_ior(material: u32) -> f32 {
    if material == 0u { return 1.0; }
    return optical_properties(material).y;
}
fn medium_attenuation(material: u32, distance: f32) -> vec3<f32> {
    if material == 0u { return vec3<f32>(1.0); }
    let base = material * MATERIAL_WORDS + 12u;
    let absorption = vec3<f32>(bitcast<f32>(materials[base]), bitcast<f32>(materials[base + 1u]), bitcast<f32>(materials[base + 2u]));
    return exp(-absorption * max(distance, 0.0));
}

// eta is etaIncident/etaTransmitted. Exact unpolarized dielectric Fresnel,
// including total internal reflection; identical indices are an identity boundary.
fn fresnel_dielectric(cosine: f32, eta: f32) -> f32 {
    if eta == 1.0 { return 0.0; }
    let c = clamp(cosine, 0.0, 1.0);
    let sin2 = eta * eta * max(0.0, 1.0 - c * c);
    if sin2 >= 1.0 { return 1.0; }
    let transmitted = sqrt(max(0.0, 1.0 - sin2));
    let parallel = (c - eta * transmitted) / (c + eta * transmitted);
    let perpendicular = (eta * c - transmitted) / (eta * c + transmitted);
    return 0.5 * (parallel * parallel + perpendicular * perpendicular);
}

fn liquid_normal(hit: Hit, direction: vec3<f32>) -> vec3<f32> {
    let base = hit.material * MATERIAL_WORDS + 16u;
    let strength = bitcast<f32>(materials[base]);
    let outward = select(hit.normal, -hit.normal, hit.to_medium == 0u);
    if strength == 0.0 || outward.y < 0.5 { return hit.normal; }
    let wavelength = bitcast<f32>(materials[base + 1u]);
    let speed = bitcast<f32>(materials[base + 2u]);
    let frequency = TAU / wavelength;
    let position = hit.point.xz * frequency;
    let phase = params.clock.x * (speed * frequency);
    // Analytic slopes of three traveling world-space waves. No voxel-ID seams,
    // no texture readback, no per-frame volume edits or finite-difference taps.
    let slope = strength * (
        vec2<f32>(0.8, 0.6) * (0.50 * cos(dot(position, vec2<f32>(0.8, 0.6)) - phase)) +
        vec2<f32>(-0.6, 0.8) * (0.30 * cos(dot(position, vec2<f32>(-0.6, 0.8)) * 1.73 + phase * 1.31)) +
        vec2<f32>(0.28, -0.96) * (0.20 * cos(dot(position, vec2<f32>(0.28, -0.96)) * 0.63 - phase * 0.77)));
    let perturbed = normalize(hit.normal + vec3<f32>(-slope.x, 0.0, -slope.y) * hit.normal.y);
    // Keep shading normals on the incident side; offsets always use geometry.
    if dot(perturbed, -direction) <= 0.001 { return hit.normal; }
    return perturbed;
}

fn ggx_lambda(cosine: f32, alpha: f32) -> f32 {
    let c2 = max(cosine * cosine, 1e-12);
    return 0.5 * (sqrt(1.0 + alpha * alpha * max(0.0, 1.0 - c2) / c2) - 1.0);
}
fn ggx_visible_normal(wo: vec3<f32>, alpha: f32, state: ptr<function, u32>) -> vec3<f32> {
    let stretched = normalize(vec3<f32>(alpha * wo.xy, wo.z));
    var tangent = vec3<f32>(1.0, 0.0, 0.0);
    if stretched.z < 0.99999 { tangent = normalize(cross(vec3<f32>(0.0, 0.0, 1.0), stretched)); }
    let bitangent = cross(stretched, tangent);
    let radius = sqrt(random(state));
    let phi = TAU * random(state);
    let x = radius * cos(phi);
    var y = radius * sin(phi);
    y = mix(sqrt(max(0.0, 1.0 - x * x)), y, (1.0 + stretched.z) * 0.5);
    let z = sqrt(max(0.0, 1.0 - x * x - y * y));
    let normal = tangent * x + bitangent * y + stretched * z;
    return normalize(vec3<f32>(normal.xy * alpha, max(1e-6, normal.z)));
}

struct DielectricSample { direction: vec3<f32>, weight: f32, transmitted: bool };
fn dielectric_sample(direction: vec3<f32>, normal: vec3<f32>, geometric: vec3<f32>, eta: f32, roughness: f32, state: ptr<function, u32>) -> DielectricSample {
    if eta == 1.0 { return DielectricSample(direction, 1.0, true); }
    let frame = normal_frame(normal);
    let wo = to_local(frame, -direction);
    let alpha = max(1e-4, roughness * roughness);
    var micro = normal;
    if roughness > 0.0 { micro = to_world(frame, ggx_visible_normal(wo, alpha, state)); }
    let fresnel = fresnel_dielectric(dot(-direction, micro), eta);
    let transmitted = random(state) >= fresnel;
    var wi = reflect(direction, micro);
    if transmitted { wi = refract(direction, micro, eta); }
    if (transmitted && dot(wi, geometric) >= 0.0) || (!transmitted && dot(wi, geometric) <= 0.0) || dot(wi, wi) == 0.0 {
        return DielectricSample(vec3<f32>(0.0), 0.0, transmitted);
    }
    var weight = select(1.0, eta * eta, transmitted);
    if roughness > 0.0 {
        let incoming_lambda = ggx_lambda(wo.z, alpha);
        let outgoing_lambda = ggx_lambda(dot(wi, normal), alpha);
        weight *= (1.0 + incoming_lambda) / (1.0 + incoming_lambda + outgoing_lambda);
    }
    return DielectricSample(normalize(wi), weight, transmitted);
}

// Straight light visibility: absorption + interface Fresnel, without bending the
// shadow connection. This bounded NEE approximation does not synthesize caustics.
fn shadow_transmittance(start: vec3<f32>, direction: vec3<f32>, limit: f32, initial_medium: u32) -> vec3<f32> {
    var origin = start;
    var medium = initial_medium;
    var visibility = vec3<f32>(1.0);
    var remaining = limit;
    for (var interfaces = 0u; interfaces < MAX_INTERFACES; interfaces += 1u) {
        let hit = trace_medium(origin, direction, remaining, medium);
        visibility *= medium_attenuation(medium, hit.distance);
        if hit.material == 0u { return visibility; }
        if hit.to_medium != 0u && !is_transmissive(hit.to_medium) { return vec3<f32>(0.0); }
        let optics = optical_properties(hit.material);
        let eta = medium_ior(medium) / medium_ior(hit.to_medium);
        visibility *= optics.x * (1.0 - fresnel_dielectric(dot(-direction, hit.normal), eta));
        if max(visibility.r, max(visibility.g, visibility.b)) < 0.0001 { return vec3<f32>(0.0); }
        remaining -= hit.distance;
        if remaining <= 0.0 { return visibility; }
        medium = hit.to_medium;
        origin = offset_surface(hit.point, -hit.normal);
    }
    return vec3<f32>(0.0);
}

fn rough_direct_light(hit: Hit, incident: vec3<f32>, normal: vec3<f32>, eta: f32, roughness: f32, state: ptr<function, u32>) -> vec3<f32> {
    if roughness == 0.0 || eta == 1.0 { return vec3<f32>(0.0); }
    let alpha = max(1e-4, roughness * roughness);
    let cosine_o = dot(-incident, normal);
    var sum = vec3<f32>(0.0);
    let samples = select(params.quality.y, 1u, params.quality.x == 0u);
    for (var sample = 0u; sample < samples; sample += 1u) {
        let light = sample_light(hit.point, state);
        let wi = light.direction;
        let cosine_i = dot(wi, normal);
        if cosine_i <= 0.0 || cosine_o <= 0.0 || dot(wi, hit.normal) <= 0.0 { continue; }
        let halfway = normalize(wi - incident);
        let nh = max(0.0, dot(normal, halfway));
        let denominator = max(0.0, 1.0 - nh * nh) + alpha * alpha * nh * nh;
        let distribution_pi = alpha * alpha / (denominator * denominator);
        let geometry = 1.0 / (1.0 + ggx_lambda(cosine_o, alpha) + ggx_lambda(cosine_i, alpha));
        let fresnel = fresnel_dielectric(dot(-incident, halfway), eta);
        let visibility = light_visibility(hit, light);
        let factor = min(MAX_MICROFACET_GAIN, distribution_pi * geometry * fresnel / (4.0 * cosine_o));
        sum += light.radiance * visibility * factor;
    }
    return sum / f32(samples);
}

fn flat_irradiance(hit: Hit, state: ptr<function, u32>) -> vec3<f32> {
    let visibility = ao_visibility(hit, state);
    var ambient = vec3<f32>(params.viewport.z);
    if params.clock.z > 0.5 { ambient *= tone_map(environment(hit.normal)); }
    let light = ambient * visibility + (1.0 - params.viewport.z) * direct_light(hit, state);
    let emission = bitcast<f32>(materials[hit.material * MATERIAL_WORDS + 7u]);
    return min(max(light, vec3<f32>(emission)) * params.viewport.w, vec3<f32>(SDR_RADIANCE_LIMIT));
}
fn flat_surface(hit: Hit, state: ptr<function, u32>) -> vec3<f32> {
    return hit_color(hit).rgb * flat_irradiance(hit, state);
}

// Preview follows one refracted ray, accumulating Fresnel environment reflection
// at each interface. Reflected scene geometry and Monte Carlo GGX use path mode.
fn flat_transmission(start: vec3<f32>, initial: vec3<f32>, first: Hit, background: vec4<f32>, state: ptr<function, u32>) -> vec4<f32> {
    var origin = start;
    var direction = initial;
    var medium = first.from_medium;
    var throughput = vec3<f32>(1.0);
    var result = vec3<f32>(0.0);
    for (var interfaces = 0u; interfaces <= MAX_INTERFACES; interfaces += 1u) {
        var hit = first;
        if interfaces != 0u { hit = trace_medium(origin, direction, UNBOUNDED_RAY, medium); }
        throughput *= medium_attenuation(medium, hit.distance);
        if hit.material == 0u {
            let coverage = 1.0 - (1.0 - background.a) * clamp(max(throughput.r, max(throughput.g, throughput.b)), 0.0, 1.0);
            return flat_encode(result + throughput * pow(background.rgb, vec3<f32>(2.2)) * background.a, coverage);
        }
        let optics = optical_properties(hit.material);
        if optics.x == 0.0 || (hit.to_medium != 0u && !is_transmissive(hit.to_medium)) { return flat_encode(result + throughput * pow(max(flat_surface(hit, state), vec3<f32>(0.0)), vec3<f32>(2.2)), 1.0); }
        if interfaces == MAX_INTERFACES { break; }
        let normal = liquid_normal(hit, direction);
        let eta = medium_ior(medium) / medium_ior(hit.to_medium);
        let fresnel = fresnel_dielectric(dot(-direction, normal), eta);
        let reflection = reflect(direction, normal);
        if fresnel < 1.0 { result += throughput * (optics.x * fresnel) * pow(tone_map(environment(reflection) * params.artistic.w), vec3<f32>(2.2)); }
        if optics.x < 1.0 {
            // Fully transmissive water has no diffuse lobe. Do not execute an
            // entire all-light visibility pass only to multiply it by zero.
            result += throughput * (1.0 - optics.x) * pow(max(flat_surface(hit, state), vec3<f32>(0.0)), vec3<f32>(2.2));
        }
        if fresnel == 1.0 { throughput *= optics.x; direction = reflection; origin = offset_surface(hit.point, hit.normal); }
        else {
            throughput *= optics.x * (1.0 - fresnel);
            direction = normalize(refract(direction, normal, eta));
            medium = hit.to_medium;
            origin = offset_surface(hit.point, -hit.normal);
        }
    }
    return flat_encode(result, 1.0);
}

fn flat_encode(linear: vec3<f32>, coverage: f32) -> vec4<f32> {
    let straight = clamp(linear / max(coverage, 1e-12), vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(pow(straight, vec3<f32>(1.0 / 2.2)) * coverage, coverage);
}
