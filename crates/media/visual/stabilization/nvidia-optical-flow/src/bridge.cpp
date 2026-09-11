#include <cuda.h>
#if defined(_WIN32)
#include <windows.h>
#else
#include <dlfcn.h>
#endif
#include <nvOpticalFlowCuda.h>

#include <cstddef>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <new>

namespace {

struct Buffer {
    NvOFGPUBufferHandle handle = nullptr;
    CUdeviceptr pointer = 0;
    size_t pitch = 0;
};

struct Context {
    void* library = nullptr;
    NV_OF_CUDA_API_FUNCTION_LIST api{};
    NvOFHandle handle = nullptr;
    CUcontext cuda_context = nullptr;
    CUstream stream = nullptr;
    uint32_t width = 0;
    uint32_t height = 0;
    uint32_t flow_width = 0;
    uint32_t flow_height = 0;
    Buffer input;
    Buffer reference;
    Buffer forward;
    Buffer backward;
    Buffer forward_cost;
    Buffer backward_cost;
};

void set_error(char* error, size_t error_size, const char* operation, const char* detail) {
    if (error != nullptr && error_size != 0) {
        std::snprintf(error, error_size, "%s: %s", operation, detail);
    }
}

const char* status_name(NV_OF_STATUS status) {
    switch (status) {
        case NV_OF_SUCCESS: return "success";
        case NV_OF_ERR_OF_NOT_AVAILABLE: return "optical flow is unavailable";
        case NV_OF_ERR_UNSUPPORTED_DEVICE: return "unsupported GPU";
        case NV_OF_ERR_DEVICE_DOES_NOT_EXIST: return "CUDA device no longer exists";
        case NV_OF_ERR_INVALID_PTR: return "invalid pointer";
        case NV_OF_ERR_INVALID_PARAM: return "invalid parameter";
        case NV_OF_ERR_INVALID_CALL: return "invalid API call order";
        case NV_OF_ERR_INVALID_VERSION: return "unsupported SDK API version";
        case NV_OF_ERR_OUT_OF_MEMORY: return "out of memory";
        case NV_OF_ERR_NOT_INITIALIZED: return "optical flow is not initialized";
        case NV_OF_ERR_UNSUPPORTED_FEATURE: return "unsupported optical flow feature";
        case NV_OF_ERR_GENERIC: return "driver optical flow error";
        default: return "unknown optical flow error";
    }
}

bool check_of(Context* context, NV_OF_STATUS status, const char* operation, char* error, size_t error_size) {
    if (status == NV_OF_SUCCESS) {
        return true;
    }
    char detail[256] = {};
    uint32_t detail_size = sizeof(detail);
    if (context != nullptr && context->handle != nullptr && context->api.nvOFGetLastError != nullptr) {
        context->api.nvOFGetLastError(context->handle, detail, &detail_size);
    }
    set_error(error, error_size, operation, detail[0] == '\0' ? status_name(status) : detail);
    return false;
}

bool check_cuda(CUresult status, const char* operation, char* error, size_t error_size) {
    if (status == CUDA_SUCCESS) {
        return true;
    }
    const char* detail = nullptr;
    cuGetErrorString(status, &detail);
    set_error(error, error_size, operation, detail == nullptr ? "CUDA driver error" : detail);
    return false;
}

bool create_buffer(
    Context* context,
    NV_OF_BUFFER_DESCRIPTOR descriptor,
    Buffer* buffer,
    char* error,
    size_t error_size
) {
    if (!check_of(
            context,
            context->api.nvOFCreateGPUBufferCuda(
                context->handle,
                &descriptor,
                NV_OF_CUDA_BUFFER_TYPE_CUDEVICEPTR,
                &buffer->handle),
            "create NVIDIA optical flow buffer",
            error,
            error_size)) {
        return false;
    }
    buffer->pointer = context->api.nvOFGPUBufferGetCUdeviceptr(buffer->handle);
    NV_OF_CUDA_BUFFER_STRIDE_INFO strides{};
    if (buffer->pointer == 0 || !check_of(
            context,
            context->api.nvOFGPUBufferGetStrideInfo(buffer->handle, &strides),
            "query NVIDIA optical flow buffer stride",
            error,
            error_size)) {
        return false;
    }
    buffer->pitch = strides.strideInfo[0].strideXInBytes;
    return true;
}

void destroy_buffer(Context* context, Buffer* buffer) {
    if (buffer->handle != nullptr && context->api.nvOFDestroyGPUBufferCuda != nullptr) {
        context->api.nvOFDestroyGPUBufferCuda(buffer->handle);
        buffer->handle = nullptr;
    }
}

void destroy(Context* context) {
    if (context == nullptr) {
        return;
    }
    if (context->cuda_context != nullptr) {
        cuCtxSetCurrent(context->cuda_context);
    }
    destroy_buffer(context, &context->backward_cost);
    destroy_buffer(context, &context->forward_cost);
    destroy_buffer(context, &context->backward);
    destroy_buffer(context, &context->forward);
    destroy_buffer(context, &context->reference);
    destroy_buffer(context, &context->input);
    if (context->handle != nullptr && context->api.nvOFDestroy != nullptr) {
        context->api.nvOFDestroy(context->handle);
    }
    if (context->library != nullptr) {
#if defined(_WIN32)
        FreeLibrary(static_cast<HMODULE>(context->library));
#else
        dlclose(context->library);
#endif
    }
    delete context;
}

bool copy_frame(
    Context* context,
    CUdeviceptr source,
    const Buffer& destination,
    char* error,
    size_t error_size
) {
    CUDA_MEMCPY2D copy{};
    copy.srcMemoryType = CU_MEMORYTYPE_DEVICE;
    copy.srcDevice = source;
    copy.srcPitch = static_cast<size_t>(context->width) * sizeof(uint32_t);
    copy.dstMemoryType = CU_MEMORYTYPE_DEVICE;
    copy.dstDevice = destination.pointer;
    copy.dstPitch = destination.pitch;
    copy.WidthInBytes = static_cast<size_t>(context->width) * sizeof(uint32_t);
    copy.Height = context->height;
    return check_cuda(cuMemcpy2DAsync(&copy, context->stream), "copy optical flow input", error, error_size);
}

bool copy_to_host(
    Context* context,
    const Buffer& source,
    void* destination,
    size_t element_size,
    char* error,
    size_t error_size
) {
    CUDA_MEMCPY2D copy{};
    copy.srcMemoryType = CU_MEMORYTYPE_DEVICE;
    copy.srcDevice = source.pointer;
    copy.srcPitch = source.pitch;
    copy.dstMemoryType = CU_MEMORYTYPE_HOST;
    copy.dstHost = destination;
    copy.dstPitch = static_cast<size_t>(context->flow_width) * element_size;
    copy.WidthInBytes = copy.dstPitch;
    copy.Height = context->flow_height;
    return check_cuda(cuMemcpy2DAsync(&copy, context->stream), "read optical flow output", error, error_size);
}

}  // namespace

