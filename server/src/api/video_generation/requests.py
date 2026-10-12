import hashlib
import os
from io import BytesIO
from pathlib import Path
from uuid import uuid4

import msgspec
import soundfile

import env

from api.media import PcmAudio, decode_audio

from api.video_generation.catalog import MODELS
from api.video_generation.minimax_h3.config import ReferenceSpec
from api.video_generation.minimax_h3.inference import GenerationRequest as H3Request
from api.video_generation.minimax_h3.lora import (
    DEFAULT_LORA_WEIGHT_NAME,
    SKETCH_LORA_ID,
)
from api.video_generation.wan.inference import (
    WAN21_MODEL_ID,
    WAN21_FRAME_RATE,
    WAN21_FRAMES,
    WAN22_MODEL_ID,
    WAN22_FRAME_RATE,
    WAN22_FRAMES,
    GenerationRequest as WanRequest,
)
from api.video_generation.protocol import (
    GenerationRequest,
    InputDefinition,
    InputValue,
    MediaInput,
    MediaValue,
    ModelInfo,
    NumberInput,
    NumberValue,
    SelectInput,
    SelectValue,
    TextInput,
    TextValue,
)

MAXIMUM_REQUEST_MEDIA_BYTES = 512 * 1024 * 1024


def model_info(request: GenerationRequest) -> ModelInfo:
    for model in MODELS:
        if model.id == request.model:
            return model
    raise ValueError(f"Unknown video-generation model {request.model}")


def selected_value(value: InputValue | None) -> str | None:
    return value.value if isinstance(value, SelectValue) else None


def is_visible(definition: InputDefinition, inputs: dict[str, InputValue]) -> bool:
    condition = definition.visible_when
    return condition is None or selected_value(inputs.get(condition.input)) in condition.values


def validate_value(definition: InputDefinition, value: InputValue) -> None:
    match definition, value:
        case TextInput(), TextValue():
            if len(value.value) > definition.max_length:
                raise ValueError(f"Input {definition.key} is too long")
            if definition.required and not value.value.strip():
                raise ValueError(f"Input {definition.key} is required")
        case SelectInput(), SelectValue():
            if value.value not in {option.value for option in definition.options}:
                raise ValueError(f"Input {definition.key} has an invalid selection")
        case NumberInput(), NumberValue():
            number = value.value.fraction()
            if not definition.minimum.fraction() <= number <= definition.maximum.fraction():
                raise ValueError(f"Input {definition.key} is outside its range")
            step = definition.step.fraction()
            if (number - definition.minimum.fraction()) % step:
                raise ValueError(f"Input {definition.key} does not align to its step")
        case MediaInput(), MediaValue():
            if not definition.minimum_items <= len(value.items) <= definition.maximum_items:
                raise ValueError(f"Input {definition.key} has an invalid number of files")
            accepted = set(definition.accepted)
            if any(item.kind not in accepted for item in value.items):
                raise ValueError(f"Input {definition.key} contains an unsupported media kind")
        case _:
            raise ValueError(f"Input {definition.key} has the wrong value type")


def validate_inputs(request: GenerationRequest) -> None:
    definitions = {value.key: value for value in model_info(request).inputs}
    active = {
        key
        for key, definition in definitions.items()
        if is_visible(definition, request.inputs)
    }
    if unknown := request.inputs.keys() - active:
        raise ValueError(f"Unknown or inactive inputs: {sorted(unknown)}")
    if missing := active - request.inputs.keys():
        raise ValueError(f"Missing inputs: {sorted(missing)}")
    for key, value in request.inputs.items():
        validate_value(definitions[key], value)
    total_media = sum(
        len(item.data if isinstance(item.data, bytes) else item.data.data)
        for value in request.inputs.values()
        if isinstance(value, MediaValue)
        for item in value.items
    )
    if total_media > MAXIMUM_REQUEST_MEDIA_BYTES:
        raise ValueError("Video-generation media exceeds the 512 MiB request limit")


def text(request: GenerationRequest, key: str) -> str:
    value = request.inputs[key]
    if not isinstance(value, TextValue):
        raise ValueError(f"Input {key} must be text")
    return value.value


def selection(request: GenerationRequest, key: str) -> str:
    value = request.inputs[key]
    if not isinstance(value, SelectValue):
        raise ValueError(f"Input {key} must be a selection")
    return value.value


def number(request: GenerationRequest, key: str):
    value = request.inputs[key]
    if not isinstance(value, NumberValue):
        raise ValueError(f"Input {key} must be a number")
    return value.value.fraction()


def integer(request: GenerationRequest, key: str) -> int:
    value = number(request, key)
    if value.denominator != 1:
        raise ValueError(f"Input {key} must be an integer")
    return value.numerator


def media(request: GenerationRequest, key: str) -> MediaValue:
    value = request.inputs[key]
    if not isinstance(value, MediaValue):
        raise ValueError(f"Input {key} must contain media")
    return value


def request_digest(request: GenerationRequest) -> str:
    payload = request.model_dump(mode="python")
    for value in payload["inputs"].values():
        if value["kind"] != "media":
            continue
        for item in value["items"]:
            media_data = item["data"]
            data = media_data if isinstance(media_data, bytes) else media_data["data"]
            digest = {
                "sha256": hashlib.sha256(data).hexdigest(),
                "bytes": len(data),
            }
            if isinstance(media_data, bytes) or media_data["kind"] == "encoded":
                item["data"] = digest
            else:
                media_data["data"] = digest
    payload = msgspec.msgpack.encode(payload)
    return hashlib.sha256(payload).hexdigest()


