import random
from collections.abc import Callable
from dataclasses import dataclass
from fractions import Fraction
from functools import lru_cache

import torch
import torchaudio
from torch import Tensor
from torch.nn import functional as F

from api.tts.index_tts_2_0.audio_features import (
    decode_audio,
    mel_spectrogram,
    speaker_features,
)
from api.tts.index_tts_2_0.benchmarking import StageTimings, measure_stage
from api.tts.index_tts_2_0.emotion_text import load_emotion_text_analyzer
from api.tts.index_tts_2_0.gpt import GenerationOptions
from api.tts.index_tts_2_0.text import GlossaryTerm
from api.tts.index_tts_2_0.timing import (
    allocate_duration_frames,
    allocate_silence_aware_durations,
    effective_speed_factor,
    frames_for_speed,
    low_energy_token_mask,
    warp_embeddings,
)
from api.media import AudioUpload
from api.tts.requests import (
    AudioEmotion,
    DurationTiming,
    FactorEmotion,
    IndexSampling,
    IndexTts2Request,
    SpeakerEmotion,
    SpeedTiming,
    TextEmotion,
)

from .model_loader import IndexTts25Models


_SAMPLE_RATE = 22_050
_HOP_LENGTH = 256
_NATURAL_FRAMES_PER_EMBEDDING = Fraction(43, 25)
_MAX_ACOUSTIC_SEQUENCE = 8_192
_EMOTION_BIASES = (0.9375, 0.875, 1.0, 1.0, 0.9375, 0.9375, 0.6875, 0.5625)


@dataclass(frozen=True, slots=True, eq=False)
class ReferenceFeatures:
    conditioning: Tensor
    prompt_condition: Tensor
    mel: Tensor
    style: Tensor


@dataclass(frozen=True, slots=True)
class SynthesisOutput:
    waveform: Tensor
    sample_rate: int
    speed_factor: Fraction


@dataclass(frozen=True, slots=True)
class EmotionCondition:
    conditioning: Tensor
    vector: Tensor


@torch.inference_mode()
def synthesize(
    models: IndexTts25Models,
    request: IndexTts2Request,
    progress: Callable[[float, str], None] | None = None,
    timings: StageTimings | None = None,
    semantic_tokens: list[list[int]] | None = None,
) -> SynthesisOutput:
    if request.language is None:
        raise ValueError("IndexTTS 2.5 requires a language")
    if progress is not None:
        progress(0.05, "Preparing reference voice")
    with measure_stage(timings, "reference", models.device):
        reference = _reference_features(models, request.voice)
    emotion = _emotion_condition(models, request, reference)
    glossary = [
        GlossaryTerm(entry.term, entry.chinese, entry.english)
        for entry in request.glossary
    ]
    segments = models.tokenizer.segments(
        request.text,
        glossary,
        request.language,
        request.max_text_tokens_per_segment,
    )
    if not segments:
        raise ValueError("Text produced no IndexTTS tokens")
    duration_frames = (
        allocate_duration_frames(
            request.timing.seconds,
            request.intersegment_silence,
            [len(segment) for segment in segments],
        )
        if isinstance(request.timing, DurationTiming)
        else None
    )
    maximum_frames = _MAX_ACOUSTIC_SEQUENCE - reference.prompt_condition.size(1)
    if duration_frames is not None and max(duration_frames) > maximum_frames:
        raise ValueError("Duration exceeds the acoustic model sequence limit")

    rendered_segments: list[Tensor] = []
    natural_samples = 0
    language = torch.tensor(
        [models.tokenizer.language_index(request.language)], device=models.device
    )
    for index, segment in enumerate(segments):
        if progress is not None:
            progress(
                0.15 + 0.75 * index / len(segments),
                f"Synthesizing segment {index + 1} of {len(segments)}",
            )
        embeddings, natural_frames = _semantic_embeddings(
            models,
            segment,
            language,
            reference,
            emotion,
            request.sampling,
            timings,
            semantic_tokens,
        )
        if duration_frames is not None:
            natural_waveform = _render(
                models, embeddings, natural_frames, reference, timings
            )
            natural_samples += natural_waveform.shape[-1]
            target_frames = duration_frames[index]
            if target_frames == natural_frames:
                waveform = natural_waveform
            else:
                silence = low_energy_token_mask(
                    natural_waveform,
                    token_count=embeddings.size(1),
                )
                token_durations = allocate_silence_aware_durations(
                    natural_frames,
                    target_frames,
                    silence,
                )
                resized = warp_embeddings(embeddings, token_durations, target_frames)
                waveform = _render(
                    models, resized, target_frames, reference, timings
                )
        else:
            if not isinstance(request.timing, SpeedTiming):
                raise RuntimeError("IndexTTS timing mode was not validated")
            target_frames = frames_for_speed(natural_frames, request.timing.factor)
            if target_frames > maximum_frames:
                raise ValueError(
                    "Speed factor exceeds the acoustic model sequence limit"
                )
            waveform = _render(models, embeddings, target_frames, reference, timings)
            natural_samples += natural_frames * _HOP_LENGTH
        rendered_segments.append(waveform)

    silence_samples = round(request.intersegment_silence * _SAMPLE_RATE)
    output_parts: list[Tensor] = []
    for index, waveform in enumerate(rendered_segments):
        output_parts.append(waveform)
        if silence_samples and index < len(rendered_segments) - 1:
            output_parts.append(
                torch.zeros(
                    waveform.shape[0],
                    silence_samples,
                    dtype=waveform.dtype,
                    device=waveform.device,
                )
            )
    output = torch.cat(output_parts, dim=1).clamp(-1, 1)
    natural_samples += silence_samples * (len(rendered_segments) - 1)
    speed = effective_speed_factor(natural_samples, output.shape[-1])
    if progress is not None:
        progress(0.95, "Finalizing speech")
    return SynthesisOutput(output.cpu(), _SAMPLE_RATE, speed)


