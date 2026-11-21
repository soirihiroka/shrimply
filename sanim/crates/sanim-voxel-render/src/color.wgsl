// Shared fitted tone-curve coefficients, including its analytic inverse.
const TONE_A: f32 = 2.51;
const TONE_B: f32 = 0.03;
const TONE_C: f32 = 2.43;
const TONE_D: f32 = 0.59;
const TONE_E: f32 = 0.14;
const SDR_RADIANCE_LIMIT: f32 = 1e10;

fn tone_map(linear: vec3<f32>) -> vec3<f32> {
    // Radiance above this threshold is already saturated in the RGBA8 output.
    // Bound the quadratic terms so finite high-emission materials cannot produce inf/inf.
    let x = clamp(linear, vec3<f32>(0.0), vec3<f32>(SDR_RADIANCE_LIMIT));
    let mapped = clamp((x * (TONE_A * x + TONE_B)) / (x * (TONE_C * x + TONE_D) + TONE_E), vec3<f32>(0.0), vec3<f32>(1.0));
    return pow(mapped, vec3<f32>(1.0 / 2.2));
}

fn inverse_tone_map(display: vec3<f32>) -> vec3<f32> {
    let y = pow(clamp(display, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(2.2));
    let a = TONE_A - TONE_C * y;
    let b = TONE_B - TONE_D * y;
    return (-b + sqrt(b * b + 4.0 * TONE_E * a * y)) / (2.0 * a);
}

