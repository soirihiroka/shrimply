from __future__ import annotations

from collections.abc import Callable
from pathlib import Path
from typing import Self

from pydantic import BaseModel, ConfigDict, model_validator

from api import gpu
from api.video_generation.protocol import ModelId, WorkerState

WAN21_MODEL_ID = "Wan-AI/Wan2.1-T2V-1.3B-Diffusers"
WAN22_MODEL_ID = "Wan-AI/Wan2.2-TI2V-5B-Diffusers"
WAN21_FRAMES = 81
WAN21_FRAME_RATE = 16
WAN22_FRAMES = 121
WAN22_FRAME_RATE = 24
WAN21_RESOLUTIONS = frozenset(((832, 480), (480, 832)))
WAN22_RESOLUTIONS = frozenset(((1280, 704), (704, 1280)))


class GenerationRequest(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)

    model: ModelId
    workflow: str
    prompt: str
    negative_prompt: str
    output: Path
    width: int
    height: int
    frames: int
    frame_rate: int
    steps: int
    guidance_scale: float
    seed: int
    image: Path | None = None

    @model_validator(mode="after")
    def validate_request(self) -> Self:
        if self.model not in {WAN21_MODEL_ID, WAN22_MODEL_ID}:
            raise ValueError(f"unsupported Wan model: {self.model}")
        if self.workflow not in {"t2v", "i2v"}:
            raise ValueError(f"unsupported Wan workflow: {self.workflow}")
        if self.model == WAN21_MODEL_ID and self.workflow != "t2v":
            raise ValueError("Wan 2.1 T2V 1.3B only supports text-to-video")
        if self.workflow == "i2v" and self.image is None:
            raise ValueError("Wan image-to-video requires a first frame")
        if not self.prompt.strip():
            raise ValueError("prompt cannot be empty")
        if not 2 <= self.steps <= 100:
            raise ValueError("Wan steps must be between 2 and 100")
        if not 1 <= self.guidance_scale <= 10:
            raise ValueError("Wan guidance scale must be between 1 and 10")
        if self.frames % 4 != 1:
            raise ValueError("Wan frame count must be 4n+1")
        expected_frames, expected_rate, resolutions = (
            (WAN21_FRAMES, WAN21_FRAME_RATE, WAN21_RESOLUTIONS)
            if self.model == WAN21_MODEL_ID
            else (WAN22_FRAMES, WAN22_FRAME_RATE, WAN22_RESOLUTIONS)
        )
        if (self.frames, self.frame_rate) != (expected_frames, expected_rate):
            raise ValueError("Wan frame count or frame rate does not match the model")
        if (self.width, self.height) not in resolutions:
            raise ValueError("Wan resolution does not match the model")
        return self


def generate(
    request: GenerationRequest,
    progress: Callable[[str, WorkerState], None],
    device: str,
) -> None:
    import numpy as np
    import torch
    from diffusers import AutoencoderKLWan, WanImageToVideoPipeline, WanPipeline
    from diffusers.pipelines.wan.pipeline_output import WanPipelineOutput

    from api.media import encode_video, load_image

    if not gpu.bf16_supported(device):
        raise RuntimeError("Wan requires a BF16-capable CUDA device")

    vae = AutoencoderKLWan.from_pretrained(
        request.model,
        subfolder="vae",
        dtype=torch.float32,
    )
    if vae is None:
        raise RuntimeError(f"Could not load Wan VAE {request.model}")

    def on_step_end(
        _pipe: WanPipeline | WanImageToVideoPipeline,
        step: int,
        _timestep: torch.Tensor,
        values: dict[str, torch.Tensor],
    ) -> dict[str, torch.Tensor]:
        progress(
            f"Generating Wan video… step {step + 1}/{request.steps}",
            "generating",
        )
        return values

    generator = torch.Generator(device="cpu").manual_seed(request.seed)
    if request.workflow == "i2v":
        if request.image is None:
            raise RuntimeError("Wan image-to-video requires a first frame")
        pipe = WanImageToVideoPipeline.from_pretrained(
            request.model,
            vae=vae,
            dtype=torch.bfloat16,
        )
        if pipe is None:
            raise RuntimeError(f"Could not load Wan pipeline {request.model}")
        pipe.vae.enable_tiling()
        pipe.enable_model_cpu_offload(device=device)
        pipe.set_progress_bar_config(disable=True)
        with load_image(request.image) as image:
            with torch.inference_mode():
                output = pipe(
                    image=image,
                    prompt=request.prompt,
                    negative_prompt=request.negative_prompt,
                    height=request.height,
                    width=request.width,
                    num_frames=request.frames,
                    num_inference_steps=request.steps,
                    guidance_scale=request.guidance_scale,
                    generator=generator,
                    output_type="np",
                    callback_on_step_end=on_step_end,
                    callback_on_step_end_tensor_inputs=[],
                )
    else:
        pipe = WanPipeline.from_pretrained(
            request.model,
            vae=vae,
            dtype=torch.bfloat16,
        )
        if pipe is None:
            raise RuntimeError(f"Could not load Wan pipeline {request.model}")
        pipe.vae.enable_tiling()
        pipe.enable_model_cpu_offload(device=device)
        pipe.set_progress_bar_config(disable=True)
        with torch.inference_mode():
            output = pipe(
                prompt=request.prompt,
                negative_prompt=request.negative_prompt,
                height=request.height,
                width=request.width,
                num_frames=request.frames,
                num_inference_steps=request.steps,
                guidance_scale=request.guidance_scale,
                generator=generator,
                output_type="np",
                callback_on_step_end=on_step_end,
                callback_on_step_end_tensor_inputs=[],
            )
    if not isinstance(output, WanPipelineOutput):
        raise TypeError("Wan pipeline returned an invalid result")
    frames = getattr(output, "frames", None)
    if not isinstance(frames, np.ndarray) or frames.ndim != 5 or frames.shape[0] != 1:
        raise TypeError("Wan pipeline returned invalid video frames")
    progress("Encoding Wan video…", "decoding")
    expected = (request.frames, request.height, request.width, 3)
    if frames[0].shape != expected:
        raise RuntimeError(
            f"Wan returned frames of shape {frames[0].shape}, expected {expected}"
        )
    encode_video(
        request.output, frames[0], request.frame_rate, torch.cuda.current_device()
    )
