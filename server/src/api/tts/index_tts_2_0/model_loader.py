from dataclasses import dataclass
from functools import cache
from pathlib import Path
from typing import Literal

import torch
from huggingface_hub import hf_hub_download, snapshot_download
from pydantic import BaseModel, ConfigDict
from safetensors.torch import load_file
from torch import Tensor
from transformers import SeamlessM4TFeatureExtractor, Wav2Vec2BertModel

from api.model_progress import StreamedModelProgress

from .acoustic_model import AcousticModel
from .gpt import ConditioningConfig, UnifiedVoice
from .semantic_codec import RepCodec
from .speaker_encoder import SpeakerEncoder
from .text import TextTokenizer
from .vocoder import Vocoder, VocoderConfig


_EMOTION_PROTOTYPE_COUNTS = (3, 17, 2, 8, 4, 5, 10, 24)


@dataclass(frozen=True, slots=True)
class ModelPaths:
    main: Path
    w2v_bert: Path
    semantic_codec: Path
    speaker_encoder: Path
    vocoder: Path


@dataclass(frozen=True, slots=True, eq=False)
class IndexModels:
    device: torch.device
    dtype: torch.dtype
    feature_extractor: SeamlessM4TFeatureExtractor
    semantic_model: Wav2Vec2BertModel
    semantic_mean: Tensor
    semantic_standard_deviation: Tensor
    semantic_codec: RepCodec
    gpt: UnifiedVoice
    acoustic: AcousticModel
    speaker_encoder: SpeakerEncoder
    vocoder: Vocoder
    emotion_prototypes: tuple[Tensor, ...]
    speaker_prototypes: tuple[Tensor, ...]
    tokenizer: TextTokenizer
    emotion_text_model: Path


class VocoderFileConfig(BaseModel):
    model_config = ConfigDict(extra="ignore", strict=True)

    resblock: Literal["1"]
    num_mels: int
    upsample_initial_channel: int
    upsample_rates: list[int]
    upsample_kernel_sizes: list[int]
    resblock_kernel_sizes: list[int]
    resblock_dilation_sizes: list[list[int]]
    activation: Literal["snakebeta"]
    snake_logscale: bool
    use_bias_at_final: bool
    use_tanh_at_final: bool

    def runtime_config(self) -> VocoderConfig:
        return VocoderConfig(
            mel_channels=self.num_mels,
            initial_channels=self.upsample_initial_channel,
            upsample_rates=tuple(self.upsample_rates),
            upsample_kernel_sizes=tuple(self.upsample_kernel_sizes),
            residual_kernel_sizes=tuple(self.resblock_kernel_sizes),
            residual_dilations=tuple(
                tuple(values) for values in self.resblock_dilation_sizes
            ),
            snake_logscale=self.snake_logscale,
            use_final_tanh=self.use_tanh_at_final,
            final_bias=self.use_bias_at_final,
        )


@cache
def download_model_paths(cache_directory: Path) -> ModelPaths:
    cache_directory.mkdir(parents=True, exist_ok=True)
    cache = str(cache_directory)
    main = Path(
        snapshot_download(
            "IndexTeam/IndexTTS-2",
            cache_dir=cache,
            allow_patterns=[
                "bpe.model",
                "gpt.pth",
                "s2mel.pth",
                "wav2vec2bert_stats.pt",
                "feat1.pt",
                "feat2.pt",
                "qwen0.6bemo4-merge/*",
            ],
        )
    )
    w2v_bert = Path(
        snapshot_download(
            "facebook/w2v-bert-2.0",
            cache_dir=cache,
            allow_patterns=[
                "config.json",
                "model.safetensors",
                "preprocessor_config.json",
            ],
        )
    )
    semantic_codec = Path(
        hf_hub_download(
            "amphion/MaskGCT",
            "semantic_codec/model.safetensors",
            cache_dir=cache,
            tqdm_class=StreamedModelProgress,
        )
    )
    speaker_encoder = Path(
        hf_hub_download(
            "funasr/campplus",
            "campplus_cn_common.bin",
            cache_dir=cache,
            tqdm_class=StreamedModelProgress,
        )
    )
    vocoder = Path(
        snapshot_download(
            "nvidia/bigvgan_v2_22khz_80band_256x",
            cache_dir=cache,
            allow_patterns=["config.json", "bigvgan_generator.pt"],
        )
    )
    return ModelPaths(main, w2v_bert, semantic_codec, speaker_encoder, vocoder)


