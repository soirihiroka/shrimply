import logging
import os
import shutil
import tempfile
import threading
from collections.abc import Callable
from dataclasses import dataclass, field
from multiprocessing.connection import Connection
from multiprocessing.process import BaseProcess
from pathlib import Path
from time import monotonic_ns
from typing import Literal, Protocol
from uuid import UUID

import env
from pydantic import BaseModel

from api import gpu

type Service = Literal[
    "3dtracking", "pneuma", "sam2", "stt", "tts", "video_generation"
]
type WorkerState = Literal[
    "reserved", "loading", "ready", "running", "decoding", "streaming", "stopped"
]

JOB_HEADER = "Shrimply-Job-ID"
HEARTBEAT_TIMEOUT_SECONDS = 30
NANOSECONDS_PER_SECOND = 1_000_000_000
HEARTBEAT_TIMEOUT_NS = HEARTBEAT_TIMEOUT_SECONDS * NANOSECONDS_PER_SECOND
WORKER_TERMINATION_GRACE_SECONDS = 5
WORKER_POLL_SECONDS = 0.25
TRANSFER_CHUNK_BYTES = 1024**2
MAXIMUM_TEMPORARY_BYTES = 16 * 1024**3


@dataclass(frozen=True, slots=True)
class WorkerKey:
    service: Service
    model: str
    configuration: tuple[tuple[str, str], ...] = ()


@dataclass(frozen=True, slots=True)
class ResourceProfile:
    resident_ram: int = 0
    resident_vram: int = 0
    active_ram: int = 0
    active_vram: int = 0
    cpu_slots: int = 1

    def for_device(self, device: str) -> "ResourceProfile":
        if device.startswith("cuda:"):
            return self
        return ResourceProfile(
            resident_ram=self.resident_ram + self.resident_vram,
            active_ram=self.active_ram + self.active_vram,
            cpu_slots=self.cpu_slots,
        )


class ComputeWorker(Protocol):
    key: WorkerKey
    profile: ResourceProfile
    state: WorkerState
    reusable: bool

    def healthy(self) -> bool: ...

    def force_stop(self) -> None: ...


class QueuedEvent(BaseModel):
    kind: Literal["queued"] = "queued"
    position: int


class InvalidJobId(ValueError):
    pass


class DuplicateJobId(FileExistsError):
    pass


class TemporaryStorageUnavailable(OSError):
    pass


class JobCancelled(InterruptedError):
    def __init__(self, code: Literal["cancelled", "heartbeat_expired"]):
        self.code = code
        super().__init__(
            "Compute job cancelled"
            if code == "cancelled"
            else "Compute job heartbeat expired"
        )


@dataclass(slots=True)
class ManagedJob:
    id: str
    cancellation: threading.Event = field(default_factory=threading.Event)
    heartbeat_deadline: int = field(
        default_factory=lambda: monotonic_ns() + HEARTBEAT_TIMEOUT_NS
    )
    cancel_code: Literal["cancelled", "heartbeat_expired"] | None = None
    state: str = "accepted"
    temporary_bytes: int = 0
    _record: "_JobRecord | None" = None

    def check_cancelled(self) -> None:
        if self.cancellation.is_set():
            raise JobCancelled(self.cancel_code or "cancelled")


@dataclass(slots=True)
class _WorkerRecord:
    id: int
    key: WorkerKey
    profile: ResourceProfile
    worker: ComputeWorker | None
    busy_job: str | None
    cohort_cutoff: int
    pending_ram: int = 0
    pending_vram: int = 0
    idle_timer: threading.Timer | None = None


@dataclass(frozen=True, slots=True)
class _RetiredWorker:
    record: _WorkerRecord
    worker: ComputeWorker
    active: bool


@dataclass(slots=True)
class _JobRecord:
    job: ManagedJob
    key: WorkerKey
    profile: ResourceProfile
    factory: Callable[[], ComputeWorker]
    queued: Callable[[QueuedEvent], None]
    sequence: int
    bypassed: bool = False
    assignment: _WorkerRecord | None = None
    admitted: threading.Event = field(default_factory=threading.Event)
    retry_count: int = 0
    failure: RuntimeError | None = None