def media_suffix(filename: str, kind: str) -> str:
    suffix = Path(filename).suffix.lower()
    if 1 < len(suffix) <= 10 and suffix[1:].isalnum():
        return suffix
    return {"image": ".png", "video": ".mp4", "audio": ".wav"}[kind]


def stage_media(request: GenerationRequest, directory: Path) -> dict[str, list[Path]]:
    staged: dict[str, list[Path]] = {}
    for key, value in request.inputs.items():
        if not isinstance(value, MediaValue):
            continue
        paths = []
        for index, item in enumerate(value.items):
            data = item.data if isinstance(item.data, bytes) else item.data.data
            digest = hashlib.sha256(data).hexdigest()
            suffix = media_suffix(item.filename, item.kind)
            if isinstance(item.data, PcmAudio):
                rate, samples = decode_audio(
                    item.data, layout="mono" if item.data.channels == 1 else "stereo"
                )
                wav = BytesIO()
                soundfile.write(wav, samples.T, rate, format="WAV", subtype="FLOAT")
                data = wav.getvalue()
                suffix = ".wav"
            path = directory / f"{key}-{index}-{digest}{suffix}"
            if not path.is_file() or path.stat().st_size != len(data):
                temporary = path.with_name(f".{path.name}.{uuid4()}.tmp")
                try:
                    temporary.write_bytes(data)
                    os.replace(temporary, path)
                finally:
                    temporary.unlink(missing_ok=True)
            paths.append(path)
        staged[key] = paths
    return staged


type PreparedRequest = H3Request | WanRequest


def parse_wan_request(request: GenerationRequest) -> WanRequest:
    directory = env.video_generation_cache_root() / request_digest(request)
    directory.mkdir(parents=True, exist_ok=True)
    resolution = selection(request, "resolution")
    width_text, height_text = resolution.split("x", maxsplit=1)
    workflow = "t2v" if request.model == WAN21_MODEL_ID else selection(request, "workflow")
    image = None
    if workflow == "i2v":
        staged = stage_media(request, directory)
        image = staged["first_frame"][0]
    frames, frame_rate = (
        (WAN21_FRAMES, WAN21_FRAME_RATE)
        if request.model == WAN21_MODEL_ID
        else (WAN22_FRAMES, WAN22_FRAME_RATE)
    )
    result = WanRequest(
        model=request.model,
        workflow=workflow,
        prompt=text(request, "prompt"),
        negative_prompt=text(request, "negative_prompt"),
        output=directory / "output.mp4",
        width=int(width_text),
        height=int(height_text),
        frames=frames,
        frame_rate=frame_rate,
        steps=integer(request, "steps"),
        guidance_scale=float(number(request, "guidance_scale")),
        seed=integer(request, "seed"),
        image=image,
    )
    return result


def parse_h3_request(request: GenerationRequest) -> H3Request:
    workflow = selection(request, "workflow")
    if request.model.endswith("Looping-Sketch-Anime") and workflow == "ref2va":
        raise ValueError("The Sketch LoRA does not support ref2va")
    resolution = selection(request, "resolution")
    if resolution == "custom":
        width, height = integer(request, "width"), integer(request, "height")
    else:
        width_text, height_text = resolution.split("x", maxsplit=1)
        width, height = int(width_text), int(height_text)

    directory = env.video_generation_cache_root() / request_digest(request)
    directory.mkdir(parents=True, exist_ok=True)
    staged = stage_media(request, directory)
    references = ()
    image = None
    last_image = None
    if workflow == "fl2va":
        first = media(request, "first_frame").items
        last = media(request, "last_frame").items
        if not first and not last:
            raise ValueError("fl2va requires a first frame, a last frame, or both")
        image = str(staged["first_frame"][0]) if first else None
        last_image = str(staged["last_frame"][0]) if last else None
    elif workflow == "ref2va":
        values = media(request, "references").items
        references = tuple(
            ReferenceSpec(value.kind, str(path))
            for value, path in zip(values, staged["references"], strict=True)
        )

    sketch = request.model.endswith("Looping-Sketch-Anime")
    quantization = selection(request, "quantization")
    memory = (
        "int8"
        if quantization == "int8"
        else {"normal": "auto", "low_vram": "stream"}[selection(request, "memory")]
    )
    result = H3Request(
        workflow=workflow,
        prompt=text(request, "prompt"),
        output=directory / "output.mp4",
        duration=number(request, "duration"),
        width=width,
        height=height,
        steps=integer(request, "steps"),
        seed=integer(request, "seed"),
        attention=selection(request, "attention"),
        memory=memory,
        checkpoint=directory / "latents.safetensors",
        lora=SKETCH_LORA_ID if sketch else None,
        lora_weight_name=DEFAULT_LORA_WEIGHT_NAME if sketch else None,
        lora_scale=float(number(request, "lora_scale")) if sketch else 1.0,
        image=image,
        last_image=last_image,
        references=references,
    )
    result.validate()
    return result


def prepare_request(request: GenerationRequest) -> PreparedRequest:
    validate_inputs(request)
    if request.model in {WAN21_MODEL_ID, WAN22_MODEL_ID}:
        return parse_wan_request(request)
    return parse_h3_request(request)


def parse_request(request: GenerationRequest) -> H3Request:
    validate_inputs(request)
    if request.model in {WAN21_MODEL_ID, WAN22_MODEL_ID}:
        raise ValueError("parse_request only accepts MiniMax H3 models")
    return parse_h3_request(request)
