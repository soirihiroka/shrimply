import logging
import os
from collections.abc import Callable
from multiprocessing.connection import Connection

import numpy as np
import torch
from huggingface_hub import snapshot_download
from pydantic import BaseModel, ConfigDict
from tqdm.auto import tqdm
from transformers import ParakeetForTDT, ParakeetProcessor
from transformers.models.qwen3_asr.modeling_qwen3_asr import (
    Qwen3ASRForConditionalGeneration,
    Qwen3ASRForTokenClassification,
)
from transformers.models.qwen3_asr.processing_qwen3_asr import Qwen3ASRProcessor
from transformers.models.whisper.modeling_whisper import WhisperForConditionalGeneration
from transformers.models.whisper.processing_whisper import WhisperProcessor

from api.stt.protocol import (
    ErrorEvent,
    ModelId,
    ProgressEvent,
    ResultEvent,
    Segment,
    Transcription,
    WorkerEvent,
    encode_event,
    SAMPLE_RATE,
)

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s %(levelname)s %(name)s: %(message)s",
)
logger = logging.getLogger("shrimply.stt_worker")

QWEN_MODEL_ID: ModelId = "Qwen/Qwen3-ASR-0.6B-hf"
QWEN_ALIGNER_ID = "Qwen/Qwen3-ForcedAligner-0.6B-hf"
PARAKEET_MODEL_ID: ModelId = "nvidia/parakeet-tdt-0.6b-v3"
WHISPER_MODEL_IDS: tuple[ModelId, ...] = (
    "openai/whisper-large-v3-turbo",
    "openai/whisper-small",
    "distil-whisper/distil-large-v3",
)
MODEL_FILE_PATTERNS = (
    "*.json",
    "*.safetensors",
    "*.txt",
    "*.model",
    "*.jinja",
    "*.tiktoken",
)
QWEN_MAX_ALIGNMENT_SAMPLES = SAMPLE_RATE * 60 * 5
QWEN_MAX_NEW_TOKENS = 1024
MAX_CAPTION_SECONDS = 5
QWEN_ALIGNMENT_LANGUAGES = {
    "Chinese",
    "English",
    "Cantonese",
    "French",
    "German",
    "Italian",
    "Portuguese",
    "Russian",
    "Spanish",
}
response_connection: Connection | None = None
active_model_id: ModelId | None = None
Transcriber = Callable[[bytes], Transcription]


class Timestamp(BaseModel):
    token: str
    start: float
    end: float


class AlignedWord(BaseModel):
    text: str
    start_time: float
    end_time: float


class QwenTranscription(BaseModel):
    language: str | None
    transcription: str


class WhisperSegment(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)

    start: torch.Tensor
    end: torch.Tensor
    tokens: torch.Tensor


class WhisperGeneration(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)

    sequences: torch.Tensor
    segments: list[list[WhisperSegment]]


class DownloadProgress(tqdm):
    def display(self, msg: str | None = None, pos: int | None = None) -> None:
        if self.total and self.unit == "B":
            if active_model_id is None:
                raise RuntimeError("Speech-to-text model is unavailable")
            percent = round(self.n * 100 / self.total)
            send_worker_event(
                ProgressEvent(
                    message=f"Downloading · {percent}%",
                    model=active_model_id,
                    state="loading",
                )
            )


def send_worker_event(event: WorkerEvent) -> None:
    if response_connection is None:
        raise RuntimeError("Speech-to-text worker connection is unavailable")
    response_connection.send_bytes(encode_event(event))


def transcribe_parakeet(
    processor: ParakeetProcessor, model: ParakeetForTDT, payload: bytes
) -> Transcription:
    samples = audio_samples(payload)
    inputs = processor(samples, sampling_rate=SAMPLE_RATE)
    encoder_frames = model.encoder._get_subsampling_output_length(
        torch.tensor([inputs["input_features"].shape[1]], device=model.device)
    ).item()
    max_new_tokens = model.max_symbols_per_step * encoder_frames
    inputs.to(model.device, dtype=model.dtype)
    with torch.inference_mode():
        output = model.generate(
            **inputs,
            max_new_tokens=max_new_tokens,
            return_dict_in_generate=True,
        )
    decoded, timestamps = processor.decode(
        output.sequences,
        durations=output.durations,
        skip_special_tokens=True,
    )
    text = decoded[0] if isinstance(decoded, list) else decoded
    token_timestamps = (
        [Timestamp.model_validate(timestamp) for timestamp in timestamps[0]]
        if timestamps
        else []
    )
    return Transcription(
        segments=timestamped_segments(token_timestamps, text, samples.size)
    )


