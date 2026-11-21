// Explicit ray-footprint minification: traversal coverage remains exact mip zero.
fn mip_bilinear(uv: vec2<f32>, layer: i32, level: i32) -> vec4<f32> {
    let dimensions = vec2<i32>(textureDimensions(tiles, level));
    let position = uv * vec2<f32>(dimensions) - 0.5;
    let corner = vec2<i32>(floor(position));
    let fraction = fract(position);
    var weighted = vec4<f32>(0.0);
    for (var y = 0; y < 2; y += 1) {
        for (var x = 0; x < 2; x += 1) {
            let coordinate = clamp(corner + vec2<i32>(x,y), vec2<i32>(0), dimensions - 1);
            let texel = textureLoad(tiles, coordinate, layer, level);
            let weight = select(1.0 - fraction.x, fraction.x, x == 1) * select(1.0 - fraction.y, fraction.y, y == 1);
            weighted += vec4<f32>(pow(texel.rgb, vec3<f32>(2.2)) * texel.a, texel.a) * weight;
        }
    }
    return weighted;
}
fn minified_color(uv: vec2<f32>, layer: i32, footprint: f32) -> vec3<f32> {
    let maximum = f32(textureNumLevels(tiles) - 1u);
    let level = clamp(log2(max(footprint, 1.0)), 0.0, maximum);
    let lower = i32(floor(level));
    let first = mip_bilinear(uv, layer, lower);
    let second = mip_bilinear(uv, layer, min(lower + 1, i32(maximum)));
    let filtered = mix(first, second, fract(level));
    return pow(filtered.rgb / max(filtered.a, 1e-8), vec3<f32>(1.0 / 2.2));
}