@dataclass(slots=True)
class WorkerLease:
    scheduler: "Scheduler"
    job: ManagedJob
    record: _WorkerRecord

    @property
    def worker(self) -> ComputeWorker:
        if self.record.worker is None:
            raise RuntimeError("Compute worker was not attached")
        return self.record.worker

    def set_state(self, state: WorkerState) -> None:
        self.scheduler.set_state(self.job, state)

    def complete(self) -> None:
        self.scheduler.finish_work(self.job, reuse=True)

    def discard(self) -> None:
        self.scheduler.finish_work(self.job, reuse=False)

    def retry_after_oom(self) -> "WorkerLease":
        return self.scheduler.retry_after_oom(self.job)


logger = logging.getLogger("shrimply.resource")
CGROUP_MEMORY_MAX = Path("/sys/fs/cgroup/memory.max")
CGROUP_MEMORY_CURRENT = Path("/sys/fs/cgroup/memory.current")
CGROUP_MEMORY_STAT = Path("/sys/fs/cgroup/memory.stat")
MEMINFO = Path("/proc/meminfo")
ZFS_ARCSTATS = Path("/proc/spl/kstat/zfs/arcstats")


class Scheduler:
    def __init__(self) -> None:
        self.lock = threading.RLock()
        self.changed = threading.Condition(self.lock)
        self.jobs: dict[str, ManagedJob] = {}
        self.cancelled_ids: dict[str, int] = {}
        self.queue: list[_JobRecord] = []
        self.workers: list[_WorkerRecord] = []
        self._failed_retirements: list[_RetiredWorker] = []
        self._sequence = 0
        self._worker_id = 0
        self._reserved_ram = 0
        self._reserved_vram = 0
        self._pending_ram = 0
        self._pending_vram = 0
        self._cpu_slots = 0
        self._temporary_bytes = 0
        self._temporary_capacity: int | None = None
        self._factories_loading = 0
        self._workers_stopping = 0
        self._switching_device = False
        self._stopping = False
        self._watchdog = threading.Thread(
            target=self._watch_heartbeats,
            name="compute-heartbeats",
            daemon=True,
        )
        self._watchdog.start()

    def register(self, raw_id: str | None) -> ManagedJob:
        if raw_id is None:
            raise InvalidJobId(f"Missing {JOB_HEADER} header")
        try:
            job_id = str(UUID(raw_id))
        except (ValueError, AttributeError) as exception:
            raise InvalidJobId(f"Malformed {JOB_HEADER} header") from exception
        with self.changed:
            while self._switching_device and not self._stopping:
                self.changed.wait()
            if self._stopping:
                raise RuntimeError("Compute scheduler is shutting down")
            now = monotonic_ns()
            self._prune_cancelled_ids_locked(now)
            if job_id in self.jobs:
                raise DuplicateJobId(f"Compute job {job_id} is already active")
            if self.cancelled_ids.get(job_id, 0) > now:
                raise JobCancelled("cancelled")
            job = ManagedJob(job_id)
            self.jobs[job_id] = job
            return job

    def heartbeat(self, raw_id: str) -> bool:
        try:
            job_id = str(UUID(raw_id))
        except ValueError:
            return False
        with self.lock:
            job = self.jobs.get(job_id)
            if job is None or job.cancellation.is_set():
                return False
            job.heartbeat_deadline = monotonic_ns() + HEARTBEAT_TIMEOUT_NS
            self._schedule_locked()
            return True

    def cancel(
        self,
        raw_id: str,
        code: Literal["cancelled", "heartbeat_expired"] = "cancelled",
        heartbeat_deadline: int | None = None,
    ) -> bool:
        try:
            job_id = str(UUID(raw_id))
        except ValueError:
            return False
        failure: BaseException | None = None
        with self.lock:
            now = monotonic_ns()
            self._prune_cancelled_ids_locked(now)
            job = self.jobs.get(job_id)
            if code == "heartbeat_expired" and (
                job is None
                or heartbeat_deadline != job.heartbeat_deadline
                or job.heartbeat_deadline > now
            ):
                return False
            self.cancelled_ids[job_id] = now + HEARTBEAT_TIMEOUT_NS
            if job is None:
                return False
            if not job.cancellation.is_set():
                job.cancel_code = code
                job.cancellation.set()
            record = job._record
            if record is not None and record in self.queue:
                self.queue.remove(record)
                job.state = code
                record.admitted.set()
                self._notify_positions_locked()
                self._schedule_locked()
            elif record is not None and record.assignment is not None:
                job.state = code
                record.admitted.set()
                if record.assignment.worker is not None:
                    retired = self._retire_worker_locked(record.assignment)
                    if retired is not None:
                        failure = self._stop_retired_unlocked([retired])
                        self._clear_barriers_locked()
                        self._schedule_locked()
            self.changed.notify_all()
        if failure is not None:
            raise failure
        return True

    def unregister(self, job: ManagedJob) -> None:
        failure: BaseException | None = None
        resources_changed = False
        with self.lock:
            record = job._record
            if record is not None and record in self.queue:
                self.queue.remove(record)
                record.admitted.set()
            elif record is not None and record.assignment is not None:
                if record.assignment.worker is None:
                    self._discard_worker_locked(record.assignment)
                else:
                    retired = self._retire_worker_locked(record.assignment)
                    if retired is not None:
                        failure = self._stop_retired_unlocked([retired])
                resources_changed = True
            job._record = None
            self._temporary_bytes -= job.temporary_bytes
            job.temporary_bytes = 0
            if not self._temporary_bytes:
                self._temporary_capacity = None
            self.jobs.pop(job.id, None)
            self._notify_positions_locked()
            if resources_changed:
                self._clear_barriers_locked()
            self._schedule_locked()
            self.changed.notify_all()
        if failure is not None:
            raise failure

    def acquire(
        self,
        job: ManagedJob,
        key: WorkerKey,
        profile: ResourceProfile,
        factory: Callable[[], ComputeWorker],
        queued: Callable[[QueuedEvent], None],
    ) -> WorkerLease:
        profile = profile.for_device(gpu.device)
        with self.lock:
            job.check_cancelled()
            if self.jobs.get(job.id) is not job or job._record is not None:
                raise RuntimeError("Compute job cannot be queued twice")
            if not self._possible_locked(profile):
                raise RuntimeError(
                    f"Compute job {key.service}:{key.model} exceeds server capacity"
                )
            self._sequence += 1
            record = _JobRecord(job, key, profile, factory, queued, self._sequence)
            job._record = record
            job.state = "queued"
            self.queue.append(record)
            self._notify_positions_locked()
            self._schedule_locked()
        return self._wait_for_assignment(record)

    def reserve_temporary(self, job: ManagedJob, size: int) -> None:
        with self.lock:
            job.check_cancelled()
            if size <= 0:
                raise ValueError("Temporary storage reservation must be positive")
            if self.jobs.get(job.id) is not job or job.temporary_bytes:
                raise RuntimeError("Compute job cannot reserve temporary storage")
            free = shutil.disk_usage(tempfile.gettempdir()).free
            capacity = self._temporary_capacity or min(MAXIMUM_TEMPORARY_BYTES, free)
            if (
                self._temporary_bytes + size > capacity
                or size > free
            ):
                raise TemporaryStorageUnavailable(
                    "Compute server has insufficient temporary storage"
                )
            self._temporary_capacity = capacity
            job.temporary_bytes = size
            self._temporary_bytes += size

    def _wait_for_assignment(self, record: _JobRecord) -> WorkerLease:
        while True:
            record.admitted.wait()
            record.job.check_cancelled()
            with self.lock:
                if record.failure is not None:
                    raise record.failure
                assignment = record.assignment
                if assignment is None:
                    record.job.check_cancelled()
                    record.admitted.clear()
                    continue
                if assignment.worker is not None:
                    return WorkerLease(self, record.job, assignment)
                self._factories_loading += 1
            try:
                try:
                    worker = record.factory()
                except Exception:
                    with self.lock:
                        self._discard_worker_locked(assignment)
                        record.assignment = None
                        record.job._record = None
                        self._schedule_locked()
                    raise
                stop = False
                retired: _RetiredWorker | None = None
                with self.lock:
                    if record.job.cancellation.is_set() or assignment not in self.workers:
                        stop = True
                    else:
                        if (
                            worker.key != record.key
                            or worker.profile.for_device(gpu.device) != record.profile
                        ):
                            stop = True
                    if stop and assignment in self.workers:
                        assignment.worker = worker
                        retired = self._retire_worker_locked(assignment)
                        record.assignment = None
                        record.job._record = None
                    elif not stop:
                        assignment.worker = worker
                        worker.state = "loading"
                        return WorkerLease(self, record.job, assignment)
                if retired is not None:
                    with self.lock:
                        failure = self._stop_retired_unlocked([retired])
                        self._clear_barriers_locked()
                        self._schedule_locked()
                    if failure is not None:
                        raise failure
                else:
                    worker.force_stop()
                if stop:
                    record.job.check_cancelled()
                    raise RuntimeError("Compute worker does not match its reservation")
            finally:
                with self.changed:
                    self._factories_loading -= 1
                    self.changed.notify_all()

    def finish_work(self, job: ManagedJob, reuse: bool) -> None:
        failure: BaseException | None = None
        with self.lock:
            record = job._record
            if record is None or record.assignment is None:
                return
            worker_record = record.assignment
            worker = worker_record.worker
            record.assignment = None
            job._record = None
            if (
                reuse
                and worker is not None
                and worker.reusable
                and worker.healthy()
                and not job.cancellation.is_set()
            ):
                self._release_pending_locked(worker_record)
                self._release_active_locked(worker_record)
                worker_record.busy_job = None
                worker.state = "ready"
                job.state = "completed"
                self._assign_cohort_locked(worker_record)
                if worker_record.busy_job is None:
                    self._schedule_idle_expiration_locked(worker_record)
            else:
                retired = self._retire_worker_locked(worker_record)
                if retired is not None:
                    failure = self._stop_retired_unlocked([retired])
                job.state = job.cancel_code or "completed"
            self._clear_barriers_locked()
            self._schedule_locked()
            self.changed.notify_all()
        if failure is not None:
            raise failure

    def retry_after_oom(self, job: ManagedJob) -> WorkerLease:
        with self.lock:
            record = job._record
            if record is None or record.assignment is None:
                job.check_cancelled()
                raise RuntimeError("Compute job has no worker to retry")
            if record.retry_count:
                raise RuntimeError("Compute job already retried after CUDA OOM")
            record.retry_count = 1
            failed = self._retire_worker_locked(record.assignment)
            record.assignment = None
            stopped = [failed] if failed is not None else []
            for idle in sorted(
                [candidate for candidate in self.workers if candidate.busy_job is None],
                key=lambda candidate: (
                    candidate.profile.resident_ram + candidate.profile.resident_vram
                ),
                reverse=True,
            ):
                victim = self._retire_worker_locked(idle)
                if victim is not None:
                    stopped.append(victim)
            record.admitted.clear()
            record.job.state = "queued"
            self.queue.insert(0, record)
            self._clear_barriers_locked()
            self._notify_positions_locked()
            failure = self._stop_retired_unlocked(stopped)
            job.check_cancelled()
            self._schedule_locked()
            if failure is not None:
                raise failure
        return self._wait_for_assignment(record)

    def set_state(self, job: ManagedJob, state: WorkerState) -> None:
        with self.lock:
            record = job._record
            if record is None or record.assignment is None:
                job.check_cancelled()
                return
            job.state = state
            if state not in {"reserved", "loading"}:
                self._release_pending_locked(record.assignment)
            if record.assignment.worker is not None:
                record.assignment.worker.state = state

    def select_device(self, selected: str) -> None:
        stopped: list[_RetiredWorker] = []
        with self.lock:
            if self._switching_device or self.jobs:
                raise BlockingIOError("Compute is busy")
            if selected == gpu.device:
                return
            self._switching_device = True
            for record in tuple(self.workers):
                retired = self._retire_worker_locked(record)
                if retired is not None:
                    stopped.append(retired)
            failure = self._stop_retired_unlocked(stopped)
            try:
                if failure is None and not self._stopping:
                    gpu.device = selected
            finally:
                self._switching_device = False
                self._schedule_locked()
                self.changed.notify_all()
        if failure is not None:
            raise failure

    def summary(self) -> dict[str, object]:
        with self.lock:
            grouped: dict[tuple[WorkerKey, str], int] = {}
            for record in self.workers:
                state = record.worker.state if record.worker is not None else "reserved"
                grouped[(record.key, state)] = grouped.get((record.key, state), 0) + 1
            for retired in self._failed_retirements:
                grouped[(retired.record.key, "stopped")] = (
                    grouped.get((retired.record.key, "stopped"), 0) + 1
                )
            return {
                "queued_jobs": len(self.queue),
                "active_jobs": sum(
                    job.state
                    not in {"queued", "completed", "failed", "cancelled", "heartbeat_expired"}
                    for job in self.jobs.values()
                ),
                "reserved_ram_bytes": self._reserved_ram,
                "reserved_vram_bytes": self._reserved_vram,
                "workers": [
                    {
                        "service": key.service,
                        "model": key.model,
                        "configuration": dict(key.configuration),
                        "state": state,
                        "copies": copies,
                    }
                    for (key, state), copies in sorted(
                        grouped.items(), key=lambda item: (item[0][0].service, item[0][0].model, item[0][1])
                    )
                ],
            }

    def shutdown(self) -> None:
        stopped: list[_RetiredWorker] = []
        with self.lock:
            self._stopping = True
            for job in tuple(self.jobs.values()):
                if not job.cancellation.is_set():
                    job.cancel_code = "cancelled"
                    job.cancellation.set()
            for queued in self.queue:
                queued.admitted.set()
            self.queue.clear()
            for worker_record in tuple(self.workers):
                if worker_record.worker is None:
                    self._discard_worker_locked(worker_record)
                else:
                    retired = self._retire_worker_locked(worker_record)
                    if retired is not None:
                        stopped.append(retired)
            for retired in self._failed_retirements:
                self._workers_stopping += 1
                stopped.append(retired)
            self._failed_retirements.clear()
            self.jobs.clear()
            self.cancelled_ids.clear()
            self._temporary_bytes = 0
            self._temporary_capacity = None
            failure = self._stop_retired_unlocked(stopped)
            self.changed.notify_all()
        with self.changed:
            while (
                self._factories_loading
                or self._workers_stopping
                or self._switching_device
            ):
                self.changed.wait()
        self._watchdog.join(timeout=WORKER_TERMINATION_GRACE_SECONDS)
        if failure is not None:
            raise failure

    def _schedule_locked(self) -> None:
        if self._stopping or self._workers_stopping:
            return
        for worker in tuple(self.workers):
            if (
                worker.busy_job is None
                and worker.worker is not None
                and not worker.worker.healthy()
            ):
                stopped = self._retire_worker_locked(worker)
                if stopped is not None:
                    failure = self._stop_retired_unlocked([stopped])
                    if failure is not None:
                        raise failure
                    self._clear_barriers_locked()
        for worker in tuple(self.workers):
            if worker.busy_job is None:
                self._assign_cohort_locked(worker)
        while self.queue:
            barrier = next((job for job in self.queue if job.bypassed), None)
            admitted = False
            for index, record in enumerate(tuple(self.queue)):
                if barrier is not None and record is not barrier:
                    break
                worker = self._idle_exact_locked(record.key, record.profile)
                if worker is not None:
                    worker.cohort_cutoff = max(
                        candidate.sequence
                        for candidate in self.queue
                        if candidate.key == record.key
                        and candidate.profile == record.profile
                    )
                    self._assign_locked(record, worker, new=False)
                    admitted = True
                    break
                if self._fits_locked(record.profile):
                    self._worker_id += 1
                    worker = _WorkerRecord(
                        self._worker_id,
                        record.key,
                        record.profile,
                        None,
                        record.job.id,
                        max(
                            (
                                candidate.sequence
                                for candidate in self.queue
                                if candidate.key == record.key
                                and candidate.profile == record.profile
                            ),
                            default=record.sequence,
                        ),
                        record.profile.resident_ram + record.profile.active_ram,
                        record.profile.resident_vram + record.profile.active_vram,
                    )
                    self.workers.append(worker)
                    self._pending_ram += worker.pending_ram
                    self._pending_vram += worker.pending_vram
                    self._reserve_locked(record.profile, include_resident=True)
                    self._assign_locked(record, worker, new=True)
                    admitted = True
                    break
                if (
                    self._cpu_slots + record.profile.cpu_slots
                    <= (os.process_cpu_count() or 1)
                    and self._evict_idle_for_locked(record.profile)
                ):
                    admitted = True
                    break
                if not self.workers and not self._factories_loading:
                    self.queue.remove(record)
                    record.job._record = None
                    record.job.state = "failed"
                    record.failure = RuntimeError(
                        f"Insufficient available memory for {record.key.service}:"
                        f"{record.key.model}. Choose a lower precision or smaller "
                        "model, or free memory on the selected device."
                    )
                    record.admitted.set()
                    self._clear_barriers_locked()
                    self._notify_positions_locked()
                    admitted = True
                    break
                if index == 0 and not record.bypassed:
                    record.bypassed = True
                    barrier = None
                    continue
                if index == 0:
                    return
            if not admitted:
                return
        self._notify_positions_locked()

    def _assign_cohort_locked(self, worker: _WorkerRecord) -> bool:
        if self._workers_stopping or worker.busy_job is not None:
            return False
        candidate = next(
            (
                job
                for job in self.queue
                if job.key == worker.key
                and job.profile == worker.profile
                and job.sequence <= worker.cohort_cutoff
            ),
            None,
        )
        if candidate is None or not self._fits_active_locked(worker.profile):
            return False
        self._assign_locked(candidate, worker, new=False)
        return True

    def _assign_locked(self, job: _JobRecord, worker: _WorkerRecord, new: bool) -> None:
        if not new:
            if worker.idle_timer is not None:
                worker.idle_timer.cancel()
                worker.idle_timer = None
            if worker.busy_job is None and worker.worker is not None:
                self._reserve_locked(worker.profile, include_resident=False)
                worker.pending_ram = worker.profile.active_ram
                worker.pending_vram = worker.profile.active_vram
                self._pending_ram += worker.pending_ram
                self._pending_vram += worker.pending_vram
        self.queue.remove(job)
        worker.busy_job = job.job.id
        job.assignment = worker
        job.job.state = "loading" if worker.worker is None else "running"
        job.admitted.set()
        self._notify_positions_locked()

    def _idle_exact_locked(
        self, key: WorkerKey, profile: ResourceProfile
    ) -> _WorkerRecord | None:
        return next(
            (
                worker
                for worker in self.workers
                if worker.busy_job is None
                and worker.key == key
                and worker.profile == profile
                and worker.worker is not None
                and worker.worker.healthy()
                and self._fits_active_locked(worker.profile)
            ),
            None,
        )

    def _evict_idle_for_locked(self, profile: ResourceProfile) -> bool:
        candidates = sorted(
            [worker for worker in self.workers if worker.busy_job is None],
            key=lambda worker: worker.profile.resident_ram + worker.profile.resident_vram,
            reverse=True,
        )
        evicted = False
        for candidate in candidates:
            if candidate not in self.workers or candidate.busy_job is not None:
                continue
            worker = self._retire_worker_locked(candidate)
            if worker is not None:
                failure = self._stop_retired_unlocked([worker])
                if failure is not None:
                    raise failure
            evicted = True
            self._clear_barriers_locked()
            if self._fits_locked(profile):
                break
        return evicted

    def _fits_locked(self, profile: ResourceProfile) -> bool:
        return self._fits_delta_locked(
            profile.resident_ram + profile.active_ram,
            profile.resident_vram + profile.active_vram,
            profile.cpu_slots,
        )

    def _fits_active_locked(self, profile: ResourceProfile) -> bool:
        return self._fits_delta_locked(
            profile.active_ram, profile.active_vram, profile.cpu_slots
        )

    def _possible_locked(self, profile: ResourceProfile) -> bool:
        if profile.cpu_slots > (os.process_cpu_count() or 1):
            return False
        memory_limit = total_memory_limit()
        if (
            memory_limit is not None
            and profile.resident_ram + profile.active_ram > memory_limit
        ):
            return False
        return not gpu.device.startswith("cuda:") or (
            profile.resident_vram + profile.active_vram <= gpu.total_vram()
        )

    def _fits_delta_locked(self, ram: int, vram: int, cpu: int) -> bool:
        cpu_capacity = os.process_cpu_count() or 1
        if self._cpu_slots + cpu > cpu_capacity:
            return False
        memory_limit = total_memory_limit()
        if memory_limit is not None and self._reserved_ram + ram > memory_limit:
            return False
        live_ram = available_ram()
        if live_ram is not None and live_ram < self._pending_ram + ram:
            return False
        if gpu.device.startswith("cuda:"):
            total_vram = gpu.total_vram()
            if (
                self._reserved_vram + vram > total_vram
                or gpu.available_vram() < self._pending_vram + vram
            ):
                return False
        return True

    def _reserve_locked(self, profile: ResourceProfile, include_resident: bool) -> None:
        if include_resident:
            self._reserved_ram += profile.resident_ram
            self._reserved_vram += profile.resident_vram
        self._reserved_ram += profile.active_ram
        self._reserved_vram += profile.active_vram
        self._cpu_slots += profile.cpu_slots

    def _release_active_locked(self, worker: _WorkerRecord) -> None:
        self._reserved_ram -= worker.profile.active_ram
        self._reserved_vram -= worker.profile.active_vram
        self._cpu_slots -= worker.profile.cpu_slots

    def _release_pending_locked(self, worker: _WorkerRecord) -> None:
        self._pending_ram -= worker.pending_ram
        self._pending_vram -= worker.pending_vram
        worker.pending_ram = 0
        worker.pending_vram = 0

    def _discard_worker_locked(self, record: _WorkerRecord) -> None:
        if record not in self.workers:
            return
        if record.worker is not None:
            raise RuntimeError("Attached compute workers must be stopped before release")
        if record.idle_timer is not None:
            record.idle_timer.cancel()
            record.idle_timer = None
        self.workers.remove(record)
        self._release_pending_locked(record)
        self._reserved_ram -= record.profile.resident_ram
        self._reserved_vram -= record.profile.resident_vram
        if record.busy_job is not None:
            self._release_active_locked(record)
            job = self.jobs.get(record.busy_job)
            if job is not None and job._record is not None:
                job._record.assignment = None
        record.busy_job = None

    def _retire_worker_locked(self, record: _WorkerRecord) -> _RetiredWorker | None:
        if record not in self.workers or record.worker is None:
            return None
        if record.idle_timer is not None:
            record.idle_timer.cancel()
            record.idle_timer = None
        self.workers.remove(record)
        active = record.busy_job is not None
        if record.busy_job is not None:
            job = self.jobs.get(record.busy_job)
            if job is not None and job._record is not None:
                job._record.assignment = None
        worker, record.worker = record.worker, None
        record.busy_job = None
        self._workers_stopping += 1
        return _RetiredWorker(record, worker, active)

    def _stop_retired_unlocked(
        self, workers: list[_RetiredWorker]
    ) -> BaseException | None:
        if not workers:
            return None
        failure: BaseException | None = None
        stopped: list[_RetiredWorker] = []
        failed: list[_RetiredWorker] = []
        self.lock.release()
        try:
            for retired in workers:
                try:
                    retired.worker.force_stop()
                    stopped.append(retired)
                except BaseException as exception:
                    failed.append(retired)
                    if failure is None:
                        failure = exception
        finally:
            self.lock.acquire()
            for retired in stopped:
                record = retired.record
                self._release_pending_locked(record)
                self._reserved_ram -= record.profile.resident_ram
                self._reserved_vram -= record.profile.resident_vram
                if retired.active:
                    self._release_active_locked(record)
            self._failed_retirements.extend(failed)
            for _ in workers:
                self._workers_stopping -= 1
            self.changed.notify_all()
        return failure

    def _schedule_idle_expiration_locked(self, worker: _WorkerRecord) -> None:
        timer = threading.Timer(
            env.MODEL_IDLE_TTL_SECONDS, self._expire_idle_worker, (worker.id,)
        )
        timer.daemon = True
        worker.idle_timer = timer
        timer.start()

    def _expire_idle_worker(self, worker_id: int) -> None:
        failure: BaseException | None = None
        with self.lock:
            worker = next(
                (candidate for candidate in self.workers if candidate.id == worker_id), None
            )
            if worker is None or worker.busy_job is not None:
                return
            retired = self._retire_worker_locked(worker)
            if retired is not None:
                failure = self._stop_retired_unlocked([retired])
            self._clear_barriers_locked()
            self._schedule_locked()
        if failure is not None:
            raise failure

    def _notify_positions_locked(self) -> None:
        while True:
            disconnected: list[_JobRecord] = []
            for position, record in enumerate(self.queue, 1):
                try:
                    record.queued(QueuedEvent(position=position))
                except (OSError, ValueError):
                    if not record.job.cancellation.is_set():
                        record.job.cancel_code = "cancelled"
                        record.job.cancellation.set()
                    record.job.state = record.job.cancel_code or "cancelled"
                    record.admitted.set()
                    disconnected.append(record)
            if not disconnected:
                return
            for record in disconnected:
                if record in self.queue:
                    self.queue.remove(record)

    def _clear_barriers_locked(self) -> None:
        for job in self.queue:
            job.bypassed = False

    def _prune_cancelled_ids_locked(self, now: int) -> None:
        self.cancelled_ids = {
            job_id: deadline
            for job_id, deadline in self.cancelled_ids.items()
            if deadline > now
        }

    def _watch_heartbeats(self) -> None:
        while True:
            with self.changed:
                if self._stopping:
                    return
                now = monotonic_ns()
                self._prune_cancelled_ids_locked(now)
                expired = [
                    (job.id, job.heartbeat_deadline)
                    for job in self.jobs.values()
                    if not job.cancellation.is_set() and job.heartbeat_deadline <= now
                ]
                next_deadline = min(
                    (
                        job.heartbeat_deadline
                        for job in self.jobs.values()
                        if not job.cancellation.is_set()
                    ),
                    default=now + HEARTBEAT_TIMEOUT_NS,
                )
            for job_id, heartbeat_deadline in expired:
                if self.cancel(job_id, "heartbeat_expired", heartbeat_deadline):
                    logger.warning("Compute job heartbeat expired job_id=%s", job_id)
            with self.changed:
                if self._stopping:
                    return
                wait_ns = max(0, next_deadline - monotonic_ns())
                self.changed.wait(timeout=wait_ns / NANOSECONDS_PER_SECOND)


