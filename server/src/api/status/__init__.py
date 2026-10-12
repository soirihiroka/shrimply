from typing import Literal

import torch
from pydantic import BaseModel

from api import gpu, resource
from api.pneuma.catalog import models as pneuma_models
from api.sam2.protocol import MODEL_IDS as SAM2_MODEL_IDS
from api.stt.protocol import MODEL_IDS as STT_MODEL_IDS
from api.tracking_3d.protocol import MODEL_IDS as TRACKING_3D_MODEL_IDS
from api.tts.protocol import MODEL_IDS as TTS_MODEL_IDS
from api.video_generation.protocol import MODEL_IDS as VIDEO_GENERATION_MODEL_IDS


class ProtocolStatus(BaseModel):
    major: int
    minor: int


class ServerVersion(BaseModel):
    version: str
    git_hash: str
    git_short_hash: str


class DeviceStatus(BaseModel):
    id: str
    name: str
    total_memory_bytes: int | None


class TorchStatus(BaseModel):
    version: str
    cuda_runtime: str | None
    cuda_available: bool
    devices: list[DeviceStatus]
    selected_device: str


class WorkerStatus(BaseModel):
    service: str
    model: str
    configuration: dict[str, str]
    state: str
    copies: int


class ComputeStatus(BaseModel):
    queued_jobs: int
    active_jobs: int
    reserved_ram_bytes: int
    reserved_vram_bytes: int
    workers: list[WorkerStatus]


class ServerStatus(BaseModel):
    protocol: ProtocolStatus
    server: ServerVersion
    status: Literal["ok", "degraded"]
    capabilities: list[str]
    torch: TorchStatus
    compute: ComputeStatus


def server_status(version: str, git_hash: str) -> ServerStatus:
    cuda_available = torch.cuda.is_available()
    devices = [DeviceStatus(id="cpu", name="CPU", total_memory_bytes=None)]
    if cuda_available:
        for index in range(torch.cuda.device_count()):
            properties = torch.cuda.get_device_properties(index)
            devices.append(
                DeviceStatus(
                    id=f"cuda:{index}",
                    name=properties.name,
                    total_memory_bytes=properties.total_memory,
                )
            )
    video_generation_available = gpu.device.startswith("cuda:")
    return ServerStatus(
        protocol=ProtocolStatus(major=4, minor=1),
        server=ServerVersion(
            version=version,
            git_hash=git_hash,
            git_short_hash=git_hash[:8],
        ),
        status="ok",
        capabilities=[f"stt:{model_id}" for model_id in STT_MODEL_IDS]
        + [f"tts:{model_id}" for model_id in TTS_MODEL_IDS]
        + [f"sam2:{model_id}" for model_id in SAM2_MODEL_IDS]
        + [f"pneuma:{model.name}" for model in pneuma_models()]
        + [
            f"video-generation:{model_id}"
            for model_id in VIDEO_GENERATION_MODEL_IDS
            if video_generation_available
        ]
        + [
            f"3dtracking:{model_id}"
            for model_id in TRACKING_3D_MODEL_IDS
            if model_id != "MIT-SPARK/VGGT-SLAM" or gpu.device.startswith("cuda:")
        ],
        torch=TorchStatus(
            version=str(torch.__version__),
            cuda_runtime=torch.version.cuda,
            cuda_available=cuda_available,
            devices=devices,
            selected_device=gpu.device,
        ),
        compute=ComputeStatus.model_validate(resource.scheduler.summary()),
    )
