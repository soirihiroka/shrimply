Voice Conversion
================

The :guilabel:`Voice Change` modifier replaces recorded speech with an
installed Pneuma voice.

Install voice models
--------------------

For a local server, put ``.safetensors`` or ``.pth`` models in
``server/models``. Set ``SHRIMPLY_PNEUMA_MODEL_DIR`` to use another directory.

With Docker Compose, put models in ``server/.docker/pneuma/models``.

Change a voice
--------------

#. Select an audio clip that contains speech.
#. Add the :guilabel:`Voice Change` modifier.
#. Choose a model from the server.
#. Preview the clip and adjust pitch, speed, or the F0 method if you need to.

Requirements and limits
-----------------------

* Runs on **CUDA or CPU**.
* **Audio limit**: up to **64 MiB** per uploaded clip and **512 MiB** decoded
  audio. Converted audio is returned as WAV.