@lru_cache(maxsize=4)
def _reference_features(models: IndexTts25Models, audio: AudioUpload) -> ReferenceFeatures:
    waveform = decode_audio(audio)
    conditioning = _semantic_conditioning(models, waveform.samples)
    mel = mel_spectrogram(waveform, models.device).to(models.dtype)
    target_length = torch.tensor([mel.size(2)], device=models.device)
    prompt = models.acoustic.regulate_length(conditioning, target_length)
    style = models.speaker_encoder(
        speaker_features(waveform).to(device=models.device, dtype=models.dtype)[None]
    )
    return ReferenceFeatures(conditioning, prompt, mel, style)


@lru_cache(maxsize=4)
def _emotion_audio_conditioning(models: IndexTts25Models, audio: AudioUpload) -> Tensor:
    waveform = decode_audio(audio)
    return _semantic_conditioning(models, waveform.samples)


def _semantic_conditioning(models: IndexTts25Models, audio_24khz: Tensor) -> Tensor:
    audio_16khz = torchaudio.functional.resample(audio_24khz, 24_000, 16_000)
    extracted = models.feature_extractor(
        audio_16khz.squeeze(0).cpu().numpy(),
        sampling_rate=16_000,
        return_tensors="pt",
    )
    input_features = extracted["input_features"]
    attention_mask = extracted["attention_mask"]
    if not isinstance(input_features, Tensor) or not isinstance(attention_mask, Tensor):
        raise RuntimeError("Wav2Vec2-BERT feature extraction returned invalid tensors")
    output = models.semantic_model(
        input_features=input_features.to(device=models.device, dtype=torch.float32),
        attention_mask=attention_mask.to(models.device),
        output_hidden_states=True,
    )
    if output.hidden_states is None or len(output.hidden_states) <= 17:
        raise RuntimeError("Wav2Vec2-BERT did not return semantic layer 17")
    features = output.hidden_states[17]
    return (features - models.semantic_mean) / models.semantic_standard_deviation


