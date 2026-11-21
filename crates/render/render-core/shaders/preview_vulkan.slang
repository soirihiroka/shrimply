import abi;
import preview;

struct LayerCompositeArguments {
  LayerCompositeParams params;
  uint *output;
  BufferIndex output_count;
};

// Keep byte-sized blend modes in storage rather than requiring optional
// eight-bit push constants. Pixel composition is the same shared kernel.
[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)]
void composite_layered_image_layer_vulkan(
    uint3 id : SV_DispatchThreadID,
    uniform LayerCompositeArguments *arguments) {
  composite_layer_pixel(id.x, arguments.params, arguments.output,
                                arguments.output_count);
}
