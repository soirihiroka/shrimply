// Software BVH traversal shares all material, lighting, projection, and outline code
// with the hardware ray-query backend. Only resource access and intersections differ.
struct SoftwareTexture
{
    float4* pixels;
    uint width;
    uint height;
    uint levels;
    uint padding;

    void GetDimensions(out uint w, out uint h) { w = width; h = height; }
    void GetDimensions(uint level, out uint w, out uint h, out uint count)
    { w = max(width >> level, 1u); h = max(height >> level, 1u); count = levels; }

    float4 texel(int2 position, uint level, uint sampler)
    {
        uint w = width, h = height, offset = 0;
        for (uint i = 0; i < level; ++i) { offset += w * h; w = max(w / 2, 1u); h = max(h / 2, 1u); }
        int x = sampler == 1 ? ((position.x % int(w)) + int(w)) % int(w) : clamp(position.x, 0, int(w) - 1);
        int y = clamp(position.y, 0, int(h) - 1);
        return pixels[offset + uint(y) * w + uint(x)];
    }
    float4 Load(int3 position) { return texel(position.xy, uint(position.z), 0); }
    float4 sample_mip(uint sampler, float2 uv, uint level)
    {
        float2 position = uv * float2(max(width >> level, 1u), max(height >> level, 1u)) - 0.5;
        int2 low = int2(floor(position));
        float2 amount = frac(position);
        return lerp(lerp(texel(low, level, sampler), texel(low + int2(1, 0), level, sampler), amount.x),
                    lerp(texel(low + int2(0, 1), level, sampler), texel(low + 1, level, sampler), amount.x), amount.y);
    }
    float4 SampleLevel(uint sampler, float2 uv, float lod)
    {
        float level = clamp(lod, 0.0, float(levels - 1));
        uint low = uint(floor(level));
        return lerp(sample_mip(sampler, uv, low), sample_mip(sampler, uv, min(low + 1, levels - 1)), frac(level));
    }
};

struct SoftwareBvhNode { float4 minimum; float4 maximum; uint4 range; };
struct SoftwareResources
{
    SceneUniforms* scene_data;
    OutputSize output_size_data;
    float4* positions_data;
    float4* normals_data;
    float4* tangents_data;
    float4* tex_coords_0_data;
    float4* tex_coords_1_data;
    float4* vertex_colors_data;
    ComputeMaterial* materials_data;
    MeshInstance* mesh_instances_data;
    SoftwareBvhNode* bvh_nodes;
    uint4* bvh_triangles;
    SoftwareTexture material_texture_data;
    SoftwareTexture environment_texture_data;
    SoftwareTexture composite_texture_data;
    uint* output_pixels_data;
    float4* outline_guide_data;
    float2* outline_distance_data;
    float4* denoiser_beauty_data;
    float4* denoiser_background_data;
    float4* denoiser_alpha_data;
    float4* denoiser_depth_normal_data;
    float4* denoised_texture_data;
    float4* denoised_background_texture_data;
    float4* denoised_alpha_texture_data;
};
struct SoftwareAddress { SoftwareResources* data; };
[[vk::push_constant]] ConstantBuffer<SoftwareAddress> software_address;

#define scene (*(software_address.data->scene_data))
#define output_size (software_address.data->output_size_data)
#define composite_size output_size
#define positions (software_address.data->positions_data)
#define normals (software_address.data->normals_data)
#define tangents (software_address.data->tangents_data)
#define tex_coords_0 (software_address.data->tex_coords_0_data)
#define tex_coords_1 (software_address.data->tex_coords_1_data)
#define vertex_colors (software_address.data->vertex_colors_data)
#define materials (software_address.data->materials_data)
#define mesh_instances (software_address.data->mesh_instances_data)
#define material_texture (software_address.data->material_texture_data)
#define environment_texture (software_address.data->environment_texture_data)
#define composite_texture (software_address.data->composite_texture_data)
#define output_pixels (software_address.data->output_pixels_data)
#define outline_guide (software_address.data->outline_guide_data)
#define outline_distance (software_address.data->outline_distance_data)
#define denoiser_beauty (software_address.data->denoiser_beauty_data)
#define denoiser_background (software_address.data->denoiser_background_data)
#define denoiser_alpha (software_address.data->denoiser_alpha_data)
#define denoiser_depth_normal (software_address.data->denoiser_depth_normal_data)
#define denoised_texture (software_address.data->denoised_texture_data)
#define denoised_background_texture (software_address.data->denoised_background_texture_data)
#define denoised_alpha_texture (software_address.data->denoised_alpha_texture_data)
#define composite_texture_output denoiser_background
#define OUTPUT_TEXEL(pixel) ((pixel).y * output_size.width + (pixel).x)
static const uint material_sampler = 0;
static const uint environment_sampler = 1;
static const uint scene_acceleration = 0;

