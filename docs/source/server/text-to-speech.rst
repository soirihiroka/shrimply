Text to Speech
==============

Create a speech item
--------------------

#. Open the add menu on an audio track.
#. Choose :guilabel:`Text to Speech`.
#. Select the new item and enter its text in the inspector.
#. Choose a model and set the controls it shows.
#. Generate the speech.

Generate speech for captions
----------------------------

Select caption clips and choose :guilabel:`Generate Speech` from the timeline
context menu. The same action on a caption track reads every caption that has
text.

Pick a model that supports caption timing. Each caption is spoken at its
current duration. The audio lands on a track where it does not overlap
existing clips.

Models and licenses
-------------------

The server supports Qwen3 TTS (built-in voices, voice cloning, and voice
design) and IndexTTS 2 and 2.5.

IndexTTS is under the :ref:`bilibili Model Use License Agreement <indextts-license>`.
Read that agreement before downloading the model.
