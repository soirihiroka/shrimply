from __future__ import annotations

import gc
import hashlib
import json
import os
import shutil
import time
import types
from dataclasses import dataclass
from datetime import UTC, datetime
from fractions import Fraction
from pathlib import Path
from collections.abc import Mapping
from typing import (
    TYPE_CHECKING,
    Callable,
    Literal,
    Protocol,
    TypedDict,
    overload,
    runtime_checkable,
)

import env
import torch
from pydantic import BaseModel, ConfigDict

from .config import MODEL_ID, ReferenceSpec, align_num_frames, validate_canvas, validate_references
from api.video_generation.protocol import WorkerState

if TYPE_CHECKING:
    import numpy as np
    from PIL.Image import Image
    from diffusers import MiniMaxH3Transformer3DModel, ModularPipeline
    from diffusers.modular_pipelines.components_manager import ComponentsManager
    from diffusers.modular_pipelines.minimax_h3 import (
        MiniMaxH3AudioReference,
        MiniMaxH3ImageReference,
        MiniMaxH3VideoReference,
    )
    from diffusers.modular_pipelines.modular_pipeline import (
        ModularPipelineBlocks,
        PipelineState,
    )

type H3Reference = (
    MiniMaxH3AudioReference | MiniMaxH3ImageReference | MiniMaxH3VideoReference
)


class ManifestDuration(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True)
    numerator: int
    denominator: int


class ManifestReference(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True)
    kind: str
    source: str


class RequestManifest(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True)
    workflow: str
    prompt: str
    duration: ManifestDuration
    width: int
    height: int
    steps: int
    seed: int
    model: str
    lora: str | None
    lora_weight_name: str | None
    lora_scale: float
    image: str | None
    last_image: str | None
    references: list[ManifestReference]


class CheckpointStatus(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True)
    stage: str
    updated_at: datetime
    checkpoint_path: str | None = None
    output: str | None = None
    conditioning_checkpoint: str | None = None
    decode_elapsed_seconds: float | None = None


class ConditioningState(TypedDict):
    prompt_embeds: torch.Tensor
    text_token_tags: torch.Tensor
    condition_latents: list[torch.Tensor]
    height: int
    width: int
    num_frames: int
    keyframe_anchors: tuple[str, ...]


class GenerationArguments(TypedDict, total=False):
    prompt: str
    height: int
    width: int
    num_frames: int
    num_inference_steps: int
    generator: torch.Generator
    image: Image
    last_image: Image
    references: list[H3Reference]


class GenerationOutput(TypedDict, total=False):
    output: str
    checkpoint: str
    sampling_rate: int
    decode_elapsed_seconds: float
    peak_gpu_memory_gib: float
    frames: int
    duration: Fraction
    attention: str
    memory: str
    resumed: bool
    elapsed_seconds: float
    generation_elapsed_seconds: float
    generation_peak_gpu_memory_gib: float


@runtime_checkable
class AttentionTransformer(Protocol):
    def set_attention_backend(self, backend: str) -> None: ...


class PipelineResult(Protocol):
    @overload
    def __getitem__(
        self,
        key: Literal[
            "latents", "audio_latents", "audio", "prompt_embeds", "text_token_tags"
        ],
    ) -> torch.Tensor: ...

    @overload
    def __getitem__(self, key: Literal["videos"]) -> np.ndarray: ...

    @overload
    def __getitem__(
        self, key: Literal["sampling_rate", "height", "width", "num_frames"]
    ) -> int: ...

    @overload
    def __getitem__(self, key: Literal["condition_latents"]) -> list[torch.Tensor]: ...

    @overload
    def __getitem__(
        self, key: Literal["keyframe_anchors"]
    ) -> list[str] | tuple[str, ...]: ...


class CommitConfig(Protocol):
    _commit_hash: str | None


class TextEncoderModel(Protocol):
    language_model: torch.nn.Module
    visual: torch.nn.Module


class TextEncoder(Protocol):
    config: CommitConfig
    model: TextEncoderModel


class VideoVae(Protocol):
    config: CommitConfig
    encoder: torch.nn.Module
    decoder: torch.nn.Module
    quant_conv: torch.nn.Module
    post_quant_conv: torch.nn.Module


@runtime_checkable
class H3Pipeline(Protocol):
    _blocks: ModularPipelineBlocks
    components: Mapping[str, torch.nn.Module | None]
    text_encoder: TextEncoder
    transformer: MiniMaxH3Transformer3DModel
    vae: VideoVae
    audio_vae: torch.nn.Module

    def load_components(
        self,
        *,
        names: list[str] | None = None,
        dtype: torch.dtype | dict[str, torch.dtype] | None = None,
    ) -> None: ...

    def __call__(
        self,
        *,
        output: list[str],
        state: PipelineState | None = None,
        prompt: str | None = None,
        height: int | None = None,
        width: int | None = None,
        num_frames: int | None = None,
        num_inference_steps: int | None = None,
        generator: torch.Generator | None = None,
        image: Image | None = None,
        last_image: Image | None = None,
        references: list[H3Reference] | None = None,
    ) -> PipelineResult: ...


@runtime_checkable
class DestructiveOffloadHook(Protocol):
    model: torch.nn.Module
    offload: Callable[[], None]


@dataclass(frozen=True)
class GenerationRequest:
    workflow: str
    prompt: str
    output: Path
    duration: Fraction = Fraction(5)
    width: int = 768
    height: int = 768
    steps: int = 30
    seed: int = 42
    model: str = MODEL_ID
    attention: str = "auto"
    memory: str = "auto"
    checkpoint: Path | None = None
    restart: bool = False
    lora: str | None = None
    lora_weight_name: str | None = None
    lora_scale: float = 1.0
    image: str | None = None
    last_image: str | None = None
    references: tuple[ReferenceSpec, ...] = ()

    def validate(self) -> None:
        if self.workflow not in {"t2va", "fl2va", "ref2va"}:
            raise ValueError(f"unknown workflow: {self.workflow}")
        if not self.prompt.strip():
            raise ValueError("prompt cannot be empty")
        if self.steps < 2:
            raise ValueError("steps must be at least 2 scheduler points")
        if self.memory not in {"auto", "bf16", "hybrid", "stream", "int8"}:
            raise ValueError("memory must be auto, bf16, hybrid, stream, or int8")
        if not 0 <= self.lora_scale <= 4:
            raise ValueError("LoRA scale must be between 0 and 4")
        validate_canvas(self.width, self.height)
        align_num_frames(self.duration)
        if self.workflow == "fl2va" and not (self.image or self.last_image):
            raise ValueError("fl2va requires --image and/or --last-image")
        if self.workflow == "ref2va":
            validate_references(list(self.references))