struct SoftwareRayQuery<let Flags : uint>
{
    RayDesc ray;
    uint node_index;
    uint leaf_index;
    uint leaf_end;
    uint candidate;
    float2 coordinates;
    float distance;
    uint committed;
    float2 committed_coordinates;
    float committed_distance;

    [mutating] void TraceRayInline(uint acceleration, uint flags, uint mask, RayDesc input_ray)
    {
        ray = input_ray; node_index = 0; leaf_index = 0; leaf_end = 0;
        committed = 0xffffffffu; committed_distance = ray.TMax;
    }

    bool overlaps(SoftwareBvhNode node)
    {
        float3 inverse = float3(ray.Direction.x < 0 ? -1.0 : 1.0,
            ray.Direction.y < 0 ? -1.0 : 1.0, ray.Direction.z < 0 ? -1.0 : 1.0)
            / max(abs(ray.Direction), float3(1e-20));
        float3 left = (node.minimum.xyz - ray.Origin) * inverse;
        float3 right = (node.maximum.xyz - ray.Origin) * inverse;
        float3 near = min(left, right), far = max(left, right);
        return max(max(max(near.x, near.y), near.z), ray.TMin)
            <= min(min(min(far.x, far.y), far.z), committed_distance);
    }

    [mutating] bool triangle(uint index)
    {
        uint4 reference = software_address.data->bvh_triangles[index];
        MeshInstance instance = mesh_instances[reference.x];
        uint first = (reference.y == 0 ? instance.geometry_offsets.x : instance.geometry_offsets.z) + reference.z * 3;
        float3 a = mul(instance.model, float4(positions[first].xyz, 1.0)).xyz;
        float3 b = mul(instance.model, float4(positions[first + 1].xyz, 1.0)).xyz;
        float3 c = mul(instance.model, float4(positions[first + 2].xyz, 1.0)).xyz;
        float3 edge1 = b - a, edge2 = c - a;
        float3 p = cross(ray.Direction, edge2);
        float determinant = dot(edge1, p);
        if (abs(determinant) < 1e-10) return false;
        float3 delta = ray.Origin - a;
        float u = dot(delta, p) / determinant;
        if (u < 0.0 || u > 1.0) return false;
        float3 q = cross(delta, edge1);
        float v = dot(ray.Direction, q) / determinant;
        if (v < 0.0 || u + v > 1.0) return false;
        float t = dot(edge2, q) / determinant;
        if (t < ray.TMin || t > committed_distance) return false;
        candidate = index; coordinates = float2(u, v); distance = t;
        return true;
    }

    [mutating] bool Proceed()
    {
        while (true) {
            while (leaf_index < leaf_end) {
                uint index = leaf_index++;
                if (triangle(index)) return true;
            }
            if (node_index >= software_address.data->bvh_nodes[0].range.z) return false;
            SoftwareBvhNode node = software_address.data->bvh_nodes[node_index];
            if (!overlaps(node)) { node_index = node.range.z; continue; }
            ++node_index;
            leaf_index = node.range.x; leaf_end = leaf_index + node.range.y;
        }
    }
    uint CandidateType() { return CANDIDATE_NON_OPAQUE_TRIANGLE; }
    uint CandidateInstanceIndex() { return software_address.data->bvh_triangles[candidate].x; }
    uint CandidateGeometryIndex() { return software_address.data->bvh_triangles[candidate].y; }
    uint CandidatePrimitiveIndex() { return software_address.data->bvh_triangles[candidate].z; }
    float2 CandidateTriangleBarycentrics() { return coordinates; }
    [mutating] void CommitNonOpaqueTriangleHit()
    { committed = candidate; committed_coordinates = coordinates; committed_distance = distance; }
    uint CommittedStatus() { return committed == 0xffffffffu ? COMMITTED_NOTHING : COMMITTED_TRIANGLE_HIT; }
    uint CommittedInstanceIndex() { return software_address.data->bvh_triangles[committed].x; }
    uint CommittedGeometryIndex() { return software_address.data->bvh_triangles[committed].y; }
    uint CommittedPrimitiveIndex() { return software_address.data->bvh_triangles[committed].z; }
    float2 CommittedTriangleBarycentrics() { return committed_coordinates; }
    float CommittedRayT() { return committed_distance; }
};
#define RayQuery SoftwareRayQuery
