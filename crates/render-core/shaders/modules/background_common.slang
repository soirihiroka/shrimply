static const float PI = 3.14159265358979323846;
static const float TAU = 6.28318530717958647692;
static const float TEST_PATTERN_HORIZONTAL_MARGIN = 1.0 / 8.0;
static const float TEST_PATTERN_TOP_END = 7.0 / 12.0;
static const float TEST_PATTERN_MIDDLE_SPLIT = 2.0 / 3.0;
static const float TEST_PATTERN_MIDDLE_END = 3.0 / 4.0;
static const float TEST_PATTERN_BAR_WIDTH = 3.0 / 28.0;
static const float TEST_PATTERN_WHITE_START = 2.0 / 7.0;
static const float TEST_PATTERN_WHITE_END = 1.0 / 2.0;
static const float TEST_PATTERN_PLUGE_START = 5.0 / 8.0;
static const float TEST_PATTERN_PLUGE_WIDTH = 1.0 / 28.0;
static const float TEST_PATTERN_DARK_START = 43.0 / 56.0;
static const uint MAX_CENTERED_LINE_COUNT = 4096;
static const uint CENTERED_ANGLE_RANDOM_AXIS = 0;
static const uint CENTERED_LENGTH_RANDOM_AXIS = 1;
static const uint CENTERED_OFFSET_RANDOM_AXIS = 2;
static const uint CENTERED_WIDTH_RANDOM_AXIS = 3;

enum class BackgroundKind : uint32_t {
    ColorGradient,
    Grid,
    WhiteNoise,
    PerlinNoise,
    CenteredLines,
    Rainbow,
    Checkerboard,
    Voronoi,
    TestPattern,
};

struct CommonSettings { uint width; uint height; BackgroundKind kind; float time; };
struct GradientSettings { uint mode; uint curve; float angle; float scale; float4 color_a; float4 color_b; float2 center; float2 position; float cycle_position; float2 padding; };
struct GridSettings { float4 background; float4 horizontal; float4 vertical; float2 spacing; float2 line_width; float2 position; float rotation; uint line_style; float dash_length; float dash_gap; float dash_position; float wobble_amount; float wobble_scale; float wobble_position; float2 middle_padding; float2 padding_randomness; uint seed; };
struct NoiseSettings { uint distribution; uint color_mode; uint pixel_size; uint epoch; float4 color_a; float4 color_b; float brightness; float contrast; uint seed; float padding; };
struct PerlinSettings { uint mode; uint octaves; uint seed; float scale; float4 color_a; float4 color_b; float lacunarity; float persistence; float contrast; float evolution; float2 position; float warp_amount; float warp_scale; };
struct CenteredLinesSettings { float4 background; float4 line; float2 center; float rotation_degrees; uint line_count; float line_width; float line_width_randomness; float line_length; float line_length_randomness; float line_offset; float line_offset_randomness; float angular_randomness; float fade_length; uint seed; };
struct RainbowSettings { uint fill; uint bands; uint band_count; float angle; float2 center; float scale; float saturation; float brightness; float alpha; float2 position; float hue_position; };
struct CheckerSettings { float4 color_a; float4 color_b; float2 cell_size; float edge_softness; float2 position; float rotation; };
struct VoronoiSettings { uint fill; uint metric; uint seed; float cell_size; float4 color_a; float4 color_b; float4 edge_color; float jitter; float edge_width; float2 position; float motion_amount; float motion_position; };
struct BackgroundUniforms { CommonSettings common; GradientSettings gradient; GridSettings grid; NoiseSettings noise; PerlinSettings perlin; CenteredLinesSettings centered_lines; RainbowSettings rainbow; CheckerSettings checker; VoronoiSettings voronoi; };

uint hashUint(uint value) {
    value ^= value >> 16;
    value *= 0x7feb352d;
    value ^= value >> 15;
    value *= 0x846ca68b;
    value ^= value >> 16;
    return value;
}

uint seededHash(uint2 p, uint seed) {
    uint state = hashUint(seed ^ 0x9e3779b9u);
    state = hashUint(state ^ hashUint(p.x ^ 0x85ebca6bu));
    return hashUint(state ^ hashUint(p.y ^ 0xc2b2ae35u));
}

float random01(uint2 p, uint seed) {
    return float(seededHash(p, seed)) / 4294967295.0;
}

float2 random2(int2 p, uint seed) {
    uint2 q = uint2(p);
    return float2(
        random01(q, seed),
        random01(q, hashUint(seed ^ 0x68bc21ebu))
    );
}

