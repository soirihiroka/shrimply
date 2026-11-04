#include "scene-parameter.slang"

module render_3d_toon;

import render_3d_abi;
import render_3d_ray;

static const uint TOON_SHADOW_HASH_MULTIPLIER = 0x9e3779b9u;
static const uint TOON_SHADOW_HASH_CHANNEL = 0x85ebca6bu;
static const uint TOON_SHADOW_HASH_MIX = 0x7feb352du;
static const float TOON_SHADOW_HASH_SCALE = 1.0 / 16777216.0;
static const float TOON_GOLDEN_ANGLE = 2.39996322972865332;
static const float3 TOON_LUMINANCE = float3(0.2126, 0.7152, 0.0722);
static const float TOON_EPSILON = 0.00001;
static const int TOON_MAX_HATCH_DIRECTIONS = 6;
static const float KUWAHARA_DEPTH_REJECTION = 0.1;
static const float KUWAHARA_NORMAL_REJECTION_COSINE = 0.5;

public struct ToonTextureSample
{
    public bool hit;
    public float3 color;
    public float3 normal;
    public float depth;
};

public interface IToonTextureTracer
{
    ToonTextureSample sample(float2 position, uint2 dimensions);
}

public float3 toon_screen_space_kuwahara<T : IToonTextureTracer>(
    float3 center_color,
    float3 center_normal,
    float center_depth,
    float2 center_position,
    uint2 dimensions,
    ToonSettings settings,
    T tracer)
{
    int radius = int(settings.kuwahara_radius);
    if (radius == 0 || settings.kuwahara_strength == 0.0)
        return center_color;

    float3 best_mean = center_color;
    float best_variance = RAY_MAXIMUM;
    for (int quadrant = 0; quadrant < 4; ++quadrant)
    {
        int2 direction = int2(
            (quadrant & 1) == 0 ? -1 : 1,
            (quadrant & 2) == 0 ? -1 : 1);
        float3 sum = center_color;
        float3 squared_sum = center_color * center_color;
        float sample_count = 1.0;
        for (int y = 0; y <= radius; ++y)
        {
            for (int x = 0; x <= radius; ++x)
            {
                if (x == 0 && y == 0)
                    continue;
                float2 sample_position = center_position
                    + float2(direction * int2(x, y));
                if (any(sample_position < float2(0.0))
                    || any(sample_position >= float2(dimensions)))
                    continue;
                ToonTextureSample sample = tracer.sample(sample_position, dimensions);
                if (!sample.hit)
                    continue;
                float relative_depth = abs(center_depth - sample.depth)
                    / max(min(center_depth, sample.depth), RAY_MINIMUM);
                if (relative_depth > KUWAHARA_DEPTH_REJECTION
                    || dot(center_normal, sample.normal)
                        < KUWAHARA_NORMAL_REJECTION_COSINE)
                    continue;
                sum += sample.color;
                squared_sum += sample.color * sample.color;
                sample_count += 1.0;
            }
        }
        float3 mean = sum / sample_count;
        float3 variance = max(squared_sum / sample_count - mean * mean, 0.0);
        float variance_score = dot(variance, TOON_LUMINANCE);
        if (variance_score < best_variance)
        {
            best_variance = variance_score;
            best_mean = mean;
        }
    }
    return lerp(center_color, best_mean, settings.kuwahara_strength);
}

float toon_shadow_hash(int index, uint channel)
{
    uint state = asuint(index) * TOON_SHADOW_HASH_MULTIPLIER ^ channel;
    state ^= state >> 16;
    state *= TOON_SHADOW_HASH_MIX;
    state ^= state >> 15;
    return float(state & 0x00ffffffu) * TOON_SHADOW_HASH_SCALE;
}

float toon_shadow_hash(int2 index, uint channel)
{
    uint state = asuint(index.x) * TOON_SHADOW_HASH_MULTIPLIER
        ^ asuint(index.y) * TOON_SHADOW_HASH_CHANNEL
        ^ channel;
    state ^= state >> 16;
    state *= TOON_SHADOW_HASH_MIX;
    state ^= state >> 15;
    return float(state & 0x00ffffffu) * TOON_SHADOW_HASH_SCALE;
}

