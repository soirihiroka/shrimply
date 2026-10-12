import logging
from functools import partial
from http import HTTPStatus

import msgspec
from fastapi import Request
from fastapi.responses import Response
from pydantic import ValidationError

from api import log_request, request_content_type, resource, respond_error, stream_events
from api import pneuma
from api.pneuma.protocol import CONVERSION_REQUEST_VALIDATOR, ErrorEvent
from api.tts.errors import log_failure, public_error_message

logger = logging.getLogger("shrimply.server")
MAXIMUM_REQUEST_BYTES = 65 * 1024 * 1024


async def handle_request(request: Request) -> Response:
    log_request(request, "Pneuma conversion")
    try:
        job = resource.register_job(request.headers.get(resource.JOB_HEADER))
    except resource.InvalidJobId as exception:
        return respond_error(HTTPStatus.BAD_REQUEST, "invalid_job_id", str(exception))
    except resource.DuplicateJobId as exception:
        return respond_error(HTTPStatus.CONFLICT, "duplicate_job_id", str(exception))
    except resource.JobCancelled as exception:
        return respond_error(HTTPStatus.CONFLICT, exception.code, str(exception))
    if request_content_type(request) != "application/msgpack":
        resource.finish_job(job)
        return respond_error(
            HTTPStatus.UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type",
            "Expected application/msgpack",
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
    if content_length <= 0 or content_length > MAXIMUM_REQUEST_BYTES:
        resource.finish_job(job)
        return respond_error(
            HTTPStatus.BAD_REQUEST,
            "invalid_content_length",
            "Pneuma request must be between 1 byte and 65 MiB",
        )
    try:
        received = bytearray()
        async for chunk in request.stream():
            job.check_cancelled()
            if len(chunk) > content_length - len(received):
                raise ValueError("Pneuma request body exceeds Content-Length")
            received.extend(chunk)
        payload = bytes(received)
    except resource.JobCancelled as exception:
        resource.finish_job(job)
        return respond_error(HTTPStatus.CONFLICT, exception.code, str(exception))
    except ValueError as exception:
        resource.finish_job(job)
        return respond_error(HTTPStatus.BAD_REQUEST, "invalid_content_length", str(exception))
    except Exception:
        resource.finish_job(job)
        raise
    if len(payload) != content_length:
        resource.finish_job(job)
        return respond_error(
            HTTPStatus.BAD_REQUEST,
            "invalid_content_length",
            "Pneuma request length does not match Content-Length",
        )
    try:
        conversion_request = CONVERSION_REQUEST_VALIDATOR.validate_python(
            msgspec.msgpack.decode(payload)
        )
    except (msgspec.DecodeError, ValidationError) as exception:
        resource.finish_job(job)
        return respond_error(HTTPStatus.BAD_REQUEST, "invalid_request", str(exception))
    client = request.client.host if request.client is not None else "unknown"
    logger.info(
        "Accepted Pneuma request from %s model=%s audio_bytes=%d",
        client,
        conversion_request.model,
        len(conversion_request.audio if isinstance(conversion_request.audio, bytes) else conversion_request.audio.data),
    )

    def failure(exception: Exception) -> ErrorEvent:
        try:
            job.check_cancelled()
        except resource.JobCancelled as cancellation:
            exception = cancellation
        if isinstance(exception, resource.JobCancelled):
            return ErrorEvent(code=exception.code, message=str(exception))
        log_failure(logger, "Pneuma request failed", exception)
        return ErrorEvent(
            code="worker_failed", message=public_error_message(exception)
        )

    return stream_events(
        partial(pneuma.convert, conversion_request, job),
        failure,
        "pneuma-request",
        job,
    )
