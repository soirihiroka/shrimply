override PATH_TRACED: bool = false;
override TRANSMISSIVE: bool = false;
override FOLIAGE: bool = false;
override INSTANCED: bool = false;
const TAU: f32 = 6.28318530718;
const UNBOUNDED_RAY: f32 = 1e30;
struct Params {
    dimensions: vec4<u32>,
    pages: vec4<u32>,
    position: vec4<f32>,
    right: vec4<f32>,
    up: vec4<f32>,
    forward: vec4<f32>,
    viewport: vec4<f32>,
    background: vec4<f32>,
    sky_top: vec4<f32>,
    composite: vec4<f32>,
    mode: vec4<u32>,
    environment_upper: vec4<f32>,
    environment_horizon: vec4<f32>,
    environment_lower: vec4<f32>,
    quality: vec4<u32>,
    artistic: vec4<f32>,
    clock: vec4<f32>,
    reconstruction: vec4<u32>,
};
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> pages: array<u32>;
@group(0) @binding(2) var<storage, read> voxels: array<u32>;
@group(0) @binding(3) var<storage, read> materials: array<u32>;
@group(0) @binding(4) var tiles: texture_2d_array<f32>;
@group(0) @binding(5) var nearest_sampler: sampler;

@vertex fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    return vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}

fn page_at(cell: vec3<i32>) -> u32 {
    if any(cell < vec3<i32>(0)) || any(cell >= vec3<i32>(params.dimensions.xyz)) { return 0u; }
    let brick = vec3<u32>(cell) / CHUNK_EDGE;
    return pages[brick.x + params.pages.x * (brick.y + params.pages.y * brick.z)];
}

fn material_at(cell: vec3<i32>) -> u32 {
    let page = page_at(cell);
    if page == 0u { return 0u; }
    if (page & 0x80000000u) != 0u { return page & 0x7fffffffu; }
    if INSTANCED { return instance_material_at(cell, page); }
    let edge = CHUNK_EDGE;
    let local = vec3<u32>(cell) % edge;
    return voxels[(page - 1u) * edge * edge * edge + local.x + edge * (local.y + edge * local.z)];
}

struct Crossing { time: f32, normal: vec3<f32>, axes: vec3<bool> };
fn next_boundary(origin: vec3<f32>, direction: vec3<f32>, cell: vec3<i32>, edge: i32) -> Crossing {
    var next = Crossing(1e30, vec3<f32>(0.0), vec3<bool>(false));
    var times = vec3<f32>(1e30);
    for (var axis = 0u; axis < 3u; axis += 1u) {
        if direction[axis] != 0.0 {
            // Traversal steps are one cell or one power-of-two model brick.
            let base = cell[axis] & -edge;
            let boundary = f32(select(base, base + edge, direction[axis] > 0.0));
            let time = (boundary - origin[axis]) / direction[axis];
            times[axis] = time;
            if time < next.time {
                next.time = time;
                next.normal = vec3<f32>(0.0);
                next.normal[axis] = -sign(direction[axis]);
            }
        }
    }
    next.axes = times == vec3<f32>(next.time);
    return next;
}

fn entered_cell(point: vec3<f32>, direction: vec3<f32>) -> vec3<i32> {
    let lower = floor(point);
    return vec3<i32>(lower) - vec3<i32>(select(vec3<u32>(0u), vec3<u32>(1u), (direction < vec3<f32>(0.0)) & (point == lower)));
}

fn face_index(normal: vec3<f32>) -> u32 {
    if normal.x > 0.5 { return 0u; }
    if normal.x < -0.5 { return 1u; }
    if normal.y > 0.5 { return 2u; }
    if normal.y < -0.5 { return 3u; }
    if normal.z > 0.5 { return 4u; }
    return 5u;
}

fn face_uv(point: vec3<f32>, normal: vec3<f32>) -> vec2<f32> {
    let p = fract(point);
    if normal.x > 0.5 { return vec2<f32>(1.0 - p.z, 1.0 - p.y); }
    if normal.x < -0.5 { return vec2<f32>(p.z, 1.0 - p.y); }
    if normal.y > 0.5 { return vec2<f32>(p.x, p.z); }
    if normal.y < -0.5 { return vec2<f32>(p.x, 1.0 - p.z); }
    if normal.z > 0.5 { return vec2<f32>(p.x, 1.0 - p.y); }
    return vec2<f32>(1.0 - p.x, 1.0 - p.y);
}

