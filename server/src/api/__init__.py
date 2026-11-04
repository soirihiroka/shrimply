import logging
from collections.abc import Callable, Iterator
from http import HTTPStatus
from queue import Empty, Full, Queue
from threading import Event, Thread
from time import monotonic_ns
from typing import TypeVar

import msgspec
from fastapi import Request
from fastapi.responses import Response, StreamingResponse
from pydantic import BaseModel

from api import resource

MESSAGE_PACK = "application/msgpack"
MESSAGE_PACK_STREAM = "application/x-msgpack-stream"
STREAM_HEADER_BYTES = 8
STREAM_KEEPALIVE_SECONDS = 5
NANOSECONDS_PER_SECOND = 1_000_000_000
STREAM_KEEPALIVE_NS = STREAM_KEEPALIVE_SECONDS * NANOSECONDS_PER_SECOND
STREAM_QUEUE_POLL_SECONDS = 0.25
request_logger = logging.getLogger("shrimply.server.request")
stream_logger = logging.getLogger("shrimply.server.stream")


class ApiError(BaseModel):
    code: str
    message: str


class ErrorResponse(BaseModel):
    error: ApiError


def respond(response_status: HTTPStatus, value: BaseModel) -> Response:
    payload = msgspec.msgpack.encode(value.model_dump(mode="python", by_alias=True))
    return Response(
        content=payload,
        status_code=response_status,
        media_type=MESSAGE_PACK,
        headers={"Cache-Control": "no-store"},
    )


def respond_error(status: HTTPStatus, code: str, message: str) -> Response:
    return respond(status, ErrorResponse(error=ApiError(code=code, message=message)))


def request_content_type(request: Request) -> str:
    return request.headers.get("content-type", "").partition(";")[0].strip().lower()


def log_request(request: Request, operation: str) -> None:
    client = request.client.host if request.client is not None else "unknown"
    request_logger.info(
        "Received %s request method=%s path=%s client=%s content_length=%s "
        "content_type=%s",
        operation,
        request.method,
        request.url.path,
        client,
        request.headers.get("content-length", "missing"),
        request_content_type(request) or "missing",
    )


EventType = TypeVar("EventType", bound=BaseModel)


def stream_events(
    run: Callable[[Callable[[EventType], None]], None],
    failure: Callable[[Exception], EventType],
    thread_name: str,
    job: resource.ManagedJob,
    cleanup: Callable[[], None] | None = None,
) -> StreamingResponse:
    events: Queue[bytes | None] = Queue(maxsize=1)
    disconnected = Event()
    latest_keepalive: bytes | None = None
    stream_logger.info("Preparing event stream thread=%s", thread_name)

    def queue_frame(frame: bytes, check_cancelled: bool = True) -> None:
        while not disconnected.is_set():
            if check_cancelled:
                job.check_cancelled()
            try:
                events.put(frame, timeout=STREAM_QUEUE_POLL_SECONDS)
                return
            except Full:
                continue
        raise OSError("Client disconnected")

    def send_event(event: EventType) -> None:
        nonlocal latest_keepalive
        job.check_cancelled()
        if disconnected.is_set():
            raise OSError("Client disconnected")
        if getattr(event, "kind", None) == "error":
            resource.finish_job(job)
        payload = msgspec.msgpack.encode(event.model_dump(mode="python"))
        stream_logger.debug(
            "Queueing event thread=%s event=%s payload_bytes=%d",
            thread_name,
            type(event).__name__,
            len(payload),
        )
        frame = len(payload).to_bytes(STREAM_HEADER_BYTES, "little") + payload
        if getattr(event, "kind", None) in {"queued", "progress"}:
            latest_keepalive = frame
            try:
                events.put_nowait(frame)
            except Full:
                pass
        else:
            queue_frame(frame)
        if disconnected.is_set():
            raise OSError("Client disconnected")

    def work() -> None:
        stream_logger.info("Event worker started thread=%s", thread_name)
        try:
            run(send_event)
        except Exception as exception:
            resource.finish_job(job)
            if not disconnected.is_set():
                event = failure(exception)
                payload = msgspec.msgpack.encode(event.model_dump(mode="python"))
                queue_frame(
                    len(payload).to_bytes(STREAM_HEADER_BYTES, "little") + payload,
                    check_cancelled=False,
                )
        finally:
            try:
                if cleanup is not None:
                    cleanup()
            finally:
                resource.finish_job(job)
            if not disconnected.is_set():
                while not disconnected.is_set():
                    try:
                        events.put(None, timeout=STREAM_QUEUE_POLL_SECONDS)
                        break
                    except Full:
                        continue
            stream_logger.info(
                "Event worker finished thread=%s disconnected=%s",
                thread_name,
                disconnected.is_set(),
            )

    def generate() -> Iterator[bytes]:
        worker = Thread(target=work, name=thread_name, daemon=True)
        stream_logger.info("Starting event stream thread=%s", thread_name)
        try:
            worker.start()
        except Exception:
            try:
                if cleanup is not None:
                    cleanup()
            finally:
                resource.finish_job(job)
            raise
        try:
            next_keepalive = monotonic_ns() + STREAM_KEEPALIVE_NS
            while True:
                try:
                    wait_ns = max(0, next_keepalive - monotonic_ns())
                    event = events.get(timeout=wait_ns / NANOSECONDS_PER_SECOND)
                except Empty:
                    event = latest_keepalive
                    if event is None:
                        next_keepalive = monotonic_ns() + STREAM_KEEPALIVE_NS
                        continue
                if event is None:
                    break
                stream_logger.debug(
                    "Sending event frame thread=%s frame_bytes=%d",
                    thread_name,
                    len(event),
                )
                yield event
                next_keepalive = monotonic_ns() + STREAM_KEEPALIVE_NS
        finally:
            disconnected.set()
            resource.scheduler.cancel(job.id)
            try:
                events.get_nowait()
            except Empty:
                pass
            stream_logger.info("Event stream closed thread=%s", thread_name)

    return StreamingResponse(
        generate(),
        media_type=MESSAGE_PACK_STREAM,
        headers={"Cache-Control": "no-store"},
    )
