import abi;
import background;
import background_common;

struct BackgroundArguments {
  uint* output;
  BackgroundUniforms uniforms;
};

[shader("compute")][numthreads(16, 16, 1)]
void render_background_vulkan(uint3 id : SV_DispatchThreadID, uniform BackgroundArguments *arguments) {
  write_background_pixel(id, arguments.output, arguments.uniforms);
}
