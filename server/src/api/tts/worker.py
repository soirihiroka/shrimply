from collections import OrderedDict
from hashlib import sha256
import logging
import os
from io import BytesIO
from multiprocessing.connection import Connection
from pathlib import Path
import warnings

import msgspec
import numpy as np
from numpy.typing import NDArray
import soundfile
import torch
from huggingface_hub import snapshot_download
from huggingface_hub.constants import HF_HUB_CACHE

from api.model_progress import StreamedModelProgress, stream_model_progress
from api.media import PcmAudio
from api.tts.errors import log_failure, public_error_message
from api.tts.index_tts_2_0.audio_features import decode_audio
from api.tts.index_tts_2_0.inference import synthesize as synthesize_index
from api.tts.index_tts_2_0.model_loader import (
    IndexModels,
    download_model_paths,
    load_models,
)
from api.tts.index_tts_2_5.inference import synthesize as synthesize_index_2_5
from api.tts.index_tts_2_5.model_loader import (
    IndexTts25Models,
    download_model_paths as download_model_paths_2_5,
    load_models as load_models_2_5,
)

from api.tts.qwen import Qwen3TTSModel, VoiceClonePromptItem, prepare_qwen_model
from api.tts.protocol import (
    ErrorEvent,
    ModelId,
    Precision,
    ProgressEvent,
    Rational,
    ResultEvent,
    Speech,
    WorkerEvent,
    SYNTHESIS_REQUEST_VALIDATOR,
)
from api.tts.requests import (
    IndexTts2Request,
    QwenCustomVoiceRequest,
    QwenVoiceCloneRequest,
    QwenVoiceDesignRequest,
    parse_request,
    request_precision,
)

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s %(levelname)s %(name)s: %(message)s",
)
logger = logging.getLogger("shrimply.tts_worker")
response_connection: Connection | None = None
VOICE_CLONE_PROMPT_CACHE_ENTRIES = 8
type VoiceClonePromptCacheKey = tuple[bytes, int | None, int | None, str | None, bool]
type VoiceClonePromptCache = OrderedDict[
    VoiceClonePromptCacheKey, list[VoiceClonePromptItem]
]


def send_worker_event(event: WorkerEvent) -> None:
    if response_connection is None:
        raise RuntimeError("Text-to-speech worker connection is unavailable")
    response_connection.send_bytes(
        msgspec.msgpack.encode(event.model_dump(mode="python"))
    )


def synthesize_qwen(
    model: Qwen3TTSModel,
    request: QwenCustomVoiceRequest | QwenVoiceCloneRequest | QwenVoiceDesignRequest,
    voice_clone_prompts: VoiceClonePromptCache,
) -> Speech:
    with warnings.catch_warnings():
        warnings.filterwarnings(
            "ignore",
            message=r"Passing `repetition_penalty` with `inputs_embeds`.*",
            category=UserWarning,
            module="transformers.generation.utils",
        )
        match request:
            case QwenCustomVoiceRequest():
                wavs, sample_rate = model.generate_custom_voice(
                    text=request.text,
                    language=request.language,
                    speaker=request.speaker,
                    instruct=request.instruction,
                )
            case QwenVoiceCloneRequest():
                cache_key = (
                    sha256(request.reference_audio.data).digest(),
                    request.reference_audio.sample_rate
                    if isinstance(request.reference_audio, PcmAudio) else None,
                    request.reference_audio.channels
                    if isinstance(request.reference_audio, PcmAudio) else None,
                    request.reference_text,
                    request.audio_only,
                )
                voice_clone_prompt = voice_clone_prompts.pop(cache_key, None)
                if voice_clone_prompt is None:
                    reference = decode_audio(request.reference_audio)
                    voice_clone_prompt = model.create_voice_clone_prompt(
                        ref_audio=(
                            reference.samples.squeeze(0).numpy(),
                            reference.sample_rate,
                        ),
                        ref_text=request.reference_text,
                        x_vector_only_mode=request.audio_only,
                    )
                    if len(voice_clone_prompts) == VOICE_CLONE_PROMPT_CACHE_ENTRIES:
                        voice_clone_prompts.popitem(last=False)
                else:
                    logger.info("Reusing cached Qwen voice clone prompt")
                voice_clone_prompts[cache_key] = voice_clone_prompt
                wavs, sample_rate = model.generate_voice_clone(
                    text=request.text,
                    language=request.language,
                    voice_clone_prompt=voice_clone_prompt,
                )
            case QwenVoiceDesignRequest():
                wavs, sample_rate = model.generate_voice_design(
                    text=request.text,
                    language=request.language,
                    instruct=request.instruction,
                )
    if len(wavs) != 1:
        raise ValueError(f"Qwen returned {len(wavs)} audio results instead of one")
    return speech_from_samples(np.asarray(wavs[0], dtype=np.float32), sample_rate)


