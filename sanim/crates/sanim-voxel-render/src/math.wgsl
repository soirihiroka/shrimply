// Shared sampling and orthonormal frames for diffuse, dielectric and area lights.
fn random(state: ptr<function, u32>) -> f32 {
    *state = *state * 747796405u + 2891336453u;
    let word = ((*state >> ((*state >> 28u) + 4u)) ^ *state) * 277803737u;
    // Keep 24 exact mantissa bits: full u32 conversion can round up to 2^32.
    return f32(((word >> 22u) ^ word) >> 8u) / 16777216.0;
}

struct Frame { tangent: vec3<f32>, bitangent: vec3<f32>, normal: vec3<f32> };
fn normal_frame(normal: vec3<f32>) -> Frame {
    let helper = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(normal.y) > 0.9);
    let tangent = normalize(cross(helper, normal));
    return Frame(tangent, cross(normal, tangent), normal);
}
fn to_local(frame: Frame, w: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(w, frame.tangent), dot(w, frame.bitangent), dot(w, frame.normal));
}
fn to_world(frame: Frame, w: vec3<f32>) -> vec3<f32> {
    return frame.tangent * w.x + frame.bitangent * w.y + frame.normal * w.z;
}
fn cosine_direction(normal: vec3<f32>, state: ptr<function, u32>) -> vec3<f32> {
    let radius = sqrt(random(state));
    let angle = random(state) * TAU;
    return normalize(to_world(normal_frame(normal), vec3<f32>(radius * cos(angle), radius * sin(angle), sqrt(max(0.0, 1.0 - radius * radius)))));
}