def _configure_attention(pipe: H3Pipeline, attention: str) -> str:
    if attention == "default":
        return "default"
    if attention not in {"auto", "flash3"}:
        raise ValueError("attention must be auto, flash3, or default")
    transformer = getattr(pipe, "transformer", None) or getattr(
        pipe, "transformer_ref", None
    )
    if not isinstance(transformer, AttentionTransformer):
        raise TypeError("MiniMax H3 pipeline has no attention transformer")
    try:
        transformer.set_attention_backend("_flash_3_hub")
        return "flash3"
    except Exception:
        if attention == "flash3":
            raise
        return "default"


def _build_references(specs: tuple[ReferenceSpec, ...]) -> list[H3Reference]:
    from api.media import decode_audio, load_image
    from diffusers.modular_pipelines.minimax_h3 import (
        MiniMaxH3AudioReference,
        MiniMaxH3ImageReference,
        MiniMaxH3VideoReference,
    )

    references: list[H3Reference] = []
    for spec in specs:
        match spec.kind:
            case "image":
                references.append(MiniMaxH3ImageReference(image=load_image(spec.source)))
            case "video":
                references.append(MiniMaxH3VideoReference.from_file(spec.source))
            case "audio":
                rate, samples = decode_audio(Path(spec.source), layout="stereo")
                references.append(MiniMaxH3AudioReference(
                    audio=torch.from_numpy(samples), sample_rate=rate
                ))
            case _:
                raise ValueError(f"Unsupported MiniMax H3 reference: {spec.kind}")
    return references


def _default_checkpoint_path(output: Path) -> Path:
    return output.with_suffix(output.suffix + ".latents.safetensors")


def _conditioning_checkpoint_path(checkpoint: Path) -> Path:
    stem = checkpoint.name.removesuffix(".latents.safetensors")
    return checkpoint.with_name(stem + ".conditioning.safetensors")


def _request_manifest(request: GenerationRequest) -> RequestManifest:
    return RequestManifest(
        workflow=request.workflow,
        prompt=request.prompt,
        duration=ManifestDuration(
            numerator=request.duration.numerator,
            denominator=request.duration.denominator,
        ),
        width=request.width,
        height=request.height,
        steps=request.steps,
        seed=request.seed,
        model=request.model,
        lora=request.lora,
        lora_weight_name=request.lora_weight_name,
        lora_scale=request.lora_scale,
        image=request.image,
        last_image=request.last_image,
        references=[
            ManifestReference(kind=reference.kind, source=reference.source)
            for reference in request.references
        ],
    )


def _manifest_signature(manifest: RequestManifest) -> str:
    encoded = json.dumps(
        manifest.model_dump(mode="json"),
        sort_keys=True,
        separators=(",", ":"),
    ).encode()
    return hashlib.sha256(encoded).hexdigest()


def _write_status(
    checkpoint: Path,
    stage: str,
    *,
    checkpoint_path: str | None = None,
    output: str | None = None,
    conditioning_checkpoint: str | None = None,
    decode_elapsed_seconds: float | None = None,
) -> None:
    path = checkpoint.with_suffix(checkpoint.suffix + ".status.json")
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    status = CheckpointStatus(
        stage=stage,
        updated_at=datetime.now(UTC),
        checkpoint_path=checkpoint_path,
        output=output,
        conditioning_checkpoint=conditioning_checkpoint,
        decode_elapsed_seconds=decode_elapsed_seconds,
    )
    try:
        temporary.write_text(status.model_dump_json(indent=2, exclude_none=True) + "\n")
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def _save_latent_checkpoint(
    checkpoint: Path,
    request: GenerationRequest,
    latents: torch.Tensor,
    audio_latents: torch.Tensor,
) -> None:
    from safetensors.torch import save_file

    checkpoint.parent.mkdir(parents=True, exist_ok=True)
    temporary = checkpoint.with_name(f".{checkpoint.name}.{os.getpid()}.tmp")
    manifest = _request_manifest(request)
    try:
        save_file(
            {
                "latents": latents.detach().to("cpu").contiguous(),
                "audio_latents": audio_latents.detach().to("cpu").contiguous(),
            },
            str(temporary),
            metadata={
                "stage": "denoised",
                "manifest": manifest.model_dump_json(),
                "signature": _manifest_signature(manifest),
            },
        )
        os.replace(temporary, checkpoint)
    finally:
        temporary.unlink(missing_ok=True)
    _write_status(checkpoint, "denoised", checkpoint_path=str(checkpoint))


def _load_latent_checkpoint(
    checkpoint: Path, expected_request: GenerationRequest | None = None
) -> tuple[torch.Tensor, torch.Tensor, RequestManifest]:
    from safetensors import safe_open
    from safetensors.torch import load_file

    if not checkpoint.is_file():
        raise FileNotFoundError(f"latent checkpoint not found: {checkpoint}")
    with safe_open(checkpoint, framework="pt", device="cpu") as handle:
        metadata = handle.metadata() or {}
    if metadata.get("stage") != "denoised":
        raise ValueError(f"checkpoint is not a completed denoised state: {checkpoint}")
    manifest = RequestManifest.model_validate_json(metadata.get("manifest", "{}"))
    signature = metadata.get("signature")
    if signature != _manifest_signature(manifest):
        raise ValueError(f"checkpoint metadata signature is invalid: {checkpoint}")
    if expected_request is not None and signature != _manifest_signature(_request_manifest(expected_request)):
        raise ValueError(
            f"checkpoint belongs to different generation arguments: {checkpoint}; use --restart to replace it"
        )
    tensors = load_file(checkpoint, device="cpu")
    return tensors["latents"], tensors["audio_latents"], manifest


