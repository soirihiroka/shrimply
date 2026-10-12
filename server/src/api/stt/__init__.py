import logging
import multiprocessing
from collections.abc import Callable
from dataclasses import dataclass
from multiprocessing.connection import Connection
from multiprocessing.process import BaseProcess
from pathlib import Path

import msgspec

from api import gpu, resource
from api.media import decode_audio
from api.stt import resources as stt_resources
from api.stt.protocol import (
    ErrorEvent,
    ModelId,
    ProgressEvent,
    ResultEvent,
    WorkerEvent,
    decode_event,
    TRANSCRIPTION_REQUEST_VALIDATOR,
    SAMPLE_RATE,
)

logger = logging.getLogger("shrimply.stt")
spawn_context = multiprocessing.get_context("spawn")


def run_worker(connection: Connection, model_id: ModelId, device: str) -> None:
    from api.stt.worker import run

    run(connection, model_id, device)


@dataclass(slots=True)
class SttWorker:
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
            "Stopped speech-to-text worker pid=%s exit_code=%s",
            pid,
            self.process.exitcode,
        )


def start_worker(
    key: resource.WorkerKey,
    profile: resource.ResourceProfile,
    model_id: ModelId,
) -> SttWorker:
    parent_connection, child_connection = spawn_context.Pipe()
    process = spawn_context.Process(
        target=run_worker,
        args=(child_connection, model_id, gpu.device),
        name=f"stt:{model_id}",
    )
    try:
        process.start()
    except Exception:
        parent_connection.close()
        child_connection.close()
        raise
    child_connection.close()
    logger.info("Started speech-to-text worker pid=%s model=%s", process.pid, model_id)
    return SttWorker(key, profile, process, parent_connection)


def _receive(worker: SttWorker, job: resource.ManagedJob) -> WorkerEvent:
    while True:
        job.check_cancelled()
        if worker.connection.poll(resource.WORKER_POLL_SECONDS):
            return decode_event(worker.connection.recv_bytes())
        if not worker.healthy():
            raise RuntimeError("Speech-to-text worker stopped unexpectedly")


def transcribe(
    model_id: ModelId,
    path: str,
    audio_bytes: int,
    job: resource.ManagedJob,
    send_event: Callable[[WorkerEvent | resource.QueuedEvent], None],
    *,
    message_pack: bool = False,
) -> None:
    key = resource.WorkerKey("stt", model_id)
    profile = stt_resources.request(model_id, message_pack=message_pack)
    lease = resource.scheduler.acquire(
        job,
        key,
        profile,
        lambda: start_worker(key, profile, model_id),
        send_event,
    )
    retried_after_oom = False
    try:
        buffer = bytearray()
        with Path(path).open("rb") as audio:
            while chunk := audio.read(resource.TRANSFER_CHUNK_BYTES):
                job.check_cancelled()
                buffer.extend(chunk)
        if len(buffer) != audio_bytes:
            raise RuntimeError("Speech-to-text upload changed before processing")
        payload: bytes | bytearray = buffer
        del buffer
        if message_pack:
            uploaded = TRANSCRIPTION_REQUEST_VALIDATOR.validate_python(
                msgspec.msgpack.decode(payload)
            )
            if uploaded.model != model_id:
                raise RuntimeError("Speech-to-text upload model changed before processing")
            send_event(ProgressEvent(
                message="Decoding and resampling audio...",
                model=model_id,
                state="transcribing",
            ))
            _, samples = decode_audio(
                uploaded.audio, sample_rate=SAMPLE_RATE, check_cancelled=job.check_cancelled
            )
            del uploaded, payload
            job.check_cancelled()
            payload = samples.astype("<f4", copy=False).tobytes()
            del samples
        while True:
            worker = lease.worker
            if not isinstance(worker, SttWorker):
                raise TypeError("Scheduler returned an invalid speech-to-text worker")
            starting = worker.state == "loading"
            send_event(
                ProgressEvent(
                    message="Starting worker..." if starting else "Worker ready",
                    model=model_id,
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
                            "Speech-to-text worker pid=%s model=%s: %s",
                            worker.process.pid,
                            model_id,
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
                            "Speech-to-text worker pid=%s model=%s completed request",
                            worker.process.pid,
                            model_id,
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
                                    model=model_id,
                                    state="loading",
                                )
                            )
                            retry = True
                            break
                        send_event(event)
                        lease.discard()
                        logger.error(
                            "Speech-to-text worker pid=%s model=%s failed: %s: %s",
                            worker.process.pid,
                            model_id,
                            event.code,
                            event.message,
                        )
                        return
            if retry:
                continue
    except Exception:
        lease.discard()
        job.check_cancelled()
        raise
