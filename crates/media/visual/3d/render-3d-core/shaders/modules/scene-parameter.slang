#ifndef SCENE_UNIFORMS
// Vulkan physical-pointer helpers must borrow the scene to avoid per-thread
// copies of its light and ground arrays. Keep other backend ABIs unchanged.
#ifdef SHRIMPLY_VULKAN
#define SCENE_UNIFORMS __constref SceneUniforms
#else
#define SCENE_UNIFORMS SceneUniforms
#endif
#endif

#ifndef SCENE_ARRAY_VIEW
// Hoist physical inline-array bases before induction-variable loops.
#ifdef SHRIMPLY_VULKAN
#define SCENE_ARRAY_VIEW(type, value, member) type* scene_array_##member = &(value).member[0]
#define SCENE_ARRAY_AT(value, member, index) scene_array_##member[index]
#else
#define SCENE_ARRAY_VIEW(type, value, member)
#define SCENE_ARRAY_AT(value, member, index) (value).member[index]
#endif
#endif