float toon_shadow_dots(float2 position, ToonSettings settings)
{
    float base_diameter = settings.shadow_dot_size;
    float period = max(1.0 / settings.shadow_dot_density, base_diameter + 1.0);
    float softness = max(settings.shadow_pattern_softness, 0.001);
    int2 base_cell = int2(floor(position / period));
    float coverage = 0.0;
    for (int y = -1; y <= 1; ++y)
    {
        for (int x = -1; x <= 1; ++x)
        {
            int2 cell = base_cell + int2(x, y);
            float size_scale = lerp(
                1.0 - settings.shadow_dot_size_randomness,
                1.0 + settings.shadow_dot_size_randomness,
                toon_shadow_hash(cell, TOON_SHADOW_HASH_CHANNEL));
            float diameter = max(base_diameter * size_scale, 0.25);
            float available_spacing = max(period - diameter, 0.0);
            float2 jitter = float2(
                toon_shadow_hash(cell, 0u),
                toon_shadow_hash(cell, TOON_SHADOW_HASH_MULTIPLIER)) - 0.5;
            float2 center = (float2(cell) + 0.5) * period
                + jitter * available_spacing
                    * settings.shadow_dot_distribution_randomness;
            float radius = diameter * 0.5;
            coverage = max(
                coverage,
                1.0 - smoothstep(
                    max(radius - softness, 0.0),
                    radius + softness,
                    length(position - center)));
        }
    }
    return coverage;
}

float toon_shadow_lines(
    float2 position,
    ToonSettings settings,
    float direction,
    uint family)
{
    float sine;
    float cosine;
    sincos(direction, sine, cosine);
    float coordinate = dot(position, float2(-sine, cosine));
    float base_width = settings.shadow_line_width;
    float period = max(1.0 / settings.shadow_line_density, base_width + 1.0);
    float softness = max(settings.shadow_pattern_softness, 0.001);
    int base_line = int(floor(coordinate / period));
    float coverage = 0.0;
    for (int offset = -1; offset <= 1; ++offset)
    {
        int line = base_line + offset;
        float width_scale = lerp(
            1.0 - settings.shadow_line_width_randomness,
            1.0 + settings.shadow_line_width_randomness,
            toon_shadow_hash(
                line,
                TOON_SHADOW_HASH_CHANNEL ^ family * TOON_SHADOW_HASH_MULTIPLIER));
        float line_width = max(base_width * width_scale, 0.25);
        float available_spacing = max(period - line_width, 0.0);
        float jitter = (toon_shadow_hash(
            line,
            family * TOON_SHADOW_HASH_CHANNEL) - 0.5)
            * available_spacing
            * settings.shadow_line_distribution_randomness;
        float line_center = (float(line) + 0.5) * period + jitter;
        float half_width = line_width * 0.5;
        coverage = max(
            coverage,
            1.0 - smoothstep(
                max(half_width - softness, 0.0),
                half_width + softness,
                abs(coordinate - line_center)));
    }
    return coverage;
}

public float toon_tone_index(float illumination, ToonSettings settings)
{
    return round(saturate(illumination) * (settings.bands - 1.0));
}

public float toon_tone_value(float tone_index, ToonSettings settings)
{
    return tone_index / (settings.bands - 1.0);
}

public float toon_quantized_intensity(float intensity, ToonSettings settings)
{
    return toon_tone_value(toon_tone_index(intensity, settings), settings);
}

public float toon_shadow_pattern(
    float2 position,
    float tone_index,
    ToonSettings settings)
{
    float darkness = 1.0 - toon_tone_value(tone_index, settings);
    if (darkness <= 0.0)
        return 0.0;

    switch (settings.shadow_kind)
    {
    case ToonShadowKind::Dots:
        return toon_shadow_dots(position, settings);
    case ToonShadowKind::Lines:
        return toon_shadow_lines(
            position,
            settings,
            settings.shadow_line_direction,
            0u);
    case ToonShadowKind::Crosshatch:
        int direction_count = int(ceil(
            darkness * settings.shadow_crosshatch_max_directions));
        float coverage = 0.0;
        for (int family = 0; family < TOON_MAX_HATCH_DIRECTIONS; ++family)
        {
            if (family >= direction_count)
                break;
            float direction = settings.shadow_line_direction
                + float(family) * TOON_GOLDEN_ANGLE;
            coverage = max(
                coverage,
                toon_shadow_lines(
                    position,
                    settings,
                    direction,
                    uint(family)));
        }
        return coverage;
    default:
        return darkness;
    }
}

public float toon_light_response(
    float3 normal,
    float3 light,
    float3 radiance,
    float visibility)
{
    return dot(radiance, TOON_LUMINANCE)
        * saturate(dot(normal, light))
        * visibility;
}

public float3 toon_specular_light(
    float3 normal,
    float3 view,
    float3 light,
    float3 radiance,
    float visibility,
    ToonSettings settings)
{
    float3 half_direction = normalize(view + light);
    float highlight = saturate(dot(normal, half_direction));
    float specular = step(1.0 - settings.specular_size, highlight)
        * settings.specular_strength
        * visibility;
    return radiance * specular;
}

public float3 toon_tone_fill(
    float3 base_color,
    float3 light_color,
    float tone,
    ToonSettings settings)
{
    float base_luminance = dot(base_color, TOON_LUMINANCE);
    float light_luminance = dot(light_color, TOON_LUMINANCE);
    float3 tint = light_luminance > TOON_EPSILON
        ? light_color / light_luminance
        : float3(1.0);
    float3 tinted = base_color * tint;
    float tinted_luminance = dot(tinted, TOON_LUMINANCE);
    float target_luminance = base_luminance
        * lerp(settings.shadow_darkest_tone, 1.0, tone);
    return tinted_luminance > TOON_EPSILON
        ? saturate(tinted * target_luminance / tinted_luminance)
        : base_color * lerp(settings.shadow_darkest_tone, 1.0, tone);
}

