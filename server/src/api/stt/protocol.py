from typing import Annotated, Literal, get_args

import msgspec
from pydantic import BaseModel, ConfigDict, Field, TypeAdapter, model_validator

from api.media import AudioUpload

type ModelId = Literal[
    "nvidia/parakeet-tdt-0.6b-v3",
    "Qwen/Qwen3-ASR-0.6B-hf",
    "openai/whisper-large-v3-turbo",
    "openai/whisper-small",
    "distil-whisper/distil-large-v3",
]
type WorkerState = Literal["stopped", "loading", "transcribing", "ready"]

MODEL_IDS = get_args(ModelId.__value__)
MODEL_ID_VALIDATOR = TypeAdapter(ModelId)
SAMPLE_RATE = 16_000


class TranscriptionRequest(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True)

    model: ModelId
    audio: AudioUpload


TRANSCRIPTION_REQUEST_VALIDATOR = TypeAdapter(TranscriptionRequest)


class Segment(BaseModel):
    start_frame: int = Field(ge=0)
    end_frame: int = Field(ge=0)
    text: str = Field(min_length=1)

    @model_validator(mode="after")
    def validate_range(self) -> "Segment":
        if self.end_frame < self.start_frame:
            raise ValueError("Segment ends before it starts")
        return self


class Transcription(BaseModel):
    segments: list[Segment]


class ProgressEvent(BaseModel):
    kind: Literal["progress"] = "progress"
    message: str
    model: ModelId
    state: WorkerState


class ResultEvent(BaseModel):
    kind: Literal["result"] = "result"
    result: Transcription


class ErrorEvent(BaseModel):
    kind: Literal["error"] = "error"
    code: str
    message: str


class QueuedEvent(BaseModel):
    kind: Literal["queued"] = "queued"
    position: int = Field(ge=1)


type WorkerEvent = Annotated[
    ProgressEvent | ResultEvent | ErrorEvent, Field(discriminator="kind")
]
type StreamEvent = Annotated[
    QueuedEvent | ProgressEvent | ResultEvent | ErrorEvent,
    Field(discriminator="kind"),
]
WORKER_EVENT_VALIDATOR = TypeAdapter(WorkerEvent)


def encode_event(event: WorkerEvent) -> bytes:
    return msgspec.msgpack.encode(event.model_dump(mode="python"))


def decode_event(payload: bytes) -> WorkerEvent:
    return WORKER_EVENT_VALIDATOR.validate_python(msgspec.msgpack.decode(payload))
