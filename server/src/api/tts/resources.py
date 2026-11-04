from dataclasses import dataclass

import torch

from api.resource import ResourceProfile
from api.tts.protocol import ModelId

GIB = 1024**3


@dataclass(frozen=True, slots=True)
class ModelResources:
    parameters: int
    vram_workspace: int
    ram_workspace: int = 2 * GIB
    cuda_ram_parameters: int = 0
    reduced_precision_parameters: int | None = None
    cuda_ram_bytes_per_parameter: int | None = None


# Parameter counts come from each model's published safetensors metadata.
MODELS = {
    # Qwen's 170,557,441-parameter speech tokenizer is packaged separately but
    # loaded onto the same device as the main model.
    "Qwen/Qwen3-TTS-12Hz-0.6B-CustomVoice": ModelResources(
        parameters=905_788_672 + 170_557_441,
        vram_workspace=GIB,
    ),
    "Qwen/Qwen3-TTS-12Hz-1.7B-CustomVoice": ModelResources(
        parameters=1_916_676_352 + 170_557_441,
        vram_workspace=2 * GIB,
    ),
    "Qwen/Qwen3-TTS-12Hz-0.6B-Base": ModelResources(
        parameters=914_643_008 + 170_557_441,
        vram_workspace=GIB,
    ),
    "Qwen/Qwen3-TTS-12Hz-1.7B-Base": ModelResources(
        parameters=1_928_677_440 + 170_557_441,
        vram_workspace=2 * GIB,
    ),
    "Qwen/Qwen3-TTS-12Hz-1.7B-VoiceDesign": ModelResources(
        parameters=1_916_676_352 + 170_557_441,
        vram_workspace=2 * GIB,
    ),
    "IndexTeam/IndexTTS-2": ModelResources(
        parameters=2_550_000_000,
        vram_workspace=2 * GIB,
        ram_workspace=3 * GIB,
        cuda_ram_parameters=580_493_120,
    ),
    "IndexTeam/IndexTTS-2.5": ModelResources(
        parameters=2_503_000_000,
        vram_workspace=2 * GIB,
        ram_workspace=3 * GIB,
        cuda_ram_parameters=580_493_120,
        reduced_precision_parameters=812_289_460 + 596_049_920,
        cuda_ram_bytes_per_parameter=4,
    ),
}


def request(model_id: ModelId, dtype: torch.dtype, cuda: bool) -> ResourceProfile:
    model = MODELS[model_id]
    bytes_per_parameter = dtype.itemsize
    reduced_parameters = model.reduced_precision_parameters or model.parameters
    full_precision_parameters = model.parameters - reduced_parameters
    weight_bytes = (
        reduced_parameters * bytes_per_parameter + full_precision_parameters * 4
    )
    if cuda:
        ram_bytes_per_parameter = (
            model.cuda_ram_bytes_per_parameter
            if model.cuda_ram_bytes_per_parameter is not None
            else bytes_per_parameter
        )
        resident_ram = model.cuda_ram_parameters * ram_bytes_per_parameter
    else:
        resident_ram = 0
    return ResourceProfile(
        resident_ram=resident_ram,
        resident_vram=weight_bytes,
        active_ram=model.ram_workspace,
        active_vram=model.vram_workspace,
    )