def _save_conditioning_checkpoint(
    checkpoint: Path,
    request: GenerationRequest,
    values: ConditioningState,
) -> None:
    """Atomically persist everything FL2VA needs before loading the denoiser."""
    from safetensors.torch import save_file

    checkpoint.parent.mkdir(parents=True, exist_ok=True)
    temporary = checkpoint.with_name(f".{checkpoint.name}.{os.getpid()}.tmp")
    manifest = _request_manifest(request)
    condition_latents = values["condition_latents"]
    tensors = {
        "prompt_embeds": values["prompt_embeds"].detach().to("cpu").contiguous(),
        "text_token_tags": values["text_token_tags"].detach().to("cpu").contiguous(),
        **{
            f"condition_latents.{index}": latent.detach().to("cpu").contiguous()
            for index, latent in enumerate(condition_latents)
        },
    }
    try:
        save_file(
            tensors,
            str(temporary),
            metadata={
                "stage": "conditioned",
                "manifest": manifest.model_dump_json(),
                "signature": _manifest_signature(manifest),
                "height": str(values["height"]),
                "width": str(values["width"]),
                "num_frames": str(
                    values.get("num_frames") or align_num_frames(request.duration)
                ),
                "keyframe_anchors": json.dumps(list(values["keyframe_anchors"])),
                "condition_count": str(len(condition_latents)),
            },
        )
        os.replace(temporary, checkpoint)
    finally:
        temporary.unlink(missing_ok=True)
    _write_status(checkpoint, "conditioned", checkpoint_path=str(checkpoint))


def _load_conditioning_checkpoint(
    checkpoint: Path, request: GenerationRequest
) -> ConditioningState:
    from safetensors import safe_open
    from safetensors.torch import load_file

    if not checkpoint.is_file():
        raise FileNotFoundError(f"conditioning checkpoint not found: {checkpoint}")
    with safe_open(checkpoint, framework="pt", device="cpu") as handle:
        metadata = handle.metadata() or {}
    if metadata.get("stage") != "conditioned":
        raise ValueError(f"checkpoint is not a completed conditioning state: {checkpoint}")
    manifest = RequestManifest.model_validate_json(metadata.get("manifest", "{}"))
    signature = metadata.get("signature")
    if signature != _manifest_signature(manifest):
        raise ValueError(f"checkpoint metadata signature is invalid: {checkpoint}")
    if signature != _manifest_signature(_request_manifest(request)):
        raise ValueError(
            f"conditioning checkpoint belongs to different generation arguments: {checkpoint}; "
            "use --restart to replace it"
        )
    tensors = load_file(checkpoint, device="cpu")
    condition_count = int(metadata["condition_count"])
    anchors = json.loads(metadata["keyframe_anchors"])
    if not isinstance(anchors, list) or not all(
        isinstance(anchor, str) for anchor in anchors
    ):
        raise ValueError("conditioning checkpoint has invalid keyframe anchors")
    result: ConditioningState = {
        "prompt_embeds": tensors["prompt_embeds"],
        "text_token_tags": tensors["text_token_tags"],
        "condition_latents": [
            tensors[f"condition_latents.{index}"]
            for index in range(condition_count)
        ],
        "height": int(metadata["height"]),
        "width": int(metadata["width"]),
        "num_frames": (
            align_num_frames(request.duration)
            if metadata.get("num_frames") in {None, "None"}
            else int(metadata["num_frames"])
        ),
        "keyframe_anchors": tuple(anchors),
    }
    return result


def _prepare_disk_group_cache(path: Path) -> None:
    """Discard an interrupted cache build; Diffusers trusts any existing group file."""
    marker = path / ".complete"
    if path.exists() and not marker.is_file():
        shutil.rmtree(path)
    path.mkdir(parents=True, exist_ok=True)


def _enable_disk_groups(module: torch.nn.Module, path: Path) -> None:
    from diffusers.hooks import apply_group_offloading

    _prepare_disk_group_cache(path)
    apply_group_offloading(
        module,
        onload_device="cuda",
        offload_device="cpu",
        offload_type="block_level",
        num_blocks_per_group=1,
        use_stream=False,
        offload_to_disk_path=str(path),
    )
    (path / ".complete").write_text("MiniMax H3 disk group cache ready\n")


class _OffloadEveryResidentComponent:
    """Keep only the currently executing top-level component on the GPU."""

    def __call__(
        self,
        hooks: list[DestructiveOffloadHook],
        model_id: str,
        model: torch.nn.Module,
        execution_device: torch.device,
    ) -> list[DestructiveOffloadHook]:
        del model_id, model, execution_device
        return list(hooks)


def _discard_hook_to_empty_cpu(self: DestructiveOffloadHook) -> None:
    """Release an exhausted component without copying its weights from CUDA to RAM."""
    self.model.to_empty(device="cpu")


def _make_transformer_offload_destructive(
    manager: ComponentsManager, transformer: MiniMaxH3Transformer3DModel
) -> None:
    """H3 never calls its denoiser again after decode starts, so its GPU copy can be discarded."""
    for user_hook in manager.model_hooks or []:
        if not isinstance(user_hook, DestructiveOffloadHook):
            raise TypeError("ComponentsManager returned an invalid offload hook")
        if user_hook.model is transformer:
            user_hook.offload = types.MethodType(_discard_hook_to_empty_cpu, user_hook)
            return
    raise RuntimeError("ComponentsManager did not attach an offload hook to the H3 transformer")


def _pop_decode_blocks(
    blocks: ModularPipelineBlocks,
) -> list[tuple[str, ModularPipelineBlocks]]:
    """Remove either nested `decode` or workflow-flattened `decode.*` blocks."""
    names = [name for name in blocks.sub_blocks if name == "decode" or name.startswith("decode.")]
    return [(name, blocks.sub_blocks.pop(name)) for name in names]


def _restore_decode_blocks(
    blocks: ModularPipelineBlocks,
    removed: list[tuple[str, ModularPipelineBlocks]],
) -> None:
    # MiniMax H3's decode blocks are terminal, so appending preserves execution order.
    for name, block in removed:
        blocks.sub_blocks[name] = block


def _component_is_complete(path: Path) -> bool:
    return (path / ".complete").is_file() and (path / "config.json").is_file()


def _mark_component_complete(path: Path) -> None:
    (path / ".complete").write_text("MiniMax H3 int8 component ready\n")


def _apply_lora(
    transformer: MiniMaxH3Transformer3DModel, request: GenerationRequest
) -> None:
    if not request.lora:
        return
    from .lora import load_lora

    converted = load_lora(
        transformer,
        request.lora,
        request.lora_scale,
        request.lora_weight_name,
        workflow=request.workflow,
    )
    print(f"Loaded LoRA {converted} at scale {request.lora_scale:g}", flush=True)