extern "C" Context* shrimply_nvof_create(
    CUcontext cuda_context,
    CUstream stream,
    uint32_t width,
    uint32_t height,
    uint32_t quality,
    uint32_t output_grid,
    char* error,
    size_t error_size
) {
    Context* context = new (std::nothrow) Context();
    if (context == nullptr) {
        set_error(error, error_size, "create NVIDIA optical flow", "out of host memory");
        return nullptr;
    }
    context->cuda_context = cuda_context;
    context->stream = stream;
    context->width = width;
    context->height = height;
    if ((quality != NV_OF_PERF_LEVEL_SLOW
            && quality != NV_OF_PERF_LEVEL_MEDIUM
            && quality != NV_OF_PERF_LEVEL_FAST)
        || (output_grid != NV_OF_OUTPUT_VECTOR_GRID_SIZE_1
            && output_grid != NV_OF_OUTPUT_VECTOR_GRID_SIZE_2
            && output_grid != NV_OF_OUTPUT_VECTOR_GRID_SIZE_4)) {
        set_error(error, error_size, "create NVIDIA optical flow", "invalid quality or output grid");
        destroy(context);
        return nullptr;
    }
    context->flow_width = (width + output_grid - 1) / output_grid;
    context->flow_height = (height + output_grid - 1) / output_grid;

    if (!check_cuda(cuCtxSetCurrent(cuda_context), "bind CUDA context", error, error_size)) {
        destroy(context);
        return nullptr;
    }
#if defined(_WIN32)
    context->library = LoadLibraryA("nvofapi64.dll");
#else
    context->library = dlopen("libnvidia-opticalflow.so.1", RTLD_NOW | RTLD_LOCAL);
#endif
    if (context->library == nullptr) {
#if defined(_WIN32)
        char detail[64];
        std::snprintf(detail, sizeof detail, "Windows error %lu", GetLastError());
        set_error(error, error_size, "load NVIDIA optical flow driver", detail);
#else
        set_error(error, error_size, "load NVIDIA optical flow driver", dlerror());
#endif
        destroy(context);
        return nullptr;
    }
    using CreateInstance = NV_OF_STATUS (*)(uint32_t, NV_OF_CUDA_API_FUNCTION_LIST*);
    CreateInstance create_instance = nullptr;
#if defined(_WIN32)
    auto symbol = GetProcAddress(static_cast<HMODULE>(context->library),
                                 "NvOFAPICreateInstanceCuda");
#else
    void* symbol = dlsym(context->library, "NvOFAPICreateInstanceCuda");
#endif
    static_assert(sizeof(create_instance) == sizeof(symbol));
    std::memcpy(&create_instance, &symbol, sizeof(create_instance));
    if (create_instance == nullptr) {
#if defined(_WIN32)
        char detail[64];
        std::snprintf(detail, sizeof detail, "Windows error %lu", GetLastError());
        set_error(error, error_size, "load NVIDIA optical flow entry point", detail);
#else
        set_error(error, error_size, "load NVIDIA optical flow entry point", dlerror());
#endif
        destroy(context);
        return nullptr;
    }
    if (!check_of(context, create_instance(NV_OF_API_VERSION, &context->api), "load NVIDIA optical flow API", error, error_size)
        || !check_of(context, context->api.nvCreateOpticalFlowCuda(cuda_context, &context->handle), "create NVIDIA optical flow session", error, error_size)
        || !check_of(context, context->api.nvOFSetIOCudaStreams(context->handle, stream, stream), "set NVIDIA optical flow stream", error, error_size)) {
        destroy(context);
        return nullptr;
    }

    NV_OF_INIT_PARAMS init{};
    init.width = width;
    init.height = height;
    init.outGridSize = static_cast<NV_OF_OUTPUT_VECTOR_GRID_SIZE>(output_grid);
    init.mode = NV_OF_MODE_OPTICALFLOW;
    init.perfLevel = static_cast<NV_OF_PERF_LEVEL>(quality);
    init.enableOutputCost = NV_OF_TRUE;
    init.predDirection = NV_OF_PRED_DIRECTION_BOTH;
    init.inputBufferFormat = NV_OF_BUFFER_FORMAT_ABGR8;
    if (!check_of(context, context->api.nvOFInit(context->handle, &init), "initialize NVIDIA optical flow", error, error_size)) {
        destroy(context);
        return nullptr;
    }

    const NV_OF_BUFFER_DESCRIPTOR input_desc{
        width, height, NV_OF_BUFFER_USAGE_INPUT, NV_OF_BUFFER_FORMAT_ABGR8};
    const NV_OF_BUFFER_DESCRIPTOR output_desc{
        context->flow_width, context->flow_height, NV_OF_BUFFER_USAGE_OUTPUT, NV_OF_BUFFER_FORMAT_SHORT2};
    const NV_OF_BUFFER_DESCRIPTOR cost_desc{
        context->flow_width, context->flow_height, NV_OF_BUFFER_USAGE_COST, NV_OF_BUFFER_FORMAT_UINT8};
    if (!create_buffer(context, input_desc, &context->input, error, error_size)
        || !create_buffer(context, input_desc, &context->reference, error, error_size)
        || !create_buffer(context, output_desc, &context->forward, error, error_size)
        || !create_buffer(context, output_desc, &context->backward, error, error_size)
        || !create_buffer(context, cost_desc, &context->forward_cost, error, error_size)
        || !create_buffer(context, cost_desc, &context->backward_cost, error, error_size)) {
        destroy(context);
        return nullptr;
    }
    return context;
}

