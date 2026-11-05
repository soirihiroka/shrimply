#ifndef SCENE_UNIFORMS
// Vulkan physical-pointer helpers must borrow the scene to avoid per-thread
// copies of its light and ground arrays. Keep other backend ABIs unchanged.
#ifdef SHRIMPLY_VULKAN
#define SCENE_UNIFORMS __constref SceneUniforms
#else
#define SCENE_UNIFORMS SceneUniforms
#endif
#endif