def synthesize_indextts(models: IndexModels, request: IndexTts2Request) -> Speech:
    def report(value: float, message: str) -> None:
        del value
        send_worker_event(
            ProgressEvent(
                message=message,
                model=request.model,
                state="generating",
            )
        )

    output = synthesize_index(models, request, report)
    return speech_from_samples(
        output.waveform.squeeze(0).contiguous().float().numpy(),
        output.sample_rate,
        speed_factor=Rational(
            numerator=output.speed_factor.numerator,
            denominator=output.speed_factor.denominator,
        ),
    )


def synthesize_indextts_2_5(
    models: IndexTts25Models, request: IndexTts2Request
) -> Speech:
    def report(value: float, message: str) -> None:
        del value
        send_worker_event(
            ProgressEvent(
                message=message,
                model="IndexTeam/IndexTTS-2.5",
                state="generating",
            )
        )

    output = synthesize_index_2_5(models, request, report)
    return speech_from_samples(
        output.waveform.squeeze(0).contiguous().float().numpy(),
        output.sample_rate,
        speed_factor=Rational(
            numerator=output.speed_factor.numerator,
            denominator=output.speed_factor.denominator,
        ),
    )


def speech_from_samples(
    samples: NDArray[np.float32],
    sample_rate: int,
    speed_factor: Rational | None = None,
) -> Speech:
    if samples.ndim != 1 or samples.size == 0:
        raise ValueError("The TTS model returned invalid mono audio")
    if not np.isfinite(samples).all():
        raise ValueError("The TTS model returned non-finite audio")
    wav = BytesIO()
    soundfile.write(wav, samples, sample_rate, format="WAV", subtype="FLOAT")
    return Speech(
        wav=wav.getvalue(),
        speed_factor=speed_factor or Rational(numerator=1, denominator=1),
    )


def model_dtype(
    device: torch.device, model_id: ModelId, precision: Precision
) -> torch.dtype:
    match precision:
        case "auto":
            if device.type != "cuda" or model_id == "IndexTeam/IndexTTS-2":
                return torch.float32
            with torch.cuda.device(device):
                return (
                    torch.bfloat16 if torch.cuda.is_bf16_supported() else torch.float32
                )
        case "bfloat16":
            return torch.bfloat16
        case "float16":
            return torch.float16
        case "float32":
            return torch.float32


