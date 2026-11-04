from typing import Literal

import torch

type Precision = Literal["auto", "bfloat16", "float16", "float32"]

device = "cuda:0" if torch.cuda.is_available() else "cpu"


def available_vram() -> int:
    free, _total = torch.cuda.mem_get_info(device)
    return free


def total_vram() -> int:
    return torch.cuda.get_device_properties(device).total_memory


def bf16_supported(selected: str | torch.device | None = None) -> bool:
    selected_device = torch.device(device if selected is None else selected)
    if selected_device.type != "cuda":
        return False
    with torch.cuda.device(selected_device):
        return torch.cuda.is_bf16_supported(including_emulation=False)


def inference_dtype(
    selected: str | torch.device, precision: Precision = "auto"
) -> torch.dtype:
    selected_device = torch.device(selected)
    match precision:
        case "auto":
            if selected_device.type != "cuda":
                return torch.float32
            return torch.bfloat16 if bf16_supported(selected_device) else torch.float16
        case "bfloat16":
            if selected_device.type == "cuda" and not bf16_supported(selected_device):
                raise ValueError(
                    "The selected GPU does not support native BF16. "
                    "Choose Automatic, FP16, or FP32."
                )
            return torch.bfloat16
        case "float16":
            return torch.float16
        case "float32":
            return torch.float32


def is_out_of_memory(message: str) -> bool:
    lowered = message.lower()
    return "out of memory" in lowered or "not enough gpu memory" in lowered


def select_device(selected: str) -> None:
    from api import resource

    available = ("cpu",) + tuple(
        f"cuda:{index}" for index in range(torch.cuda.device_count())
    )
    if selected not in available:
        raise ValueError(f"Compute device {selected!r} is unavailable")
    resource.scheduler.select_device(selected)
