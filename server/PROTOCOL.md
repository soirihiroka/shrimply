# Shrimply Compute Server Protocol

This document records the HTTP/MessagePack wire contract used between Shrimply
and the compute server (protocol 4.1). It is intended for contributors and
integrators implementing compatible clients or servers.

## Endpoints

The server (`server/src/main.py`) exposes:

- `GET /` — server status, protocol version, capabilities, Torch devices, and compute load.
- `GET /tts/models`, `GET /pneuma/models`, `GET /video-generation/models` — model catalogs.
- `PUT /compute/device?device=<id>` — select active compute device (`cpu`, `cuda:0`, ...).
- `PUT /compute/jobs/{job_id}/heartbeat` — keep an active job alive.
- `DELETE /compute/jobs/{job_id}` — cancel an active job.
- `POST /transcriptions` — speech-to-text transcription.
- `POST /speech` — text-to-speech synthesis.
- `POST /pneuma/conversions` — voice conversion.
- `POST /video-generations` — video generation.
- `POST /sam2/analyses` — SAM2 video segmentation.
- `POST /3dtracking/analyses` — 3D camera tracking.

## MessagePack audio API (Protocol 4.1)

Protocol 4.1 adds two reusable audio shapes. `data` is a MessagePack binary
value, not a filename or base64 string:

```text
{kind: "encoded", data: <encoded audio bytes>}
{kind: "pcm", data: <float32 bytes>, sample_rate: 48000, channels: 2}
```

- Encoded buffers are identified by content via PyAV/FFmpeg independently of filename.
- Raw PCM buffers accept mono or stereo interleaved little-endian float32 samples.
- Decoded audio allocations are bounded at 512 MiB; invalid or non-finite samples fail the request.

### Transcription (`POST /transcriptions`)

Send MessagePack requests with `Content-Type: application/msgpack`:

```text
{model: "openai/whisper-small", audio: <either audio shape>}
```

- Responses use streamed MessagePack events.
- `Shrimply-Job-ID` UUID header, job heartbeat, and cancellation requirements apply.
- Segment frame indices always refer to 16 kHz audio, regardless of upload rate.
- Maximum audio buffer and decoded mono output are each 512 MiB.

### Other audio endpoints

- **TTS (`POST /speech`)**: audio-input `value` accepts either shape, 64 MiB upload limit (65 MiB total request). Voice and emotion references have a 15-second duration limit.
- **Voice conversion (`POST /pneuma/conversions`)**: `audio` accepts either shape, 64 MiB upload limit (65 MiB total request), 512 MiB decoded audio limit.
- **Video generation (`POST /video-generations`)**: audio media `data` accepts either shape; 256 MiB per-item and 512 MiB combined media limits (513 MiB total request).

### Image and archive loaders

- Video-generation images and SAM2 archive frames use Pillow raster decoders (`NON_RASTER_FORMATS`: EPS, PDF, PSD, WMF are rejected). Images are EXIF-transposed, converted to RGB, transparent pixels composited onto white, and animated/multi-page images use frame 0.
- SAM2 archives require 1024×1024 frames after orientation (8 GiB max archive, 16 MiB max frame).
- 3D tracking archives have an 8 GiB max archive and 64 MiB max JPEG frame limit.

## Legacy client compatibility

- `POST /transcriptions` still accepts raw 16 kHz mono float32 bodies as `application/octet-stream` with the `model` query parameter.
- Legacy TTS `{wav: <WAV bytes>}`, Pneuma binary `audio`, and video-generation binary `data` fields remain accepted.
- Returned speech and converted audio remain WAV. SAM2 archive framing is unchanged.
- Updated editors require an updated server for Protocol 4.1 upload shapes.