@cache
def load_models(
    paths: ModelPaths, device: torch.device, dtype: torch.dtype
) -> IndexModels:
    feature_extractor = SeamlessM4TFeatureExtractor.from_pretrained(
        paths.w2v_bert, local_files_only=True
    )
    semantic_model = Wav2Vec2BertModel.from_pretrained(
        paths.w2v_bert,
        local_files_only=True,
        dtype=dtype,
        device_map=device,
    )
    semantic_model.eval()
    statistics = _load_state_dict(paths.main / "wav2vec2bert_stats.pt")
    semantic_mean = statistics["mean"].to(device=device, dtype=dtype)
    semantic_standard_deviation = torch.sqrt(statistics["var"]).to(
        device=device, dtype=dtype
    )

    with torch.device("meta"):
        semantic_codec = RepCodec(
            codebook_size=8_192,
            hidden_size=1_024,
            codebook_dim=8,
            vocos_dim=384,
            vocos_intermediate_dim=2_048,
            vocos_num_layers=12,
        )
    semantic_codec.to_empty(device=device)
    semantic_codec.load_state_dict(
        load_file(paths.semantic_codec, device=str(device)), strict=True
    )
    semantic_codec.to(device=device, dtype=dtype).eval()

    default_dtype = torch.get_default_dtype()
    torch.set_default_dtype(dtype)
    try:
        with torch.device("meta"):
            gpt = UnifiedVoice(
                layers=24,
                model_dim=1_280,
                heads=20,
                max_text_tokens=600,
                max_mel_tokens=1_815,
                mel_length_compression=1_024,
                number_text_tokens=12_000,
                start_text_token=0,
                stop_text_token=1,
                number_mel_codes=8_194,
                start_mel_token=8_192,
                stop_mel_token=8_193,
                condition_module=ConditioningConfig(
                    output_size=512,
                    linear_units=2_048,
                    attention_heads=8,
                    num_blocks=6,
                    input_layer="conv2d2",
                    perceiver_multiplier=2,
                ),
                emo_condition_module=ConditioningConfig(
                    output_size=512,
                    linear_units=1_024,
                    attention_heads=4,
                    num_blocks=4,
                    input_layer="conv2d2",
                    perceiver_multiplier=2,
                ),
            )
    finally:
        torch.set_default_dtype(default_dtype)
    gpt.to_empty(device=device)
    gpt.load_state_dict(
        _load_state_dict(paths.main / "gpt.pth", "model", fallback=True),
        strict=True,
    )
    gpt.to(device=device, dtype=dtype).eval()
    gpt.prepare_for_inference()

    with torch.device("meta"):
        acoustic = AcousticModel()
    acoustic.to_empty(device=device)
    acoustic_state = _load_grouped_state(paths.main / "s2mel.pth", "net")
    for name, module in acoustic.models.items():
        state = acoustic_state.get(name)
        if state is None:
            raise ValueError(f"Acoustic checkpoint is missing module {name}")
        module.load_state_dict(state, strict=True)
    acoustic.to(device=device, dtype=dtype).eval()
    acoustic.prepare()

    with torch.device("meta"):
        speaker_encoder = SpeakerEncoder()
    speaker_encoder.to_empty(device=device)
    speaker_encoder.load_state_dict(
        _load_state_dict(paths.speaker_encoder), strict=True
    )
    speaker_encoder.to(device=device, dtype=dtype).eval()

    vocoder_config = VocoderFileConfig.model_validate_json(
        (paths.vocoder / "config.json").read_text(encoding="utf-8")
    )
    with torch.device("meta"):
        vocoder = Vocoder(vocoder_config.runtime_config())
    vocoder.to_empty(device=device)
    vocoder.load_state_dict(
        _load_state_dict(paths.vocoder / "bigvgan_generator.pt", "generator"),
        strict=True,
    )
    vocoder.to(device=device, dtype=dtype).eval()

    emotion_values = _load_tensor(paths.main / "feat2.pt").to(
        device=device, dtype=dtype
    )
    speaker_values = _load_tensor(paths.main / "feat1.pt").to(
        device=device, dtype=dtype
    )
    emotion_prototypes = tuple(emotion_values.split(_EMOTION_PROTOTYPE_COUNTS, dim=0))
    speaker_prototypes = tuple(speaker_values.split(_EMOTION_PROTOTYPE_COUNTS, dim=0))
    if len(emotion_prototypes) != 8 or len(speaker_prototypes) != 8:
        raise ValueError("Emotion prototype checkpoint has an invalid shape")

    tokenizer = TextTokenizer(
        paths.main / "bpe.model",
        paths.main.parent / "indextts-normalizer-cache",
    )
    return IndexModels(
        device=device,
        dtype=dtype,
        feature_extractor=feature_extractor,
        semantic_model=semantic_model,
        semantic_mean=semantic_mean,
        semantic_standard_deviation=semantic_standard_deviation,
        semantic_codec=semantic_codec,
        gpt=gpt,
        acoustic=acoustic,
        speaker_encoder=speaker_encoder,
        vocoder=vocoder,
        emotion_prototypes=emotion_prototypes,
        speaker_prototypes=speaker_prototypes,
        tokenizer=tokenizer,
        emotion_text_model=paths.main / "qwen0.6bemo4-merge",
    )