def _build_int8_pipeline(
    request: GenerationRequest, cache_dir: str | None
) -> H3Pipeline:
    import torch
    from diffusers import MiniMaxH3Transformer3DModel, ModularPipeline, TorchAoConfig
    from diffusers.hooks import apply_group_offloading
    from torchao.quantization import Int8WeightOnlyConfig
    from transformers import Qwen3VLForConditionalGeneration
    from transformers import TorchAoConfig as TransformersTorchAoConfig

    pipe = ModularPipeline.from_pretrained(
        request.model, workflow=request.workflow, cache_dir=cache_dir
    )
    if pipe is None:
        raise RuntimeError(f"Could not load MiniMax H3 pipeline {request.model}")
    transformer_name = "transformer_ref" if request.workflow == "ref2va" else "transformer"
    quantized_root = env.minimax_h3_quantized_cache_root()
    transformer_cache = quantized_root / transformer_name
    text_encoder_cache = quantized_root / "text_encoder"

    if _component_is_complete(transformer_cache):
        print(f"Loading persisted int8 {transformer_name} from {transformer_cache}", flush=True)
        transformer = MiniMaxH3Transformer3DModel.from_pretrained(
            transformer_cache,
            dtype=torch.bfloat16,
            low_cpu_mem_usage=True,
        )
        assert transformer is not None
    else:
        print(f"Quantizing {transformer_name}; this component will be persisted after conversion", flush=True)
        transformer = MiniMaxH3Transformer3DModel.from_pretrained(
            request.model,
            subfolder=transformer_name,
            cache_dir=cache_dir,
            dtype=torch.bfloat16,
            quantization_config=TorchAoConfig(
                Int8WeightOnlyConfig(version=2),
                modules_to_not_convert=[
                    "proj_in",
                    "audio_proj_in",
                    "context_embedder",
                    "time_embedder",
                    "time_proj",
                    "token_refiner",
                    "norm_out",
                    "proj_out",
                    "audio_proj_out",
                ],
            ),
            low_cpu_mem_usage=True,
        )
        assert transformer is not None
        transformer_cache.mkdir(parents=True, exist_ok=True)
        transformer.save_pretrained(transformer_cache, safe_serialization=True)
        _mark_component_complete(transformer_cache)
        print(f"Persisted int8 {transformer_name} to {transformer_cache}", flush=True)

    if _component_is_complete(text_encoder_cache):
        print(f"Loading persisted int8 text_encoder from {text_encoder_cache}", flush=True)
        text_encoder = Qwen3VLForConditionalGeneration.from_pretrained(
            text_encoder_cache,
            dtype=torch.bfloat16,
            low_cpu_mem_usage=True,
        )
    else:
        print("Quantizing text_encoder; this component will be persisted after conversion", flush=True)
        text_encoder = Qwen3VLForConditionalGeneration.from_pretrained(
            request.model,
            subfolder="text_encoder",
            cache_dir=cache_dir,
            dtype=torch.bfloat16,
            quantization_config=TransformersTorchAoConfig(
                Int8WeightOnlyConfig(version=2),
                modules_to_not_convert=[
                    "model.visual",
                    "model.language_model.embed_tokens",
                    "model.language_model.norm",
                    "lm_head",
                ],
            ),
            low_cpu_mem_usage=True,
        )
        text_encoder_cache.mkdir(parents=True, exist_ok=True)
        text_encoder.save_pretrained(text_encoder_cache, safe_serialization=True)
        _mark_component_complete(text_encoder_cache)
        print(f"Persisted int8 text_encoder to {text_encoder_cache}", flush=True)
    if not isinstance(transformer, MiniMaxH3Transformer3DModel):
        raise TypeError("MiniMax H3 transformer does not implement its runtime interface")
    if text_encoder is None:
        raise RuntimeError("Could not load MiniMax H3 text encoder")
    pipe.update_components(**{transformer_name: transformer, "text_encoder": text_encoder})
    pipe.load_components(dtype=torch.bfloat16)
    _apply_lora(transformer, request)
    transformer.requires_grad_(False)
    text_encoder.requires_grad_(False)
    transformer.enable_group_offload(
        onload_device=torch.device("cuda"),
        offload_device=torch.device("cpu"),
        offload_type="block_level",
        num_blocks_per_group=1,
        use_stream=True,
    )
    apply_group_offloading(
        text_encoder.model,
        onload_device=torch.device("cuda"),
        offload_device=torch.device("cpu"),
        offload_type="leaf_level",
        use_stream=True,
    )
    pipe.vae.enable_group_offload(
        onload_device=torch.device("cuda"),
        offload_device=torch.device("cpu"),
        offload_type="leaf_level",
        use_stream=False,
    )
    if not isinstance(pipe, H3Pipeline):
        raise TypeError("MiniMax H3 pipeline does not implement its runtime interface")
    return pipe


def _build_streamed_bf16_pipeline(
    request: GenerationRequest, cache_dir: str | None
) -> H3Pipeline:
    """Keep BF16 weights in host RAM and move individual model blocks to CUDA."""
    import torch
    from diffusers import MiniMaxH3Transformer3DModel, ModularPipeline
    from diffusers.hooks import apply_group_offloading
    from transformers import Qwen3VLForConditionalGeneration

    pipe = ModularPipeline.from_pretrained(
        request.model, workflow=request.workflow, cache_dir=cache_dir
    )
    if pipe is None:
        raise RuntimeError(f"Could not load MiniMax H3 pipeline {request.model}")
    transformer_name = "transformer_ref" if request.workflow == "ref2va" else "transformer"
    transformer = MiniMaxH3Transformer3DModel.from_pretrained(
        request.model,
        subfolder=transformer_name,
        cache_dir=cache_dir,
        dtype=torch.bfloat16,
        low_cpu_mem_usage=True,
    )
    if not isinstance(transformer, MiniMaxH3Transformer3DModel):
        raise TypeError("MiniMax H3 transformer does not implement its runtime interface")
    text_encoder = Qwen3VLForConditionalGeneration.from_pretrained(
        request.model,
        subfolder="text_encoder",
        cache_dir=cache_dir,
        dtype=torch.bfloat16,
        low_cpu_mem_usage=True,
    )
    if text_encoder is None:
        raise RuntimeError("Could not load MiniMax H3 text encoder")
    pipe.update_components(**{transformer_name: transformer, "text_encoder": text_encoder})
    pipe.load_components(dtype=torch.bfloat16)
    _apply_lora(transformer, request)
    transformer.requires_grad_(False)
    text_encoder.requires_grad_(False)

    transformer.enable_group_offload(
        onload_device=torch.device("cuda"),
        offload_device=torch.device("cpu"),
        offload_type="block_level",
        num_blocks_per_group=1,
        use_stream=False,
    )
    apply_group_offloading(
        text_encoder.model,
        onload_device=torch.device("cuda"),
        offload_device=torch.device("cpu"),
        offload_type="leaf_level",
        use_stream=False,
    )
    pipe.vae.enable_group_offload(
        onload_device=torch.device("cuda"),
        offload_device=torch.device("cpu"),
        offload_type="leaf_level",
        use_stream=False,
    )
    if not isinstance(pipe, H3Pipeline):
        raise TypeError("MiniMax H3 pipeline does not implement its runtime interface")
    return pipe


