from fractions import Fraction
from typing import Annotated, Literal, Self

from pydantic import BaseModel, BeforeValidator, ConfigDict, Field, TypeAdapter, model_validator

from api.media import AudioUpload

type ModelId = Literal[
    "Qwen/Qwen3-TTS-12Hz-0.6B-CustomVoice",
    "Qwen/Qwen3-TTS-12Hz-1.7B-CustomVoice",
    "Qwen/Qwen3-TTS-12Hz-0.6B-Base",
    "Qwen/Qwen3-TTS-12Hz-1.7B-Base",
    "Qwen/Qwen3-TTS-12Hz-1.7B-VoiceDesign",
    "IndexTeam/IndexTTS-2",
    "IndexTeam/IndexTTS-2.5",
]
type WorkerState = Literal["stopped", "loading", "generating", "ready"]
type Precision = Literal["auto", "bfloat16", "float16", "float32"]
type InputPurpose = Literal["text", "duration", "speed_factor"]
MODEL_IDS: tuple[ModelId, ...] = (
    "Qwen/Qwen3-TTS-12Hz-0.6B-CustomVoice",
    "Qwen/Qwen3-TTS-12Hz-1.7B-CustomVoice",
    "Qwen/Qwen3-TTS-12Hz-0.6B-Base",
    "Qwen/Qwen3-TTS-12Hz-1.7B-Base",
    "Qwen/Qwen3-TTS-12Hz-1.7B-VoiceDesign",
    "IndexTeam/IndexTTS-2",
    "IndexTeam/IndexTTS-2.5",
)


class ProtocolModel(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True)


class Rational(ProtocolModel):
    numerator: int
    denominator: int = Field(gt=0)


def validate_reference_audio(value):
    if isinstance(value, dict):
        if set(value) == {"wav"}:
            wav = value["wav"]
            if (
                not isinstance(wav, bytes)
                or len(wav) < 44
                or wav[:4] != b"RIFF"
                or wav[8:12] != b"WAVE"
            ):
                raise ValueError("Audio must contain WAV data")
            value = {"kind": "encoded", "data": wav}
        data = value.get("data")
    else:
        data = getattr(value, "data", None)
    if isinstance(data, bytes) and len(data) > 64 * 1024 * 1024:
        raise ValueError("Reference audio exceeds the 64 MiB limit")
    return value


type Audio = Annotated[AudioUpload, BeforeValidator(validate_reference_audio)]


class VisibleWhen(ProtocolModel):
    input: str = Field(min_length=1)
    values: list[str | bool] = Field(min_length=1)


class SelectOption(ProtocolModel):
    value: str = Field(min_length=1)
    label: str = Field(min_length=1)
    purpose: InputPurpose | None = None


class TextInput(ProtocolModel):
    kind: Literal["text"] = "text"
    key: str = Field(min_length=1)
    label: str = Field(min_length=1)
    default: str = ""
    required: bool = False
    multiline: bool = False
    max_length: int = Field(default=4096, gt=0)
    purpose: InputPurpose | None = None
    visible_when: VisibleWhen | None = None


class SelectInput(ProtocolModel):
    kind: Literal["select"] = "select"
    key: str = Field(min_length=1)
    label: str = Field(min_length=1)
    options: list[SelectOption] = Field(min_length=1)
    default: str = Field(min_length=1)
    visible_when: VisibleWhen | None = None

    @model_validator(mode="after")
    def validate_default(self) -> Self:
        option_values = [option.value for option in self.options]
        if len(option_values) != len(set(option_values)):
            raise ValueError(f"Input {self.key} has duplicate options")
        if self.default not in option_values:
            raise ValueError(f"Default option {self.default!r} does not exist")
        return self


class AudioInput(ProtocolModel):
    kind: Literal["audio"] = "audio"
    key: str = Field(min_length=1)
    label: str = Field(min_length=1)
    required: bool = True
    visible_when: VisibleWhen | None = None


class ToggleInput(ProtocolModel):
    kind: Literal["toggle"] = "toggle"
    key: str = Field(min_length=1)
    label: str = Field(min_length=1)
    default: bool = False
    visible_when: VisibleWhen | None = None


class NumberInput(ProtocolModel):
    kind: Literal["number"] = "number"
    key: str = Field(min_length=1)
    label: str = Field(min_length=1)
    default: Rational
    minimum: Rational
    maximum: Rational
    step: Rational
    presentation: Literal["number", "slider"] = "number"
    purpose: InputPurpose | None = None
    visible_when: VisibleWhen | None = None

    @model_validator(mode="after")
    def validate_range(self) -> Self:
        default = Fraction(self.default.numerator, self.default.denominator)
        minimum = Fraction(self.minimum.numerator, self.minimum.denominator)
        maximum = Fraction(self.maximum.numerator, self.maximum.denominator)
        if self.step.numerator <= 0:
            raise ValueError("Number input step must be positive")
        if minimum > maximum or not minimum <= default <= maximum:
            raise ValueError("Number input default must be within its range")
        return self