def force_stop_process(
    process: BaseProcess | None, connection: Connection | None = None
) -> None:
    failure: BaseException | None = None
    if connection is not None:
        try:
            connection.close()
        except BaseException as exception:
            failure = exception
    if process is None:
        if failure is not None:
            raise failure
        return
    try:
        process.join(timeout=0)
        if process.is_alive():
            process.terminate()
            process.join(timeout=WORKER_TERMINATION_GRACE_SECONDS)
    except BaseException as exception:
        failure = failure or exception
    try:
        if process.is_alive():
            process.kill()
            process.join()
    except BaseException as exception:
        failure = failure or exception
    try:
        alive = process.is_alive()
    except BaseException as exception:
        raise RuntimeError("Could not verify compute worker termination") from exception
    if alive:
        raise RuntimeError(f"Compute worker process {process.pid} did not stop") from failure
    if failure is not None:
        logger.warning(
            "Compute worker pid=%s required termination fallback: %s",
            process.pid,
            failure,
        )


def total_memory_limit() -> int | None:
    system_total = None
    try:
        values = dict(
            line.rstrip(" kB").split(":", maxsplit=1)
            for line in MEMINFO.read_text(encoding="utf-8").splitlines()
        )
        system_total = int(values["MemTotal"].strip()) * 1024
    except (OSError, KeyError, ValueError):
        pass
    try:
        limit = CGROUP_MEMORY_MAX.read_text(encoding="utf-8").strip()
        if limit == "max":
            return system_total
        cgroup_limit = int(limit)
        return cgroup_limit if system_total is None else min(cgroup_limit, system_total)
    except (OSError, ValueError):
        return system_total