def transcribe_whisper(
    processor: WhisperProcessor,
    model: WhisperForConditionalGeneration,
    payload: bytes,
) -> Transcription:
    samples = audio_samples(payload)
    inputs = processor(
        samples,
        sampling_rate=SAMPLE_RATE,
        return_tensors="pt",
        truncation=False,
        padding="max_length",
        return_attention_mask=True,
    )
    inputs.to(model.device, dtype=model.dtype)
    with torch.inference_mode():
        generated = WhisperGeneration.model_validate(
            model.generate(**inputs, return_timestamps=True, return_segments=True),
        )
    segments = []
    for segment in generated.segments[0]:
        text = processor.decode(segment.tokens, skip_special_tokens=True).strip()
        if text:
            segments.append(
                Segment(
                    start_frame=round(float(segment.start) * SAMPLE_RATE),
                    end_frame=round(float(segment.end) * SAMPLE_RATE),
                    text=text,
                )
            )
    if not segments:
        text = processor.batch_decode(generated.sequences, skip_special_tokens=True)[
            0
        ].strip()
        if text:
            segments.append(Segment(start_frame=0, end_frame=samples.size, text=text))
    if segments:
        segments[0].start_frame = 0
        segments[-1].end_frame = samples.size
    return Transcription(segments=segments)


def transcribe_qwen(
    processor: Qwen3ASRProcessor,
    model: Qwen3ASRForConditionalGeneration,
    aligner_processor: Qwen3ASRProcessor,
    aligner: Qwen3ASRForTokenClassification,
    payload: bytes,
) -> Transcription:
    samples = audio_samples(payload)
    if samples.size > QWEN_MAX_ALIGNMENT_SAMPLES:
        raise ValueError("Qwen transcription chunks must be at most five minutes")
    inputs = processor.apply_transcription_request(samples)
    inputs.to(model.device, dtype=model.dtype)
    generate = getattr(model, "generate", None)
    if not callable(generate):
        raise TypeError("Qwen ASR model does not support generation")
    with torch.inference_mode():
        output_ids = generate(**inputs, max_new_tokens=QWEN_MAX_NEW_TOKENS)
    if not isinstance(output_ids, torch.Tensor):
        raise TypeError("Qwen ASR returned invalid token IDs")
    generated_ids = output_ids[:, inputs["input_ids"].shape[1] :]
    decoded_output = processor.decode(generated_ids, return_format="parsed")
    if not isinstance(decoded_output, list) or not decoded_output:
        raise TypeError("Qwen returned an invalid transcription")
    decoded = QwenTranscription.model_validate(decoded_output[0])
    text = decoded.transcription.strip()
    if not text:
        return Transcription(segments=[])
    if decoded.language not in QWEN_ALIGNMENT_LANGUAGES:
        logger.warning(
            "No bundled alignment tokenizer for detected language=%s",
            decoded.language,
        )
        return Transcription(
            segments=[Segment(start_frame=0, end_frame=samples.size, text=text)]
        )
    send_worker_event(
        ProgressEvent(
            message="Aligning words...", model=QWEN_MODEL_ID, state="transcribing"
        )
    )
    aligner_inputs, word_lists = aligner_processor.prepare_forced_aligner_inputs(
        samples,
        text,
        language=decoded.language,
    )
    aligner_inputs.to(aligner.device, dtype=aligner.dtype)
    with torch.inference_mode():
        alignment = aligner(**aligner_inputs)
    aligned_batches = aligner_processor.decode_forced_alignment(
        logits=alignment.logits,
        input_ids=aligner_inputs["input_ids"],
        word_lists=word_lists,
        timestamp_token_id=aligner.config.timestamp_token_id,
    )
    aligned = [AlignedWord.model_validate(word) for word in aligned_batches[0]]
    return Transcription(
        segments=aligned_segments(aligned, text, decoded.language, samples.size)
    )


