import asyncio
import logging
import os
import tempfile
from http import HTTPStatus

import msgspec
from fastapi import Request
from fastapi.responses import Response
from pydantic import ValidationError

from api import log_request, request_content_type, respond_error, stream_events
from api import resource, sam2
from api.sam2.protocol import (
    ANALYSIS_REQUEST_VALIDATOR,
    ARCHIVE_MAGIC,
    MAXIMUM_ARCHIVE_BYTES,
    AnalysisRequest,
    ErrorEvent,
    MAXIMUM_FRAME_BYTES,
)

logger = logging.getLogger("shrimply.server")
CONTENT_TYPE = "application/x-shrimply-sam2-analysis"
MAXIMUM_HEADER_BYTES = 1024 * 1024


def read_header(path: str, job: resource.ManagedJob) -> AnalysisRequest:
    with open(path, "rb") as file:
        if file.read(len(ARCHIVE_MAGIC)) != ARCHIVE_MAGIC:
            raise ValueError("Invalid SAM2 proxy archive magic")
        header_length = int.from_bytes(file.read(8), "little")
        if header_length <= 0 or header_length > MAXIMUM_HEADER_BYTES:
            raise ValueError("Invalid SAM2 proxy archive header length")
        request = ANALYSIS_REQUEST_VALIDATOR.validate_python(
            msgspec.msgpack.decode(file.read(header_length))
        )
        size = os.path.getsize(path)
        for _ in range(request.frame_count):
            job.check_cancelled()
            length_bytes = file.read(8)
            if len(length_bytes) != 8:
                raise ValueError("Truncated SAM2 proxy archive")
            length = int.from_bytes(length_bytes, "little")
            if length <= 0 or length > MAXIMUM_FRAME_BYTES:
                raise ValueError("Invalid SAM2 proxy frame length")
            if length > size - file.tell():
                raise ValueError("Truncated SAM2 proxy archive")
            file.seek(length, 1)
        if file.tell() != size:
            raise ValueError("Unexpected trailing SAM2 proxy archive data")
        return request


async def handle_request(request: Request) -> Response:
    log_request(request, "SAM2 analysis")
    try:
        job = resource.register_job(request.headers.get(resource.JOB_HEADER))
    except resource.InvalidJobId as exception:
        return respond_error(HTTPStatus.BAD_REQUEST, "invalid_job_id", str(exception))
    except resource.DuplicateJobId as exception:
        return respond_error(HTTPStatus.CONFLICT, "duplicate_job_id", str(exception))
    except resource.JobCancelled as exception:
        return respond_error(HTTPStatus.CONFLICT, exception.code, str(exception))

    def reject(response: Response) -> Response:
        resource.finish_job(job)
        return response

    if request_content_type(request) != CONTENT_TYPE:
        return reject(
            respond_error(
                HTTPStatus.UNSUPPORTED_MEDIA_TYPE,
                "unsupported_media_type",
                f"Expected {CONTENT_TYPE}",
            )
        )
    try:
        content_length = int(request.headers.get("content-length", ""))
    except ValueError:
        content_length = 0
    if content_length <= 0 or content_length > MAXIMUM_ARCHIVE_BYTES:
        return reject(
            respond_error(
                HTTPStatus.BAD_REQUEST,
                "invalid_content_length",
                "SAM2 request must be between 1 byte and 8 GiB",
            )
        )
    try:
        resource.scheduler.reserve_temporary(job, content_length)
    except resource.TemporaryStorageUnavailable as exception:
        return reject(
            respond_error(
                HTTPStatus.INSUFFICIENT_STORAGE,
                "temporary_storage_unavailable",
                str(exception),
            )
        )
    path: str | None = None
    try:
        with tempfile.NamedTemporaryFile(
            prefix="shrimply-sam2-", delete=False
        ) as temporary:
            path = temporary.name
            remaining = content_length
            async for chunk in request.stream():
                job.check_cancelled()
                if len(chunk) > remaining:
                    raise ValueError("SAM2 request body exceeds Content-Length")
                temporary.write(chunk)
                remaining -= len(chunk)
            if remaining:
                raise ValueError("Truncated SAM2 request body")
        analysis_request = await asyncio.to_thread(read_header, path, job)
    except (ValueError, msgspec.DecodeError, ValidationError) as exception:
        if path is not None:
            os.unlink(path)
        return reject(
            respond_error(HTTPStatus.BAD_REQUEST, "invalid_request", str(exception))
        )
    except resource.JobCancelled as exception:
        if path is not None:
            os.unlink(path)
        return reject(
            respond_error(HTTPStatus.CONFLICT, exception.code, str(exception))
        )
    except Exception:
        if path is not None:
            os.unlink(path)
        resource.finish_job(job)
        raise
    assert path is not None
    client = request.client.host if request.client is not None else "unknown"
    logger.info(
        "Accepted SAM2 request from %s model=%s with %d archive bytes",
        client,
        analysis_request.model,
        content_length,
    )

    def analyze(send_event):
        sam2.analyze(job, analysis_request.model, path, send_event)

    def failure(exception: Exception) -> ErrorEvent:
        try:
            job.check_cancelled()
        except resource.JobCancelled as cancellation:
            exception = cancellation
        if isinstance(exception, resource.JobCancelled):
            return ErrorEvent(code=exception.code, message=str(exception))
        logger.exception("SAM2 request failed")
        return ErrorEvent(code="worker_failed", message=str(exception))

    return stream_events(
        analyze, failure, "sam2-request", job, lambda: os.unlink(path)
    )
