#define SHRIMPLY_SOFTWARE_RAYS 1
#include "obj_compute.slang"

// Keep the pointer-based Vulkan ABI checked against its Rust producer at build time.
[shader("compute")][numthreads(1, 1, 1)]
void reflect_software(uniform SoftwareResources resources, uniform SoftwareTexture texture,
                      uniform ComputeMaterial material, uniform ComputePbr pbr,
                      uniform SceneUniforms uniforms, uniform MeshInstance instance,
                      uniform SoftwareBvhNode node) {}
