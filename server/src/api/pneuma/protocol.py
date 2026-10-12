from typing import Annotated, Literal

from pydantic import BaseModel, ConfigDict, Field, TypeAdapter, model_validator

from api.media import AudioUpload

type PitchMethod = Literal["crepe", "rmvpe", "fcpe", "swift-f0"]
type WorkerState = Literal["stopped", "loading", "converting", "ready"]


class ProtocolModel(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True)


class ModelMetadata(ProtocolModel):
    experiment_name: str | None = None
    version: str | None = None
    saved_at: str | None = None


class ModelInfo(ProtocolModel):
    name: str = Field(min_length=1)
    metadata: ModelMetadata = ModelMetadata()


class ModelsResponse(ProtocolModel):
    models: list[ModelInfo]


class ConversionRequest(ProtocolModel):
    model: str = Field(min_length=1, max_length=255)
    audio: bytes | AudioUpload
    file_name: str = Field(default="audio", min_length=1, max_length=255)
    pitch_offset: int = Field(default=0, ge=-32, le=32)
    f0_method: PitchMethod = "rmvpe"
    speed: float = Field(default=1.0, ge=0.5, le=2.0)
    maintain_pitch: bool = True

    @model_validator(mode="after")
    def validate_audio_size(self) -> "ConversionRequest":
        data = self.audio if isinstance(self.audio, bytes) else self.audio.data
        if not 0 < len(data) <= 64 * 1024 * 1024:
            raise ValueError("Audio must be between 1 byte and 64 MiB")
        return self


class ConvertedAudio(ProtocolModel):
    wav: bytes = Field(min_length=44)


class ProgressEvent(ProtocolModel):
    kind: Literal["progress"] = "progress"
    message: str
    model: str
    state: WorkerState


class ResultEvent(ProtocolModel):
    kind: Literal["result"] = "result"
    result: ConvertedAudio


class ErrorEvent(ProtocolModel):
    kind: Literal["error"] = "error"
    code: str
    message: str


class QueuedEvent(ProtocolModel):
    kind: Literal["queued"] = "queued"
    position: int = Field(ge=1)


type WorkerEvent = Annotated[
    ProgressEvent | ResultEvent | ErrorEvent, Field(discriminator="kind")
]
type StreamEvent = Annotated[
    QueuedEvent | ProgressEvent | ResultEvent | ErrorEvent,
    Field(discriminator="kind"),
]

CONVERSION_REQUEST_VALIDATOR = TypeAdapter(ConversionRequest)
WORKER_EVENT_VALIDATOR = TypeAdapter(WorkerEvent)
