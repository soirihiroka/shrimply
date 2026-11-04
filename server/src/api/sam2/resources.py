from api import gpu
from api.resource import ResourceProfile
from api.sam2.protocol import ModelId

MIB = 1024**2
MODEL_PARAMETERS = {
    "facebook/sam2.1-hiera-tiny": 38_963_010,
    "facebook/sam2.1-hiera-small": 46_060_866,
    "facebook/sam2.1-hiera-base-plus": 80_850_690,
    "facebook/sam2.1-hiera-large": 224_447_154,
}


def profile(model_id: ModelId) -> ResourceProfile:
    # Weights remain resident; the measured inference workspace is transient.
    weight_bytes = MODEL_PARAMETERS[model_id] * gpu.inference_dtype(gpu.device).itemsize
    return ResourceProfile(
        resident_vram=weight_bytes if gpu.device.startswith("cuda:") else 0,
        active_vram=512 * MIB if gpu.device.startswith("cuda:") else 0,
        resident_ram=weight_bytes if not gpu.device.startswith("cuda:") else 0,
        active_ram=512 * MIB if not gpu.device.startswith("cuda:") else 0,
    )