def _emotion_condition(
    models: IndexTts25Models,
    request: IndexTts2Request,
    reference: ReferenceFeatures,
) -> EmotionCondition:
    speaker_lengths = torch.tensor(
        [reference.conditioning.shape[1]], device=models.device
    )
    base = models.gpt.emotion_vector(reference.conditioning, speaker_lengths)
    match request.emotion:
        case SpeakerEmotion():
            return EmotionCondition(reference.conditioning, base)
        case AudioEmotion(audio=audio, strength=strength):
            conditioning = _emotion_audio_conditioning(models, audio)
            lengths = torch.tensor([conditioning.shape[1]], device=models.device)
            vector = models.gpt.merge_emotion_vectors(
                reference.conditioning,
                conditioning,
                speaker_lengths,
                lengths,
                strength,
            )
            return EmotionCondition(conditioning, vector)
        case FactorEmotion(factors=factors, strength=strength, randomize=randomize):
            values = factors.values_in_model_order()
        case TextEmotion(text=text, strength=strength, randomize=randomize):
            analyzer = load_emotion_text_analyzer(
                models.emotion_text_model, models.device, models.gpt_dtype
            )
            values = analyzer.analyze(text or request.text)
    adjusted = [
        max(0.0, value) * bias
        for value, bias in zip(values, _EMOTION_BIASES, strict=True)
    ]
    total = sum(adjusted)
    normalized = [value * 0.8 / total for value in adjusted] if total > 0.8 else adjusted
    factors = [value * strength for value in normalized]
    vector = torch.zeros_like(base)
    for factor, emotion_group, speaker_group in zip(
        factors,
        models.emotion_prototypes,
        models.speaker_prototypes,
        strict=True,
    ):
        if randomize:
            index = random.randrange(emotion_group.shape[0])
        else:
            similarities = F.cosine_similarity(
                reference.style.float(), speaker_group.float(), dim=1
            )
            index = int(torch.argmax(similarities))
        vector = vector + factor * emotion_group[index].unsqueeze(0)
    return EmotionCondition(
        reference.conditioning,
        vector + (1 - sum(factors)) * base,
    )


def _semantic_embeddings(
    models: IndexTts25Models,
    token_ids: list[int],
    language: Tensor,
    reference: ReferenceFeatures,
    emotion: EmotionCondition,
    sampling: IndexSampling,
    timings: StageTimings | None,
    semantic_tokens: list[list[int]] | None,
) -> tuple[Tensor, int]:
    text = torch.tensor(token_ids, dtype=torch.long, device=models.device)[None]
    emotion_lengths = torch.tensor(
        [emotion.conditioning.shape[1]], device=models.device
    )
    with measure_stage(timings, "gpt_generation", models.device):
        codes = models.gpt.inference_speech(
            reference.style,
            text,
            language,
            emotion.conditioning,
            emotion_lengths,
            emotion.vector,
            GenerationOptions(
                do_sample=sampling.do_sample,
                top_p=sampling.top_p,
                top_k=sampling.top_k,
                temperature=sampling.temperature,
                length_penalty=sampling.length_penalty,
                num_beams=sampling.num_beams,
                repetition_penalty=sampling.repetition_penalty,
                maximum_tokens=sampling.max_mel_tokens,
                typical_sampling=sampling.typical_sampling,
                typical_mass=sampling.typical_mass,
            ),
        )
    stop_positions = (codes[0] == models.gpt.stop_mel_token).nonzero()
    code_length = (
        int(stop_positions[0, 0]) if stop_positions.numel() else codes.shape[-1]
    )
    if code_length < 1:
        raise RuntimeError("IndexTTS generated no semantic audio tokens")
    if semantic_tokens is not None:
        semantic_tokens.append(codes[0, :code_length].tolist())
    with measure_stage(timings, "semantic_codec", models.device):
        embeddings = models.semantic_codec.decode(codes[:, :code_length])
    natural_frames = max(
        1,
        embeddings.size(1)
        * _NATURAL_FRAMES_PER_EMBEDDING.numerator
        // _NATURAL_FRAMES_PER_EMBEDDING.denominator,
    )
    return embeddings, natural_frames


def _render(
    models: IndexTts25Models,
    embeddings: Tensor,
    target_frames: int,
    reference: ReferenceFeatures,
    timings: StageTimings | None,
) -> Tensor:
    lengths = torch.tensor([target_frames], device=models.device)
    generated_condition = models.acoustic.regulate_length(embeddings, lengths)
    condition = torch.cat((reference.prompt_condition, generated_condition), dim=1)
    combined_lengths = torch.tensor([condition.size(1)], device=models.device)
    with measure_stage(timings, "acoustic_flow", models.device):
        generated_mel = models.acoustic.generate_mel(
            condition,
            combined_lengths,
            reference.mel,
            reference.style,
        )
    generated_mel = generated_mel[:, :, reference.mel.size(-1) :]
    with measure_stage(timings, "vocoder", models.device):
        waveform = models.vocoder(generated_mel.to(models.dtype)).squeeze(1)
    return waveform
