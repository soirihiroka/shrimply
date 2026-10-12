import asyncio
import logging
import os
import tempfile
from http import HTTPStatus
from pathlib import Path

import msgspec
from fastapi import Request
from fastapi.responses import Response
from pydantic import ValidationError

from api import log_request, request_content_type, resource, respond_error, stream_events
from api import stt
from api.stt.protocol import MODEL_ID_VALIDATOR, TRANSCRIPTION_REQUEST_VALIDATOR, ErrorEvent, ModelId
from api.stt.resources import MAXIMUM_AUDIO_BYTES

logger = logging.getLogger("shrimply.server")
MESSAGE_PACK_OVERHEAD_BYTES = 1024**2


def uploaded_model(path: str) -> ModelId:
    uploaded = TRANSCRIPTION_REQUEST_VALIDATOR.validate_python(
        msgspec.msgpack.decode(Path(path).read_bytes())
    )
    return uploaded.model


async def handle_request(request: Request) -> Response:
    log_request(request, "speech-to-text")
    try:
        job = resource.register_job(request.headers.get(resource.JOB_HEADER))
    except resource.InvalidJobId as exception:
        return respond_error(HTTPStatus.BAD_REQUEST, "invalid_job_id", str(exception))
    except resource.DuplicateJobId as exception:
        return respond_error(HTTPStatus.CONFLICT, "duplicate_job_id", str(exception))
    except resource.JobCancelled as exception:
        return respond_error(HTTPStatus.CONFLICT, exception.code, str(exception))
    content_type = request_content_type(request)
    model_id: ModelId | None = None
    message_pack = content_type == "application/msgpack"
    if content_type not in ("application/octet-stream", "application/msgpack"):
        resource.finish_job(job)
        return respond_error(
            HTTPStatus.UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type",
            "Expected application/msgpack or application/octet-stream",
        )
    if not message_pack:
        model_values = request.query_params.getlist("model")
        if len(model_values) != 1:
            resource.finish_job(job)
            return respond_error(
                HTTPStatus.BAD_REQUEST, "model_required",
                "Exactly one speech-to-text model is required",
            )
        try:
            model_id = MODEL_ID_VALIDATOR.validate_python(model_values[0])
        except ValidationError:
            resource.finish_job(job)
            return respond_error(
                HTTPStatus.BAD_REQUEST, "unsupported_model",
                f"Unsupported speech-to-text model: {model_values[0]}",
            )
    try:
        content_length = int(request.headers.get("content-length", ""))
    except ValueError:
        resource.finish_job(job)
        return respond_error(
            HTTPStatus.LENGTH_REQUIRED,
            "content_length_required",
            "A valid Content-Length header is required",
        )
    maximum_request = MAXIMUM_AUDIO_BYTES + (
        MESSAGE_PACK_OVERHEAD_BYTES if message_pack else 0
    )
    if content_length <= 0 or content_length > maximum_request:
        resource.finish_job(job)
        return respond_error(
            HTTPStatus.BAD_REQUEST,
            "invalid_content_length",
            "Audio request exceeds the 512 MiB audio limit or is empty",
        )
    path: str | None = None
    try:
        with tempfile.NamedTemporaryFile(prefix="shrimply-stt-", delete=False) as audio:
            path = audio.name
            remaining = content_length
            async for chunk in request.stream():
                job.check_cancelled()
                if len(chunk) > remaining:
                    raise ValueError("Audio body exceeds Content-Length")
                audio.write(chunk)
                remaining -= len(chunk)
            if remaining:
                raise ValueError("Audio length does not match Content-Length")
    except resource.JobCancelled as exception:
        if path is not None:
            os.unlink(path)
        resource.finish_job(job)
        return respond_error(HTTPStatus.CONFLICT, exception.code, str(exception))
    except ValueError as exception:
        if path is not None:
            os.unlink(path)
        resource.finish_job(job)
        return respond_error(HTTPStatus.BAD_REQUEST, "invalid_content_length", str(exception))
    except Exception:
        if path is not None:
            os.unlink(path)
        resource.finish_job(job)
        raise
    assert path is not None
    if message_pack:
        try:
            model_id = await asyncio.to_thread(uploaded_model, path)
            job.check_cancelled()
        except (msgspec.DecodeError, ValidationError) as exception:
            os.unlink(path)
            resource.finish_job(job)
            return respond_error(HTTPStatus.BAD_REQUEST, "invalid_request", str(exception))
        except resource.JobCancelled as exception:
            os.unlink(path)
            resource.finish_job(job)
            return respond_error(HTTPStatus.CONFLICT, exception.code, str(exception))
        except Exception:
            os.unlink(path)
            resource.finish_job(job)
            raise
    elif content_length % 4 != 0:
        os.unlink(path)
        resource.finish_job(job)
        return respond_error(
            HTTPStatus.BAD_REQUEST,
            "invalid_audio",
            "Audio length is not a multiple of one little-endian f32 sample",
        )
    assert model_id is not None
    client = request.client.host if request.client is not None else "unknown"
    logger.info(
        "Accepted speech-to-text request from %s model=%s with %d audio bytes",
        client,
        model_id,
        content_length,
    )

    def failure(exception: Exception) -> ErrorEvent:
        try:
            job.check_cancelled()
        except resource.JobCancelled as cancellation:
            exception = cancellation
        if isinstance(exception, resource.JobCancelled):
            return ErrorEvent(code=exception.code, message=str(exception))
        logger.exception("Speech-to-text request failed")
        return ErrorEvent(code="worker_failed", message=str(exception))

    def transcribe(send_event):
        stt.transcribe(
            model_id, path, content_length, job, send_event, message_pack=message_pack
        )

    return stream_events(
        transcribe, failure, "stt-request", job, lambda: os.unlink(path)
    )
