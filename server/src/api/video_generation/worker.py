import logging
from multiprocessing.connection import Connection

import msgspec

from api.model_progress import stream_model_progress
from api.video_generation.minimax_h3.inference import (
    GenerationRequest as H3Request,
    align_num_frames,
    decode_checkpoint,
    generate,
)
from api.video_generation.protocol import (
    ErrorEvent,
    GenerationResult,
    ModelId,
    ProgressEvent,
    Rational,
    WorkerCheckpointEvent,
    WorkerStage,
    WorkerState,
    WorkerResultEvent,
)
from api.video_generation.wan.inference import (
    GenerationRequest as WanRequest,
    generate as generate_wan,
)
from api.tts.errors import log_failure, public_error_message

logger = logging.getLogger("shrimply.video_generation.worker")


def send(
    connection: Connection,
    event: ProgressEvent | WorkerCheckpointEvent | WorkerResultEvent | ErrorEvent,
) -> None:
    connection.send_bytes(msgspec.msgpack.encode(event.model_dump(mode="python")))


def run(
    connection: Connection,
    request: H3Request | WanRequest,
    model_id: ModelId,
    device: str,
    stage: WorkerStage,
) -> None:
    try:
        import torch

        if not device.startswith("cuda:"):
            raise RuntimeError("Video generation requires a CUDA device")
        torch.cuda.set_device(device)
        from api import gpu

        if stage == "generation" and not gpu.bf16_supported(device):
            raise RuntimeError("Video generation requires a GPU with native BF16 support")
        torch.set_num_threads(1)

        def progress(message: str, state: WorkerState) -> None:
            send(
                connection,
                ProgressEvent(message=message, model=model_id, state=state),
            )

        if isinstance(request, WanRequest):
            if stage != "generation":
                raise RuntimeError("Wan does not use a separate decode worker")
            progress("Loading Wan pipeline…", "loading")
            with stream_model_progress(
                lambda message: progress(message, "loading"), include_diffusers=True
            ):
                generate_wan(request, progress, device)
            send(
                connection,
                WorkerResultEvent(
                    output=str(request.output),
                    result=GenerationResult(
                        duration=Rational(
                            numerator=request.frames,
                            denominator=request.frame_rate,
                        ),
                        frame_rate=Rational(
                            numerator=request.frame_rate,
                            denominator=1,
                        ),
                        width=request.width,
                        height=request.height,
                        video_streams=1,
                        audio_streams=0,
                    ),
                ),
            )
            return

        if stage == "generation":
            progress("Loading MiniMax H3…", "loading")
            with stream_model_progress(
                lambda message: progress(message, "loading"), include_diffusers=True
            ):
                output = generate(request, progress=progress, decode_output=False)
            send(
                connection,
                WorkerCheckpointEvent(
                    checkpoint=output["checkpoint"],
                    frames=output["frames"],
                ),
            )
            return

        if request.checkpoint is None:
            raise RuntimeError("MiniMax H3 decode requires a latent checkpoint")
        progress("Loading MiniMax H3 decoder…", "loading")
        with stream_model_progress(
            lambda message: progress(message, "loading"), include_diffusers=True
        ):
            decode_checkpoint(request.checkpoint, request.output, request.model, progress)
        frames = align_num_frames(request.duration)
        send(
            connection,
            WorkerResultEvent(
                output=str(request.output),
                result=GenerationResult(
                    duration=Rational(numerator=frames, denominator=24),
                    frame_rate=Rational(numerator=24, denominator=1),
                    width=request.width,
                    height=request.height,
                    video_streams=1,
                    audio_streams=1,
                ),
            ),
        )
    except Exception as exception:
        log_failure(logger, "Video-generation worker failed", exception)
        try:
            send(
                connection,
                ErrorEvent(code="generation_failed", message=public_error_message(exception)),
            )
        except (BrokenPipeError, EOFError, OSError):
            pass
    finally:
        connection.close()
