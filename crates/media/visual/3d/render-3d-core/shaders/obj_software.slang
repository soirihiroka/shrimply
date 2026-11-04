#define SHRIMPLY_SOFTWARE_RAYS 1
#include "obj_compute.slang"

// Keep the pointer-based Vulkan ABI checked against its Rust producer at build time.
[shader("compute")][numthreads(1, 1, 1)]
void reflect_software(uniform SoftwareResources resources, uniform SoftwareTexture texture,
                      uniform ComputeMaterial material, uniform ComputePbr pbr,
                      uniform SceneUniforms uniforms, uniform MeshInstance instance,
                      uniform SoftwareBvhNode node) {}

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