extern "C" int shrimply_nvof_estimate(
    Context* context,
    CUdeviceptr input,
    CUdeviceptr reference,
    int use_temporal_hints,
    int disable_temporal_hints,
    NV_OF_FLOW_VECTOR* forward,
    NV_OF_FLOW_VECTOR* backward,
    uint8_t* forward_cost,
    uint8_t* backward_cost,
    char* error,
    size_t error_size
) {
    if (context == nullptr || input == 0 || reference == 0 || forward == nullptr
        || backward == nullptr || forward_cost == nullptr || backward_cost == nullptr) {
        set_error(error, error_size, "estimate NVIDIA optical flow", "invalid pointer");
        return -1;
    }
    if (!check_cuda(cuCtxSetCurrent(context->cuda_context), "bind CUDA context", error, error_size)
        || !copy_frame(context, input, context->input, error, error_size)
        || !copy_frame(context, reference, context->reference, error, error_size)) {
        return -1;
    }

    NV_OF_EXECUTE_INPUT_PARAMS execute_input{};
    execute_input.inputFrame = context->input.handle;
    execute_input.referenceFrame = context->reference.handle;
    execute_input.disableTemporalHints = (!use_temporal_hints || disable_temporal_hints)
        ? NV_OF_TRUE
        : NV_OF_FALSE;
    NV_OF_EXECUTE_OUTPUT_PARAMS execute_output{};
    execute_output.outputBuffer = context->forward.handle;
    execute_output.outputCostBuffer = context->forward_cost.handle;
    execute_output.bwdOutputBuffer = context->backward.handle;
    execute_output.bwdOutputCostBuffer = context->backward_cost.handle;
    if (!check_of(
            context,
            context->api.nvOFExecute(context->handle, &execute_input, &execute_output),
            "execute NVIDIA optical flow",
            error,
            error_size)) {
        return -1;
    }
    if (!copy_to_host(context, context->forward, forward, sizeof(NV_OF_FLOW_VECTOR), error, error_size)
        || !copy_to_host(context, context->backward, backward, sizeof(NV_OF_FLOW_VECTOR), error, error_size)
        || !copy_to_host(context, context->forward_cost, forward_cost, sizeof(uint8_t), error, error_size)
        || !copy_to_host(context, context->backward_cost, backward_cost, sizeof(uint8_t), error, error_size)
        || !check_cuda(cuStreamSynchronize(context->stream), "synchronize NVIDIA optical flow", error, error_size)) {
        return -1;
    }
    return 0;
}

extern "C" void shrimply_nvof_destroy(Context* context) {
    destroy(context);
}
