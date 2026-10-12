from typing import Annotated, Literal

import msgspec
from pydantic import BaseModel, ConfigDict, Field, TypeAdapter, model_validator

type ModelId = Literal[
    "facebook/sam2.1-hiera-tiny",
    "facebook/sam2.1-hiera-small",
    "facebook/sam2.1-hiera-base-plus",
    "facebook/sam2.1-hiera-large",
]
type WorkerState = Literal["stopped", "loading", "analyzing", "ready"]

MODEL_IDS = (
    "facebook/sam2.1-hiera-tiny",
    "facebook/sam2.1-hiera-small",
    "facebook/sam2.1-hiera-base-plus",
    "facebook/sam2.1-hiera-large",
)
ARCHIVE_MAGIC = b"SHRMSA01"
ARCHIVE_VERSION = 1
MASK_BYTES = 256 * 256
MAXIMUM_ARCHIVE_BYTES = 8 * 1024**3
MAXIMUM_FRAME_BYTES = 16 * 1024**2


class ProtocolModel(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True)


class Point(ProtocolModel):
    x: float = Field(ge=0, le=1)
    y: float = Field(ge=0, le=1)
    label: Literal[0, 1]


class Box(ProtocolModel):
    minimum: list[float] = Field(min_length=2, max_length=2)
    maximum: list[float] = Field(min_length=2, max_length=2)

    @model_validator(mode="after")
    def validate_coordinates(self) -> "Box":
        values = (*self.minimum, *self.maximum)
        if any(value < 0 or value > 1 for value in values):
            raise ValueError("Box coordinates must be normalized")
        if self.minimum[0] > self.maximum[0] or self.minimum[1] > self.maximum[1]:
            raise ValueError("Box minimum must not exceed maximum")
        return self


class AnalysisRequest(ProtocolModel):
    version: Literal[1]
    model: ModelId
    frame_count: int = Field(gt=0)
    seed_frame: int = Field(ge=0)
    points: list[Point]
    box: Box | None = None

    @model_validator(mode="after")
    def validate_request(self) -> "AnalysisRequest":
        if self.seed_frame >= self.frame_count:
            raise ValueError("Seed frame is outside the proxy clip")
        if not self.points and self.box is None:
            raise ValueError("At least one point or box is required")
        return self


ANALYSIS_REQUEST_VALIDATOR = TypeAdapter(AnalysisRequest)


class ProgressEvent(ProtocolModel):
    kind: Literal["progress"] = "progress"
    message: str
    completed_frames: int = Field(ge=0)
    total_frames: int = Field(gt=0)


class QueuedEvent(ProtocolModel):
    kind: Literal["queued"] = "queued"
    position: int = Field(gt=0)


class MaskEvent(ProtocolModel):
    kind: Literal["mask"] = "mask"
    frame_index: int = Field(ge=0)
    mask: bytes = Field(min_length=MASK_BYTES, max_length=MASK_BYTES)


class ResultEvent(ProtocolModel):
    kind: Literal["result"] = "result"
    completed_frames: int = Field(gt=0)


class ErrorEvent(ProtocolModel):
    kind: Literal["error"] = "error"
    code: str
    message: str


type WorkerEvent = Annotated[
    ProgressEvent | MaskEvent | ResultEvent | ErrorEvent, Field(discriminator="kind")
]
type PublicEvent = QueuedEvent | WorkerEvent
WORKER_EVENT_VALIDATOR = TypeAdapter(WorkerEvent)


def encode_event(event: WorkerEvent) -> bytes:
    return msgspec.msgpack.encode(event.model_dump(mode="python"))


def decode_event(payload: bytes) -> WorkerEvent:
    return WORKER_EVENT_VALIDATOR.validate_python(msgspec.msgpack.decode(payload))