def audio_samples(payload: bytes) -> np.ndarray:
    samples = np.frombuffer(payload, dtype="<f4")
    if not np.isfinite(samples).all():
        raise ValueError("Audio contains a non-finite sample")
    return samples


def aligned_segments(
    words: list[AlignedWord], text: str, language: str | None, total_frames: int
) -> list[Segment]:
    if not words:
        return [Segment(start_frame=0, end_frame=total_frames, text=text)]
    separator = "" if language in ("Chinese", "Cantonese") else " "
    segments = []
    segment_words: list[str] = []
    segment_start = words[0].start_time
    segment_end = segment_start
    for word in words:
        if segment_words and word.end_time - segment_start > MAX_CAPTION_SECONDS:
            segments.append(
                Segment(
                    start_frame=round(segment_start * SAMPLE_RATE),
                    end_frame=round(segment_end * SAMPLE_RATE),
                    text=separator.join(segment_words),
                )
            )
            segment_words = []
            segment_start = word.start_time
        segment_words.append(word.text)
        segment_end = word.end_time
    segments.append(
        Segment(
            start_frame=round(segment_start * SAMPLE_RATE),
            end_frame=round(segment_end * SAMPLE_RATE),
            text=separator.join(segment_words),
        )
    )
    segments[0].start_frame = 0
    segments[-1].end_frame = total_frames
    return segments


def timestamped_segments(
    timestamps: list[Timestamp], text: str, total_frames: int
) -> list[Segment]:
    segments: list[Segment] = []
    segment_text = ""
    segment_start = 0
    segment_end = 0
    for timestamp in timestamps:
        token = timestamp.token
        if not token:
            continue
        if not segment_text:
            segment_start = round(timestamp.start * SAMPLE_RATE)
        segment_text += token
        segment_end = round(timestamp.end * SAMPLE_RATE)
        if any(separator in token for separator in ".?!"):
            append_segment(
                segments, segment_start, segment_end, segment_text, total_frames
            )
            segment_text = ""
    append_segment(segments, segment_start, segment_end, segment_text, total_frames)
    if not segments and text.strip():
        segments.append(
            Segment(start_frame=0, end_frame=total_frames, text=text.strip())
        )
    if segments:
        segments[0].start_frame = 0
        segments[-1].end_frame = total_frames
    return segments


def append_segment(
    segments: list[Segment],
    start: int,
    end: int,
    text: str,
    total_frames: int,
) -> None:
    text = text.strip()
    start = min(max(start, 0), total_frames)
    end = min(max(end, start), total_frames)
    if text and end > start:
        segments.append(Segment(start_frame=start, end_frame=end, text=text))


