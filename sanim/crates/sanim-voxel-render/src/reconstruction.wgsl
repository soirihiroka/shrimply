// Geometry-guided diffuse reconstruction. Textures and final alpha never blur.
struct PostParams {
    position: vec4<f32>, right: vec4<f32>, up: vec4<f32>, forward: vec4<f32>,
    previous_position: vec4<f32>, previous_right: vec4<f32>, previous_up: vec4<f32>, previous_forward: vec4<f32>,
    size: vec4<u32>, control: vec4<f32>,
};
@group(0) @binding(0) var<uniform> post: PostParams;
@group(0) @binding(1) var illumination: texture_2d<f32>;
@group(0) @binding(2) var guide: texture_2d<f32>;
@group(0) @binding(3) var albedo: texture_2d<f32>;
@group(0) @binding(4) var previous_light: texture_2d<f32>;
@group(0) @binding(5) var previous_guide: texture_2d<f32>;

@vertex fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    return vec4<f32>(f32((index << 1u) & 2u) * 2.0 - 1.0, f32(index & 2u) * 2.0 - 1.0, 0.0, 1.0);
}
fn in_bounds(pixel: vec2<i32>) -> bool { return all(pixel >= vec2<i32>(0)) && all(pixel < vec2<i32>(post.size.xy)); }
const UNRESOLVED_POINT: f32 = 1e20;
fn valid_point(point: vec3<f32>) -> bool { return all(abs(point) < vec3<f32>(UNRESOLVED_POINT)); }
fn world_point(pixel: vec2<i32>, geometry: vec4<f32>, position: vec4<f32>, right: vec4<f32>, up: vec4<f32>, forward: vec4<f32>) -> vec3<f32> {
    let uv = (vec2<f32>(pixel) + 0.5) / vec2<f32>(post.size.xy);
    let offset = right.xyz * ((uv.x * 2.0 - 1.0) * right.w) + up.xyz * ((1.0 - uv.y * 2.0) * up.w);
    let normal = decode_normal(geometry.xy);
    let origin = position.xyz + select(offset, vec3<f32>(0.0), forward.w > 0.5);
    let direction = forward.xyz + select(vec3<f32>(0.0), offset, forward.w > 0.5);
    let denominator = dot(normal, direction);
    if abs(denominator) < 1e-6 { return vec3<f32>(UNRESOLVED_POINT); }
    return origin + direction * ((geometry.z - dot(normal, origin)) / denominator);
}
fn current_point(pixel: vec2<i32>, geometry: vec4<f32>) -> vec3<f32> { return world_point(pixel, geometry, post.position, post.right, post.up, post.forward); }
fn compatible(center: vec4<f32>, neighbor: vec4<f32>, point: vec3<f32>, other: vec3<f32>) -> bool {
    return center.w > 0.0 && center.w == neighbor.w && valid_point(point) && valid_point(other) && dot(decode_normal(center.xy), decode_normal(neighbor.xy)) > 0.98 && abs(dot(point - other, decode_normal(center.xy))) < MAX_PLANE_ERROR;
}
fn luminance(color: vec3<f32>) -> f32 { return dot(color, vec3<f32>(0.2126, 0.7152, 0.0722)); }
fn kernel(offset: i32) -> f32 { let distance = abs(offset); return select(select(6.0, 4.0, distance == 1), 1.0, distance == 2); }

@fragment fn spatial(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(position.xy);
    let center_guide = textureLoad(guide, pixel, 0);
    let center = textureLoad(illumination, pixel, 0);
    if center_guide.w == 0.0 { return center; }
    let point = current_point(pixel, center_guide);
    if !valid_point(point) { return center; }
    let color = load_lighting(center.rgb);
    let center_luminance = luminance(color);
    var total = vec3<f32>(0.0);
    var weights = vec3<f32>(0.0);
    for (var y = -2; y <= 2; y += 1) {
        for (var x = -2; x <= 2; x += 1) {
            let neighbor = pixel + vec2<i32>(x, y) * i32(post.size.z);
            if !in_bounds(neighbor) { continue; }
            let candidate = textureLoad(guide, neighbor, 0);
            if !compatible(center_guide, candidate, point, current_point(neighbor, candidate)) { continue; }
            let measured = textureLoad(illumination, neighbor, 0).rgb;
            let sample = load_lighting(measured);
            let difference = select(0.0, abs(luminance(sample) - center_luminance), all(center.rgb >= vec3<f32>(ZERO_LIGHT_LOG)));
            // Keep real light boundaries while suppressing noisy visibility.
            let weight = kernel(x) * kernel(y) * exp(-difference / (0.2 + max(center_luminance, luminance(sample)) * 0.5));
            let valid_weight = vec3<f32>(measured >= vec3<f32>(ZERO_LIGHT_LOG)) * weight;
            total += sample * valid_weight;
            weights += valid_weight;
        }
    }
    return vec4<f32>(store_measured(total / max(weights, vec3<f32>(1e-8)), weights > vec3<f32>(0.0)), center.a);
}

