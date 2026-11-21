// Independent low-discrepancy sampling, normal packing and HDR-safe storage.
const MAX_PLANE_ERROR: f32 = 0.005;
fn scramble(word: u32) -> u32 {
    var value = word;
    value ^= value >> 16u; value *= 0x7feb352du;
    value ^= value >> 15u; value *= 0x846ca68bu;
    return value ^ (value >> 16u);
}
fn unit_float(word: u32) -> f32 { return f32(word >> 8u) / 16777216.0; }
fn sample_square(index: u32, count: u32, seed: u32) -> vec2<f32> {
    let rotation = unit_float(scramble(seed));
    let first = fract((f32(index % count) + rotation) / f32(count));
    let second = unit_float(reverseBits(index) ^ scramble(seed ^ 0x9e3779b9u));
    return vec2<f32>(first, second);
}
fn encode_normal(normal: vec3<f32>) -> vec2<f32> {
    let projected = normal / (abs(normal.x) + abs(normal.y) + abs(normal.z));
    if projected.z >= 0.0 { return projected.xy; }
    return (vec2<f32>(1.0) - abs(projected.yx)) * select(vec2<f32>(-1.0), vec2<f32>(1.0), projected.xy >= vec2<f32>(0.0));
}
fn decode_normal(encoded: vec2<f32>) -> vec3<f32> {
    var normal = vec3<f32>(encoded, 1.0 - abs(encoded.x) - abs(encoded.y));
    if normal.z < 0.0 { normal = vec3<f32>((vec2<f32>(1.0) - abs(normal.yx)) * select(vec2<f32>(-1.0), vec2<f32>(1.0), normal.xy >= vec2<f32>(0.0)), normal.z); }
    return normalize(normal);
}
// Log packing covers tiny albedo and extreme HDR without half-float underflow.
// Keep kernel-weighted sums well below f32 overflow; tone-map only after albedo.
const ZERO_LIGHT_LOG: f32 = -128.0;
const INVALID_LIGHT_LOG: f32 = -65504.0;
const MAX_LIGHT_LOG: f32 = 116.0;
fn store_lighting(linear: vec3<f32>) -> vec3<f32> {
    return select(vec3<f32>(ZERO_LIGHT_LOG), min(log2(max(linear, vec3<f32>(1e-37))), vec3<f32>(MAX_LIGHT_LOG)), linear > vec3<f32>(0.0));
}
fn load_lighting(encoded: vec3<f32>) -> vec3<f32> {
    return select(vec3<f32>(0.0), exp2(clamp(encoded, vec3<f32>(-123.0), vec3<f32>(MAX_LIGHT_LOG))), encoded > vec3<f32>(ZERO_LIGHT_LOG));
}
fn store_measured(linear: vec3<f32>, valid: vec3<bool>) -> vec3<f32> { return select(vec3<f32>(INVALID_LIGHT_LOG), store_lighting(linear), valid); }
fn store_albedo(albedo: vec3<f32>) -> vec3<f32> { return select(vec3<f32>(INVALID_LIGHT_LOG), log2(max(albedo, vec3<f32>(1e-37))), albedo > vec3<f32>(0.0)); }
fn load_albedo(encoded: vec3<f32>) -> vec3<f32> { return select(vec3<f32>(0.0), exp2(max(encoded, vec3<f32>(-123.0))), encoded > vec3<f32>(INVALID_LIGHT_LOG)); }
