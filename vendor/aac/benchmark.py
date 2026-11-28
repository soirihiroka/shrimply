#!/usr/bin/env python3
"""Compare AAC-LC encoders at measured payload rates with aligned stereo PCM."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import resource
import statistics
import subprocess
import tempfile
import time

import numpy as np

RATE = 48_000
PRIMING = 1024
MAX_LAG = 4096


def run(*args):
    return subprocess.check_output(args, stderr=subprocess.PIPE)


def ffmpeg(*args):
    return run("ffmpeg", "-hide_banner", "-v", "error", "-xerror", "-y", *args)


def decode(file):
    return np.frombuffer(
        ffmpeg("-i", file, "-map", "0:a:0", "-f", "f32le", "-ac", "2", "-ar", str(RATE), "-"),
        dtype="<f4",
    ).reshape(-1, 2)


def cases():
    rng = np.random.default_rng(148)
    t = np.arange(RATE * 3 + 315) / RATE
    tone = np.stack([0.35 * np.sin(2 * np.pi * hz * t) for hz in (997, 1451)], axis=1)
    phase = (t / 3 + 0.51) * 2 * np.pi
    stereo = np.stack(
        [0.3 * np.sin(2 * np.pi * 613 * t + offset * phase) for offset in (1, -1)], axis=1
    )
    # Vary attack phase relative to the long window, with a silent lead-in for each attack.
    onsets = RATE // 4 + np.arange(8) * (RATE // 3 + 57)
    transient = np.zeros_like(tone)
    length = 1100
    decay = np.exp(-np.arange(length) / 145)
    for onset in onsets:
        transient[onset : onset + length] = 0.33 * rng.uniform(-1, 1, (length, 2)) * decay[:, None]
    return [("tones", tone, []), ("stereo", stereo, []), ("attacks", transient, onsets.tolist())]


def payload_size(path):
    data = path.read_bytes()
    position = overhead = 0
    while position < len(data):
        header = data[position : position + 7]
        assert len(header) == 7 and header[0] == 0xff and header[1] & 0xf6 == 0xf0
        assert header[2] >> 6 == 1, "Expected AAC-LC"
        assert (header[2] >> 2) & 15 == 3 and ((header[2] & 1) << 2) | (header[3] >> 6) == 2
        assert header[6] & 3 == 0, "Expected one raw data block per ADTS frame"
        frame = ((header[3] & 3) << 11) | (header[4] << 3) | (header[5] >> 5)
        header_size = 7 if header[1] & 1 else 9
        assert frame > header_size
        overhead += header_size
        position += frame
    assert position == len(data), "Truncated ADTS frame"
    return len(data) - overhead


def align(reference, decoded, known_lag):
    # Select a single joint L/R lag. Periodic probes use the known exact codec delay.
    if known_lag is None:
        count = min(len(reference), len(decoded) - MAX_LAG)
        span = count + MAX_LAG
        fft_size = 1 << (2 * span - 1).bit_length()
        ref = np.fft.rfft(reference[:count], n=fft_size, axis=0)
        test = np.fft.rfft(decoded[:span], n=fft_size, axis=0)
        correlation = np.fft.irfft((test * ref.conj()).sum(axis=1), n=fft_size)[: MAX_LAG + 1]
        power = np.r_[0, np.cumsum(np.square(decoded[:span], dtype=np.float64).sum(axis=1))]
        valid = power[count:] - power[: MAX_LAG + 1]
        lag = int(np.argmax(correlation / np.sqrt(np.maximum(valid, 1e-20))))
    else:
        lag = known_lag
    if len(decoded) - lag < len(reference):
        raise ValueError(f"Decoder truncated PCM at lag {lag}: {len(decoded)} vs {len(reference)}")
    aligned = decoded[lag : lag + len(reference)]
    if not np.all(np.isfinite(aligned)):
        raise ValueError("Decoder produced non-finite PCM")
    return lag, aligned


def db_ratio(numerator, denominator):
    return 10 * np.log10(np.maximum(numerator, 1e-20) / np.maximum(denominator, 1e-20))


def stereo_view(samples):
    left, right = samples.T
    return np.stack((left, right, (left + right) * 0.5, (left - right) * 0.5), axis=1)


def spectrum(audio, size):
    windows = np.lib.stride_tricks.sliding_window_view(audio, size, axis=0)[:: size // 2]
    return np.square(np.abs(np.fft.rfft(windows * np.hanning(size), axis=-1)))


def metrics(reference, decoded, onsets):
    ref, actual = map(stereo_view, (reference, decoded))
    ref_energy, actual_energy = (np.square(signal).sum(axis=0) for signal in (ref, actual))
    result = {
        "gain_db_lr_ms": db_ratio(actual_energy, ref_energy).tolist(),
        "waveform_snr_db_lr_ms": db_ratio(ref_energy, np.square(ref - actual).sum(axis=0)).tolist(),
        "clipped_samples": int((np.abs(decoded) >= 1.0).sum()),
        "stereo_balance_delta_db": float(db_ratio(*actual_energy[2:]) - db_ratio(*ref_energy[2:])),
    }

    # Diagnostics for distinct time/frequency resolutions, above an 80 dB
    # per-channel floor. Keep left, right, mid and side apart in the report.
    for size in (256, 1024, 4096):
        x, y = spectrum(ref, size), spectrum(actual, size)
        floor = np.maximum(x.max(axis=(0, 2), keepdims=True), 1e-12) * 1e-8
        audible = x >= floor
        difference = db_ratio(np.maximum(x, floor), np.maximum(y, floor))
        weights = np.sqrt(x) * audible
        result[f"spectral_rmse_{size}_db_lr_ms"] = np.sqrt(
            (weights * np.square(difference)).sum(axis=(0, 2))
            / np.maximum(weights.sum(axis=(0, 2)), 1e-20)
        ).tolist()
    if onsets:
        error = np.square(reference - decoded)
        power = np.square(reference)
        result["pre_echo_db"] = [
            float(db_ratio(error[onset - 480 : onset].sum(), power[onset : onset + 480].sum()))
            for onset in onsets
        ]
    return result


def trials(command, repeat):
    wall, cpu = [], []
    for _ in range(repeat + 1):
        usage = resource.getrusage(resource.RUSAGE_CHILDREN)
        start = time.perf_counter()
        try:
            run(*command)
        except subprocess.CalledProcessError as error:
            raise RuntimeError(error.stderr.decode(errors="replace")) from error
        wall.append(time.perf_counter() - start)
        end = resource.getrusage(resource.RUSAGE_CHILDREN)
        cpu.append(end.ru_utime + end.ru_stime - usage.ru_utime - usage.ru_stime)
    return wall[1:], cpu[1:]


def matched_rates(rows):
    key = "visqol_moslqo"
    by_item = {}
    for row in rows:
        by_item.setdefault(row["item"], {}).setdefault(row["encoder"], []).append(row)
    results = []
    for item, encoders in by_item.items():
        if len(encoders) < 2:
            continue
        low = max(min(row["payload_bps"] for row in group) for group in encoders.values())
        high = min(max(row["payload_bps"] for row in group) for group in encoders.values())
        for rate in (96_000, 128_000, 192_000, 256_000):
            if not low <= rate <= high:
                continue
            for encoder, group in encoders.items():
                group.sort(key=lambda row: row["payload_bps"])
                record = dict(item=item, encoder=encoder, interpolated_at_bps=rate)
                for metric in (key, "spectral_rmse_1024_db_lr_ms", "stereo_balance_delta_db"):
                    if not all(metric in row for row in group):
                        continue
                    values = np.array([row[metric] for row in group])
                    rates = np.log([row["payload_bps"] for row in group])
                    record[metric] = (
                        float(np.interp(np.log(rate), rates, values))
                        if values.ndim == 1 else
                        [float(np.interp(np.log(rate), rates, channel)) for channel in values.T]
                    )
                results.append(record)
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("encoder", type=Path, help="Optimized Rust ADTS encoder example")
    parser.add_argument("--corpus", nargs="*", type=Path, default=[])
    parser.add_argument("--bitrates", nargs="+", type=int, default=[64, 96, 128, 192, 256, 320])
    parser.add_argument("--repeat", type=int, default=3)
    parser.add_argument("--visqol", type=Path, help="ViSQOL CLI built with its audio model")
    parser.add_argument("--fdk", action="store_true", help="Require libfdk_aac in the benchmark FFmpeg")
    parser.add_argument("--smoke", action="store_true", help="Verify the Shrimply codec only")
    args = parser.parse_args()
    if args.repeat < 1:
        parser.error("--repeat must be positive")
    encoders = ["shrimply"] if args.smoke else ["shrimply", "aac"]
    if args.fdk:
        if b"libfdk_aac" not in run("ffmpeg", "-encoders"):
            parser.error("--fdk requires an FFmpeg build with libfdk_aac")
        encoders.append("libfdk_aac")
    rows = []
    with tempfile.TemporaryDirectory(prefix="shrimply-aac-") as folder:
        raw, adts, aligned, source_wav, output_wav = (
            Path(folder) / name for name in ("source.f32", "output.aac", "decoded.f32", "source.wav", "decoded.wav")
        )
        pcm = ("-f", "f32le", "-ar", str(RATE), "-ac", "2")
        for item, reference, onsets in cases() + [(str(file), decode(file), []) for file in args.corpus]:
            reference = np.asarray(reference, dtype="<f4")
            data = reference.tobytes()
            raw.write_bytes(data)
            if args.visqol:
                ffmpeg(*pcm, "-i", raw, "-c:a", "pcm_s24le", source_wav)
            seconds = len(reference) / RATE
            for bitrate in ([192] if args.smoke else args.bitrates):
                for codec in encoders:
                    bps = str(bitrate * 1000)
                    if codec == "shrimply":
                        command = (args.encoder.resolve(), raw, adts, str(RATE), bps)
                    else:
                        options = ("-vbr", "0", "-afterburner", "1") if codec == "libfdk_aac" else ()
                        command = ("ffmpeg", "-hide_banner", "-v", "error", "-xerror", "-y",
                                   *pcm, "-i", raw, "-threads", "1", "-c:a", codec,
                                   "-profile:a", "aac_low", "-b:a", bps, *options, "-f", "adts", adts)
                    wall, cpu = trials(command, 1 if args.smoke else args.repeat)
                    decoded = decode(adts)
                    known_lag = PRIMING if codec in ("shrimply", "aac") else None
                    lag, decoded = align(reference, decoded, known_lag)
                    row = dict(item=item, pcm_sha256=hashlib.sha256(data).hexdigest(),
                               encoder=codec, target_bps=int(bps), duration_seconds=seconds,
                               payload_bps=round(8 * payload_size(adts) / seconds),
                               delay_samples=lag, wall_seconds=wall, cpu_seconds=cpu,
                               realtime=seconds / statistics.median(wall),
                               **metrics(reference, decoded, onsets))
                    if args.visqol:
                        aligned.write_bytes(decoded.astype("<f4").tobytes())
                        ffmpeg(*pcm, "-i", aligned, "-c:a", "pcm_s24le", output_wav)
                        score = run(args.visqol.resolve(), "--reference_file", source_wav,
                                    "--degraded_file", output_wav).decode()
                        row["visqol_moslqo"] = float(re.search(r"MOS-LQO:\s*([0-9.]+)", score)[1])
                    rows.append(row)
                    if args.smoke and item == "tones":
                        assert min(row["waveform_snr_db_lr_ms"][:2]) > 24, row
                        assert max(map(abs, row["gain_db_lr_ms"][:2])) < 0.5, row
                    if args.smoke and item == "attacks":
                        assert min(row["waveform_snr_db_lr_ms"][:2]) > 12, row
    print(json.dumps(dict(
        sample_rate=RATE,
        ffmpeg=run("ffmpeg", "-version").decode().splitlines()[:2],
        machine=os.uname().machine,
        rows=rows,
        matched_rates=matched_rates(rows),
    ), indent=2, allow_nan=False))


if __name__ == "__main__":
    main()