def _build_hybrid_pipeline(
    request: GenerationRequest, cache_dir: str | None
) -> H3Pipeline:
    """Keep the denoiser on GPU and stream conditioner blocks directly from disk."""
    import torch
    from diffusers import MiniMaxH3Transformer3DModel, ModularPipeline
    from diffusers.modular_pipelines.components_manager import ComponentsManager

    manager = ComponentsManager()
    pipe = ModularPipeline.from_pretrained(
        request.model,
        workflow=request.workflow,
        components_manager=manager,
        cache_dir=cache_dir,
    )
    if pipe is None:
        raise RuntimeError(f"Could not load MiniMax H3 pipeline {request.model}")
    if not isinstance(pipe, H3Pipeline):
        raise TypeError("MiniMax H3 pipeline does not implement its runtime interface")
    # Decode runs in a separate process from a durable latent checkpoint. Prune
    # it before workflow-filtered loading so t2va never maps the 9.8 GB video VAE.
    decode_blocks = _pop_decode_blocks(pipe._blocks)
    required_names = list(
        dict.fromkeys(
            spec.name
            for spec in pipe._blocks.expected_components
            if spec.name is not None
        )
    )
    pipe.load_components(names=required_names, dtype=torch.bfloat16)
    _restore_decode_blocks(pipe._blocks, decode_blocks)

    text_encoder = pipe.text_encoder
    transformer_name = "transformer_ref" if request.workflow == "ref2va" else "transformer"
    transformer = getattr(pipe, transformer_name)
    if not isinstance(transformer, MiniMaxH3Transformer3DModel):
        raise TypeError("MiniMax H3 pipeline has an invalid transformer")
    _apply_lora(transformer, request)
    if request.workflow != "t2va":
        # Keyframe/reference encoding needs the convolutional VAE encoder but
        # not the much larger ViT decoder. Decode happens later from the durable
        # checkpoint, so discard those unused weights before component offload.
        pipe.vae.decoder.to_empty(device="cpu")
        pipe.vae.post_quant_conv.to_empty(device="cpu")
    revision = getattr(text_encoder.config, "_commit_hash", None) or "current"
    cache_root = env.minimax_h3_disk_offload_cache_root() / revision / "text_encoder"

    # The language stack dominates the 62 GB conditioner. Each decoder layer is
    # saved independently, read directly to CUDA, executed, then replaced by an
    # empty CPU tensor. Qwen's multimodal wrapper calls `embed_tokens` directly,
    # before entering `language_model.forward`, so that layer needs its own hook.
    # The vision tower is only needed by conditioned workflows.
    print(f"Preparing disk-backed conditioner groups in {cache_root}", flush=True)
    language_model = text_encoder.model.language_model
    embed_tokens = getattr(language_model, "embed_tokens", None)
    if not isinstance(embed_tokens, torch.nn.Module):
        raise TypeError("MiniMax H3 text encoder has invalid token embeddings")
    _enable_disk_groups(embed_tokens, cache_root / "embed_tokens")
    _enable_disk_groups(language_model, cache_root / "language_model")
    if request.workflow != "t2va":
        _enable_disk_groups(text_encoder.model.visual, cache_root / "visual")

    manager.enable_auto_cpu_offload(
        device="cuda",
        offload_strategy=_OffloadEveryResidentComponent(),
    )
    # Once denoising ends, decoding is the only remaining stage. Dropping the
    # transformer's GPU storage avoids an impossible 62 GB CUDA-to-CPU copy.
    _make_transformer_offload_destructive(manager, transformer)
    return pipe


def _replace_blocks(pipe: H3Pipeline, names: tuple[str, ...]) -> None:
    """Restrict a modular pipeline to named top-level stages, preserving order."""
    from diffusers.modular_pipelines.modular_pipeline import SequentialPipelineBlocks

    selected: dict[str, ModularPipelineBlocks] = {}
    for requested in names:
        if requested in pipe._blocks.sub_blocks:
            selected[requested] = pipe._blocks.sub_blocks[requested]
            continue
        prefix = requested + "."
        for actual, block in pipe._blocks.sub_blocks.items():
            if actual.startswith(prefix):
                selected[actual.removeprefix(prefix)] = block
    if not selected:
        raise KeyError(f"none of the requested pipeline stages exist: {names}")
    pipe._blocks = SequentialPipelineBlocks.from_blocks_dict(selected)


def _build_fl2va_conditioning_pipeline(
    request: GenerationRequest, cache_dir: str | None
) -> H3Pipeline:
    """Load only media/text/VAE encoders; the transformer is deliberately absent."""
    import torch
    from diffusers import ModularPipeline
    from diffusers.modular_pipelines.components_manager import ComponentsManager

    manager = ComponentsManager()
    pipe = ModularPipeline.from_pretrained(
        request.model,
        workflow="fl2va",
        components_manager=manager,
        cache_dir=cache_dir,
    )
    if pipe is None:
        raise RuntimeError(f"Could not load MiniMax H3 pipeline {request.model}")
    if not isinstance(pipe, H3Pipeline):
        raise TypeError("MiniMax H3 pipeline does not implement its runtime interface")
    _replace_blocks(pipe, ("before_encode", "text_encoder", "vae_encoder"))
    required_names = list(
        dict.fromkeys(
            spec.name
            for spec in pipe._blocks.expected_components
            if spec.name is not None
        )
    )
    pipe.load_components(names=required_names, dtype=torch.bfloat16)

    # Conditioning uses only the convolutional encoder half of the video VAE.
    pipe.vae.decoder.to_empty(device="cpu")
    pipe.vae.post_quant_conv.to_empty(device="cpu")
    text_encoder = pipe.text_encoder
    revision = getattr(text_encoder.config, "_commit_hash", None) or "current"
    cache_root = env.minimax_h3_disk_offload_cache_root() / revision / "text_encoder"
    print(f"Preparing disk-backed conditioner groups in {cache_root}", flush=True)
    language_model = text_encoder.model.language_model
    embed_tokens = getattr(language_model, "embed_tokens", None)
    if not isinstance(embed_tokens, torch.nn.Module):
        raise TypeError("MiniMax H3 text encoder has invalid token embeddings")
    _enable_disk_groups(embed_tokens, cache_root / "embed_tokens")
    _enable_disk_groups(language_model, cache_root / "language_model")
    _enable_disk_groups(text_encoder.model.visual, cache_root / "visual")
    manager.enable_auto_cpu_offload(device="cuda", offload_strategy=_OffloadEveryResidentComponent())
    return pipe


