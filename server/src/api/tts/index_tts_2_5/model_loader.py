from dataclasses import dataclass
from functools import cache
from pathlib import Path

import torch
from huggingface_hub import hf_hub_download, snapshot_download
from torch import Tensor
from transformers import SeamlessM4TFeatureExtractor, Wav2Vec2BertModel

from api.model_progress import StreamedModelProgress
from api.tts.index_tts_2_0.acoustic_model import AcousticModel
from api.tts.index_tts_2_0.gpt import ConditioningConfig
from api.tts.index_tts_2_0.model_loader import (
    VocoderFileConfig,
    _load_grouped_state,
    _load_state_dict,
    _load_tensor,
)
from api.tts.index_tts_2_0.speaker_encoder import SpeakerEncoder
from api.tts.index_tts_2_0.vocoder import Vocoder

from .gpt import UnifiedVoice
from .semantic_codec import EnhancedCodec
from .text import MultilingualTextTokenizer


_EMOTION_PROTOTYPE_COUNTS = (3, 17, 2, 8, 4, 5, 10, 24)


@dataclass(frozen=True, slots=True)
class ModelPaths:
    main: Path
    w2v_bert: Path
    speaker_encoder: Path
    vocoder: Path


@dataclass(frozen=True, slots=True, eq=False)
class IndexTts25Models:
    device: torch.device
    dtype: torch.dtype
    gpt_dtype: torch.dtype
    feature_extractor: SeamlessM4TFeatureExtractor
    semantic_model: Wav2Vec2BertModel
    semantic_mean: Tensor
    semantic_standard_deviation: Tensor
    semantic_codec: EnhancedCodec
    gpt: UnifiedVoice
    acoustic: AcousticModel
    speaker_encoder: SpeakerEncoder
    vocoder: Vocoder
    emotion_prototypes: tuple[Tensor, ...]
    speaker_prototypes: tuple[Tensor, ...]
    tokenizer: MultilingualTextTokenizer
    emotion_text_model: Path


@cache
def download_model_paths(cache_directory: Path) -> ModelPaths:
    cache_directory.mkdir(parents=True, exist_ok=True)
    cache = str(cache_directory)
    main = Path(
        snapshot_download(
            "IndexTeam/IndexTTS-2.5",
            cache_dir=cache,
            allow_patterns=[
                "codec.pth",
                "feat1.pt",
                "feat2.pt",
                "gpt.pth",
                "multilingual_zh_ja_yue_char_del.tiktoken",
                "qwen0.6bemo4-merge/*",
                "s2mel.pth",
                "wav2vec2bert_stats.pt",
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
    return ModelPaths(main, w2v_bert, speaker_encoder, vocoder)


@cache
def load_models(
    paths: ModelPaths, device: torch.device, dtype: torch.dtype
) -> IndexTts25Models:
    feature_extractor = SeamlessM4TFeatureExtractor.from_pretrained(
        paths.w2v_bert, local_files_only=True
    )
    semantic_model = Wav2Vec2BertModel.from_pretrained(
        paths.w2v_bert,
        local_files_only=True,
        dtype=torch.float32,
        device_map=device,
    )
    semantic_model.eval()
    statistics = _load_state_dict(paths.main / "wav2vec2bert_stats.pt")
    semantic_mean = statistics["mean"].to(device=device, dtype=torch.float32)
    semantic_standard_deviation = torch.sqrt(statistics["var"]).to(
        device=device, dtype=torch.float32
    )

    with torch.device("meta"):
        semantic_codec = EnhancedCodec()
    semantic_codec.to_empty(device=device)
    semantic_codec.load_state_dict(
        _load_state_dict(paths.main / "codec.pth", "model"), strict=True
    )
    semantic_codec.to(device=device, dtype=torch.float32).eval()

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
                number_text_tokens=60_509,
                start_text_token=0,
                stop_text_token=1,
                number_mel_codes=8_194,
                start_mel_token=8_192,
                stop_mel_token=8_193,
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
    acoustic.to(device=device, dtype=torch.float32).eval()
    acoustic.prepare()

    with torch.device("meta"):
        speaker_encoder = SpeakerEncoder()
    speaker_encoder.to_empty(device=device)
    speaker_encoder.load_state_dict(
        _load_state_dict(paths.speaker_encoder), strict=True
    )
    speaker_encoder.to(device=device, dtype=torch.float32).eval()

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
    vocoder.to(device=device, dtype=torch.float32).eval()

    emotion_values = _load_tensor(paths.main / "feat2.pt").to(
        device=device, dtype=dtype
    )
    speaker_values = _load_tensor(paths.main / "feat1.pt").to(
        device=device, dtype=torch.float32
    )
    emotion_prototypes = tuple(emotion_values.split(_EMOTION_PROTOTYPE_COUNTS, dim=0))
    speaker_prototypes = tuple(speaker_values.split(_EMOTION_PROTOTYPE_COUNTS, dim=0))
    if len(emotion_prototypes) != 8 or len(speaker_prototypes) != 8:
        raise ValueError("Emotion prototype checkpoint has an invalid shape")

    tokenizer = MultilingualTextTokenizer(
        paths.main / "multilingual_zh_ja_yue_char_del.tiktoken",
        paths.main.parent / "indextts-2.5-normalizer-cache",
    )
    return IndexTts25Models(
        device=device,
        dtype=torch.float32,
        gpt_dtype=dtype,
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