def run(
    connection: Connection,
    model_id: ModelId,
    device_name: str,
    precision: Precision,
) -> None:
    global response_connection
    response_connection = connection
    device = torch.device(device_name)
    logger.info(
        "Worker pid=%d initializing model=%s device=%s precision=%s",
        os.getpid(),
        model_id,
        device,
        precision,
    )
    try:
        send_worker_event(
            ProgressEvent(message="Checking model...", model=model_id, state="loading")
        )
        qwen_model: Qwen3TTSModel | None = None
        voice_clone_prompts: VoiceClonePromptCache = OrderedDict()
        index_models: IndexModels | None = None
        index_2_5_models: IndexTts25Models | None = None
        dtype = model_dtype(device, model_id, precision)
        def model_progress(message: str) -> None:
            send_worker_event(
                ProgressEvent(message=message, model=model_id, state="loading")
            )

        with stream_model_progress(model_progress):
            if model_id == "IndexTeam/IndexTTS-2":
                paths = download_model_paths(Path(HF_HUB_CACHE))
                model_progress("Loading weights...")
                index_models = load_models(paths, device, dtype)
            elif model_id == "IndexTeam/IndexTTS-2.5":
                paths_2_5 = download_model_paths_2_5(Path(HF_HUB_CACHE))
                model_progress("Loading weights...")
                index_2_5_models = load_models_2_5(paths_2_5, device, dtype)
            else:
                model_path = snapshot_download(
                    model_id,
                    ignore_patterns=["*.fp32.safetensors"],
                    max_workers=1,
                    tqdm_class=StreamedModelProgress,
                )
                model_progress("Loading weights...")
                qwen_model = Qwen3TTSModel.from_pretrained(
                    model_path,
                    device_map=device,
                    dtype=dtype,
                    attn_implementation="sdpa",
                )
                model_progress("Preparing model runtime...")
                prepare_qwen_model(qwen_model)
        send_worker_event(
            ProgressEvent(message="Model ready", model=model_id, state="ready")
        )
        logger.info("Model is ready model=%s dtype=%s", model_id, dtype)
    except Exception as exception:
        log_failure(logger, "Text-to-speech worker initialization failed", exception)
        message = (
            "Not enough GPU memory to load the text-to-speech model. "
            "Try a lower precision."
            if isinstance(exception, torch.OutOfMemoryError)
            else public_error_message(exception)
        )
        send_worker_event(
            ErrorEvent(
                code="worker_initialization_failed",
                message=message,
            )
        )
        connection.close()
        return
    while True:
        try:
            payload = connection.recv_bytes()
        except EOFError:
            break
        try:
            wire_request = SYNTHESIS_REQUEST_VALIDATOR.validate_python(
                msgspec.msgpack.decode(payload)
            )
            request = parse_request(wire_request)
            if request.model != model_id:
                raise ValueError(
                    f"Worker loaded {model_id}, but received request for {request.model}"
                )
            if request_precision(wire_request) != precision:
                raise ValueError(
                    f"Worker loaded {precision}, but received a different precision"
                )
            send_worker_event(
                ProgressEvent(
                    message="Generating...", model=model_id, state="generating"
                )
            )
            logger.info(
                "Generating speech model=%s text_length=%d", model_id, len(request.text)
            )
            if isinstance(request, IndexTts2Request):
                if request.model == "IndexTeam/IndexTTS-2.5":
                    if index_2_5_models is None:
                        raise RuntimeError("IndexTTS 2.5 worker did not load its model")
                    speech = synthesize_indextts_2_5(index_2_5_models, request)
                else:
                    if index_models is None:
                        raise RuntimeError("IndexTTS 2 worker did not load its model")
                    speech = synthesize_indextts(index_models, request)
            else:
                if qwen_model is None:
                    raise RuntimeError("Qwen worker did not load its model")
                speech = synthesize_qwen(qwen_model, request, voice_clone_prompts)
            if device.type == "cuda":
                with torch.cuda.device(device):
                    torch.cuda.empty_cache()
            send_worker_event(ResultEvent(result=speech))
            del payload, wire_request, request, speech
            logger.info("Speech generation completed model=%s", model_id)
        except Exception as exception:
            log_failure(logger, "Speech generation failed", exception)
            send_worker_event(
                ErrorEvent(
                    code="generation_failed",
                    message=public_error_message(exception),
                )
            )
            connection.close()
            return
    logger.info("Worker input closed; shutting down")
    connection.close()