float4 mixColor(float4 a, float4 b, float t) { return lerp(a, b, saturate(t)); }
uint packRgba(float4 color) {
    uint4 bytes = uint4(round(saturate(color) * 255.0));
    return bytes.x | (bytes.y << 8) | (bytes.z << 16) | (bytes.w << 24);
}

float2 rotatePoint(float2 p, float degrees) {
    if (degrees == 0.0) return p;
    float angle = degrees * PI / 180.0;
    float c = cos(angle);
    float s = sin(angle);
    return float2(c * p.x - s * p.y, s * p.x + c * p.y);
}

float curveValue(float value, uint curve) {
    if (curve == 0) return 0.0;
    if (curve == 1) return value;
    if (curve == 2) return value * value * (3.0 - 2.0 * value);
    return value * value * value * (value * (value * 6.0 - 15.0) + 10.0);
}

float valueNoise(float2 p, uint seed) {
    int2 cell = int2(floor(p));
    float2 f = frac(p);
    f = f * f * (3.0 - 2.0 * f);
    float a = random01(uint2(cell), seed);
    float b = random01(uint2(cell + int2(1, 0)), seed);
    float c = random01(uint2(cell + int2(0, 1)), seed);
    float d = random01(uint2(cell + 1), seed);
    return lerp(lerp(a, b, f.x), lerp(c, d, f.x), f.y);
}

float4 gradientColor(BackgroundUniforms uniforms, float2 pixel) {
    GradientSettings g = uniforms.gradient;
    if (g.mode == 0) return g.color_a;
    float2 extent = float2(uniforms.common.width, uniforms.common.height);
    float2 p = pixel - g.center * extent - g.position;
    p = rotatePoint(p, g.angle);
    float value;
    if (g.mode == 1) value = p.x / (extent.x * max(g.scale, 0.001)) + 0.5;
    else if (g.mode == 2) value = length(p) / (min(extent.x, extent.y) * 0.5 * max(g.scale, 0.001));
    else value = atan2(p.y, p.x) / TAU + 0.5;
    value = frac(value + g.cycle_position);
    return mixColor(g.color_a, g.color_b, curveValue(value, g.curve));
}

float lineDash(float along, GridSettings g) {
    if (g.line_style == 0) return 1.0;
    float length = g.line_style == 2 ? max(g.line_width.x, g.line_width.y) : max(g.dash_length, 0.1);
    float period = length + max(g.dash_gap, 0.1);
    return frac((along + g.dash_position) / period) * period < length ? 1.0 : 0.0;
}

float4 gridColor(BackgroundUniforms uniforms, float2 pixel) {
    GridSettings g = uniforms.grid;
    float2 extent = float2(uniforms.common.width, uniforms.common.height);
    float2 p = rotatePoint(pixel - extent * 0.5, g.rotation);
    p -= g.position;
    if (g.wobble_amount != 0.0) {
        float wobble_scale = max(g.wobble_scale, 0.1);
        p += float2(
            sin(p.y / wobble_scale * TAU + g.wobble_position),
            sin(p.x / wobble_scale * TAU + g.wobble_position)
        ) * g.wobble_amount;
    }
    float2 paddingScale = float2(1.0);
    if ((g.middle_padding.x != 0.0 || g.middle_padding.y != 0.0)
        && (g.padding_randomness.x != 0.0 || g.padding_randomness.y != 0.0)) {
        float2 randomPadding = float2(random01(uint2(0, 1), g.seed), random01(uint2(0, 2), g.seed));
        paddingScale = max(float2(1.0) + (randomPadding * 2.0 - 1.0) * g.padding_randomness, float2(0.0));
    }
    float2 padding = g.middle_padding * paddingScale * extent;
    float2 outsidePadding = float2(1.0);
    if (padding.x != 0.0 || padding.y != 0.0) {
        float2 halfPadding = padding * 0.5;
        outsidePadding = step(halfPadding, abs(p));
        p -= sign(p) * halfPadding;
    }
    float2 spacing = max(g.spacing, float2(0.1));
    float dx = min(frac(p.x / spacing.x), 1.0 - frac(p.x / spacing.x)) * spacing.x;
    float dy = min(frac(p.y / spacing.y), 1.0 - frac(p.y / spacing.y)) * spacing.y;
    float vertical = dx <= g.line_width.x * 0.5 ? lineDash(p.y, g) * outsidePadding.x : 0.0;
    float horizontal = dy <= g.line_width.y * 0.5 ? lineDash(p.x, g) * outsidePadding.y : 0.0;
    float4 color = mixColor(g.background, g.vertical, vertical * g.vertical.a);
    return mixColor(color, g.horizontal, horizontal * g.horizontal.a);
}

