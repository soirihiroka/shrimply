import argparse
import math
import os
import threading
import urllib.request
from fractions import Fraction
from pathlib import Path
from uuid import uuid4

import av
import msgspec

HEARTBEAT_SECONDS = 5
REQUEST_TIMEOUT_SECONDS = 5
import numpy as np
from pydantic import TypeAdapter

from api.video_generation.protocol import (
    ErrorEvent,
    GenerationRequest,
    GenerationResult,
    InputValue,
    Media,
    MediaKind,
    MediaValue,
    NumberValue,
    OutputChunkEvent,
    OutputStartEvent,
    ProgressEvent,
    PublicEvent,
    Rational,
    ResultEvent,
    SelectValue,
    TextValue,
)

PUBLIC_EVENT_VALIDATOR = TypeAdapter(PublicEvent)


def text(value: str) -> TextValue:
    return TextValue(kind="text", value=value)


def select(value: str) -> SelectValue:
    return SelectValue(kind="select", value=value)


def number(numerator: int, denominator: int = 1) -> NumberValue:
    return NumberValue(
        kind="number",
        value=Rational(numerator=numerator, denominator=denominator),
    )


def media(kind: MediaKind, path: Path) -> Media:
    return Media(kind=kind, filename=path.name, data=path.read_bytes())


def common(
    workflow: str, resolution: str, prompt: str
) -> dict[str, InputValue]:
    return {
        "workflow": select(workflow),
        "prompt": text(prompt),
        "resolution": select(resolution),
        "duration": number(5),
        "steps": number(30),
        "seed": number(42),
        "attention": select("auto"),
        "quantization": select("bf16"),
        "memory": select("normal"),
    }


def request_case(
    case: str, image: Path, reference_video: Path
) -> tuple[GenerationRequest, str]:
    if case == "base-t2va":
        return (
            GenerationRequest(
                model="MiniMaxAI/MiniMax-H3",
                inputs=common(
                    "t2va",
                    "768x768",
                    "A red fox walks through a snowy pine forest at sunrise. Gentle wind, birds, and crisp footsteps form a natural synchronized soundscape.",
                ),
            ),
            "base-t2va-768x768.mp4",
        )
    if case == "sketch-fl2va":
        inputs = common(
            "fl2va",
            "576x768",
            "A charming hand-drawn looping anime animation. The character gently sways while hair and clothing move in a light breeze. Soft movement sounds and playful instrumental music, no speech.",
        )
        inputs["first_frame"] = MediaValue(
            kind="media", items=[media("image", image)]
        )
        inputs["last_frame"] = MediaValue(kind="media", items=[])
        inputs["lora_scale"] = number(1)
        return (
            GenerationRequest(
                model="Inner-Reflections/MiniMax-H3-Looping-Sketch-Anime",
                inputs=inputs,
            ),
            "sketch-fl2va-576x768.mp4",
        )
    inputs = common(
        "ref2va",
        "768x768",
        "Create a coherent scene following the subject image, the referenced motion, and the referenced ambient audio, with synchronized movement and sound.",
    )
    inputs["references"] = MediaValue(
        kind="media",
        items=[
            media("image", image),
            media("video", reference_video),
            media("audio", reference_video),
        ],
    )
    return (
        GenerationRequest(model="MiniMaxAI/MiniMax-H3", inputs=inputs),
        "base-ref2va-multimodal.mp4",
    )


def render(
    server: str,
    request: GenerationRequest,
    output: Path,
) -> GenerationResult:
    payload = msgspec.msgpack.encode(request.model_dump(mode="python"))
    job_id = str(uuid4())
    heartbeat_stopped = threading.Event()
    heartbeat = threading.Thread(
        target=_heartbeat,
        args=(server, job_id, heartbeat_stopped),
        name=f"compute-heartbeat:{job_id}",
        daemon=True,
    )
    http_request = urllib.request.Request(
        f"{server.rstrip('/')}/video-generations",
        data=payload,
        headers={
            "Accept": "application/x-msgpack-stream",
            "Content-Type": "application/msgpack",
            "Content-Length": str(len(payload)),
            "Shrimply-Job-ID": job_id,
        },
        method="POST",
    )
    temporary = output.with_name(f".{output.name}.partial")
    result: GenerationResult | None = None
    completed = False
    expected_bytes = None
    received_bytes = 0
    last_progress: str | None = None
    heartbeat.start()
    try:
        with urllib.request.urlopen(http_request) as response, temporary.open("wb") as target:
            while header := response.read(8):
                if len(header) != 8:
                    raise RuntimeError("truncated event header")
                size = int.from_bytes(header, "little")
                chunks = bytearray()
                while len(chunks) < size:
                    chunk = response.read(size - len(chunks))
                    if not chunk:
                        raise RuntimeError("truncated event payload")
                    chunks.extend(chunk)
                event = PUBLIC_EVENT_VALIDATOR.validate_python(
                    msgspec.msgpack.decode(chunks)
                )
                if isinstance(event, ProgressEvent):
                    if event.message != last_progress:
                        last_progress = event.message
                        print(last_progress, flush=True)
                elif isinstance(event, OutputStartEvent):
                    expected_bytes = event.bytes
                elif isinstance(event, OutputChunkEvent):
                    target.write(event.data)
                    received_bytes += len(event.data)
                elif isinstance(event, ResultEvent):
                    result = event.result
                elif isinstance(event, ErrorEvent):
                    raise RuntimeError(event.message)
        if result is None or expected_bytes != received_bytes:
            raise RuntimeError("generation stream did not contain a complete result")
        os.replace(temporary, output)
        completed = True
        return result
    finally:
        heartbeat_stopped.set()
        heartbeat.join()
        if not completed:
            try:
                urllib.request.urlopen(
                    urllib.request.Request(
                        f"{server.rstrip('/')}/compute/jobs/{job_id}", method="DELETE"
                    ),
                    timeout=REQUEST_TIMEOUT_SECONDS,
                ).close()
            except OSError:
                pass
        temporary.unlink(missing_ok=True)


