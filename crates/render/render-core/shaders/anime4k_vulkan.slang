import abi;
import anime4k;

struct ConvolutionArguments {
  ConvolutionParams params;
  ConvolutionTerm* terms;
  uint64_t term_count;
  float4* output;
  uint64_t output_count;
};
[[vk::binding(0, 0)]] ConstantBuffer<ConvolutionArguments> arguments;

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)]
void convolution_vulkan(uint3 id : SV_DispatchThreadID) {
  convolution(id, arguments.params, arguments.terms, arguments.term_count,
              arguments.output, arguments.output_count);
}
