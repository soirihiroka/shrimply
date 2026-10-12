import argparse
from decimal import Decimal
from fractions import Fraction
import json
import os
from pathlib import Path
import random
import statistics
import subprocess
import sys
from time import perf_counter_ns
from typing import Callable, Literal, TYPE_CHECKING, cast

SERVER_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SERVER_ROOT / "src"))

if TYPE_CHECKING:
    from api.tts.index_tts_2_0.inference import SynthesisOutput as IndexOutput
    from api.tts.index_tts_2_5.inference import SynthesisOutput as Index25Output

type BenchmarkOutput = IndexOutput | Index25Output


type ModelVersion = Literal["2", "2.5"]
type Runtime = Literal["stable", "beta"]
type Precision = Literal["reduced", "float32", "float16", "bfloat16"]
NANOSECONDS_PER_SECOND = 1_000_000_000
DEFAULT_TEXT = "The quick brown fox jumps over the lazy dog."
MAXIMUM_WAVEFORM_ERROR = 0.03
MINIMUM_SNR_DECIBELS = 40.0


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Benchmark the local IndexTTS Stable and Beta runtimes"
    )
    parser.add_argument("--model", choices=("2", "2.5", "both"), default="both")
    parser.add_argument(
        "--runtime", choices=("stable", "beta", "both"), default="both"
    )
    parser.add_argument("--voice", type=Path)
    parser.add_argument("--text", default=DEFAULT_TEXT)
    parser.add_argument("--language", choices=("zh", "en", "ja", "es", "ar"), default="en")
    parser.add_argument("--precision", choices=("reduced", "float32", "float16", "bfloat16"), default="reduced")
    parser.add_argument("--sampling", choices=("deterministic", "default"), default="deterministic")
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--max-mel-tokens", type=int, default=1_500)
    parser.add_argument("--profile-kernels", action="store_true")
    parser.add_argument("--output", type=Path, default=Path("index-tts-benchmark"))
    parser.add_argument("--worker-config", type=Path, help=argparse.SUPPRESS)
    return parser


def _selection(value: str, both: tuple[str, str]) -> tuple[str, ...]:
    return both if value == "both" else (value,)


def _run_worker(config_path: Path) -> None:
    config = json.loads(config_path.read_text(encoding="utf-8"))
    if not isinstance(config, dict):
        raise ValueError("Benchmark worker configuration must be an object")
    result = _benchmark(config)
    result_path = Path(config["result_path"])
    temporary = result_path.with_name(f".{result_path.name}.{os.getpid()}.partial")
    try:
        temporary.write_text(json.dumps(result, indent=2), encoding="utf-8")
        os.replace(temporary, result_path)
    finally:
        temporary.unlink(missing_ok=True)


def _dtype(torch, model: ModelVersion, precision: Precision):
    match precision:
        case "reduced":
            return torch.float16 if model == "2" else torch.bfloat16
        case "float32":
            return torch.float32
        case "float16":
            return torch.float16
        case "bfloat16":
            return torch.bfloat16


def _request(config: dict[str, object], voice: bytes):
    from api.media import EncodedAudio
    from api.tts.requests import (
        IndexLanguage,
        IndexSampling,
        IndexTts2Request,
        SpeakerEmotion,
        SpeedTiming,
    )

    model = cast(ModelVersion, config["model"])
    sampled = config["sampling"] == "default"
    return IndexTts2Request(
        model="IndexTeam/IndexTTS-2" if model == "2" else "IndexTeam/IndexTTS-2.5",
        text=cast(str, config["text"]),
        language=None if model == "2" else cast(IndexLanguage, config["language"]),
        voice=EncodedAudio(data=voice),
        emotion=SpeakerEmotion(),
        timing=SpeedTiming(factor=Fraction(1)),
        sampling=IndexSampling(
            do_sample=sampled,
            typical_sampling=False,
            typical_mass=0.9,
            top_p=0.8,
            top_k=30,
            temperature=0.8,
            length_penalty=0.0,
            num_beams=3 if sampled else 1,
            repetition_penalty=10.0,
            max_mel_tokens=cast(int, config["max_mel_tokens"]),
        ),
        max_text_tokens_per_segment=120,
        intersegment_silence=Fraction(1, 5),
        glossary=[],
    )


