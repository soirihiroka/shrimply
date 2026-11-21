Compute Server
==============

The compute server is optional. It runs transcription, text-to-speech, video
segmentation, voice conversion, camera tracking, and video generation on a
local or remote Linux machine.

Requirements
------------

* **OS**: Linux (other OS untested).
* **Local run**: Python 3.14 and `uv <https://docs.astral.sh/uv/>`__.
* **Docker run**: Docker with the NVIDIA Container Toolkit for GPU access.
* **GPU vs. CPU**:

  * **Video Generation** and **VGGT-SLAM** camera tracking require an NVIDIA
    CUDA GPU and are hidden in Shrimply when the server is set to CPU.
  * **Transcription**, **Text to Speech**, **Video Segmentation (SAM2)**,
    **Voice Conversion**, and **COLMAP** camera tracking work on CUDA or CPU.

Run locally
-----------

From the repository root:

.. code-block:: console

   $ make dev-server

Or from the ``server`` directory:

.. code-block:: console

   $ uv run --locked src/main.py

Models download the first time you use them. Leave the server running until
the job finishes.

Configuration
~~~~~~~~~~~~~

Useful environment variables:

* ``SHRIMPLY_SERVER_HOST`` (default ``0.0.0.0``) and ``SHRIMPLY_SERVER_PORT``
  (default ``8787``) — listen address and port.
* ``SHRIMPLY_SERVER_SHARE=1`` — opens a temporary public ``gradio.live`` URL.
  Anyone with that URL can run every model; share it only with people you
  trust and stop the process when finished.
* ``SHRIMPLY_MODEL_IDLE_TTL_SECONDS`` (default ``600``) — idle time before
  unloading an unused worker model.
* ``SHRIMPLY_PNEUMA_MODEL_DIR`` — directory for voice-conversion models.

Run with Docker Compose
-----------------------

Docker Compose enables the GPU and persists downloaded models across runs:

.. code-block:: console

   $ cd server
   $ docker compose up

The default image uses **CUDA 12.6**. For **CUDA 13.0** (requires a Turing or
newer NVIDIA GPU and a compatible driver; keep CUDA 12.6 for older supported
GPUs):

.. code-block:: console

   $ SHRIMPLY_SERVER_TAG=prerelease-cuda13.0 docker compose up

To build server container images locally, see :doc:`/development`.

Connect Shrimply
----------------

The server listens on port ``8787`` by default (``http://127.0.0.1:8787``
locally). In Shrimply, open :menuselection:`Preferences --> External`, select
the server under :guilabel:`Inference Servers`, and choose a compute device.

Shrimply only displays features supported by the selected server and device.

Supported audio formats
-----------------------

Audio uploads accept WAV, MP3, FLAC, AIFF, M4A/AAC, ALAC, and Ogg/Vorbis/Opus
(decoded and resampled automatically to each model's required rate). Per-feature
upload and duration limits are listed on each feature page below.

Features
--------

* :doc:`transcription`
* :doc:`text-to-speech`
* :doc:`video-segmentation`
* :doc:`voice-conversion`
* :doc:`camera-tracking`
* :doc:`video-generation`

Troubleshooting
---------------

* **No models or missing features**: check the selected server and device in
  :menuselection:`Preferences --> External`. Video Generation and VGGT-SLAM
  only appear when a CUDA device is selected.
* **First request takes a long time**: models download on first use and are
  cached afterward.
* **Upload or size error**: check the file size and duration limits on the
  feature's page (for example, 15 seconds for TTS voice references, or 64–512
  MiB upload caps).
* **Device switch rejected (server busy)**: wait for active compute jobs to
  finish or cancel them before switching compute devices.

.. toctree::
   :maxdepth: 1
   :hidden:

   transcription
   text-to-speech
   video-segmentation
   voice-conversion
   camera-tracking
   video-generation