float centeredLineRandom(uint line, uint axis, uint seed) {
    return random01(uint2(line, axis), seed);
}

float4 centeredLinesColor(BackgroundUniforms uniforms, float2 pixel) {
    CenteredLinesSettings c = uniforms.centered_lines;
    float2 extent = float2(uniforms.common.width, uniforms.common.height);
    float2 origin = c.center * extent;
    float2 p = pixel - origin;
    float radius = length(p);
    float radial_scale = min(extent.x, extent.y) * 0.5;
    uint line_count = clamp(c.line_count, 1u, MAX_CENTERED_LINE_COUNT);
    float sector_angle = TAU / float(line_count);
    float pixel_angle = atan2(p.y, p.x);
    float rotation = radians(c.rotation_degrees);
    float angle = pixel_angle - rotation + PI;
    angle -= floor(angle / TAU) * TAU;
    int sector = int(floor(angle / sector_angle));
    float coverage = 0.0;

    for (int neighbor = -1; neighbor <= 1; ++neighbor) {
        int candidate = (sector + neighbor) % int(line_count);
        if (candidate < 0) candidate += int(line_count);
        uint line = uint(candidate);
        float angle_jitter = (centeredLineRandom(line, CENTERED_ANGLE_RANDOM_AXIS, c.seed) - 0.5)
            * saturate(c.angular_randomness);
        float line_angle = (float(line) + 0.5 + angle_jitter) * sector_angle - PI + rotation;
        float angle_delta = atan2(sin(pixel_angle - line_angle), cos(pixel_angle - line_angle));
        float distance_from_line = abs(sin(angle_delta)) * radius;
        float line_width = max(c.line_width, 0.0) * max(
            1.0 + (centeredLineRandom(line, CENTERED_WIDTH_RANDOM_AXIS, c.seed) * 2.0 - 1.0)
                * saturate(c.line_width_randomness),
            0.0
        );
        float line_length = max(c.line_length, 0.0) * max(
            1.0 + (centeredLineRandom(line, CENTERED_LENGTH_RANDOM_AXIS, c.seed) * 2.0 - 1.0)
                * saturate(c.line_length_randomness),
            0.0
        );
        float line_offset = max(
            c.line_offset
                + (centeredLineRandom(line, CENTERED_OFFSET_RANDOM_AXIS, c.seed) * 2.0 - 1.0)
                    * max(c.line_offset_randomness, 0.0),
            0.0
        );
        if (line_width <= 0.0 || line_length <= 0.0
            || radius < line_offset || radius > line_offset + line_length) continue;

        float tapered_width = line_width * radius / radial_scale;
        float edge_coverage = saturate(tapered_width * 0.5 + 0.5 - distance_from_line);
        float fade_distance = max(c.fade_length, 0.0);
        float fade = fade_distance > 0.0 ? saturate((radius - line_offset) / fade_distance) : 1.0;
        coverage = max(coverage, edge_coverage * fade);
    }
    return mixColor(c.background, c.line, coverage * c.line.a);
}

float4 noiseColor(BackgroundUniforms uniforms, uint2 pixel) {
    NoiseSettings n = uniforms.noise;
    uint2 cell = pixel / max(n.pixel_size, 1);
    uint seed = n.seed ^ hashUint(n.epoch);
    float value = random01(cell, seed);
    if (n.distribution == 1)
        value = (value + random01(cell.yx, seed + 1) + random01(cell + 17, seed + 2)) / 3.0;
    if (n.distribution == 2) value = value >= 0.5 ? 1.0 : 0.0;
    value = saturate((value - 0.5) * n.contrast + 0.5 + n.brightness);
    if (n.color_mode == 1)
        return float4(value, random01(cell, seed + 3), random01(cell, seed + 7), 1.0);
    if (n.color_mode == 2) return mixColor(n.color_a, n.color_b, value);
    return float4(value, value, value, lerp(n.color_a.a, n.color_b.a, value));
}

float4 perlinColor(BackgroundUniforms uniforms, float2 pixel) {
    PerlinSettings n = uniforms.perlin;
    float2 p = (pixel - n.position) / max(n.scale, 0.1);
    if (n.warp_amount != 0.0) {
        float warp_scale = n.scale / max(n.warp_scale, 0.1);
        p += (float2(valueNoise(p * warp_scale, n.seed + 31), valueNoise(p * warp_scale, n.seed + 67)) - 0.5) * n.warp_amount;
    }
    float value = 0.0;
    float amplitude = 1.0;
    float total = 0.0;
    float frequency = 1.0;
    for (uint octave = 0; octave < min(n.octaves, 8); ++octave) {
        float sample = valueNoise(p * frequency + n.evolution, n.seed + octave * 101);
        if (n.mode == 1) sample = abs(sample * 2.0 - 1.0);
        else if (n.mode == 2) sample = 1.0 - abs(sample * 2.0 - 1.0);
        value += sample * amplitude;
        total += amplitude;
        frequency *= max(n.lacunarity, 1.0);
        amplitude *= saturate(n.persistence);
    }
    value = saturate(((value / max(total, 0.001)) - 0.5) * n.contrast + 0.5);
    return mixColor(n.color_a, n.color_b, value);
}