def _run_fl2va_conditioning(
    pipe: H3Pipeline, kwargs: GenerationArguments
) -> ConditioningState:
    output_names = [
        "prompt_embeds",
        "text_token_tags",
        "condition_latents",
        "height",
        "width",
        "num_frames",
        "keyframe_anchors",
    ]
    values = pipe(**kwargs, output=output_names)
    prompt_embeds = values["prompt_embeds"]
    text_token_tags = values["text_token_tags"]
    raw_condition_latents = values["condition_latents"]
    height = values["height"]
    width = values["width"]
    num_frames = kwargs["num_frames"]
    raw_anchors = values["keyframe_anchors"]
    if not isinstance(prompt_embeds, torch.Tensor):
        raise TypeError("MiniMax H3 conditioning returned invalid prompt embeddings")
    if not isinstance(text_token_tags, torch.Tensor):
        raise TypeError("MiniMax H3 conditioning returned invalid text token tags")
    if not isinstance(raw_condition_latents, list):
        raise TypeError("MiniMax H3 conditioning returned invalid condition latents")
    condition_latents: list[torch.Tensor] = []
    for latent in raw_condition_latents:
        if not isinstance(latent, torch.Tensor):
            raise TypeError("MiniMax H3 conditioning returned invalid condition latents")
        condition_latents.append(latent)
    if not isinstance(height, int) or not isinstance(width, int):
        raise TypeError("MiniMax H3 conditioning returned an invalid canvas")
    if not isinstance(num_frames, int):
        raise TypeError("MiniMax H3 generation arguments have an invalid frame count")
    if not isinstance(raw_anchors, (list, tuple)):
        raise TypeError("MiniMax H3 conditioning returned invalid keyframe anchors")
    anchors: list[str] = []
    for anchor in raw_anchors:
        if not isinstance(anchor, str):
            raise TypeError("MiniMax H3 conditioning returned invalid keyframe anchors")
        anchors.append(anchor)
    return {
        "prompt_embeds": prompt_embeds,
        "text_token_tags": text_token_tags,
        "condition_latents": condition_latents,
        "height": height,
        "width": width,
        "num_frames": num_frames,
        "keyframe_anchors": tuple(anchors),
    }


def _release_pipeline_destructively(pipe: H3Pipeline) -> None:
    """Free CUDA storage without copying large model weights into constrained RAM."""
    import torch

    for component in pipe.components.values():
        if isinstance(component, torch.nn.Module):
            try:
                component.to_empty(device="cpu")
            except (AttributeError, RuntimeError):
                pass
    del pipe
    gc.collect()
    torch.cuda.empty_cache()


def _build_fl2va_denoise_pipeline(
    request: GenerationRequest, cache_dir: str | None
) -> tuple[H3Pipeline, str]:
    """Load the denoiser only, after conditioning is safely on disk."""
    import torch
    from diffusers import MiniMaxH3Transformer3DModel, ModularPipeline
    from diffusers.modular_pipelines.components_manager import ComponentsManager

    manager = ComponentsManager()
    pipe = ModularPipeline.from_pretrained(
        request.model,
        workflow="fl2va",
        components_manager=manager,
        cache_dir=cache_dir,
    )
    if pipe is None:
        raise RuntimeError(f"Could not load MiniMax H3 pipeline {request.model}")
    if not isinstance(pipe, H3Pipeline):
        raise TypeError("MiniMax H3 pipeline does not implement its runtime interface")
    _replace_blocks(pipe, ("denoise",))
    required_names = list(
        dict.fromkeys(
            spec.name
            for spec in pipe._blocks.expected_components
            if spec.name is not None
        )
    )
    pipe.load_components(names=required_names, dtype=torch.bfloat16)
    transformer = pipe.transformer
    if not isinstance(transformer, MiniMaxH3Transformer3DModel):
        raise TypeError("MiniMax H3 pipeline has an invalid transformer")
    _apply_lora(transformer, request)
    manager.enable_auto_cpu_offload(device="cuda", offload_strategy=_OffloadEveryResidentComponent())
    _make_transformer_offload_destructive(manager, transformer)
    return pipe, _configure_attention(pipe, request.attention)


def _run_fl2va_denoise(
    pipe: H3Pipeline,
    conditioned: ConditioningState,
    request: GenerationRequest,
) -> tuple[torch.Tensor, torch.Tensor]:
    import torch
    from diffusers.modular_pipelines.modular_pipeline import PipelineState

    state = PipelineState()
    for name, value in conditioned.items():
        state.set(name, value)
    # The conditional wrapper uses this only to select its FL2VA branch. The
    # resized source pixels are no longer needed once condition_latents exists.
    state.set("image", True)
    state.set("generator", torch.Generator(device="cpu").manual_seed(request.seed))
    state.set("num_inference_steps", request.steps)
    results = pipe(state=state, output=["latents", "audio_latents"])
    latents = results["latents"]
    audio_latents = results["audio_latents"]
    if not isinstance(latents, torch.Tensor) or not isinstance(
        audio_latents, torch.Tensor
    ):
        raise TypeError("MiniMax H3 denoiser returned invalid latents")
    return latents, audio_latents


