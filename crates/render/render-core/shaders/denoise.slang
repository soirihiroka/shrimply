import abi;

// Spatial edge-avoiding a-trous filtering, Dammertz et al. (HPG 2010).
// Independent implementation: https://doi.org/10.2312/EGGH/HPG10/067-075
// HDR color stays on the GPU; depth, normal, alpha and radiance stop edge leakage.
static const int ATROUS_RADIUS = 2;
static const float ATROUS_KERNEL[5] = { 1.0, 4.0, 6.0, 4.0, 1.0 };
static const float ATROUS_NORMAL_POWER = 32.0;
static const float ATROUS_RELATIVE_DEPTH = 0.02;
static const float ATROUS_RELATIVE_COLOR = 0.5;
static const float ATROUS_ALPHA_SCALE = 32.0;
static const float ATROUS_MINIMUM = 1e-6;

[shader("compute")][numthreads(8, 8, 1)]
void denoise_atrous(uint3 id : SV_DispatchThreadID,
                    uniform float4* input, uniform float4* output,
                    uniform float4* depth_normal, uniform uint width,
                    uniform uint height, uniform uint step) {
    uint2 pixel = id.xy;
    if (pixel.x >= width || pixel.y >= height) return;
    uint index = pixel.y * width + pixel.x;
    float4 center = input[index];
    float4 guide = depth_normal[index];
    // A zero normal marks a background pixel. Its color and coverage stay exact.
    if (dot(guide.yzw, guide.yzw) < ATROUS_MINIMUM || center.a <= 0.0) {
        output[index] = center;
        return;
    }
    float color_scale = max(dot(center.rgb, center.rgb) * ATROUS_RELATIVE_COLOR,
                            ATROUS_MINIMUM);
    float3 sum = 0;
    float total = 0;
    for (int y = -ATROUS_RADIUS; y <= ATROUS_RADIUS; ++y) {
        for (int x = -ATROUS_RADIUS; x <= ATROUS_RADIUS; ++x) {
            int2 position = int2(pixel) + int2(x, y) * int(step);
            if (any(position < 0) || any(position >= int2(width, height))) continue;
            uint other = uint(position.y) * width + uint(position.x);
            float4 neighbor = input[other];
            float4 other_guide = depth_normal[other];
            float3 difference = neighbor.rgb - center.rgb;
            float depth_scale = max(abs(guide.x) * ATROUS_RELATIVE_DEPTH * float(step), ATROUS_MINIMUM);
            float weight = ATROUS_KERNEL[x + ATROUS_RADIUS] * ATROUS_KERNEL[y + ATROUS_RADIUS]
                * pow(saturate(dot(guide.yzw, other_guide.yzw)), ATROUS_NORMAL_POWER)
                * exp(-abs(guide.x - other_guide.x) / depth_scale
                      - dot(difference, difference) / color_scale
                      - abs(center.a - neighbor.a) * ATROUS_ALPHA_SCALE);
            sum += neighbor.rgb * weight;
            total += weight;
        }
    }
    output[index] = float4(sum / max(total, ATROUS_MINIMUM), center.a);
}
