Compute Server
==============

The compute server is optional. It runs transcription, text-to-speech, video
segmentation, voice conversion, camera tracking, and video generation on this
machine.

Run locally
-----------

The server needs Python 3.14. From the repository root:

.. code-block:: console

   $ make dev-server

From the ``server`` directory:

.. code-block:: console

   $ uv run --locked src/main.py

Models download the first time you use them. Leave the server running until
the job finishes.

Connect Shrimply
----------------

The server listens at ``http://127.0.0.1:8787``. In Shrimply, open
:menuselection:`Preferences --> External`, select the local server under
:guilabel:`Inference Servers`, and pick a device.

Shrimply shows the features that server offers. Anything it does not offer
stays out of the editor.

Share access
------------

``SHRIMPLY_SERVER_SHARE=1`` opens a temporary public ``gradio.live`` URL.
That URL can run every model. Share it with people you trust, and stop the
process when you want it gone.

.. code-block:: console

   $ SHRIMPLY_SERVER_SHARE=1 uv run --locked src/main.py

Containers
----------

Docker Compose enables the GPU and keeps downloaded models between runs.

.. code-block:: console

   $ cd server
   $ docker compose up

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

If a model is missing or the connection fails, check the selected server in
:menuselection:`Preferences --> External`. The first request waits while its
model downloads.

.. toctree::
   :maxdepth: 1
   :hidden:

   transcription
   text-to-speech
   video-segmentation
   voice-conversion
   camera-tracking
   video-generation
