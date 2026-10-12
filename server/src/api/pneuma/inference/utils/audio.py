from pathlib import Path
from typing import BinaryIO

import numpy as np
import av

from api.media import decode_audio

from api.pneuma.resources import MAXIMUM_DECODED_AUDIO_BYTES
from api.pneuma.inference.utils.types import MonoAudioArray

type AudioInputFile = Path | BinaryIO


def get_audio_sample_rate(
    file: AudioInputFile, *, source_name: str | None = None
) -> int:
    try:
        with av.open(str(file) if isinstance(file, Path) else file, mode="r") as container:
            if not container.streams.audio:
                raise ValueError("Media contains no audio stream")
            sample_rate = container.streams.audio[0].codec_context.sample_rate
            if sample_rate <= 0:
                raise ValueError("Audio is missing a sample rate")
            return sample_rate
    except Exception as exception:
        raise RuntimeError(f"Failed to read audio sample rate: {exception}") from exception


def _as_float32_array(value: np.ndarray) -> np.ndarray:
    array = np.asarray(value, dtype=np.float32)
    if array.dtype != np.float32:
        raise TypeError(f"Expected float32 array, got {array.dtype}")
    return array


def load_audio_with_sr(
    file: AudioInputFile, sr: int | None = None, *, source_name: str | None = None
) -> tuple[int, MonoAudioArray]:
    try:
        rate, samples = decode_audio(
            file, sample_rate=sr, maximum_bytes=MAXIMUM_DECODED_AUDIO_BYTES
        )
        return rate, samples.reshape(-1)
    except Exception as exception:
        raise RuntimeError(
            f"Failed to load audio {source_name or str(file)}: {exception}"
        ) from exception


def load_audio(file: Path, sr: int) -> MonoAudioArray:
    _, audio = load_audio_with_sr(file, sr=sr)
    return audio


def normalize_audio(
    audio: MonoAudioArray,
    *,
    max_volume: float = 0.9,
    blend: float = 0.75,
) -> MonoAudioArray:
    audio = _as_float32_array(audio)
    peak = float(np.abs(audio).max()) if audio.size else 0.0
    if peak <= 0.0:
        return audio
    normalized = (audio / peak * (max_volume * blend)) + (1.0 - blend) * audio
    return _as_float32_array(normalized)