fn unpack_color(rgba: u32) -> vec4<f32> {
    return vec4<f32>(f32((rgba >> 24u) & 255u), f32((rgba >> 16u) & 255u), f32((rgba >> 8u) & 255u), f32(rgba & 255u)) / 255.0;
}

fn surface_sample(material: u32, point: vec3<f32>, normal: vec3<f32>, footprint: f32) -> vec4<f32> {
    let offset = material * MATERIAL_WORDS;
    let tint = unpack_color(materials[offset]);
    let layer = materials[offset + 1u + face_index(normal)];
    if layer == 0xffffffffu { return tint; }
    var uv = face_uv(point, normal);
    if FOLIAGE && materials[offset + 11u] == 1u { uv = cross_uv(point, normal); }
    let index = i32(layer & 0x7fffffffu);
    let original = textureSampleLevel(tiles, nearest_sampler, uv, index, 0.0);
    if footprint <= 1.0 { return tint * original; }
    return tint * vec4<f32>(minified_color(uv, index, footprint), original.a);
}
fn surface_color(material: u32, point: vec3<f32>, normal: vec3<f32>) -> vec4<f32> { return surface_sample(material, point, normal, 0.0); }

fn local_ao(cell: vec3<i32>, point: vec3<f32>, normal: vec3<f32>) -> f32 {
    var a = vec3<i32>(0);
    var b = vec3<i32>(0);
    let local = fract(point);
    if abs(normal.x) > 0.5 {
        a.y = select(-1, 1, local.y > 0.5); b.z = select(-1, 1, local.z > 0.5);
    } else if abs(normal.y) > 0.5 {
        a.x = select(-1, 1, local.x > 0.5); b.z = select(-1, 1, local.z > 0.5);
    } else {
        a.x = select(-1, 1, local.x > 0.5); b.y = select(-1, 1, local.y > 0.5);
    }
    let outside = cell + vec3<i32>(normal);
    let side_a = f32(select(0u, 1u, material_at(outside + a) != 0u));
    let side_b = f32(select(0u, 1u, material_at(outside + b) != 0u));
    let corner = f32(select(0u, 1u, material_at(outside + a + b) != 0u));
    return 1.0 - params.artistic.y * (side_a + side_b + corner) / 3.0;
}

struct Hit { material: u32, point: vec3<f32>, normal: vec3<f32>, cell: vec3<i32>, uv_point: vec3<f32>, distance: f32, from_medium: u32, to_medium: u32 };
struct CellRay { origin: vec3<f32>, direction: vec3<f32>, cell: vec3<i32>, entry_time: f32, exit_time: f32, normal: vec3<f32>, crossed_face: bool };
fn hit_color(hit: Hit) -> vec4<f32> {
    let normal = select(hit.normal, -hit.normal, TRANSMISSIVE && hit.to_medium == 0u);
    if params.reconstruction.x == 0u && params.reconstruction.y == 1u { return surface_color(hit.material, hit.uv_point, normal); }
    let delta = hit.point - params.position.xyz;
    let view_depth = max(0.0, dot(delta, params.forward.xyz));
    var direction = params.forward.xyz;
    if params.forward.w > 0.5 { direction = normalize(delta); }
    let pixel_width = 2.0 * params.up.w / params.viewport.y * select(1.0, view_depth, params.forward.w > 0.5);
    let grazing = max(0.1, abs(dot(normal, direction)));
    let texture_size = textureDimensions(tiles, 0);
    let footprint = pixel_width * f32(max(texture_size.x, texture_size.y)) / grazing;
    return surface_sample(hit.material, hit.uv_point, normal, footprint);
}
fn trace(origin: vec3<f32>, direction: vec3<f32>, limit: f32) -> Hit {
    return trace_medium(origin, direction, limit, 0u);
}

