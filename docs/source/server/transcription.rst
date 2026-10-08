Transcription
=============

Selected audio becomes timed captions on a new caption track.

Create captions
---------------

#. Select one or more audio clips, or select an audio track.
#. Open the timeline context menu and choose :guilabel:`Transcribe`.
#. Choose a speech-to-text model. The server can offer Parakeet, Qwen3 ASR,
   Whisper, and Distil-Whisper.
#. Select :guilabel:`Transcribe` and wait for it to finish.

If no models appear, check the server in
:menuselection:`Preferences --> External`.

Follow edit points
------------------

:guilabel:`Follow cuts` snaps a caption boundary to a nearby edit.
:guilabel:`Snap source` chooses which edits count.
:guilabel:`Snap tolerance` is how close the boundary has to be.

Turn :guilabel:`Follow cuts` off to keep the model's own timing.

If no speech is found, the project stays unchanged.
