import logging
import os
from io import BytesIO
from multiprocessing.connection import Connection
from typing import Literal

import msgspec
import numpy as np
import soundfile
import torch

import env

from api.media import decode_audio
from api.pneuma.catalog import MODEL_DIRECTORY
from api.pneuma.protocol import (
    CONVERSION_REQUEST_VALIDATOR,
    ConvertedAudio,
    ErrorEvent,
    ProgressEvent,
    ResultEvent,
    WorkerEvent,
)
from api.tts.errors import log_failure, public_error_message

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s %(levelname)s %(name)s: %(message)s",
)
logger = logging.getLogger("shrimply.pneuma_worker")


def send_event(connection: Connection, event: WorkerEvent) -> None:
    connection.send_bytes(msgspec.msgpack.encode(event.model_dump(mode="python")))


def status_message(
    rows: list[list[str]],
) -> tuple[Literal["error", "loading", "converting"], str]:
    values = dict(rows)
    message = values.get("message", "Converting...")
    if values.get("status") == "error":
        return "error", message
    return "loading" if message.startswith("Loading ") else "converting", message


def run(connection: Connection, device_name: str) -> None:
    env.set_pneuma_device(device_name)

    from api.pneuma.inference.configs.config import get_config
    from api.pneuma.inference.conversion import convert_voice_iter

    logger.info("Worker pid=%d initializing device=%s", os.getpid(), device_name)
    send_event(
        connection,
        ProgressEvent(message="Worker ready", model="", state="ready"),
    )
    while True:
        try:
            payload = connection.recv_bytes()
        except EOFError:
            return
        try:
            request = CONVERSION_REQUEST_VALIDATOR.validate_python(
                msgspec.msgpack.decode(payload)
            )
            send_event(
                connection,
                ProgressEvent(
                    message="Reading audio...",
                    model=request.model,
                    state="converting",
                ),
            )
            rate, samples = decode_audio(request.audio)
            input_audio = (rate, samples.reshape(-1))
            completed = False
            conversion = convert_voice_iter(
                request.model,
                input_audio,
                request.pitch_offset,
                request.f0_method,
                config=get_config(),
                weight_root=MODEL_DIRECTORY,
                speed=request.speed,
                maintain_pitch=request.maintain_pitch,
            )
            for rows, output in conversion:
                status, message = status_message(rows)
                if status == "error":
                    send_event(
                        connection,
                        ErrorEvent(code="conversion_failed", message=message),
                    )
                    completed = True
                    break
                if output is None:
                    send_event(
                        connection,
                        ProgressEvent(
                            message=message,
                            model=request.model,
                            state=status,
                        ),
                    )
                    continue
                sample_rate, samples = output
                audio = np.asarray(samples, dtype=np.float32).reshape(-1)
                wav = BytesIO()
                soundfile.write(wav, audio, sample_rate, format="WAV", subtype="FLOAT")
                result = ResultEvent(result=ConvertedAudio(wav=wav.getvalue()))
                conversion.close()
                del payload, request, input_audio, output, audio, wav
                if torch.cuda.is_available():
                    torch.cuda.empty_cache()
                send_event(connection, result)
                del result
                completed = True
                break
            if not completed:
                raise RuntimeError("Pneuma conversion finished without output audio")
        except Exception as exception:
            log_failure(logger, "Pneuma conversion failed", exception)
            send_event(
                connection,
                ErrorEvent(
                    code="conversion_failed",
                    message=public_error_message(exception),
                ),
            )
