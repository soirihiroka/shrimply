from collections.abc import Callable, Iterator
from contextlib import ExitStack
from fractions import Fraction
from io import BytesIO
from pathlib import Path
from tempfile import NamedTemporaryFile
from typing import Annotated, BinaryIO, Literal, Self
import warnings

import av
import numpy as np
from PIL import Image, ImageOps
from pydantic import BaseModel, ConfigDict, Field, model_validator

MAXIMUM_AUDIO_BYTES = 512 * 1024**2
FLOAT_BYTES = 4
PCM_BLOCK_FRAMES = 4096
NON_RASTER_FORMATS = frozenset(("EPS", "PDF", "PSD", "WMF"))


class EncodedAudio(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True, frozen=True)

    kind: Literal["encoded"] = "encoded"
    data: bytes = Field(min_length=1, max_length=MAXIMUM_AUDIO_BYTES)


class PcmAudio(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True, frozen=True)

    kind: Literal["pcm"] = "pcm"
    data: bytes = Field(min_length=FLOAT_BYTES, max_length=MAXIMUM_AUDIO_BYTES)
    sample_rate: int = Field(gt=0)
    channels: Literal[1, 2]

    @model_validator(mode="after")
    def validate_frames(self) -> Self:
        if len(self.data) % (FLOAT_BYTES * self.channels):
            raise ValueError("PCM audio contains an incomplete float32 frame")
        return self


type AudioUpload = Annotated[EncodedAudio | PcmAudio, Field(discriminator="kind")]
type AudioSource = AudioUpload | bytes | Path | BinaryIO


def _pcm_frames(samples: np.ndarray, sample_rate: int) -> Iterator[av.AudioFrame]:
    channels, length = samples.shape
    if channels not in (1, 2):
        raise ValueError(f"Expected mono or stereo PCM, got {channels} channels")
    for start in range(0, length, PCM_BLOCK_FRAMES):
        frame = av.AudioFrame.from_ndarray(
            np.ascontiguousarray(samples[:, start : start + PCM_BLOCK_FRAMES]),
            format="fltp",
            layout="mono" if channels == 1 else "stereo",
        )
        frame.sample_rate = sample_rate
        frame.pts = start
        frame.time_base = Fraction(1, sample_rate)
        yield frame


def encode_video(
    output: Path,
    frames: np.ndarray,
    fps: int,
    gpu_index: int,
    *,
    audio: tuple[int, np.ndarray] | None = None,
) -> None:
    """Write frames from a CUDA generation worker to H.264 with NVENC."""
    if frames.ndim != 4 or frames.shape[-1] != 3 or not frames.shape[0]:
        raise ValueError(f"Expected RGB video frames, got shape {frames.shape}")
    output.parent.mkdir(parents=True, exist_ok=True)
    with NamedTemporaryFile(dir=output.parent, suffix=".mp4", delete=False) as temp:
        partial = Path(temp.name)
    try:
        with av.open(str(partial), mode="w", format="mp4") as container:
            video = container.add_stream(
                "h264_nvenc", rate=fps, options={"gpu": str(gpu_index)}
            )
            video.height, video.width = frames.shape[1:3]
            video.pix_fmt = "yuv420p"
            sound = None
            if audio is not None:
                codec = "libfdk_aac" if "libfdk_aac" in av.codecs_available else "aac"
                sound = container.add_stream(codec, rate=audio[0])
                sound.layout = "stereo"
            container.start_encoding()
            for pixels in frames:
                if np.issubdtype(pixels.dtype, np.floating):
                    pixels = np.rint(np.clip(pixels, 0, 1) * 255)
                frame = av.VideoFrame.from_ndarray(
                    np.ascontiguousarray(pixels, dtype=np.uint8), format="rgb24"
                )
                container.mux(video.encode(frame))
            container.mux(video.encode())
            if audio is not None and sound is not None:
                for pcm in _pcm_frames(audio[1], audio[0]):
                    container.mux(sound.encode(pcm))
                container.mux(sound.encode())
        partial.replace(output)
    finally:
        partial.unlink(missing_ok=True)