public float3 toon_palette_quantize(float3 color, float levels)
{
    return round(saturate(color) * (levels - 1.0)) / (levels - 1.0);
}

public float toon_outline_fresnel(float facing, float width)
{
    return smoothstep(saturate(1.0 - width * 0.08), 1.0, facing);
}

public interface IToonShadowTracer
{
    float visibility(
        float3 world_position,
        float3 normal,
        float3 direction,
        float maximum);
}

public struct ToonDirectLighting
{
    public float illumination;
    public float shadow_illuminance;
    public float blocked_illuminance;
    public float3 light_color;
    public float3 specular_color;
};

public ToonDirectLighting integrate_toon_direct_lights<T : IToonShadowTracer>(
    SCENE_UNIFORMS scene,
    float3 world_position,
    float3 normal,
    float3 view,
    ToonSettings settings,
    T tracer)
{
    ToonDirectLighting result = {
        0.0,
        0.0,
        0.0,
        float3(0.0),
        float3(0.0)
    };
    for (uint light_index = 0; light_index < scene.sun_light_count; ++light_index)
    {
        SunLight source = scene.sun_lights[light_index];
        float3 light = normalize(-source.direction_angular_radius.xyz);
        float visibility = tracer.visibility(
            world_position,
            normal,
            light,
            RAY_MAXIMUM);
        float3 radiance = source.color_intensity.rgb * source.color_intensity.w;
        float light_amount = saturate(dot(normal, light));
        float light_illuminance = toon_light_response(
            normal,
            light,
            radiance,
            1.0);
        result.illumination += light_illuminance;
        float incident_illuminance = dot(radiance, TOON_LUMINANCE);
        result.shadow_illuminance += incident_illuminance;
        result.blocked_illuminance += max(
            incident_illuminance - light_illuminance * visibility,
            0.0);
        result.light_color += radiance * light_amount;
        result.specular_color += toon_specular_light(
            normal,
            view,
            light,
            radiance,
            1.0,
            settings);
    }
    for (uint light_index = 0; light_index < scene.point_light_count; ++light_index)
    {
        PointLight source = scene.point_lights[light_index];
        float3 delta = source.position_radius.xyz - world_position;
        float distance_squared = dot(delta, delta);
        float distance = sqrt(distance_squared);
        if (distance >= source.range_padding.x)
            continue;
        float3 light = delta / max(distance, RAY_MINIMUM);
        float visibility = tracer.visibility(
            world_position,
            normal,
            light,
            distance);
        float falloff = pow(1.0 - distance / source.range_padding.x, 2.0)
            / max(distance_squared, 0.01);
        float3 radiance = source.color_intensity.rgb
            * source.color_intensity.w * falloff;
        float light_amount = saturate(dot(normal, light));
        float light_illuminance = toon_light_response(
            normal,
            light,
            radiance,
            1.0);
        result.illumination += light_illuminance;
        float incident_illuminance = dot(radiance, TOON_LUMINANCE);
        result.shadow_illuminance += incident_illuminance;
        result.blocked_illuminance += max(
            incident_illuminance - light_illuminance * visibility,
            0.0);
        result.light_color += radiance * light_amount;
        result.specular_color += toon_specular_light(
            normal,
            view,
            light,
            radiance,
            1.0,
            settings);
    }
    return result;
}

public float3 toon_surface_color(
    float3 base_color,
    float3 normal,
    float3 view,
    float2 pixel_position,
    float3 environment_light,
    float3 environment_reflection,
    ToonSettings settings,
    ToonDirectLighting lighting)
{
    lighting.illumination += dot(environment_light, TOON_LUMINANCE);
    lighting.light_color += environment_light;
    float tone_index = toon_tone_index(lighting.illumination, settings);
    float3 color = toon_tone_fill(
        base_color,
        lighting.light_color,
        toon_tone_value(tone_index, settings),
        settings);
    color += environment_reflection + lighting.specular_color;
    float rim = pow(1.0 - saturate(dot(normal, view)), settings.rim_power)
        * settings.rim.w;
    color += settings.rim.rgb * toon_quantized_intensity(rim, settings);
    float shadow_occlusion = lighting.shadow_illuminance > RAY_MINIMUM
        ? saturate(lighting.blocked_illuminance / lighting.shadow_illuminance)
        : 0.0;
    float shadow_pattern = toon_shadow_pattern(
        pixel_position,
        toon_tone_index(1.0 - shadow_occlusion, settings),
        settings);
    return lerp(
        color,
        settings.shadow.rgb,
        settings.shadow.w * shadow_pattern);
}