@fragment fn temporal(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(position.xy);
    let geometry = textureLoad(guide, pixel, 0);
    let current = textureLoad(illumination, pixel, 0);
    if geometry.w == 0.0 || post.control.w == 0.0 { return vec4<f32>(current.rgb, 1.0); }
    let point = current_point(pixel, geometry);
    if !valid_point(point) { return vec4<f32>(current.rgb, 1.0); }
    let delta = point - post.previous_position.xyz;
    let depth = dot(delta, post.previous_forward.xyz);
    if post.previous_forward.w > 0.5 && depth <= 0.0 { return vec4<f32>(current.rgb, 1.0); }
    let scale = select(1.0, depth, post.previous_forward.w > 0.5);
    let screen = vec2<f32>(dot(delta, post.previous_right.xyz) / (post.previous_right.w * scale), dot(delta, post.previous_up.xyz) / (post.previous_up.w * scale));
    let previous_pixel = vec2<i32>(floor(vec2<f32>((screen.x + 1.0) * 0.5, (1.0 - screen.y) * 0.5) * vec2<f32>(post.size.xy)));
    if !in_bounds(previous_pixel) { return vec4<f32>(current.rgb, 1.0); }
    let old_guide = textureLoad(previous_guide, previous_pixel, 0);
    let old_point = world_point(previous_pixel, old_guide, post.previous_position, post.previous_right, post.previous_up, post.previous_forward);
    if !compatible(geometry, old_guide, point, old_point) { return vec4<f32>(current.rgb, 1.0); }
    let valid = current.rgb >= vec3<f32>(ZERO_LIGHT_LOG);
    var minimum = select(vec3<f32>(exp2(MAX_LIGHT_LOG)), load_lighting(current.rgb), valid);
    var maximum = select(vec3<f32>(0.0), load_lighting(current.rgb), valid);
    for (var y = -1; y <= 1; y += 1) {
        for (var x = -1; x <= 1; x += 1) {
            let neighbor = pixel + vec2<i32>(x, y);
            if !in_bounds(neighbor) { continue; }
            let candidate = textureLoad(guide, neighbor, 0);
            if !compatible(geometry, candidate, point, current_point(neighbor, candidate)) { continue; }
            let measured = textureLoad(illumination, neighbor, 0).rgb;
            let color = load_lighting(measured);
            minimum = select(minimum, min(minimum, color), measured >= vec3<f32>(ZERO_LIGHT_LOG));
            maximum = select(maximum, max(maximum, color), measured >= vec3<f32>(ZERO_LIGHT_LOG));
        }
    }
    let old = textureLoad(previous_light, previous_pixel, 0);
    let expansion = (maximum - minimum) * 0.25 + vec3<f32>(0.005);
    let previous = clamp(load_lighting(old.rgb), minimum - expansion, maximum + expansion);
    let count = min(old.a + 1.0, f32(post.size.w));
    let blend = select(vec3<f32>(1.0), vec3<f32>(1.0 / count), old.rgb >= vec3<f32>(ZERO_LIGHT_LOG));
    let blended = mix(previous, load_lighting(current.rgb), blend);
    return vec4<f32>(store_measured(blended, valid), count);
}

@fragment fn composite(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(position.xy);
    let light = textureLoad(illumination, pixel, 0);
    let base = textureLoad(albedo, pixel, 0);
    if textureLoad(guide, pixel, 0).w == 0.0 { return vec4<f32>(light.rgb, base.a) * post.control.z; }
    let linear = load_lighting(light.rgb) * load_albedo(base.rgb);
    var color = min(linear, vec3<f32>(base.a));
    if post.control.x > 0.5 { color = tone_map(linear * post.control.y); }
    return vec4<f32>(color, base.a) * post.control.z;
}