float3 hsv(float h, float s, float v) {
    float3 p = abs(frac(h + float3(0.0, 2.0 / 3.0, 1.0 / 3.0)) * 6.0 - 3.0);
    return v * lerp(float3(1.0), saturate(p - 1.0), s);
}

float4 rainbowColor(BackgroundUniforms uniforms, float2 pixel) {
    RainbowSettings r = uniforms.rainbow;
    float2 extent = float2(uniforms.common.width, uniforms.common.height);
    float2 p = rotatePoint(pixel - r.center * extent - r.position, r.angle);
    float value;
    if (r.fill == 0) value = p.x / (extent.x * max(r.scale, 0.001));
    else if (r.fill == 1) value = length(p) / (min(extent.x, extent.y) * max(r.scale, 0.001));
    else value = atan2(p.y, p.x) / TAU;
    value = frac(value + r.hue_position);
    if (r.bands != 0) value = floor(value * max(r.band_count, 2)) / max(r.band_count, 2);
    return float4(hsv(value, saturate(r.saturation), max(r.brightness, 0.0)), saturate(r.alpha));
}

float4 checkerColor(BackgroundUniforms uniforms, float2 pixel) {
    CheckerSettings c = uniforms.checker;
    float2 extent = float2(uniforms.common.width, uniforms.common.height);
    float2 p = rotatePoint(pixel - extent * 0.5 - c.position, c.rotation);
    float2 cell = max(c.cell_size, float2(0.1));
    float2 position = p / cell;
    int parityIndex = int(floor(position.x)) + int(floor(position.y));
    float parity = float(parityIndex & 1);
    float2 edge = min(frac(position), 1.0 - frac(position)) * cell;
    float blend = c.edge_softness <= 0.0 ? parity : lerp(0.5, parity, smoothstep(0.0, c.edge_softness, min(edge.x, edge.y)));
    return mixColor(c.color_a, c.color_b, blend);
}

float metricDistance(float2 value, uint metric) {
    value = abs(value);
    if (metric == 1) return value.x + value.y;
    if (metric == 2) return max(value.x, value.y);
    return dot(value, value);
}

float4 voronoiColor(BackgroundUniforms uniforms, float2 pixel) {
    VoronoiSettings v = uniforms.voronoi;
    float2 p = (pixel - v.position) / max(v.cell_size, 0.1);
    int2 base = int2(floor(p));
    float nearest = 1000.0;
    float second = 1000.0;
    float identity = 0.0;
    for (int y = -1; y <= 1; ++y) {
        for (int x = -1; x <= 1; ++x) {
            int2 cell = base + int2(x, y);
            float2 random = random2(cell, v.seed);
            float2 site = float2(cell) + 0.5 + (random - 0.5) * v.jitter;
            if (v.motion_amount != 0.0)
                site += sin(random.yx * TAU + v.motion_position) * v.motion_amount;
            float distance = metricDistance(site - p, v.metric);
            if (distance < nearest) {
                second = nearest;
                nearest = distance;
                identity = random.x;
            } else if (distance < second) {
                second = distance;
            }
        }
    }
    if (v.metric == 0) {
        nearest = sqrt(nearest);
        second = sqrt(second);
    }
    float edge = second - nearest;
    if (v.fill == 0) return mixColor(v.color_a, v.color_b, nearest);
    if (v.fill == 2)
        return mixColor(v.edge_color, v.color_b, smoothstep(v.edge_width / max(v.cell_size, 0.1), v.edge_width * 2.0 / max(v.cell_size, 0.1), edge));
    float4 fill = mixColor(v.color_a, v.color_b, identity);
    return mixColor(v.edge_color, fill, smoothstep(0.0, v.edge_width / max(v.cell_size, 0.1), edge));
}