def decode_audio(
    source: AudioSource,
    *,
    sample_rate: int | None = None,
    layout: Literal["mono", "stereo"] = "mono",
    maximum_seconds: int | None = None,
    maximum_bytes: int = MAXIMUM_AUDIO_BYTES,
    check_cancelled: Callable[[], None] | None = None,
) -> tuple[int, np.ndarray]:
    """Return planar float32 samples at the requested rate and channel layout."""
    if sample_rate is not None and sample_rate <= 0:
        raise ValueError("Audio sample rate must be positive")
    if maximum_seconds is not None and maximum_seconds <= 0:
        raise ValueError("Audio duration limit must be positive")
    with ExitStack() as stack:
        if isinstance(source, PcmAudio):
            source_rate = source.sample_rate
            source_channels = source.channels
            samples = np.frombuffer(source.data, dtype="<f4").reshape(-1, source.channels)
            frames = _pcm_frames(samples.T, source_rate)
        else:
            if isinstance(source, EncodedAudio):
                source = source.data
            if isinstance(source, bytes):
                source = BytesIO(source)
            container = stack.enter_context(
                av.open(str(source) if isinstance(source, Path) else source, mode="r")
            )
            if not container.streams.audio:
                raise ValueError("Media contains no audio stream")
            stream = container.streams.audio[0]
            source_rate = stream.codec_context.sample_rate
            source_channels = len(stream.codec_context.layout.channels)
            frames = container.decode(stream)
        if source_rate <= 0 or source_channels <= 0:
            raise ValueError("Audio is missing a sample rate or channel layout")
        output_rate = sample_rate or source_rate
        channels = 1 if layout == "mono" else 2
        maximum_frames = maximum_bytes // (FLOAT_BYTES * channels)
        duration_frames = (
            maximum_seconds * output_rate if maximum_seconds is not None else None
        )
        resampler = av.AudioResampler(
            format="fltp",
            layout="stereo" if layout == "stereo" and source_channels > 2 else None,
            rate=output_rate,
        )
        chunks: list[np.ndarray] = []
        count = 0
        source_duration = Fraction(0)

        def collect(frame: av.AudioFrame | None) -> None:
            nonlocal count
            for converted in resampler.resample(frame):
                samples = converted.to_ndarray()
                if layout == "mono" and samples.shape[0] != 1:
                    samples = samples.mean(axis=0, keepdims=True)
                elif layout == "stereo" and samples.shape[0] == 1:
                    samples = np.repeat(samples, 2, axis=0)
                if duration_frames is not None:
                    samples = samples[:, : max(0, duration_frames - count)]
                if count + samples.shape[1] > maximum_frames:
                    raise ValueError("Decoded or resampled audio is too large")
                if not np.isfinite(samples).all():
                    raise ValueError("Audio contains non-finite samples")
                if samples.shape[1]:
                    chunks.append(samples)
                    count += samples.shape[1]

        for frame in frames:
            if check_cancelled is not None:
                check_cancelled()
            if frame.sample_rate <= 0:
                raise ValueError("Audio frame is missing a sample rate")
            if not np.isfinite(frame.to_ndarray()).all():
                raise ValueError("Audio contains non-finite samples")
            frame.pts = None
            collect(frame)
            source_duration += Fraction(frame.samples, frame.sample_rate)
            if maximum_seconds is not None and source_duration >= maximum_seconds:
                break
        collect(None)
        if not count:
            raise ValueError("Audio contains no decodable samples")
        return output_rate, np.concatenate(chunks, axis=1)


def load_image(source: Path | str | BinaryIO) -> Image.Image:
    with warnings.catch_warnings():
        warnings.simplefilter("error", Image.DecompressionBombWarning)
        with Image.open(source) as original:
            if original.format in NON_RASTER_FORMATS:
                raise ValueError("Vector and layered image formats are not supported")
            original.seek(0)
            image = ImageOps.exif_transpose(original)
            if image.mode in ("RGBA", "LA") or "transparency" in image.info:
                rgba = image.convert("RGBA")
                background = Image.new("RGBA", rgba.size, "white")
                return Image.alpha_composite(background, rgba).convert("RGB")
            return image.convert("RGB")