def _run_to_latents(
    pipe: H3Pipeline, kwargs: GenerationArguments
) -> tuple[torch.Tensor, torch.Tensor]:
    """Run every generation stage except decoding and return the final latents."""
    decode_blocks = _pop_decode_blocks(pipe._blocks)
    try:
        results = pipe(**kwargs, output=["latents", "audio_latents"])
    finally:
        _restore_decode_blocks(pipe._blocks, decode_blocks)
    latents = results["latents"]
    audio_latents = results["audio_latents"]
    if not isinstance(latents, torch.Tensor) or not isinstance(
        audio_latents, torch.Tensor
    ):
        raise TypeError("MiniMax H3 pipeline returned invalid latents")
    return latents, audio_latents


def _release_generation_pipeline(pipe: H3Pipeline, workflow: str) -> None:
    import torch

    transformer_name = "transformer_ref" if workflow == "ref2va" else "transformer"
    transformer = getattr(pipe, transformer_name, None)
    if not isinstance(transformer, torch.nn.Module):
        raise TypeError("MiniMax H3 pipeline has an invalid transformer")
    first_parameter = next(transformer.parameters(), None)
    if first_parameter is not None and first_parameter.device.type == "cuda":
        transformer.to_empty(device="cpu")
    del pipe
    gc.collect()
    torch.cuda.empty_cache()


def _build_decode_pipeline(
    model: str, workflow: str, cache_dir: str | None
) -> H3Pipeline:
    import torch
    from diffusers import ModularPipeline
    from diffusers.modular_pipelines.modular_pipeline import SequentialPipelineBlocks

    pipe = ModularPipeline.from_pretrained(model, workflow=workflow, cache_dir=cache_dir)
    if pipe is None:
        raise RuntimeError(f"Could not load MiniMax H3 pipeline {model}")
    if not isinstance(pipe, H3Pipeline):
        raise TypeError("MiniMax H3 pipeline does not implement its runtime interface")
    decode_blocks = _pop_decode_blocks(pipe._blocks)
    if not decode_blocks:
        raise RuntimeError("MiniMax H3 workflow has no decode blocks")
    pipe._blocks = SequentialPipelineBlocks.from_blocks_dict(
        {name.removeprefix("decode."): block for name, block in decode_blocks}
    )
    pipe.load_components(
        names=["vae", "audio_vae", "video_processor"],
        dtype={"vae": torch.float32, "audio_vae": torch.float32},
    )

    # Encoding modules are not used when resuming denoised latents.
    pipe.vae.encoder.to_empty(device="cpu")
    pipe.vae.quant_conv.to_empty(device="cpu")
    pipe.vae.post_quant_conv.to("cuda")
    decode_offload = env.minimax_h3_decode_offload()
    free_bytes, _ = torch.cuda.mem_get_info()
    if decode_offload == "auto":
        # The fp32 decoder occupies about 9.1 GiB. Keeping it resident avoids
        # rereading every layer for every native spatial tile and is safe on an
        # otherwise-empty 80 GiB accelerator. Loading is incremental, so it
        # does not require the whole decoder to fit in host RAM at once.
        decode_offload = "gpu" if free_bytes >= 32 * 2**30 else "disk"
    if decode_offload == "gpu":
        print("Loading the video decoder onto the GPU", flush=True)
        pipe.vae.decoder.to("cuda")
    else:
        revision = getattr(pipe.vae.config, "_commit_hash", None) or "current"
        cache_root = env.minimax_h3_disk_offload_cache_root() / revision / "decode"
        print(f"Preparing disk-backed video decoder groups in {cache_root}", flush=True)
        _enable_disk_groups(pipe.vae.decoder, cache_root / "video_vae")
    pipe.audio_vae.to("cuda")
    if decode_offload == "gpu":
        # ModularPipeline infers its execution device from the first parameter
        # of its first component. The unused encoder remains on CPU, so place
        # only that small sentinel parameter on CUDA; otherwise the decode
        # block constructs normalization constants on CPU while latents are on
        # CUDA. The encoder is never called in this decode-only pipeline.
        first_encoder_parameter = next(pipe.vae.encoder.parameters())
        first_encoder_parameter.data = torch.empty_like(first_encoder_parameter, device="cuda")
    return pipe


def decode_checkpoint(
    checkpoint: Path,
    output: Path,
    model: str = MODEL_ID,
    progress: Callable[[str, WorkerState], None] | None = None,
) -> GenerationOutput:
    import numpy as np
    import torch
    from diffusers.modular_pipelines.modular_pipeline import PipelineState
    from diffusers.utils.export_utils import encode_video

    started = time.monotonic()
    latents, audio_latents, manifest = _load_latent_checkpoint(checkpoint)
    workflow = manifest.workflow
    cache_dir = env.minimax_h3_cache_directory()
    _write_status(checkpoint, "decoding", output=str(output))
    if progress is not None:
        progress("Decoding and muxing audio-video…", "decoding")
    pipe = _build_decode_pipeline(model, workflow, cache_dir)
    state = PipelineState()
    state.set("latents", latents.to("cuda"))
    state.set("audio_latents", audio_latents.to("cuda"))
    # Diffusers' PyAV exporter converts float [0, 1] NumPy frames to uint8;
    # its tensor path expects tensors to be uint8 already.
    state.set("output_type", "np")
    results = pipe(state=state, output=["videos", "audio", "sampling_rate"])
    raw_videos = results["videos"]
    raw_audio = results["audio"]
    sampling_rate = results["sampling_rate"]
    if (
        not isinstance(raw_videos, np.ndarray)
        or raw_videos.ndim != 5
        or raw_videos.shape[0] != 1
    ):
        raise TypeError("MiniMax H3 decoder returned invalid video frames")
    if (
        not isinstance(raw_audio, torch.Tensor)
        or raw_audio.ndim != 3
        or raw_audio.shape[0] != 1
    ):
        raise TypeError("MiniMax H3 decoder returned invalid audio")
    if not isinstance(sampling_rate, int):
        raise TypeError("MiniMax H3 decoder returned an invalid sample rate")

    output.parent.mkdir(parents=True, exist_ok=True)
    partial = output.with_name(f".{output.stem}.partial{output.suffix}")
    encode_video(
        raw_videos[0],
        fps=24,
        output_path=str(partial),
        audio=raw_audio[0],
        audio_sample_rate=sampling_rate,
    )
    os.replace(partial, output)
    elapsed = time.monotonic() - started
    _write_status(checkpoint, "complete", decode_elapsed_seconds=elapsed)
    return {
        "output": str(output),
        "checkpoint": str(checkpoint),
        "sampling_rate": sampling_rate,
        "decode_elapsed_seconds": elapsed,
        "peak_gpu_memory_gib": torch.cuda.max_memory_allocated() / 2**30,
    }