def available_ram() -> int | None:
    system_available = None
    try:
        values = dict(
            line.rstrip(" kB").split(":", maxsplit=1)
            for line in MEMINFO.read_text(encoding="utf-8").splitlines()
        )
        system_available = int(values["MemAvailable"].strip()) * 1024
        try:
            arc = {
                fields[0]: int(fields[2])
                for line in ZFS_ARCSTATS.read_text(encoding="utf-8").splitlines()
                if len(fields := line.split()) == 3 and fields[2].isdigit()
            }
            system_available = min(
                int(values["MemTotal"].strip()) * 1024,
                system_available + max(0, arc["size"] - arc["c_min"]),
            )
        except (OSError, KeyError, ValueError):
            pass
    except (OSError, KeyError, ValueError):
        pass
    try:
        limit_text = CGROUP_MEMORY_MAX.read_text(encoding="utf-8").strip()
        if limit_text == "max":
            return system_available
        try:
            statistics = dict(
                line.split(maxsplit=1)
                for line in CGROUP_MEMORY_STAT.read_text(encoding="utf-8").splitlines()
            )
            used = (
                int(statistics["anon"])
                + int(statistics["kernel"])
                + int(statistics["shmem"])
            )
        except (OSError, KeyError, ValueError):
            used = int(CGROUP_MEMORY_CURRENT.read_text(encoding="utf-8").strip())
        cgroup_available = max(0, int(limit_text) - used)
        return (
            cgroup_available
            if system_available is None
            else min(cgroup_available, system_available)
        )
    except (OSError, KeyError, ValueError):
        return system_available


scheduler = Scheduler()


def register_job(raw_id: str | None) -> ManagedJob:
    return scheduler.register(raw_id)


def heartbeat(job_id: str) -> bool:
    return scheduler.heartbeat(job_id)


def cancel(job_id: str) -> bool:
    return scheduler.cancel(job_id)


def finish_job(job: ManagedJob) -> None:
    scheduler.unregister(job)


def shutdown_all() -> None:
    scheduler.shutdown()
