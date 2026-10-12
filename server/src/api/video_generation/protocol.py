from fractions import Fraction
from typing import Annotated, Literal, Self

from pydantic import BaseModel, ConfigDict, Field, TypeAdapter, model_validator

from api.media import AudioUpload

type ModelId = Literal[
    "MiniMaxAI/MiniMax-H3",
    "Inner-Reflections/MiniMax-H3-Looping-Sketch-Anime",
    "Wan-AI/Wan2.1-T2V-1.3B-Diffusers",
    "Wan-AI/Wan2.2-TI2V-5B-Diffusers",
]
type WorkerState = Literal[
    "stopped", "loading", "conditioning", "generating", "decoding", "streaming"
]
type WorkerStage = Literal["generation", "decode"]
type MediaKind = Literal["image", "video", "audio"]
MODEL_IDS: tuple[ModelId, ...] = (
    "MiniMaxAI/MiniMax-H3",
    "Inner-Reflections/MiniMax-H3-Looping-Sketch-Anime",
    "Wan-AI/Wan2.1-T2V-1.3B-Diffusers",
    "Wan-AI/Wan2.2-TI2V-5B-Diffusers",
)
MAXIMUM_MEDIA_BYTES = 256 * 1024 * 1024


class ProtocolModel(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True)


class Rational(ProtocolModel):
    numerator: int
    denominator: int = Field(gt=0)

    def fraction(self) -> Fraction:
        return Fraction(self.numerator, self.denominator)


class VisibleWhen(ProtocolModel):
    input: str = Field(min_length=1)
    values: list[str] = Field(min_length=1)


class SelectOption(ProtocolModel):
    value: str = Field(min_length=1)
    label: str = Field(min_length=1)


class TextInput(ProtocolModel):
    kind: Literal["text"] = "text"
    key: str = Field(min_length=1)
    label: str = Field(min_length=1)
    default: str = ""
    required: bool = False
    multiline: bool = False
    max_length: int = Field(default=4096, gt=0)
    visible_when: VisibleWhen | None = None


class SelectInput(ProtocolModel):
    kind: Literal["select"] = "select"
    key: str = Field(min_length=1)
    label: str = Field(min_length=1)
    options: list[SelectOption] = Field(min_length=1)
    default: str = Field(min_length=1)
    visible_when: VisibleWhen | None = None

    @model_validator(mode="after")
    def validate_options(self) -> Self:
        values = [option.value for option in self.options]
        if len(values) != len(set(values)):
            raise ValueError(f"Input {self.key} has duplicate options")
        if self.default not in values:
            raise ValueError(f"Default option {self.default!r} does not exist")
        return self


class NumberInput(ProtocolModel):
    kind: Literal["number"] = "number"
    key: str = Field(min_length=1)
    label: str = Field(min_length=1)
    default: Rational
    minimum: Rational
    maximum: Rational
    step: Rational
    presentation: Literal["number", "slider"] = "number"
    visible_when: VisibleWhen | None = None

    @model_validator(mode="after")
    def validate_range(self) -> Self:
        default = self.default.fraction()
        minimum = self.minimum.fraction()
        maximum = self.maximum.fraction()
        if self.step.numerator <= 0:
            raise ValueError("Number input step must be positive")
        if minimum > maximum or not minimum <= default <= maximum:
            raise ValueError("Number input default must be within its range")
        return self


class MediaInput(ProtocolModel):
    kind: Literal["media"] = "media"
    key: str = Field(min_length=1)
    label: str = Field(min_length=1)
    accepted: list[MediaKind] = Field(min_length=1)
    minimum_items: int = Field(ge=0)
    maximum_items: int = Field(gt=0, le=12)
    ordered: bool = False
    visible_when: VisibleWhen | None = None

    @model_validator(mode="after")
    def validate_limits(self) -> Self:
        if self.minimum_items > self.maximum_items:
            raise ValueError("Media input minimum exceeds maximum")
        if len(self.accepted) != len(set(self.accepted)):
            raise ValueError(f"Input {self.key} has duplicate media kinds")
        return self


type InputDefinition = Annotated[
    TextInput | SelectInput | NumberInput | MediaInput,
    Field(discriminator="kind"),
]


