// Crossed, double-sided cutout cards occupy a cell without filling its box.
// Both cards use the same upright texture orientation on either side.
fn cross_uv(point: vec3<f32>, normal: vec3<f32>) -> vec2<f32> {
    let local = fract(point);
    return vec2<f32>(local.x, 1.0 - local.y);
}

fn intersect_cross(ray: CellRay, material: u32) -> Hit {
    var hit = Hit(0u, vec3<f32>(0.0), vec3<f32>(0.0), ray.cell, vec3<f32>(0.0), 0.0, 0u, 0u);
    var closest = ray.exit_time;
    let diagonal = inverseSqrt(2.0);
    let cell = vec3<f32>(ray.cell);
    let local_origin = ray.origin - cell;
    for (var plane = 0u; plane < 2u; plane += 1u) {
        let outward = vec3<f32>(diagonal, 0.0, select(-diagonal, diagonal, plane == 1u));
        let denominator = dot(ray.direction, outward);
        if denominator == 0.0 { continue; }
        let time = dot(vec3<f32>(0.5) - local_origin, outward) / denominator;
        if time < ray.entry_time || time > closest { continue; }
        let local = local_origin + ray.direction * time;
        if any(local < vec3<f32>(0.0)) || any(local > vec3<f32>(1.0)) { continue; }
        // Reconstruct the known plane instead of offsetting a rounded box hit.
        let point = cell + vec3<f32>(local.x, local.y, select(local.x, 1.0 - local.x, plane == 1u));
        let normal = select(outward, -outward, denominator > 0.0);
        if surface_color(material, point, normal).a < 0.5 { continue; }
        closest = time;
        hit = Hit(material, point, normal, ray.cell, point, time, 0u, material);
    }
    return hit;
}
