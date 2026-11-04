#include "scene-parameter.slang"

module render_3d_sampling;

import render_3d_abi;
import render_3d_ray;

static const float PI = 3.14159265358979323846;
static const float GOLDEN_ANGLE = 2.39996322972865332;
static const uint PIXEL_HASH_X = 0x9e3779b9u;
static const uint PIXEL_HASH_Y = 0x85ebca6bu;
static const uint SAMPLE_HASH = 0xc2b2ae35u;
static const uint HASH_MULTIPLIER_A = 0x7feb352du;
static const uint HASH_MULTIPLIER_B = 0x846ca68bu;
static const float UINT_TO_UNIT_FLOAT = 1.0 / 4294967296.0;
static const float3 LINEAR_LUMINANCE_WEIGHTS = float3(0.2126, 0.7152, 0.0722);
public static const uint MINIMUM_VARIANCE_SAMPLES = 2u;
static const uint INTERACTIVE_MINIMUM_SAMPLES = 2u;
static const uint INTERACTIVE_SAMPLE_DIVISOR = 8u;
static const float MINIMUM_ADAPTIVE_MEAN = 0.02;
static const float INTERACTIVE_RELATIVE_ERROR = 0.18;
static const float FINAL_RELATIVE_ERROR = 0.02;

public struct CameraRay
{
    public RayDesc object_ray;
    public float3 world_origin;
    public float3 world_direction;
    public float3 ground_origin;
    public bool valid;
};

public float luminance(float3 color)
{
    return dot(color, LINEAR_LUMINANCE_WEIGHTS);
}

public uint random_seed(uint2 pixel, uint sample_index)
{
    uint state = pixel.x * PIXEL_HASH_X
        ^ pixel.y * PIXEL_HASH_Y
        ^ sample_index * SAMPLE_HASH;
    state ^= state >> 16;
    state *= HASH_MULTIPLIER_A;
    state ^= state >> 15;
    state *= HASH_MULTIPLIER_B;
    return state ^ (state >> 16);
}

public float random_float(inout uint state)
{
    state ^= state << 13;
    state ^= state >> 17;
    state ^= state << 5;
    return float(state) * UINT_TO_UNIT_FLOAT;
}

public float2 random_disk(inout uint state)
{
    float radius = sqrt(random_float(state));
    float angle = 2.0 * PI * random_float(state);
    return radius * float2(cos(angle), sin(angle));
}

public void tangent_frame(float3 normal, out float3 tangent, out float3 bitangent)
{
    tangent = normalize(abs(normal.y) < 0.999
        ? cross(normal, float3(0.0, 1.0, 0.0))
        : cross(normal, float3(1.0, 0.0, 0.0)));
    bitangent = cross(normal, tangent);
}

public float3 local_to_world(float3 local, float3 normal)
{
    float3 tangent;
    float3 bitangent;
    tangent_frame(normal, tangent, bitangent);
    return normalize(local.x * tangent + local.y * bitangent + local.z * normal);
}

public float3 sample_cosine_hemisphere(float3 normal, inout uint state)
{
    float2 disk = random_disk(state);
    return local_to_world(
        float3(disk, sqrt(max(0.0, 1.0 - dot(disk, disk)))),
        normal);
}

public float3 sample_unit_sphere(inout uint state)
{
    float z = 1.0 - 2.0 * random_float(state);
    float angle = 2.0 * PI * random_float(state);
    float radius = sqrt(max(0.0, 1.0 - z * z));
    return float3(radius * cos(angle), radius * sin(angle), z);
}

public float3 sample_cone(float3 axis, float angular_radius, inout uint state)
{
    if (angular_radius <= 0.0)
        return axis;
    float cosine = lerp(1.0, cos(angular_radius), random_float(state));
    float sine = sqrt(max(0.0, 1.0 - cosine * cosine));
    float angle = 2.0 * PI * random_float(state);
    return local_to_world(float3(sine * cos(angle), sine * sin(angle), cosine), axis);
}

public float2 stochastic_pixel_offset(uint index, uint count, uint2 pixel)
{
    float radius = sqrt((float(index) + 0.5) / float(max(count, 1u))) * 0.48;
    uint seed = random_seed(pixel, 0);
    float angle = float(index) * GOLDEN_ANGLE + 2.0 * PI * random_float(seed);
    return float2(cos(angle), sin(angle)) * radius;
}

public uint anti_aliasing_sample_count(AntiAliasing strategy)
{
    return strategy == AntiAliasing::None
        ? 1u
        : (strategy == AntiAliasing::RotatedGrid2x
            ? 2u
            : (strategy == AntiAliasing::Grid4x ? 4u : 8u));
}

public float2 anti_aliasing_offset(uint index, AntiAliasing strategy, uint2 pixel)
{
    if (strategy == AntiAliasing::RotatedGrid2x)
        return index == 0 ? float2(-0.25, 0.25) : float2(0.25, -0.25);
    if (strategy == AntiAliasing::Grid4x)
    {
        return float2(
            (index & 1) == 0 ? -0.25 : 0.25,
            (index & 2) == 0 ? -0.25 : 0.25);
    }
    if (strategy == AntiAliasing::Stochastic8x)
        return stochastic_pixel_offset(index, 8, pixel);
    return float2(0.0);
}

public float power_heuristic(float first_pdf, float second_pdf)
{
    float first_squared = first_pdf * first_pdf;
    float second_squared = second_pdf * second_pdf;
    return first_squared / max(first_squared + second_squared, 0.0000001);
}