// One traversal serves primary, indirect, AO and transmission queries. The
// medium is constant during this query: contiguous cells never create surfaces.
fn trace_medium(origin: vec3<f32>, direction: vec3<f32>, limit: f32, medium: u32) -> Hit {
    let miss = Hit(0u, vec3<f32>(0.0), vec3<f32>(0.0), vec3<i32>(0), vec3<f32>(0.0), select(0.0, limit, TRANSMISSIVE && medium != 0u), 0u, 0u);
    var enter = -1e30;
    var exit = limit;
    var normal = vec3<f32>(0.0);
    var exit_normal = vec3<f32>(0.0);
    for (var axis = 0u; axis < 3u; axis += 1u) {
        if direction[axis] == 0.0 {
            if origin[axis] < 0.0 || origin[axis] >= f32(params.dimensions[axis]) { return miss; }
        } else {
            let first = -origin[axis] / direction[axis];
            let second = (f32(params.dimensions[axis]) - origin[axis]) / direction[axis];
            let near = min(first, second);
            if near > enter { enter = near; normal = vec3<f32>(0.0); normal[axis] = -sign(direction[axis]); }
            let far = max(first, second);
            if far < exit { exit = far; exit_normal = vec3<f32>(0.0); exit_normal[axis] = -sign(direction[axis]); }
        }
    }
    if exit < max(enter, 0.0) { return miss; }
    if enter < 0.0 {
        normal = vec3<f32>(0.0);
        var axis = 0u;
        if abs(direction.y) > abs(direction.x) { axis = 1u; }
        if abs(direction.z) > abs(direction[axis]) { axis = 2u; }
        normal[axis] = -sign(direction[axis]);
    }
    let start = max(enter, 0.0);
    var crossed_face = enter >= 0.0;
    let local_origin = clamp(origin + direction * start, vec3<f32>(0.0), vec3<f32>(params.dimensions.xyz));
    exit -= start;
    var time = 0.0;
    var cell = entered_cell(local_origin, direction);
    for (var steps = 0u; steps < params.pages.w; steps += 1u) {
        if time >= exit { break; }
        if any(cell < vec3<i32>(0)) || any(cell >= vec3<i32>(params.dimensions.xyz)) { break; }
        let page = page_at(cell);
        let candidate = material_at(cell);
        if candidate != 0u || (TRANSMISSIVE && medium != 0u) {
            if !TRANSMISSIVE || candidate != medium {
                if FOLIAGE && candidate != 0u && materials[candidate * MATERIAL_WORDS + 11u] == 1u {
                    let cell_exit = min(exit, next_boundary(local_origin, direction, cell, 1).time);
                    let card = intersect_cross(CellRay(local_origin, direction, cell, time, cell_exit, normal, crossed_face), candidate);
                    if card.material != 0u { return Hit(card.material, card.point, card.normal, card.cell, card.uv_point, start + card.distance, medium, candidate); }
                } else {
                var point = local_origin + direction * time;
                if crossed_face {
                    // Reconstruct the actual integer entry plane, not a rounded
                    // subtraction. Preserve synthetic t=0 origins inside solids.
                    point = select(point, vec3<f32>(cell) + max(normal, vec3<f32>(0.0)), normal != vec3<f32>(0.0));
                }
                let material = select(candidate, medium, TRANSMISSIVE && candidate == 0u);
                let surface_normal = select(normal, -normal, TRANSMISSIVE && candidate == 0u);
                let face = materials[material * MATERIAL_WORDS + 1u + face_index(surface_normal)];
                let uv_point = point + direction * params.position.w;
                if face == 0xffffffffu || (face & 0x80000000u) == 0u || surface_color(material, uv_point, surface_normal).a >= 0.5 {
                    let owner = select(cell, cell + vec3<i32>(normal), TRANSMISSIVE && candidate == 0u);
                    return Hit(material, point, normal, owner, uv_point, start + time, medium, candidate);
                }
                }
            }
        }
        // Uniform bricks containing the current medium have no internal faces.
        let uniform_medium = TRANSMISSIVE && medium != 0u && (page & 0x80000000u) != 0u && (page & 0x7fffffffu) == medium;
        let edge = select(i32(CHUNK_EDGE), 1, page != 0u && !uniform_medium);
        let next = next_boundary(local_origin, direction, cell, edge);
        var next_cell = entered_cell(local_origin + direction * next.time, direction);
        for (var axis = 0u; axis < 3u; axis += 1u) {
            if next.axes[axis] {
                let base = cell[axis] & -edge;
                next_cell[axis] = select(base - 1, base + edge, direction[axis] > 0.0);
            } else if edge == 1 { next_cell[axis] = cell[axis]; }
        }
        cell = next_cell;
        time = next.time;
        normal = next.normal;
        crossed_face = true;
    }
    if TRANSMISSIVE && medium != 0u && start + exit < limit {
        var point = local_origin + direction * exit;
        let boundary = select(vec3<f32>(0.0), vec3<f32>(params.dimensions.xyz), direction > vec3<f32>(0.0));
        point = select(point, boundary, exit_normal != vec3<f32>(0.0));
        let owner = entered_cell(point, -direction);
        return Hit(medium, point, exit_normal, owner, point - direction * params.position.w, start + exit, medium, 0u);
    }
    return miss;
}

