import logging
import multiprocessing
from collections.abc import Callable
from dataclasses import dataclass
from multiprocessing.connection import Connection
from multiprocessing.process import BaseProcess

import msgspec

from api import gpu, resource
from api.tts import resources as tts_resources
from api.tts.protocol import (
    ErrorEvent,
    ModelId,
    Precision,
    ProgressEvent,
    ResultEvent,
    SynthesisRequest,
    WorkerEvent,
    WORKER_EVENT_VALIDATOR,
)
from api.tts.requests import request_precision

logger = logging.getLogger("shrimply.tts")
spawn_context = multiprocessing.get_context("spawn")


def run_worker(
    connection: Connection, model_id: ModelId, device: str, precision: Precision
) -> None:
    from api.tts.worker import run

    run(connection, model_id, device, precision)


@dataclass(slots=True)
class TtsWorker:
    key: resource.WorkerKey
    profile: resource.ResourceProfile
    process: BaseProcess
    connection: Connection
    state: resource.WorkerState = "loading"
    reusable: bool = True

    def healthy(self) -> bool:
        return self.process.is_alive() and not self.connection.closed

    def force_stop(self) -> None:
        pid = self.process.pid
        resource.force_stop_process(self.process, self.connection)
        self.state = "stopped"
        logger.info(
            "Stopped text-to-speech worker pid=%s exit_code=%s",
            pid,
            self.process.exitcode,
        )


def start_worker(
    key: resource.WorkerKey,
    profile: resource.ResourceProfile,
    model_id: ModelId,
    precision: Precision,
) -> TtsWorker:
    parent_connection, child_connection = spawn_context.Pipe()
    process = spawn_context.Process(
        target=run_worker,
        args=(child_connection, model_id, gpu.device, precision),
        name=f"tts:{model_id}",
    )
    try:
        process.start()
    except Exception:
        parent_connection.close()
        child_connection.close()
        raise
    child_connection.close()
    logger.info(
        "Started text-to-speech worker pid=%s model=%s precision=%s",
        process.pid,
        model_id,
        precision,
    )
    return TtsWorker(key, profile, process, parent_connection)


def _receive(worker: TtsWorker, job: resource.ManagedJob) -> WorkerEvent:
    while True:
        job.check_cancelled()
        if worker.connection.poll(resource.WORKER_POLL_SECONDS):
            return WORKER_EVENT_VALIDATOR.validate_python(
                msgspec.msgpack.decode(worker.connection.recv_bytes())
            )
        if not worker.healthy():
            raise RuntimeError("Text-to-speech worker stopped unexpectedly")


def synthesize(
    request: SynthesisRequest,
    job: resource.ManagedJob,
    send_event: Callable[[WorkerEvent | resource.QueuedEvent], None],
) -> None:
    precision = request_precision(request)
    key = resource.WorkerKey("tts", request.model, (("precision", precision),))
    dtype = gpu.inference_dtype(gpu.device, precision)
    profile = tts_resources.request(
        request.model,
        dtype,
        gpu.device.startswith("cuda:"),
    )
    payload = msgspec.msgpack.encode(request.model_dump(mode="python"))
    lease = resource.scheduler.acquire(
        job,
        key,
        profile,
        lambda: start_worker(key, profile, request.model, precision),
        send_event,
    )
    retried_after_oom = False
    try:
        while True:
            worker = lease.worker
            if not isinstance(worker, TtsWorker):
                raise TypeError("Scheduler returned an invalid text-to-speech worker")
            starting = worker.state == "loading"
            send_event(
                ProgressEvent(
                    message="Starting worker..." if starting else "Worker ready",
                    model=request.model,
                    state="loading" if starting else "ready",
                )
            )
            sent_request = False
            if not starting:
                job.check_cancelled()
                worker.connection.send_bytes(payload)
                lease.set_state("running")
                sent_request = True
            retry = False
            while True:
                event = _receive(worker, job)
                match event:
                    case ProgressEvent():
                        send_event(event)
                        logger.info(
                            "Text-to-speech worker pid=%s model=%s: %s",
                            worker.process.pid,
                            request.model,
                            event.message,
                        )
                        if event.state == "ready" and not sent_request:
                            job.check_cancelled()
                            worker.connection.send_bytes(payload)
                            lease.set_state("running")
                            sent_request = True
                    case ResultEvent():
                        lease.complete()
                        send_event(event)
                        logger.info(
                            "Text-to-speech worker pid=%s model=%s completed request",
                            worker.process.pid,
                            request.model,
                        )
                        return
                    case ErrorEvent():
                        if (
                            not retried_after_oom
                            and gpu.is_out_of_memory(event.message)
                        ):
                            retried_after_oom = True
                            lease = lease.retry_after_oom()
                            send_event(
                                ProgressEvent(
                                    message="Retrying after freeing GPU memory...",
                                    model=request.model,
                                    state="loading",
                                )
                            )
                            retry = True
                            break
                        send_event(event)
                        lease.discard()
                        logger.error(
                            "Text-to-speech worker pid=%s model=%s failed code=%s",
                            worker.process.pid,
                            request.model,
                            event.code,
                        )
                        return
            if retry:
                continue
    except Exception:
        lease.discard()
        job.check_cancelled()
        raise