public void adaptive_path_budget(
    PbrSettings settings,
    out uint minimum_samples,
    out uint maximum_samples,
    out float relative_error)
{
    uint requested = max(uint(settings.path_tracing), 1u);
    if (settings.render_quality == RenderQuality::Interactive)
    {
        minimum_samples = INTERACTIVE_MINIMUM_SAMPLES;
        maximum_samples = max(requested / INTERACTIVE_SAMPLE_DIVISOR, minimum_samples);
        relative_error = INTERACTIVE_RELATIVE_ERROR;
        return;
    }
    minimum_samples = requested;
    maximum_samples = requested * 2u;
    relative_error = FINAL_RELATIVE_ERROR;
}

public bool adaptive_converged(
    float4 mean,
    float4 squared_error,
    uint sample_count,
    float relative_error)
{
    if (sample_count < MINIMUM_VARIANCE_SAMPLES)
        return false;
    float4 variance = squared_error / float(sample_count - 1u);
    float color_standard_error = sqrt(
        max(max(variance.r, max(variance.g, variance.b)), 0.0) / float(sample_count));
    float alpha_standard_error = sqrt(max(variance.a, 0.0) / float(sample_count));
    float color_scale = max(
        max(abs(mean.r), max(abs(mean.g), abs(mean.b))),
        MINIMUM_ADAPTIVE_MEAN);
    return color_standard_error <= relative_error * color_scale
        && alpha_standard_error <= relative_error;
}

public CameraRay camera_ray(
    SCENE_UNIFORMS scene,
    float2 sample_position,
    uint2 dimensions,
    bool depth_of_field,
    uint seed)
{
    CameraRay result;
    result.valid = false;
    float2 uv = sample_position / float2(dimensions);
    float2 ndc = float2(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    float3 pinhole_direction;
    CameraProjection projection = CameraProjection(uint(scene.camera_lens.w));
    switch (projection)
    {
    case CameraProjection::Equirectangular:
        {
            float longitude = ndc.x * PI;
            float latitude = ndc.y * PI * 0.5;
            float latitude_radius = cos(latitude);
            float3 camera_direction = float3(
                sin(longitude) * latitude_radius,
                sin(latitude),
                -cos(longitude) * latitude_radius);
            result.world_origin = scene.camera_position.xyz;
            result.world_direction = normalize(mul(
                scene.camera_world,
                float4(camera_direction, 0.0)).xyz);
            pinhole_direction = result.world_direction;
            break;
        }
    case CameraProjection::Cylindrical:
        {
            float longitude = ndc.x * PI;
            float3 camera_direction = normalize(float3(
                sin(longitude),
                ndc.y * scene.camera_lens.z,
                -cos(longitude)));
            result.world_origin = scene.camera_position.xyz;
            result.world_direction = normalize(mul(
                scene.camera_world,
                float4(camera_direction, 0.0)).xyz);
            pinhole_direction = result.world_direction;
            break;
        }
    case CameraProjection::Fisheye:
        {
            float minimum_dimension = float(min(dimensions.x, dimensions.y));
            float2 lens_position = ndc * float2(dimensions) / minimum_dimension;
            float lens_radius = length(lens_position);
            if (lens_radius > 1.0)
                return result;
            float angle = lens_radius * scene.camera_lens.z;
            float2 lens_direction = lens_radius > RAY_MINIMUM
                ? lens_position / lens_radius
                : float2(0.0);
            float3 camera_direction = float3(
                lens_direction * sin(angle),
                -cos(angle));
            result.world_origin = scene.camera_position.xyz;
            result.world_direction = normalize(mul(
                scene.camera_world,
                float4(camera_direction, 0.0)).xyz);
            pinhole_direction = result.world_direction;
            break;
        }
    case CameraProjection::Perspective:
    case CameraProjection::Orthographic:
    default:
        {
            float4 near_point = mul(
                scene.inverse_view_projection,
                float4(ndc, 0.0, 1.0));
            float4 far_point = mul(
                scene.inverse_view_projection,
                float4(ndc, 1.0, 1.0));
            result.world_origin = near_point.xyz / near_point.w;
            float3 far_position = far_point.xyz / far_point.w;
            result.world_direction = normalize(far_position - result.world_origin);
            pinhole_direction = normalize(far_position - scene.camera_position.xyz);
            break;
        }
    }
    float3 camera_forward = normalize(
        mul(scene.camera_world, float4(0.0, 0.0, -1.0, 0.0)).xyz);
    float camera_plane_distance = dot(
        result.world_origin - scene.camera_position.xyz,
        camera_forward) / max(
            dot(result.world_direction, camera_forward),
            RAY_MINIMUM);
    result.ground_origin = result.world_origin
        - result.world_direction * camera_plane_distance;
    if (depth_of_field && scene.camera_lens.y > 0.0)
    {
        float3 pinhole_origin = scene.camera_position.xyz;
        float focus_t = scene.camera_lens.x
            / max(dot(pinhole_direction, camera_forward), RAY_MINIMUM);
        float3 focus_position = pinhole_origin + pinhole_direction * focus_t;
        float3 lens_tangent;
        float3 lens_bitangent;
        tangent_frame(camera_forward, lens_tangent, lens_bitangent);
        float2 lens_sample = random_disk(seed) * scene.camera_lens.y;
        result.world_origin = pinhole_origin
            + lens_tangent * lens_sample.x
            + lens_bitangent * lens_sample.y;
        result.ground_origin = result.world_origin;
        result.world_direction = normalize(focus_position - result.world_origin);
    }
    result.object_ray.Origin = mul(
        scene.model_inverse,
        float4(result.world_origin, 1.0)).xyz;
    result.object_ray.Direction = normalize(mul(
        scene.model_inverse,
        float4(result.world_direction, 0.0)).xyz);
    result.object_ray.TMin = RAY_MINIMUM;
    result.object_ray.TMax = RAY_MAXIMUM;
    result.valid = true;
    return result;
}