fn environment(direction: vec3<f32>) -> vec3<f32> {
    let elevation = clamp(direction.y, -1.0, 1.0);
    if elevation >= 0.0 { return mix(params.environment_horizon.rgb, params.environment_upper.rgb, elevation); }
    return mix(params.environment_horizon.rgb, params.environment_lower.rgb, -elevation);
}

fn offset_surface(point: vec3<f32>, normal: vec3<f32>) -> vec3<f32> {
    var result = point;
    for (var axis = 0u; axis < 3u; axis += 1u) {
        if normal[axis] != 0.0 {
            if abs(point[axis]) < 1.0 { result[axis] += normal[axis] * params.position.w * 5.0; }
            else {
                let bits = bitcast<u32>(point[axis]);
                result[axis] = bitcast<f32>(select(bits - 4u, bits + 4u, normal[axis] > 0.0));
            }
        }
    }
    return result;
}

fn ao_visibility(hit: Hit, state: ptr<function, u32>) -> f32 {
    if params.quality.z == 0u { return 1.0; }
    if params.quality.z == 1u { return local_ao(hit.cell, hit.uv_point, hit.normal); }
    let origin = offset_surface(hit.point, hit.normal);
    var occlusion = 0.0;
    // AO has its own deterministic stream, so toggling this artistic effect does
    // not resample the physical path or its direct shadows.
    var ao_state = *state ^ 0x9e3779b9u;
    for (var sample = 0u; sample < params.quality.w; sample += 1u) {
        let direction = cosine_direction(hit.normal, &ao_state);
        let blocker = trace(origin, direction, params.artistic.z);
        if blocker.material != 0u { occlusion += 1.0 - clamp(blocker.distance / params.artistic.z, 0.0, 1.0); }
    }
    return 1.0 - params.artistic.y * occlusion / f32(params.quality.w);
}