def _heartbeat(server: str, job_id: str, stopped: threading.Event) -> None:
    while not stopped.wait(HEARTBEAT_SECONDS):
        try:
            urllib.request.urlopen(
                urllib.request.Request(
                    f"{server.rstrip('/')}/compute/jobs/{job_id}/heartbeat",
                    method="PUT",
                ),
                timeout=REQUEST_TIMEOUT_SECONDS,
            ).close()
        except OSError:
            pass


def verify(
    path: Path,
    metadata: GenerationResult,
    width: int,
    height: int,
) -> None:
    container = av.open(path)
    video = container.streams.video
    audio = container.streams.audio
    if len(video) != 1 or len(audio) != 1:
        raise RuntimeError(f"expected one video and audio stream, got {len(video)} and {len(audio)}")
    if video[0].codec_context.name != "h264" or audio[0].codec_context.name != "aac":
        raise RuntimeError("expected H.264 video and AAC audio")
    if (video[0].width, video[0].height) != (width, height):
        raise RuntimeError("output dimensions do not match the request")
    average_rate = video[0].average_rate
    if not average_rate:
        raise RuntimeError("output video has no frame rate")
    rate = Fraction(average_rate.numerator, average_rate.denominator)
    frames = sum(1 for _ in container.decode(video=0))
    container.close()
    container = av.open(path)
    samples = []
    audio_sample_count = 0
    layouts = set()
    rates = set()
    for frame in container.decode(audio=0):
        samples.append(frame.to_ndarray().astype(np.float64, copy=False))
        audio_sample_count += frame.samples
        layouts.add(frame.layout.name)
        rates.add(frame.sample_rate)
    container.close()
    if rate != 24 or frames != 124:
        raise RuntimeError(f"expected 24 fps and 124 frames, got {rate} and {frames}")
    if rates != {32_000} or layouts != {"stereo"}:
        raise RuntimeError(f"expected 32 kHz stereo audio, got {rates} and {layouts}")
    if not samples:
        raise RuntimeError("output audio is empty")
    rms = math.sqrt(float(np.mean(np.concatenate(samples, axis=-1) ** 2)))
    if not math.isfinite(rms) or rms <= 1e-6:
        raise RuntimeError(f"output audio is silent: RMS {rms}")
    exact_duration = metadata.duration.fraction()
    if exact_duration != Fraction(124, 24):
        raise RuntimeError(f"unexpected result duration {exact_duration}")
    audio_duration = Fraction(audio_sample_count, 32_000)
    if abs(audio_duration - exact_duration) > Fraction(1, 24):
        raise RuntimeError(
            f"audio/video durations are not synchronized: {audio_duration} and {exact_duration}"
        )
    print(
        f"verified {path}: h264 {width}x{height}, 24 fps, 124 frames; aac 32 kHz stereo; RMS {rms:.6f}; video {float(exact_duration):.4f}s, audio {float(audio_duration):.4f}s",
        flush=True,
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("case", choices=["base-t2va", "sketch-fl2va", "base-ref2va"])
    parser.add_argument("--server", default="http://127.0.0.1:8787")
    parser.add_argument("--output-directory", type=Path, required=True)
    parser.add_argument("--image", type=Path, required=True)
    parser.add_argument("--reference-video", type=Path, required=True)
    arguments = parser.parse_args()
    request, filename = request_case(arguments.case, arguments.image, arguments.reference_video)
    arguments.output_directory.mkdir(parents=True, exist_ok=True)
    output = arguments.output_directory / filename
    metadata = render(arguments.server, request, output)
    dimensions = (576, 768) if arguments.case == "sketch-fl2va" else (768, 768)
    verify(output, metadata, *dimensions)


if __name__ == "__main__":
    main()