float4 testPatternColor(BackgroundUniforms uniforms, float2 pixel) {
    float2 uv = (pixel + 0.5) / float2(uniforms.common.width, uniforms.common.height);
    const float signal = 0.83;
    const float gray = 0.48;
    const float dark_gray = 0.18;
    const float4 black = float4(0.0, 0.0, 0.0, 1.0);
    const float4 white = float4(1.0, 1.0, 1.0, 1.0);

    if (uv.y < TEST_PATTERN_TOP_END) {
        if (uv.x < TEST_PATTERN_HORIZONTAL_MARGIN || uv.x >= 1.0 - TEST_PATTERN_HORIZONTAL_MARGIN)
            return float4(gray, gray, gray, 1.0);
        uint bar = uint((uv.x - TEST_PATTERN_HORIZONTAL_MARGIN) / TEST_PATTERN_BAR_WIDTH);
        if (bar == 0) return float4(signal, signal, signal, 1.0);
        if (bar == 1) return float4(signal, signal, 0.0, 1.0);
        if (bar == 2) return float4(0.0, signal, signal, 1.0);
        if (bar == 3) return float4(0.0, signal, 0.0, 1.0);
        if (bar == 4) return float4(signal, 0.0, signal, 1.0);
        if (bar == 5) return float4(signal, 0.0, 0.0, 1.0);
        return float4(0.0, 0.0, signal, 1.0);
    }

    if (uv.y < TEST_PATTERN_MIDDLE_END) {
        bool upper = uv.y < TEST_PATTERN_MIDDLE_SPLIT;
        if (uv.x < TEST_PATTERN_HORIZONTAL_MARGIN)
            return upper ? float4(0.0, signal, signal, 1.0) : float4(signal, signal, 0.0, 1.0);
        if (uv.x >= 1.0 - TEST_PATTERN_HORIZONTAL_MARGIN)
            return upper ? float4(0.0, 0.0, signal, 1.0) : float4(signal, 0.0, 0.0, 1.0);
        if (uv.x < TEST_PATTERN_HORIZONTAL_MARGIN + TEST_PATTERN_BAR_WIDTH)
            return upper ? white : black;
        if (upper) return float4(signal, signal, signal, 1.0);
        float ramp = saturate(
            (uv.x - TEST_PATTERN_HORIZONTAL_MARGIN - TEST_PATTERN_BAR_WIDTH)
                / (TEST_PATTERN_PLUGE_START - TEST_PATTERN_HORIZONTAL_MARGIN - TEST_PATTERN_BAR_WIDTH)
        );
        return float4(ramp, ramp, ramp, 1.0);
    }

    if (uv.x < TEST_PATTERN_HORIZONTAL_MARGIN || uv.x >= 1.0 - TEST_PATTERN_HORIZONTAL_MARGIN)
        return float4(dark_gray, dark_gray, dark_gray, 1.0);
    if (uv.x < TEST_PATTERN_WHITE_START)
        return black;
    if (uv.x < TEST_PATTERN_WHITE_END)
        return white;
    if (uv.x < TEST_PATTERN_PLUGE_START)
        return black;
    if (uv.x < TEST_PATTERN_PLUGE_START + TEST_PATTERN_PLUGE_WIDTH)
        return float4(0.015, 0.015, 0.015, 1.0);
    if (uv.x < TEST_PATTERN_PLUGE_START + TEST_PATTERN_PLUGE_WIDTH * 2.0)
        return black;
    if (uv.x < TEST_PATTERN_PLUGE_START + TEST_PATTERN_PLUGE_WIDTH * 3.0)
        return float4(0.04, 0.04, 0.04, 1.0);
    if (uv.x < TEST_PATTERN_DARK_START)
        return float4(0.01, 0.01, 0.01, 1.0);
    return float4(0.1, 0.1, 0.1, 1.0);
}

float4 backgroundColor(BackgroundUniforms uniforms, uint2 pixel) {
    float4 color;
    switch (uniforms.common.kind) {
        case BackgroundKind::ColorGradient: color = gradientColor(uniforms, float2(pixel)); break;
        case BackgroundKind::Grid: color = gridColor(uniforms, float2(pixel)); break;
        case BackgroundKind::WhiteNoise: color = noiseColor(uniforms, pixel); break;
        case BackgroundKind::PerlinNoise: color = perlinColor(uniforms, float2(pixel)); break;
        case BackgroundKind::CenteredLines: color = centeredLinesColor(uniforms, float2(pixel) + 0.5); break;
        case BackgroundKind::Rainbow: color = rainbowColor(uniforms, float2(pixel)); break;
        case BackgroundKind::Checkerboard: color = checkerColor(uniforms, float2(pixel)); break;
        case BackgroundKind::Voronoi: color = voronoiColor(uniforms, float2(pixel)); break;
        case BackgroundKind::TestPattern: color = testPatternColor(uniforms, float2(pixel)); break;
        default: color = float4(1.0, 0.0, 1.0, 1.0); break;
    }
    return color;
}
