#define SHRIMPLY_SOFTWARE_RAYS 1
#include "obj_compute.slang"

// Keep the pointer-based Vulkan ABI checked against its Rust producer at build time.
[shader("compute")][numthreads(1, 1, 1)]
void reflect_software(uniform SoftwareResources resources, uniform SoftwareTexture texture,
                      uniform ComputeMaterial material, uniform ComputePbr pbr,
                      uniform SceneUniforms uniforms, uniform MeshInstance instance,
                      uniform SoftwareBvhNode node) {}

static const int DENOISE_RADIUS = 2;
static const float DENOISE_COLOR_SIGMA = 0.25;
static const float DENOISE_DEPTH_SIGMA = 0.02;
static const float DENOISE_NORMAL_POWER = 32.0;

[shader("compute")][numthreads(8, 8, 1)]
void obj_software_denoise(uint3 thread_id : SV_DispatchThreadID)
{
    uint2 pixel = thread_id.xy;
    uint width = output_size.width, height = output_size.height;
    if (pixel.x >= width || pixel.y >= height) return;
    uint index = pixel.y * width + pixel.x;
    float4 center = denoiser_beauty[index];
    float4 guide = denoiser_depth_normal[index];
    float4 beauty = 0, background = 0, alpha = 0;
    float total = 0;
    for (int y = -DENOISE_RADIUS; y <= DENOISE_RADIUS; ++y) {
        for (int x = -DENOISE_RADIUS; x <= DENOISE_RADIUS; ++x) {
            int2 position = clamp(int2(pixel) + int2(x, y), int2(0), int2(width - 1, height - 1));
            uint other = uint(position.y) * width + uint(position.x);
            float4 neighbor = denoiser_beauty[other];
            float4 other_guide = denoiser_depth_normal[other];
            bool center_hit = guide.x < RAY_MAXIMUM;
            bool other_hit = other_guide.x < RAY_MAXIMUM;
            if (center_hit != other_hit) continue;
            float3 difference = neighbor.rgb - center.rgb;
            float weight = exp(-float(x*x + y*y) / float(DENOISE_RADIUS * DENOISE_RADIUS)
                - dot(difference, difference) / (DENOISE_COLOR_SIGMA * DENOISE_COLOR_SIGMA));
            if (center_hit) {
                weight *= exp(-abs(guide.x - other_guide.x) / max(abs(guide.x) * DENOISE_DEPTH_SIGMA, RAY_MINIMUM));
                weight *= pow(saturate(dot(guide.yzw, other_guide.yzw)), DENOISE_NORMAL_POWER);
            }
            beauty += neighbor * weight;
            background += denoiser_background[other] * weight;
            alpha += denoiser_alpha[other] * weight;
            total += weight;
        }
    }
    denoised_texture[index] = beauty / max(total, 1e-20);
    denoised_background_texture[index] = background / max(total, 1e-20);
    denoised_alpha_texture[index] = alpha / max(total, 1e-20);
}

[shader("compute")][numthreads(8, 8, 1)]
void obj_software_probe(uint3 thread_id : SV_DispatchThreadID) {
    uint2 pixel = thread_id.xy;
    if (pixel.x >= output_size.width || pixel.y >= output_size.height) return;
    CameraRay camera = camera_ray(scene, float2(pixel) + 0.5, uint2(output_size.width, output_size.height), false, 0);
    SoftwareRayQuery<0> query;
    query.TraceRayInline(0, 0, 0, camera.object_ray);
    bool bounds = query.overlaps(software_address.data->bvh_nodes[0]);
    while (query.Proceed()) { query.CommitNonOpaqueTriangleHit(); }
    output_pixels[pixel.y * output_size.width + pixel.x] = pack_color(float4(camera.valid ? 1.0 : 0.0, query.CommittedStatus() == COMMITTED_TRIANGLE_HIT ? 1.0 : 0.0, bounds ? 1.0 : 0.0, 1.0));
}
