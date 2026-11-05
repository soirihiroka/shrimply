#include <cuda.h>
#define OPTIX_ENABLE_SDK_MIXING 1
#include <optix.h>
#include <optix_function_table_definition.h>
#include <optix_stubs.h>

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "../../../cuda/gpu-cuda/driver.h"

typedef struct ShrimplyOptixDenoiser {
    OptixDeviceContext context;
    OptixDenoiser denoiser;
    CUdeviceptr state;
    CUdeviceptr scratch;
    size_t state_size;
    size_t scratch_size;
    uint32_t width;
    uint32_t height;
} ShrimplyOptixDenoiser;

static void write_error(char* error, size_t capacity, const char* operation, const char* detail) {
    if (error == NULL || capacity == 0) {
        return;
    }
    snprintf(error, capacity, "%s: %s", operation, detail == NULL ? "unknown error" : detail);
}

static int check_optix(OptixResult result, const char* operation, char* error, size_t capacity) {
    if (result == OPTIX_SUCCESS) {
        return 0;
    }
    write_error(error, capacity, operation, optixGetErrorString(result));
    return -1;
}

static int check_cuda(CUresult result, const char* operation, char* error, size_t capacity) {
    if (result == CUDA_SUCCESS) {
        return 0;
    }
    const char* detail = NULL;
    cuGetErrorString(result, &detail);
    write_error(error, capacity, operation, detail);
    return -1;
}

static void log_optix(unsigned int level, const char* tag, const char* message, void* data) {
    (void)data;
    fprintf(stderr, "[OptiX][%u][%s] %s\n", level, tag, message);
}

static void destroy_partial(ShrimplyOptixDenoiser* value) {
    if (value == NULL) {
        return;
    }
    if (value->scratch != 0) {
        cuMemFree(value->scratch);
    }
    if (value->state != 0) {
        cuMemFree(value->state);
    }
    if (value->denoiser != NULL) {
        optixDenoiserDestroy(value->denoiser);
    }
    if (value->context != NULL) {
        optixDeviceContextDestroy(value->context);
    }
    free(value);
}

int shrimply_optix_denoiser_create(
    CUcontext cuda_context,
    CUstream stream,
    uint32_t width,
    uint32_t height,
    ShrimplyOptixDenoiser** output,
    char* error,
    size_t error_capacity) {
    if (cuda_context == NULL || output == NULL || width == 0 || height == 0) {
        write_error(error, error_capacity, "create OptiX denoiser", "invalid argument");
        return -1;
    }
    *output = NULL;
    if (check_optix(optixInit(), "initialize OptiX", error, error_capacity) != 0) {
        return -1;
    }

    ShrimplyOptixDenoiser* value = calloc(1, sizeof(*value));
    if (value == NULL) {
        write_error(error, error_capacity, "create OptiX denoiser", "out of host memory");
        return -1;
    }
    value->width = width;
    value->height = height;

    OptixDeviceContextOptions context_options = {0};
    context_options.logCallbackFunction = log_optix;
    context_options.logCallbackLevel = 3;
    if (check_optix(
            optixDeviceContextCreate(cuda_context, &context_options, &value->context),
            "create OptiX device context",
            error,
            error_capacity) != 0) {
        destroy_partial(value);
        return -1;
    }

    OptixDenoiserOptions options = {0};
    options.guideAlbedo = 1;
    options.guideNormal = 1;
    options.denoiseAlpha = OPTIX_DENOISER_ALPHA_MODE_DENOISE;
    if (check_optix(
            optixDenoiserCreate(value->context, OPTIX_DENOISER_MODEL_KIND_AOV, &options, &value->denoiser),
            "create OptiX AOV denoiser",
            error,
            error_capacity) != 0) {
        destroy_partial(value);
        return -1;
    }

    OptixDenoiserSizes sizes = {0};
    if (check_optix(
            optixDenoiserComputeMemoryResources(value->denoiser, width, height, &sizes),
            "query OptiX denoiser memory",
            error,
            error_capacity) != 0) {
        destroy_partial(value);
        return -1;
    }
    value->state_size = sizes.stateSizeInBytes;
    value->scratch_size = sizes.withoutOverlapScratchSizeInBytes;
    if (check_cuda(cuMemAlloc(&value->state, value->state_size), "allocate OptiX state", error, error_capacity) != 0
        || check_cuda(cuMemAlloc(&value->scratch, value->scratch_size), "allocate OptiX scratch", error, error_capacity) != 0
        || check_optix(
            optixDenoiserSetup(
                value->denoiser,
                stream,
                width,
                height,
                value->state,
                value->state_size,
                value->scratch,
                value->scratch_size),
            "set up OptiX denoiser",
            error,
            error_capacity) != 0) {
        destroy_partial(value);
        return -1;
    }

    *output = value;
    return 0;
}

static OptixImage2D image(CUdeviceptr data, uint32_t width, uint32_t height) {
    OptixImage2D result = {0};
    result.data = data;
    result.width = width;
    result.height = height;
    result.rowStrideInBytes = width * sizeof(float) * 4;
    result.pixelStrideInBytes = sizeof(float) * 4;
    result.format = OPTIX_PIXEL_FORMAT_FLOAT4;
    return result;
}

int shrimply_optix_denoiser_invoke(
    ShrimplyOptixDenoiser* value,
    CUstream stream,
    CUdeviceptr beauty,
    CUdeviceptr refraction,
    CUdeviceptr albedo,
    CUdeviceptr normal,
    char* error,
    size_t error_capacity) {
    if (value == NULL
        || beauty == 0
        || refraction == 0
        || albedo == 0
        || normal == 0) {
        write_error(error, error_capacity, "invoke OptiX denoiser", "invalid argument");
        return -1;
    }

    OptixDenoiserGuideLayer guides = {0};
    guides.albedo = image(albedo, value->width, value->height);
    guides.normal = image(normal, value->width, value->height);

    OptixDenoiserLayer layers[2] = {0};
    layers[0].input = image(beauty, value->width, value->height);
    layers[0].output = image(beauty, value->width, value->height);
    layers[0].type = OPTIX_DENOISER_AOV_TYPE_BEAUTY;
    layers[1].input = image(refraction, value->width, value->height);
    layers[1].output = image(refraction, value->width, value->height);
    layers[1].type = OPTIX_DENOISER_AOV_TYPE_REFRACTION;

    OptixDenoiserParams params = {0};
    params.blendFactor = 0.0f;
    return check_optix(
        optixDenoiserInvoke(
            value->denoiser,
            stream,
            &params,
            value->state,
            value->state_size,
            &guides,
            layers,
            2,
            0,
            0,
            value->scratch,
            value->scratch_size),
        "invoke OptiX denoiser",
        error,
        error_capacity);
}

int shrimply_optix_denoiser_destroy(
    ShrimplyOptixDenoiser* value,
    char* error,
    size_t error_capacity) {
    if (value == NULL) {
        return 0;
    }
    int failed = 0;
    failed |= check_cuda(cuMemFree(value->scratch), "free OptiX scratch", error, error_capacity);
    failed |= check_cuda(cuMemFree(value->state), "free OptiX state", error, error_capacity);
    failed |= check_optix(optixDenoiserDestroy(value->denoiser), "destroy OptiX denoiser", error, error_capacity);
    failed |= check_optix(optixDeviceContextDestroy(value->context), "destroy OptiX context", error, error_capacity);
    free(value);
    return failed == 0 ? 0 : -1;
}