def _benchmark(config: dict[str, object]) -> dict[str, object]:
    import numpy as np
    import soundfile
    import torch
    from huggingface_hub.constants import HF_HUB_CACHE

    from api.tts.index_beta import prepare_index_beta

    if not torch.cuda.is_available():
        raise RuntimeError("IndexTTS benchmarking requires CUDA")
    model = cast(ModelVersion, config["model"])
    runtime = cast(Runtime, config["runtime"])
    precision = cast(Precision, config["precision"])
    device = torch.device("cuda")
    dtype = _dtype(torch, model, precision)
    if dtype == torch.bfloat16 and not torch.cuda.is_bf16_supported():
        raise RuntimeError("This GPU does not support bfloat16 IndexTTS")

    if model == "2":
        from api.tts.index_tts_2_0.inference import synthesize
        from api.tts.index_tts_2_0.model_loader import (
            download_model_paths,
            load_models,
        )
    else:
        from api.tts.index_tts_2_5.inference import synthesize
        from api.tts.index_tts_2_5.model_loader import (
            download_model_paths,
            load_models,
        )
    synthesize_model = cast(Callable[..., BenchmarkOutput], synthesize)

    paths = download_model_paths(Path(HF_HUB_CACHE))
    torch.cuda.synchronize(device)
    load_started = perf_counter_ns()
    models = load_models(paths, device, dtype)
    torch.cuda.synchronize(device)
    load_nanoseconds = perf_counter_ns() - load_started

    preparation_nanoseconds = 0
    if runtime == "beta":
        preparation_started = perf_counter_ns()
        prepare_index_beta(models)
        torch.cuda.synchronize(device)
        preparation_nanoseconds = perf_counter_ns() - preparation_started

    request = _request(config, Path(cast(str, config["voice"])).read_bytes())

    def generate(profile: bool) -> tuple[dict[str, object], BenchmarkOutput]:
        seed = cast(int, config["seed"])
        random.seed(seed)
        np.random.seed(seed)
        torch.manual_seed(seed)
        torch.cuda.manual_seed_all(seed)
        timings: dict[str, int] | None = {} if profile else None
        semantic_tokens: list[list[int]] | None = [] if profile else None
        torch.cuda.reset_peak_memory_stats(device)
        torch.cuda.synchronize(device)
        started = perf_counter_ns()
        output = synthesize_model(
            models,
            request,
            timings=timings,
            semantic_tokens=semantic_tokens,
        )
        torch.cuda.synchronize(device)
        elapsed = perf_counter_ns() - started
        samples = output.waveform.shape[-1]
        return (
            {
                "elapsed_nanoseconds": elapsed,
                "audio_samples": samples,
                "sample_rate": output.sample_rate,
                "rtf": {
                    "numerator": elapsed * output.sample_rate,
                    "denominator": samples * NANOSECONDS_PER_SECOND,
                },
                "peak_allocated_vram": torch.cuda.max_memory_allocated(device),
                "peak_reserved_vram": torch.cuda.max_memory_reserved(device),
                **(
                    {
                        "stages_nanoseconds": timings,
                        "semantic_tokens": semantic_tokens,
                    }
                    if profile
                    else {}
                ),
            },
            output,
        )

    first_run, output = generate(True)
    for _ in range(cast(int, config["warmups"]) - 1):
        _, output = generate(False)
    if config["profile_kernels"]:
        cuda_runtime = torch.cuda.cudart()
        if cuda_runtime is None:
            raise RuntimeError("CUDA runtime profiling is unavailable")
        cuda_runtime.cudaProfilerStart()
        try:
            _, output = generate(False)
        finally:
            cuda_runtime.cudaProfilerStop()
    profiled_run, output = generate(True)
    runs: list[dict[str, object]] = []
    for _ in range(cast(int, config["runs"])):
        measurement, output = generate(False)
        runs.append(measurement)

    wav_path = Path(cast(str, config["wav_path"]))
    soundfile.write(
        wav_path,
        output.waveform.squeeze(0).contiguous().float().numpy(),
        output.sample_rate,
        format="WAV",
        subtype="FLOAT",
    )
    return {
        "model": model,
        "runtime": runtime,
        "precision": str(dtype),
        "checkpoint_revision": paths.main.name,
        "load_nanoseconds": load_nanoseconds,
        "preparation_nanoseconds": preparation_nanoseconds,
        "backend": "triton+cudagraph" if runtime == "beta" else "pytorch",
        "first_run": first_run,
        "profiled_run": profiled_run,
        "runs": runs,
        "wav": str(wav_path),
    }