def build_pipeline(request: GenerationRequest) -> tuple[H3Pipeline, str, str]:
    import torch
    from diffusers import MiniMaxH3Transformer3DModel, ModularPipeline
    from diffusers.modular_pipelines.components_manager import ComponentsManager

    cache_dir = env.minimax_h3_cache_directory()
    free_bytes, _ = torch.cuda.mem_get_info()
    memory = request.memory
    if memory == "auto":
        memory = "hybrid" if free_bytes >= 70 * 2**30 else "stream"
    if memory == "int8":
        pipe = _build_int8_pipeline(request, cache_dir)
    elif memory == "hybrid":
        pipe = _build_hybrid_pipeline(request, cache_dir)
    elif memory == "stream":
        pipe = _build_streamed_bf16_pipeline(request, cache_dir)
    else:
        manager = ComponentsManager()
        pipe = ModularPipeline.from_pretrained(
            request.model,
            workflow=request.workflow,
            components_manager=manager,
            cache_dir=cache_dir,
        )
        if pipe is None:
            raise RuntimeError(f"Could not load MiniMax H3 pipeline {request.model}")
        if not isinstance(pipe, H3Pipeline):
            raise TypeError("MiniMax H3 pipeline does not implement its runtime interface")
        pipe.load_components(dtype=torch.bfloat16)
        transformer_name = "transformer_ref" if request.workflow == "ref2va" else "transformer"
        transformer = getattr(pipe, transformer_name)
        if not isinstance(transformer, MiniMaxH3Transformer3DModel):
            raise TypeError("MiniMax H3 pipeline has an invalid transformer")
        _apply_lora(transformer, request)
        manager.enable_auto_cpu_offload(device="cuda", memory_reserve_margin="12GB")
    backend = _configure_attention(pipe, request.attention)
    return pipe, backend, memory


def generate(
    request: GenerationRequest,
    progress: Callable[[str, WorkerState], None] | None = None,
    decode_output: bool = True,
) -> GenerationOutput:
    import torch
    from api.media import load_image

    request.validate()
    request.output.parent.mkdir(parents=True, exist_ok=True)
    started = time.monotonic()
    torch.cuda.reset_peak_memory_stats()
    checkpoint = request.checkpoint or _default_checkpoint_path(request.output)
    conditioning_checkpoint = _conditioning_checkpoint_path(checkpoint)
    num_frames = align_num_frames(request.duration)
    if checkpoint.exists() and not request.restart:
        _load_latent_checkpoint(checkpoint, expected_request=request)
        if not decode_output:
            return {"checkpoint": str(checkpoint), "frames": num_frames, "resumed": True}
        print(f"Resuming decode from {checkpoint}", flush=True)
        decoded = decode_checkpoint(checkpoint, request.output, request.model, progress)
        decoded.update(
            {
                "frames": num_frames,
                "resumed": True,
                "elapsed_seconds": time.monotonic() - started,
            }
        )
        return decoded

    kwargs: GenerationArguments = {
        "prompt": request.prompt,
        "height": request.height,
        "width": request.width,
        "num_frames": num_frames,
        "num_inference_steps": request.steps,
        "generator": torch.Generator(device="cpu").manual_seed(request.seed),
    }
    if request.workflow == "fl2va":
        if request.image:
            kwargs["image"] = load_image(request.image)
        if request.last_image:
            kwargs["last_image"] = load_image(request.last_image)
    elif request.workflow == "ref2va":
        kwargs["references"] = _build_references(request.references)

    cache_dir = env.minimax_h3_cache_directory()
    if request.workflow == "fl2va" and request.memory in {"auto", "hybrid"}:
        memory = "hybrid-split"
        if conditioning_checkpoint.exists() and not request.restart:
            if progress is not None:
                progress("Loading cached keyframe conditioning…", "conditioning")
            print(f"Resuming denoise from {conditioning_checkpoint}", flush=True)
            conditioned = _load_conditioning_checkpoint(conditioning_checkpoint, request)
        else:
            if progress is not None:
                progress("Encoding prompt and keyframes…", "conditioning")
            _write_status(checkpoint, "conditioning", output=str(request.output))
            condition_pipe = _build_fl2va_conditioning_pipeline(request, cache_dir)
            conditioned = _run_fl2va_conditioning(condition_pipe, kwargs)
            _save_conditioning_checkpoint(conditioning_checkpoint, request, conditioned)
            print(f"Saved resumable conditioning checkpoint to {conditioning_checkpoint}", flush=True)
            _release_pipeline_destructively(condition_pipe)
        _write_status(checkpoint, "denoising", conditioning_checkpoint=str(conditioning_checkpoint))
        if progress is not None:
            progress("Loading MiniMax H3 denoiser…", "loading")
        pipe, backend = _build_fl2va_denoise_pipeline(request, cache_dir)
        if progress is not None:
            progress("Generating audio-video latents…", "generating")
        latents, audio_latents = _run_fl2va_denoise(pipe, conditioned, request)
    else:
        if progress is not None:
            progress("Loading MiniMax H3 pipeline…", "loading")
        _write_status(checkpoint, "generating", output=str(request.output))
        pipe, backend, memory = build_pipeline(request)
        if progress is not None:
            progress("Generating audio-video latents…", "generating")
        latents, audio_latents = _run_to_latents(pipe, kwargs)
    _save_latent_checkpoint(checkpoint, request, latents, audio_latents)
    generation_elapsed = time.monotonic() - started
    generation_peak = torch.cuda.max_memory_allocated() / 2**30
    _release_generation_pipeline(pipe, request.workflow)
    if not decode_output:
        return {
            "checkpoint": str(checkpoint),
            "frames": num_frames,
            "duration": Fraction(num_frames, 24),
            "attention": backend,
            "memory": memory,
            "resumed": False,
            "generation_elapsed_seconds": generation_elapsed,
            "generation_peak_gpu_memory_gib": generation_peak,
        }
    decoded = decode_checkpoint(checkpoint, request.output, request.model, progress)
    return {
        **decoded,
        "frames": num_frames,
        "duration": Fraction(num_frames, 24),
        "attention": backend,
        "memory": memory,
        "resumed": False,
        "generation_elapsed_seconds": generation_elapsed,
        "generation_peak_gpu_memory_gib": generation_peak,
        "elapsed_seconds": time.monotonic() - started,
    }