class TableColumn(ProtocolModel):
    key: str = Field(min_length=1)
    label: str = Field(min_length=1)
    required: bool = False
    max_length: int = Field(default=1024, gt=0)


class TableInput(ProtocolModel):
    kind: Literal["table"] = "table"
    key: str = Field(min_length=1)
    label: str = Field(min_length=1)
    columns: list[TableColumn] = Field(min_length=1)
    visible_when: VisibleWhen | None = None

    @model_validator(mode="after")
    def validate_columns(self) -> Self:
        keys = [column.key for column in self.columns]
        if len(keys) != len(set(keys)):
            raise ValueError(f"Input {self.key} has duplicate table columns")
        return self


type InputDefinition = Annotated[
    TextInput | SelectInput | AudioInput | ToggleInput | NumberInput | TableInput,
    Field(discriminator="kind"),
]


class TextValue(ProtocolModel):
    kind: Literal["text"]
    value: str


class SelectValue(ProtocolModel):
    kind: Literal["select"]
    value: str


class AudioValue(ProtocolModel):
    kind: Literal["audio"]
    value: Audio


class ToggleValue(ProtocolModel):
    kind: Literal["toggle"]
    value: bool


class NumberValue(ProtocolModel):
    kind: Literal["number"]
    value: Rational


class TableValue(ProtocolModel):
    kind: Literal["table"]
    rows: list[dict[str, str]]


type InputValue = Annotated[
    TextValue | SelectValue | AudioValue | ToggleValue | NumberValue | TableValue,
    Field(discriminator="kind"),
]


class ModelInfo(ProtocolModel):
    id: ModelId
    label: str = Field(min_length=1)
    inputs: list[InputDefinition]

    @model_validator(mode="after")
    def validate_input_keys(self) -> Self:
        keys = [model_input.key for model_input in self.inputs]
        if len(keys) != len(set(keys)):
            raise ValueError(f"Model {self.id} has duplicate input keys")
        inputs = {model_input.key: model_input for model_input in self.inputs}
        purposes = [
            model_input.purpose
            for model_input in self.inputs
            if isinstance(model_input, (TextInput, NumberInput))
            and model_input.purpose is not None
        ]
        if len(purposes) != len(set(purposes)):
            raise ValueError(f"Model {self.id} has duplicate input purposes")
        for model_input in self.inputs:
            condition = model_input.visible_when
            if condition is None:
                continue
            controller = inputs.get(condition.input)
            if isinstance(controller, SelectInput):
                options = {option.value for option in controller.options}
                if any(
                    not isinstance(value, str) or value not in options
                    for value in condition.values
                ):
                    raise ValueError(
                        f"Input {model_input.key} has an invalid select condition"
                    )
            elif isinstance(controller, ToggleInput):
                if any(not isinstance(value, bool) for value in condition.values):
                    raise ValueError(
                        f"Input {model_input.key} has an invalid toggle condition"
                    )
            else:
                raise ValueError(
                    f"Input {model_input.key} has a missing or invalid controller"
                )
        option_purposes = {
            option.purpose
            for model_input in self.inputs
            if isinstance(model_input, SelectInput)
            for option in model_input.options
            if option.purpose is not None
        }
        if not option_purposes.issubset(purposes):
            raise ValueError(f"Model {self.id} has an unmatched option purpose")
        return self


class ModelsResponse(ProtocolModel):
    models: list[ModelInfo]


class SynthesisRequest(ProtocolModel):
    model: ModelId
    inputs: dict[str, InputValue]


SYNTHESIS_REQUEST_VALIDATOR = TypeAdapter(SynthesisRequest)


class Speech(ProtocolModel):
    wav: bytes = Field(min_length=44)
    speed_factor: Rational = Rational(numerator=1, denominator=1)

    @model_validator(mode="after")
    def validate_result(self) -> Self:
        if self.wav[:4] != b"RIFF" or self.wav[8:12] != b"WAVE":
            raise ValueError("Speech must contain WAV audio")
        if self.speed_factor.numerator <= 0:
            raise ValueError("Speech speed factor must be positive")
        return self


class ProgressEvent(ProtocolModel):
    kind: Literal["progress"] = "progress"
    message: str
    model: ModelId
    state: WorkerState


class ResultEvent(ProtocolModel):
    kind: Literal["result"] = "result"
    result: Speech


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
WORKER_EVENT_VALIDATOR = TypeAdapter(WorkerEvent)