def _comparison(stable: dict[str, object], beta: dict[str, object]) -> dict[str, object]:
    import numpy as np
    import soundfile

    stable_runs = cast(list[dict[str, object]], stable["runs"])
    beta_runs = cast(list[dict[str, object]], beta["runs"])
    stable_times = [cast(int, run["elapsed_nanoseconds"]) for run in stable_runs]
    beta_times = [cast(int, run["elapsed_nanoseconds"]) for run in beta_runs]
    stable_median = statistics.median_low(stable_times)
    beta_median = statistics.median_low(beta_times)
    speedup = Fraction(stable_median, beta_median)
    wins = sum(
        beta_time < stable_time
        for stable_time, beta_time in zip(stable_times, beta_times, strict=True)
    )

    stable_audio, stable_rate = soundfile.read(
        cast(str, stable["wav"]), dtype="float32"
    )
    beta_audio, beta_rate = soundfile.read(cast(str, beta["wav"]), dtype="float32")
    stable_profile = cast(dict[str, object], stable["profiled_run"])
    beta_profile = cast(dict[str, object], beta["profiled_run"])
    stable_tokens = stable_profile["semantic_tokens"]
    beta_tokens = beta_profile["semantic_tokens"]
    matching_shape = stable_audio.shape == beta_audio.shape
    finite = bool(np.isfinite(stable_audio).all() and np.isfinite(beta_audio).all())
    max_error = None
    snr_decibels = None
    waveform_close = False
    if matching_shape and finite:
        difference = stable_audio - beta_audio
        max_error = float(np.max(np.abs(difference), initial=0.0))
        noise = float(np.sum(np.square(difference), dtype=np.float64))
        signal = float(np.sum(np.square(stable_audio), dtype=np.float64))
        snr_decibels = (
            None
            if noise == 0
            else float(10 * np.log10(signal / noise)) if signal > 0 else -float("inf")
        )
        waveform_close = max_error <= MAXIMUM_WAVEFORM_ERROR and (
            noise == 0
            or (snr_decibels is not None and snr_decibels >= MINIMUM_SNR_DECIBELS)
        )
    return {
        "stable_median_nanoseconds": stable_median,
        "beta_median_nanoseconds": beta_median,
        "speedup": {"numerator": speedup.numerator, "denominator": speedup.denominator},
        "beta_wins": wins,
        "paired_runs": len(stable_times),
        "clear_runtime_win": (
            beta_median < stable_median and wins * 5 >= len(stable_times) * 4
        ),
        "semantic_tokens_match": stable_tokens == beta_tokens,
        "sample_rates_match": stable_rate == beta_rate,
        "waveform_shapes_match": matching_shape,
        "finite_audio": finite,
        "waveform_close": waveform_close,
        "maximum_absolute_error": max_error,
        "snr_decibels": snr_decibels,
    }


def _main(args: argparse.Namespace) -> None:
    if args.voice is None:
        raise ValueError("--voice is required")
    if args.warmups < 1 or args.runs < 1:
        raise ValueError("--warmups and --runs must be positive")
    if not 1 <= args.max_mel_tokens <= 1_815:
        raise ValueError("--max-mel-tokens must be between 1 and 1815")
    voice = args.voice.expanduser().resolve()
    if not voice.is_file():
        raise FileNotFoundError(voice)
    output = args.output.expanduser().resolve()
    output.mkdir(parents=True, exist_ok=True)

    models = cast(tuple[ModelVersion, ...], _selection(args.model, ("2", "2.5")))
    runtimes = cast(tuple[Runtime, ...], _selection(args.runtime, ("stable", "beta")))
    results: list[dict[str, object]] = []
    for model in models:
        for runtime in runtimes:
            stem = f"index-tts-{model.replace('.', '-')}-{runtime}"
            config_path = output / f".{stem}.json"
            result_path = output / f"{stem}.json"
            config = {
                "model": model,
                "runtime": runtime,
                "voice": str(voice),
                "text": args.text,
                "language": args.language,
                "precision": args.precision,
                "sampling": args.sampling,
                "warmups": args.warmups,
                "runs": args.runs,
                "seed": args.seed,
                "max_mel_tokens": args.max_mel_tokens,
                "profile_kernels": args.profile_kernels,
                "wav_path": str(output / f"{stem}.wav"),
                "result_path": str(result_path),
            }
            config_path.write_text(json.dumps(config), encoding="utf-8")
            try:
                subprocess.run(
                    [
                        sys.executable,
                        str(Path(__file__).resolve()),
                        "--worker-config",
                        str(config_path),
                    ],
                    check=True,
                )
            finally:
                config_path.unlink(missing_ok=True)
            results.append(json.loads(result_path.read_text(encoding="utf-8")))

    comparisons: list[dict[str, object]] = []
    if set(runtimes) == {"stable", "beta"}:
        for model in models:
            stable = next(
                result
                for result in results
                if result["model"] == model and result["runtime"] == "stable"
            )
            beta = next(
                result
                for result in results
                if result["model"] == model and result["runtime"] == "beta"
            )
            comparison = {"model": model, **_comparison(stable, beta)}
            comparisons.append(comparison)
            speedup = cast(dict[str, int], comparison["speedup"])
            decimal_speedup = Decimal(speedup["numerator"]) / Decimal(
                speedup["denominator"]
            )
            print(
                f"IndexTTS {model}: {decimal_speedup:.3f}x, "
                f"Beta won {comparison['beta_wins']}/{comparison['paired_runs']} runs"
            )

    summary = {"results": results, "comparisons": comparisons}
    (output / "summary.json").write_text(
        json.dumps(summary, indent=2), encoding="utf-8"
    )
    if args.sampling == "deterministic" and any(
        not comparison["semantic_tokens_match"]
        or not comparison["sample_rates_match"]
        or not comparison["waveform_shapes_match"]
        or not comparison["finite_audio"]
        or not comparison["waveform_close"]
        for comparison in comparisons
    ):
        raise RuntimeError("IndexTTS Beta failed deterministic parity")


def main() -> None:
    args = _parser().parse_args()
    if args.worker_config is not None:
        _run_worker(args.worker_config)
    else:
        _main(args)


if __name__ == "__main__":
    main()
