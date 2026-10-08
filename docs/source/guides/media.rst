Media
=====

.. toctree::
   :hidden:

   blender
   kdenlive-import
   manim

.. |add| image:: ../../../assets/icons/plus-symbolic.svg
   :class: action-icon
   :alt: Add
.. |import| image:: ../../../assets/icons/document-open-symbolic.svg
   :class: action-icon
   :alt: Import
.. |text| image:: ../../../assets/icons/draw-text-symbolic.svg
   :class: action-icon
   :alt: Text
.. |shape| image:: ../../../assets/icons/shapes-large-symbolic.svg
   :class: action-icon
   :alt: Shape
.. |paint| image:: ../../../assets/icons/applications-graphics-symbolic.svg
   :class: action-icon
   :alt: Paint
.. |background| image:: ../../../assets/icons/preferences-desktop-wallpaper-symbolic.svg
   :class: action-icon
   :alt: Background
.. |scene-3d| image:: ../../../assets/icons/3d-object-symbolic.svg
   :class: action-icon
   :alt: 3D Scene
.. |video-generation| image:: ../../../assets/icons/video-generation-symbolic.svg
   :class: action-icon
   :alt: Video Generation
.. |text-to-speech| image:: ../../../assets/icons/font-x-generic-symbolic.svg
   :class: action-icon
   :alt: Text to Speech
.. |audio-generator| image:: ../../../assets/icons/sound-symbolic.svg
   :class: action-icon
   :alt: Audio Generator
.. |screen-recording| image:: ../../../assets/icons/screencast-recorded-symbolic.svg
   :class: action-icon
   :alt: Screen recording
.. |microphone| image:: ../../../assets/icons/mic-1-symbolic.svg
   :class: action-icon
   :alt: Microphone

Import media
------------

Move the playhead to the desired start time, then select the destination track.
Click |add| **Add** in that track's controls and choose |import| **Import
Media…** or **Import Captions…**. The file is inserted at the playhead on the
track whose menu you opened. If several tracks of that type are selected, the
file goes on all of them.

Video and audio files can only be imported to video or audio tracks. WebVTT
files can only be imported to caption tracks. MKV and WebM imports ask for
confirmation before losslessly remuxing to MP4 in the source file's directory.
An existing MP4 is never overwritten. The timeline uses the new MP4, and after
a successful import a dialog offers to keep or delete the original. Keeping the
original is the default; cancelling or failing the import never deletes it.

Supported formats
~~~~~~~~~~~~~~~~~

Shrimply recognizes these source types:

.. list-table::
   :widths: 25 75
   :header-rows: 1

   * - Type
     - Formats
   * - Video containers
     - MP4, MOV, MKV, and WebM
   * - Video codecs
     - H.264/AVC, H.265/HEVC, AV1, VP8, VP9, and other codecs with FFmpeg
       CUDA decoding
   * - Images and documents
     - JPEG, PNG, WebP, AVIF, GIF, SVG, and PDF
   * - Audio
     - AAC, AIFF, ALAC, FLAC, M4A, MP3, Ogg, Opus, and WAV
   * - Captions
     - WebVTT
   * - Layered images
     - PSD and Krita
   * - 3D content
     - OBJ, GLB, and PLY
   * - Application sources
     - :doc:`Blender files <blender>` and :doc:`Manim scenes <manim>`

Project opening additionally accepts native Shrimply projects, Shrimply JSON,
OpenTimelineIO, and :doc:`Kdenlive projects <kdenlive-import>`.

Video decoding requires both a matching FFmpeg CUDA decoder and codec support
from the installed NVIDIA GPU.

Create media
------------

Move the playhead to an empty point on the destination track and click |add|
**Add**. The new item starts at the playhead, uses the default visual duration,
and ends early rather than overlapping the next item.
Nothing is created if the playhead is already inside an item on that track.

On a video track, the menu provides:

.. list-table::
   :widths: 8 92
   :header-rows: 1

   * - Icon
     - Description
   * - |text|
     - **Text** creates a text layer using the default font.
   * - |shape|
     - **Shape** creates a vector shape. Rectangle, ellipse, triangle, star,
       arrow, diamond, polygon, heart, and cross shapes are available in the
       inspector.
   * - |paint|
     - **Paint** creates a layer for drawing strokes directly on the canvas.
   * - |background|
     - **Background** creates a full-canvas background layer.
   * - |scene-3d|
     - **3D Scene** creates a scene that can contain shapes, text, models,
       lights, and a ground plane.
   * - |video-generation|
     - **Video Generation** creates an AI video-generation item. This requires the
       :doc:`Shrimply server <../server/index>`.

On an audio track, the menu provides:

.. list-table::
   :widths: 8 92
   :header-rows: 1

   * - Icon
     - Description
   * - |text-to-speech|
     - **Text to Speech** creates an item that generates speech from text using
       the Shrimply server.
   * - |audio-generator|
     - **Audio Generator** creates a procedural audio item configured in the
       inspector.

Record content
--------------

* On a video track, click |screen-recording| **Screen Recording**, choose a
  screen or application in the system capture dialog, and click the same button
  again to stop. The recording starts at the playhead and stops before the next
  item on the track.
* On an audio track, click |microphone| **Microphone** to start recording from
  the default microphone. Click the same button again to stop.

The active recording button turns red. Recording advances playback and places
the finished item on that track at the original playhead position. Start from
an empty point so the recording does not overlap an existing item.