class TextValue(ProtocolModel):
    kind: Literal["text"]
    value: str


class SelectValue(ProtocolModel):
    kind: Literal["select"]
    value: str


class NumberValue(ProtocolModel):
    kind: Literal["number"]
    value: Rational


class Media(ProtocolModel):
    kind: MediaKind
    filename: str = Field(min_length=1, max_length=255)
    data: bytes | AudioUpload

    @model_validator(mode="after")
    def validate_data(self) -> Self:
        if not isinstance(self.data, bytes) and self.kind != "audio":
            raise ValueError("Typed audio buffers require audio media kind")
        data = self.data if isinstance(self.data, bytes) else self.data.data
        if not 0 < len(data) <= MAXIMUM_MEDIA_BYTES:
            raise ValueError("Media must be between 1 byte and 256 MiB")
        return self


class MediaValue(ProtocolModel):
    kind: Literal["media"]
    items: list[Media]


type InputValue = Annotated[
    TextValue | SelectValue | NumberValue | MediaValue,
    Field(discriminator="kind"),
]


class ModelOutput(ProtocolModel):
    video: bool = True
    audio: bool = False


class ModelInfo(ProtocolModel):
    id: ModelId
    label: str = Field(min_length=1)
    inputs: list[InputDefinition]
    output: ModelOutput

    @model_validator(mode="after")
    def validate_inputs(self) -> Self:
        keys = [value.key for value in self.inputs]
        if len(keys) != len(set(keys)):
            raise ValueError(f"Model {self.id} has duplicate input keys")
        inputs = {value.key: value for value in self.inputs}
        for value in self.inputs:
            condition = value.visible_when
            if condition is None:
                continue
            controller = inputs.get(condition.input)
            if not isinstance(controller, SelectInput):
                raise ValueError(f"Input {value.key} has an invalid visibility controller")
            options = {option.value for option in controller.options}
            if any(expected not in options for expected in condition.values):
                raise ValueError(f"Input {value.key} has an invalid visibility value")
        return self


class ModelsResponse(ProtocolModel):
    models: list[ModelInfo]


class GenerationRequest(ProtocolModel):
    model: ModelId
    inputs: dict[str, InputValue]


GENERATION_REQUEST_VALIDATOR = TypeAdapter(GenerationRequest)


class GenerationResult(ProtocolModel):
    duration: Rational
    frame_rate: Rational
    width: int = Field(gt=0)
    height: int = Field(gt=0)
    video_streams: int = Field(ge=0)
    audio_streams: int = Field(ge=0)


class ProgressEvent(ProtocolModel):
    kind: Literal["progress"] = "progress"
    message: str
    model: ModelId
    state: WorkerState


class QueuedEvent(ProtocolModel):
    kind: Literal["queued"] = "queued"
    position: int = Field(gt=0)


class OutputStartEvent(ProtocolModel):
    kind: Literal["output_start"] = "output_start"
    content_type: Literal["video/mp4"] = "video/mp4"
    bytes: int = Field(gt=0)


class OutputChunkEvent(ProtocolModel):
    kind: Literal["output_chunk"] = "output_chunk"
    data: bytes = Field(min_length=1, max_length=1024 * 1024)


class ResultEvent(ProtocolModel):
    kind: Literal["result"] = "result"
    result: GenerationResult


class ErrorEvent(ProtocolModel):
    kind: Literal["error"] = "error"
    code: str
    message: str


type PublicEvent = (
    QueuedEvent
    | ProgressEvent
    | OutputStartEvent
    | OutputChunkEvent
    | ResultEvent
    | ErrorEvent
)


class WorkerResultEvent(ProtocolModel):
    kind: Literal["worker_result"] = "worker_result"
    output: str
    result: GenerationResult


class WorkerCheckpointEvent(ProtocolModel):
    kind: Literal["worker_checkpoint"] = "worker_checkpoint"
    checkpoint: str
    frames: int = Field(gt=0)


type WorkerEvent = Annotated[
    ProgressEvent | WorkerCheckpointEvent | WorkerResultEvent | ErrorEvent,
    Field(discriminator="kind"),
]
WORKER_EVENT_VALIDATOR = TypeAdapter(WorkerEvent)