def _load_state_dict(
    path: Path,
    key: str | None = None,
    fallback: bool = False,
) -> dict[str, Tensor]:
    loaded = torch.load(path, map_location="cpu", weights_only=True, mmap=True)
    if not isinstance(loaded, dict):
        raise ValueError(f"Checkpoint {path} must contain a mapping")
    selected = loaded.get(key) if key is not None else loaded
    if selected is None and fallback:
        selected = loaded
    if not isinstance(selected, dict):
        raise ValueError(f"Checkpoint {path} is missing state dictionary {key}")
    return _tensor_map(selected)


def _load_grouped_state(path: Path, key: str) -> dict[str, dict[str, Tensor]]:
    loaded = torch.load(path, map_location="cpu", weights_only=True, mmap=True)
    if not isinstance(loaded, dict):
        raise ValueError(f"Checkpoint {path} must contain a mapping")
    selected = loaded.get(key)
    if not isinstance(selected, dict):
        raise ValueError(f"Checkpoint {path} is missing state group {key}")
    result: dict[str, dict[str, Tensor]] = {}
    for name, values in selected.items():
        if not isinstance(name, str) or not isinstance(values, dict):
            raise ValueError(f"Checkpoint {path} contains an invalid state group")
        result[name] = _tensor_map(values)
    return result


def _load_tensor(path: Path) -> Tensor:
    loaded = torch.load(path, map_location="cpu", weights_only=True, mmap=True)
    if not isinstance(loaded, Tensor):
        raise ValueError(f"Checkpoint {path} must contain a tensor")
    return loaded


def _tensor_map(values: dict[str, Tensor]) -> dict[str, Tensor]:
    result: dict[str, Tensor] = {}
    for key, value in values.items():
        if not isinstance(key, str) or not isinstance(value, Tensor):
            raise ValueError(
                "Checkpoint state dictionaries must map strings to tensors"
            )
        result[key.removeprefix("module.")] = value
    return result
