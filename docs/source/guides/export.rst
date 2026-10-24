Export
======

Export types
------------

The export window can write a rendered video, captions, or JSON project data.

Captions and plain text
-----------------------

macOS and the GTK app export YouTube YTT, Advanced SubStation Alpha (ASS),
SubRip (SRT), WebVTT (VTT), and plain text (TXT). YTT is the default. Merge
the enabled caption tracks into one file, or write each track on its own.
Separate files are named ``name-track-N.extension``. The Qt app exports YTT.

Captions are ordered by start time. Disabled tracks and blank captions are
skipped. Shrimply asks before replacing an existing file. If there is nothing
to export, it stops and writes no file.

TXT is the caption text, with a blank line between captions. Line breaks stay.
Ruby is dropped and the base text stays.

YTT keeps YouTube styling, placement, ruby, and timed spans.

ASS keeps emphasis, fonts, size, colors, opacity, placement, rotation, and
timed text reveals. Outlines and shadows stand in for edge effects. A
background box wins over an outline. The file uses the project canvas size
and a 32-pixel base font scaled by each caption's font scale.

VTT keeps emphasis, ruby, in-cue timestamps, supported CSS styling, and
horizontal or vertical placement.

SRT keeps emphasis and the text. Ruby is written as ``base (annotation)``.
Layout and timed spans are dropped.

Vertical text in ASS, and rotated text in VTT, is written horizontally.
Overlapping captions stay as separate cues.

Video and GIF
-------------

Video export is H.264 or H.265 through NVENC, or GIF. There is no software
encoder. Containers are MP4, Matroska, and GIF. Audio is AAC, FDK AAC, or
Opus, depending on the container.

Command-line rendering
----------------------

Render a saved project without opening the launcher or editor::

   shrimply render project.shrimp -o video.mp4
   shrimply render project.shrimp -o video.mkv --codec h265 --fps 30000/1001
   shrimply render project.shrimp -o animation.gif
   shrimply render --help

The Qt and macOS applications accept the same commands through ``shrimply-qt``
and ``shrimply-appkit``. Use the executable inside the application bundle on
macOS: ``Shrimply.app/Contents/MacOS/Shrimply``.

Rendering uses the saved file, including when the project is open in another
process. Unsaved editor changes are excluded. Supported inputs are ``.shrimp``,
``.sjson``, and legacy ``.json`` projects. Convert OTIO and Kdenlive timelines
in the editor first. Projects that need frame-grid repair must also be opened
and saved in the editor first. Blender and Manim sources must already be
approved through Shrimply's source trust controls.

The output keeps the project dimensions and frame rate. The default video
codec is H.264 at 10,000 Kbps, with AAC audio at 48,000 Hz and 192 Kbps. GIF
output selects GIF encoding and has no audio. Rendering uses the existing
GPU backend: CUDA/NVENC on Linux and Windows, or Metal/VideoToolbox on macOS.
The same GPU and media dependencies as editor export are required.

Options:

* ``-o PATH`` or ``--output PATH``: required destination. Its extension selects
  MP4, Matroska (``.mkv``), or GIF.
* ``--container mp4|mkv|gif``: select the container explicitly. If the output
  has no extension, append the matching extension. Conflicting extensions fail.
* ``--codec h264|h265|gif``: select the video codec. GIF requires a GIF container.
* ``--fps RATE``: override the output frame rate with an integer, decimal,
  or fraction such as ``30000/1001``.
* ``--video-bitrate KBPS``: set the video bitrate using bitrate-based encoding.
* ``--audio-encoder aac|fdk-aac|opus``: select the audio encoder. macOS also
  accepts ``audiotoolbox-aac``. The encoder must be available in FFmpeg.
* ``--audio-bitrate KBPS`` and ``--audio-sample-rate HZ``: set audio encoding
  parameters. GIF ignores audio parameters.
* ``--overwrite``: allow replacing an existing output file. Otherwise the
  command fails if the destination already exists. The project and source
  assets cannot be used as output destinations.

The command waits until export completes, prints progress and errors to
standard error, and publishes the finished file through a temporary file.
Exit status is ``0`` for success, ``1`` for a loading or rendering failure,
and ``2`` for invalid arguments. Use ``--`` before a project path that begins
with a hyphen. Quote paths containing spaces.
