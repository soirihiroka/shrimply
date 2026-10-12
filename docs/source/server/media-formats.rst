Media formats and uploads
=========================

The compute server decodes encoded audio and resamples it to the selected
model's required rate. The editor uploads original TTS reference audio instead
of converting it to WAV first. Voice and emotion references retain their
15-second limit.

Audio support includes WAV, MP3, FLAC, AIFF, M4A/AAC, ALAC, and Ogg/Vorbis/Opus
when the installed PyAV/FFmpeg decoders support the encoding. Encoded buffers
are identified by their contents, independently of their filename. Raw PCM
buffers support mono or stereo, interleaved little-endian float32 samples.

Video-generation images and SAM2 archive frames support Pillow's available
raster decoders, including JPEG, PNG, WebP, TIFF, BMP, GIF, AVIF, JPEG 2000,
QOI, ICO, PPM, and TGA.
Images are oriented using EXIF metadata and converted to RGB. Transparent
pixels are composited onto white. Animated or multi-page images use their
first frame. SAM2 frames must still be 1024 by 1024 pixels after orientation.
SVG, PDF, and layered project formats are not accepted by these loaders.

MessagePack audio API
---------------------

Protocol 4.1 adds two reusable audio shapes. ``data`` is a MessagePack binary
value, not a filename or a base64 string:

.. code-block:: text

   {kind: "encoded", data: <encoded audio bytes>}
   {kind: "pcm", data: <float32 bytes>, sample_rate: 48000, channels: 2}

Send transcription requests to ``POST /transcriptions`` with
``Content-Type: application/msgpack``:

.. code-block:: text

   {model: "openai/whisper-small", audio: <either audio shape>}

Transcription responses retain their existing streamed MessagePack events.
The existing ``Shrimply-Job-ID`` UUID header, job heartbeat, and cancellation
requirements also apply to MessagePack requests.
Segment frame indices always refer to 16 kHz audio, regardless of the upload
rate. The maximum audio buffer and decoded mono output are each 512 MiB.

TTS audio-input ``value`` accepts either shape, with a 64 MiB upload limit.
Pneuma's ``audio`` field also accepts either shape with a 64 MiB upload limit.
Video-generation audio media accepts either shape in its ``data`` field;
its existing 256 MiB per-item and 512 MiB combined upload limits remain.
Decoded audio allocations are bounded at 512 MiB, and invalid or non-finite
samples fail the request.

Compatibility
-------------

Existing clients can continue sending raw 16 kHz mono float32 transcription
bodies as ``application/octet-stream`` with the ``model`` query parameter.
Legacy TTS ``{wav: <WAV bytes>}``, Pneuma binary ``audio``, and generation
binary ``data`` fields also remain accepted. Returned speech and converted
audio remain WAV. SAM2's archive framing is unchanged.

Updated editors require the updated server for the new upload shapes.
