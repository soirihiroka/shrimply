import abi;
import anime4k;

struct ConvolutionArguments {
  ConvolutionParams params;
  ConvolutionTerm* terms;
  uint64_t term_count;
  float4* output;
  uint64_t output_count;
};

[shader("compute")][numthreads(COMPUTE_GROUP_SIZE, 1, 1)]
void convolution_vulkan(uint3 id : SV_DispatchThreadID, uniform ConvolutionArguments *arguments) {
  convolve_pixel(id, arguments.params, arguments.terms, arguments.term_count,
              arguments.output, arguments.output_count);
}