struct PathRadiance { linear: vec3<f32>, background: vec3<f32>, coverage: f32, foreground: f32 };
fn path_radiance(start: vec3<f32>, initial: vec3<f32>, first: Hit, ambient_visibility: f32, background: vec4<f32>, state: ptr<function, u32>) -> PathRadiance {
    var origin = start;
    var direction = initial;
    var medium = first.from_medium;
    var throughput = vec3<f32>(1.0);
    var radiance = vec3<f32>(0.0);
    var eta_scale = 1.0;
    var bounce = 0u;
    var interfaces = 0u;
    var specular_sun = false;
    var camera_transmission = true;
    let events = select(params.mode.y, params.mode.y + MAX_INTERFACES, TRANSMISSIVE);
    for (var event = 0u; event < events; event += 1u) {
        var hit = first;
        if event != 0u { hit = trace_medium(origin, direction, UNBOUNDED_RAY, medium); }
        if TRANSMISSIVE && medium != 0u { throughput *= medium_attenuation(medium, hit.distance); }
        if hit.material == 0u {
            if TRANSMISSIVE && camera_transmission {
                // A directly transmitted camera ray sees the visible backdrop,
                // independently of the environment used to illuminate objects.
                let transmittance = clamp(throughput, vec3<f32>(0.0), vec3<f32>(1.0));
                let coverage = 1.0 - (1.0 - background.a) * max(transmittance.r, max(transmittance.g, transmittance.b));
                // Put the display backdrop into the same HDR working space as
                // reflected light, preserving its exact appearance/exposure on
                // an index-matched, unabsorbing path. This avoids adding two
                // independently encoded colors (which breaks premultiplication).
                radiance += inverse_tone_map(background.rgb) * transmittance * background.a / params.artistic.w;
                return PathRadiance(radiance, vec3<f32>(0.0), coverage, coverage);
            }
            var sky = environment(direction);
            if TRANSMISSIVE && specular_sun { sky += sun_radiance(direction); }
            radiance += (throughput * ambient_visibility) * sky;
            break;
        }
        let optical = optical_properties(hit.material);
        let dielectric = TRANSMISSIVE && optical.x > 0.0 && (hit.to_medium == 0u || is_transmissive(hit.to_medium));
        let encoded = hit_color(hit).rgb;
        let albedo = pow(max(encoded, vec3<f32>(0.0)), vec3<f32>(2.2));
        let emission = bitcast<f32>(materials[hit.material * MATERIAL_WORDS + 7u]);
        let indirect_visibility = select(ambient_visibility, 1.0, bounce == 0u);
        radiance += (throughput * albedo * indirect_visibility) * emission;
        if dielectric {
            if interfaces == MAX_INTERFACES { break; }
            let normal = liquid_normal(hit, direction);
            let eta = medium_ior(hit.from_medium) / medium_ior(hit.to_medium);
            // Evaluate the non-delta GGX reflection toward the finite sun. Delta
            // reflection receives the visible sun only on its specular escape.
            radiance += throughput * optical.x * rough_direct_light(hit, direction, normal, eta, optical.z, state);
            if random(state) < optical.x {
                let sampled = dielectric_sample(direction, normal, hit.normal, eta, optical.z, state);
                if sampled.weight == 0.0 { break; }
                throughput *= sampled.weight;
                if !sampled.transmitted { camera_transmission = false; }
                if sampled.transmitted {
                    medium = hit.to_medium;
                    eta_scale /= eta * eta;
                }
                direction = sampled.direction;
                origin = offset_surface(hit.point, select(hit.normal, -hit.normal, sampled.transmitted));
                specular_sun = optical.z == 0.0 && !sampled.transmitted;
                interfaces += 1u;
                continue;
            }
        }
        camera_transmission = false;
        radiance += (throughput * albedo * indirect_visibility) * direct_light(hit, state);
        if bounce + 1u == params.mode.y { break; }
        throughput *= albedo;
        if bounce >= 2u {
            let adjusted = throughput * eta_scale;
            let survival = clamp(max(adjusted.r, max(adjusted.g, adjusted.b)), 0.1, 0.95);
            if random(state) > survival { break; }
            throughput /= survival;
        }
        origin = offset_surface(hit.point, hit.normal);
        direction = cosine_direction(hit.normal, state);
        medium = hit.from_medium;
        specular_sun = false;
        bounce += 1u;
    }
    return PathRadiance(radiance, vec3<f32>(0.0), 1.0, 1.0);
}