def load_transcriber(model_id: ModelId, device: torch.device) -> Transcriber:
    send_worker_event(
        ProgressEvent(message="Checking model...", model=model_id, state="loading")
    )
    logger.info("Resolving model files for %s", model_id)
    model_path = snapshot_download(
        model_id,
        allow_patterns=list(MODEL_FILE_PATTERNS),
        ignore_patterns=["*.fp32.safetensors"],
        max_workers=1,
        tqdm_class=DownloadProgress,
    )
    send_worker_event(
        ProgressEvent(message="Loading processor...", model=model_id, state="loading")
    )
    logger.info("Loading processor from %s", model_path)
    if model_id == PARAKEET_MODEL_ID:
        processor = ParakeetProcessor.from_pretrained(model_path)
        send_worker_event(
            ProgressEvent(message="Loading weights...", model=model_id, state="loading")
        )
        model = ParakeetForTDT.from_pretrained(model_path, dtype="auto")
        send_worker_event(
            ProgressEvent(message="Moving to device...", model=model_id, state="loading")
        )
        model.to(device)
        model.eval()
        return lambda payload: transcribe_parakeet(processor, model, payload)
    if model_id in WHISPER_MODEL_IDS:
        whisper_processor = WhisperProcessor.from_pretrained(model_path)
        send_worker_event(
            ProgressEvent(message="Loading weights...", model=model_id, state="loading")
        )
        whisper_model = WhisperForConditionalGeneration.from_pretrained(
            model_path, dtype="auto"
        )
        send_worker_event(
            ProgressEvent(message="Moving to device...", model=model_id, state="loading")
        )
        whisper_model.to(device)
        whisper_model.eval()
        return lambda payload: transcribe_whisper(
            whisper_processor, whisper_model, payload
        )
    if model_id == QWEN_MODEL_ID:
        qwen_processor = Qwen3ASRProcessor.from_pretrained(model_path)
        send_worker_event(
            ProgressEvent(message="Loading weights...", model=model_id, state="loading")
        )
        qwen_model = Qwen3ASRForConditionalGeneration.from_pretrained(
            model_path, dtype="auto"
        )
        send_worker_event(
            ProgressEvent(
                message="Checking aligner...", model=model_id, state="loading"
            )
        )
        logger.info("Resolving model files for %s", QWEN_ALIGNER_ID)
        aligner_path = snapshot_download(
            QWEN_ALIGNER_ID,
            allow_patterns=list(MODEL_FILE_PATTERNS),
            ignore_patterns=["*.fp32.safetensors"],
            max_workers=1,
            tqdm_class=DownloadProgress,
        )
        send_worker_event(
            ProgressEvent(message="Loading aligner...", model=model_id, state="loading")
        )
        aligner_processor = Qwen3ASRProcessor.from_pretrained(aligner_path)
        aligner = Qwen3ASRForTokenClassification.from_pretrained(
            aligner_path, dtype="auto"
        )
        send_worker_event(
            ProgressEvent(message="Moving to device...", model=model_id, state="loading")
        )
        qwen_model.to(device)
        qwen_model.eval()
        aligner.to(device)
        aligner.eval()
        return lambda payload: transcribe_qwen(
            qwen_processor,
            qwen_model,
            aligner_processor,
            aligner,
            payload,
        )
    raise ValueError(f"Unsupported speech-to-text model: {model_id}")


def run(connection: Connection, model_id: ModelId, device_name: str) -> None:
    global active_model_id, response_connection
    response_connection = connection
    active_model_id = model_id
    device = torch.device(device_name)
    logger.info(
        "Worker pid=%d initializing model=%s device=%s", os.getpid(), model_id, device
    )
    try:
        send_worker_event(
            ProgressEvent(message="Runtime ready", model=model_id, state="loading")
        )
        transcribe = load_transcriber(model_id, device)
        send_worker_event(
            ProgressEvent(message="Model ready", model=model_id, state="ready")
        )
        logger.info("Model is ready model=%s", model_id)
    except Exception as exception:
        logger.exception("Speech-to-text worker initialization failed")
        send_worker_event(
            ErrorEvent(code="worker_initialization_failed", message=str(exception))
        )
        connection.close()
        return
    while True:
        try:
            payload = connection.recv_bytes()
        except EOFError:
            break
        try:
            send_worker_event(
                ProgressEvent(
                    message="Preparing audio...",
                    model=model_id,
                    state="transcribing",
                )
            )
            logger.info("Transcribing %d audio bytes", len(payload))
            send_worker_event(
                ProgressEvent(
                    message="Transcribing...", model=model_id, state="transcribing"
                )
            )
            result = transcribe(payload)
            if device.type == "cuda":
                with torch.cuda.device(device):
                    torch.cuda.empty_cache()
            send_worker_event(ResultEvent(result=result))
            del payload, result
            logger.info("Transcription completed model=%s", model_id)
        except Exception as exception:
            logger.exception("Transcription failed")
            send_worker_event(
                ErrorEvent(code="transcription_failed", message=str(exception))
            )
            connection.close()
            return
    logger.info("Worker input closed; shutting down")
    connection.close()