struct PixelResult { encoded: vec4<f32>, lighting: vec3<f32>, albedo: vec3<f32>, guide: vec4<f32> };
fn sample_pixel(pixel: vec4<f32>, reconstruct: bool) -> PixelResult {
    let count = select(params.reconstruction.y, params.mode.x, PATH_TRACED);
    let sampling_seed = u32(pixel.x) + u32(pixel.y) * u32(params.viewport.x) + params.mode.z * 277803737u + params.reconstruction.z * 2891336453u;
    var state = sampling_seed + 1u;
    var diffuse = vec3<f32>(0.0);
    var diffuse_weights = vec3<f32>(0.0);
    var primary_albedo = vec3<f32>(0.0);
    var guide = vec4<f32>(0.0);
    var guide_normal = vec3<f32>(0.0);
    var filterable = reconstruct;
    var sum = vec3<f32>(0.0);
    var visible_background = vec3<f32>(0.0);
    var foreground_samples = 0.0;
    var alpha = 0.0;
    for (var sample = 0u; sample < count; sample += 1u) {
        var position = pixel.xy;
        if count > 1u || PATH_TRACED || params.reconstruction.x == 2u {
            position += sample_square(sample + params.reconstruction.z * count, count, sampling_seed) - 0.5;
        }
        let screen = vec2<f32>(position.x / params.viewport.x * 2.0 - 1.0, 1.0 - position.y / params.viewport.y * 2.0);
        let offset = params.right.xyz * screen.x * params.right.w + params.up.xyz * screen.y * params.up.w;
        var origin = params.position.xyz;
        var direction = params.forward.xyz;
        if params.forward.w > 0.5 { direction = normalize(direction + offset); } else { origin += offset; }
        var medium = 0u;
        if TRANSMISSIVE {
            let candidate = material_at(entered_cell(origin, direction));
            if is_transmissive(candidate) { medium = candidate; }
        }
        let hit = trace_medium(origin, direction, UNBOUNDED_RAY, medium);
        var background = mix(params.background, params.sky_top, clamp(1.0 - position.y / params.viewport.y, 0.0, 1.0));
        if params.clock.y > 0.5 { background = vec4<f32>(tone_map(environment(direction) * params.artistic.w), 1.0); }
        var albedo = vec3<f32>(0.0);
        if reconstruct {
            if hit.material == 0u || hit.from_medium != 0u || is_transmissive(hit.material) || bitcast<f32>(materials[hit.material * MATERIAL_WORDS + 7u]) != 0.0 {
                filterable = false;
            } else {
                let encoded_albedo = hit_color(hit).rgb;
                albedo = select(encoded_albedo, pow(encoded_albedo, vec3<f32>(2.2)), PATH_TRACED);
                primary_albedo += albedo;
                if sample == 0u {
                    guide_normal = hit.normal;
                    guide = vec4<f32>(encode_normal(hit.normal), dot(hit.point, hit.normal), f32(hit.material));
                } else if guide.w != f32(hit.material) || dot(guide_normal, hit.normal) < 0.99 || abs(dot(hit.point, guide_normal) - guide.z) >= MAX_PLANE_ERROR { filterable = false; }
            }
        }
        let coverage = select(background.a, 1.0, hit.material != 0u);
        alpha += coverage;
        if PATH_TRACED {
            if hit.material == 0u { visible_background += background.rgb * coverage; }
            else {
                var visibility = 1.0;
                if !TRANSMISSIVE || !is_transmissive(hit.material) { visibility = ao_visibility(hit, &state); }
                let path = path_radiance(origin, direction, hit, visibility, background, &state);
                sum += path.linear;
                if reconstruct {
                    let valid = albedo > vec3<f32>(0.0);
                    diffuse += path.linear / select(vec3<f32>(1.0), albedo, valid);
                    diffuse_weights += vec3<f32>(valid);
                }
                visible_background += path.background;
                foreground_samples += path.foreground;
                alpha += path.coverage - 1.0;
            }
        } else if hit.material != 0u {
            if TRANSMISSIVE && (hit.from_medium != 0u || is_transmissive(hit.material)) {
                let preview = flat_transmission(origin, direction, hit, background, &state);
                sum += preview.rgb;
                alpha += preview.a - 1.0;
            } else {
                if reconstruct {
                    let irradiance = flat_irradiance(hit, &state);
                    sum += hit_color(hit).rgb * irradiance;
                    diffuse += irradiance;
                    diffuse_weights += vec3<f32>(1.0);
                } else { sum += flat_surface(hit, &state); }
            }
        } else { sum += background.rgb * coverage; }
    }
    let average_alpha = alpha / f32(count);
    if PATH_TRACED {
        let color = tone_map(sum / max(foreground_samples, 1e-12) * params.artistic.w) * foreground_samples;
        let encoded = vec4<f32>((color + visible_background) / f32(count), average_alpha);
        return PixelResult(encoded, select(vec3<f32>(-1.0), diffuse / max(diffuse_weights, vec3<f32>(1.0)), diffuse_weights > vec3<f32>(0.0)), primary_albedo / f32(count), select(vec4<f32>(0.0), guide, filterable));
    }
    let encoded = vec4<f32>(min(sum / f32(count), vec3<f32>(average_alpha)), average_alpha);
    return PixelResult(encoded, select(vec3<f32>(-1.0), diffuse / max(diffuse_weights, vec3<f32>(1.0)), diffuse_weights > vec3<f32>(0.0)), primary_albedo / f32(count), select(vec4<f32>(0.0), guide, filterable));
}

@fragment fn fragment(@builtin(position) pixel: vec4<f32>) -> @location(0) vec4<f32> {
    return sample_pixel(pixel, false).encoded * params.composite.x;
}
struct ReconstructionOutput {
    @location(0) lighting: vec4<f32>,
    @location(1) guide: vec4<f32>,
    @location(2) albedo: vec4<f32>,
};
@fragment fn reconstruct_fragment(@builtin(position) pixel: vec4<f32>) -> ReconstructionOutput {
    let result = sample_pixel(pixel, true);
    if result.guide.w == 0.0 {
        return ReconstructionOutput(result.encoded, vec4<f32>(0.0), vec4<f32>(0.0, 0.0, 0.0, result.encoded.a));
    }
    return ReconstructionOutput(vec4<f32>(store_measured(result.lighting, result.lighting >= vec3<f32>(0.0)), 1.0), result.guide, vec4<f32>(store_albedo(result.albedo), result.encoded.a));
}
