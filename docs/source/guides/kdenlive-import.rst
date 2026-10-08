Kdenlive
========

Opening a ``.kdenlive`` file converts the active timeline into a new Shrimply
project. This is a one-way conversion. Media stays at its original path, and
proxy files are ignored. The source files have to remain where the project
points.

Only the **Supported** and **Approximate** rows below are converted. Anything
else is left out, even when Shrimply has a similar feature.

Status meanings
---------------

.. list-table::
   :widths: 20 80
   :header-rows: 1

   * - Status
     - Meaning
   * - Supported
     - The handled fields come across.
   * - Approximate
     - Comes across, with some detail simplified or dropped.
   * - Not imported
     - Left out, or kept only as a media path Shrimply may not play.
   * - Import stops
     - Import fails and writes nothing.

Project and timeline
--------------------

.. list-table::
   :widths: 30 16 54
   :header-rows: 1

   * - Kdenlive feature
     - Status
     - What you get
   * - Frame rate and canvas size
     - Supported
     - Taken from the project profile.
   * - Pixel aspect, scan mode, color metadata, profile name, and audio settings
     - Not imported
     - Shrimply keeps its own defaults.
   * - Project name
     - Approximate
     - The ``.kdenlive`` filename. Kdenlive's project title is ignored.
   * - Video and audio track order
     - Supported
     - Black tracks and timeline-preview tracks are dropped.
   * - Track enabled state
     - Supported
     - Kdenlive's hide setting (audio, video, or both) becomes the track's on/off state.
   * - Track names, locks, height, collapse, targeting, and track effects
     - Not imported
     - Dropped.
   * - Gaps, clip positions, in/out trims, and source durations
     - Supported
     - Positions, gaps, and trims follow the project frame rate.
   * - Both lanes of one Kdenlive track
     - Approximate
     - Each non-empty lane becomes its own track. Mixes between lanes are dropped.
   * - Nested sequences
     - Approximate
     - Sequences used on the timeline become folded sequences. Unused bin
       sequences, and captions inside nested sequences, are left out.
   * - Mixes, crossfades, wipes, and compositions
     - Not imported
     - Dropped.
   * - Timeline markers and clip markers
     - Approximate
     - Become comments with their text. Range length is kept. Point markers
       become one-frame comments. Clip markers follow that copy's trims and
       speed, including reverse. Markers inside a nested sequence show on the
       active timeline. Overlapping comments go on separate tracks. A clip that
       is both audio and video produces one comment. Category colors use
       Shrimply's palette. The comments do not move if you later edit the
       source clip.
   * - Clip groups, zones, notes, bin folders, thumbnails, and unused bin clips
     - Not imported
     - Dropped.
   * - Preview guides and editor UI state
     - Not imported
     - Dropped.

Media and generators
--------------------

.. list-table::
   :widths: 30 16 54
   :header-rows: 1

   * - Kdenlive source
     - Status
     - What you get
   * - Video and audio files
     - Supported
     - Imported with the selected video or audio stream. Playback still depends
       on Shrimply's media support.
   * - JPEG, PNG, WebP, BMP, TIFF, GIF, SVG, and PDF
     - Supported
     - Imported as images. A PDF uses its first page.
   * - Krita and PSD layered images
     - Supported
     - Layers are imported. Scaling uses nearest-neighbor.
   * - Constant speed, reverse playback, and pitch preservation
     - Supported
     - Speed, reverse, and pitch lock come across, including reversed trims.
   * - Variable speed (time remap)
     - Not imported
     - Speed ramps are left out.
   * - Source dimensions, stream selection, and media rotation
     - Supported
     - Imported when the file records them.
   * - Proxies
     - Approximate
     - The original file is used. The proxy is ignored.
   * - Solid color
     - Supported
     - Imported as a solid background, including transparency.
   * - Color bars
     - Approximate
     - Imported as Shrimply's static Test Pattern. Kdenlive's bar styles do
       not match, so the chosen style is dropped.
   * - White noise
     - Approximate
     - Imported as Shrimply white-noise video and audio. The noise pattern and
       stereo image are not reproduced, and a trimmed start can be off.
   * - Counter, including its beep
     - Approximate
     - Direction, text style, drop-frame counting, trims, and the gray
       background become animated text. Beep tones become separate one-frame
       1 kHz sine clips. The typeface can differ. The clock background's rings,
       crosshair, and sweep are left out.
   * - Other generators and playlists
     - Not imported
     - Left as media paths Shrimply may not play.
   * - Title clips and title templates
     - Not imported
     - Left out.
   * - Image sequences and slideshows
     - Not imported
     - Slideshow timing is left out.
   * - Missing-media placeholders
     - Not imported
     - Stay missing. Shrimply keeps the saved path.

Color bars, white noise, and counter clips are usually stored in separate
``.mlt`` files next to the project. Those files have to be readable during
import. After conversion, the Shrimply item no longer needs the generator file.

Video effects
-------------

Only enabled effects on a timeline clip are imported. Effects on the bin clip,
the track, or the sequence are skipped.

.. list-table::
   :widths: 30 16 54
   :header-rows: 1

   * - Kdenlive effect
     - Status
     - What you get
   * - Transform
     - Approximate
     - Position, scale, rotation, anchor, opacity, and their keyframes come
       across, including non-uniform scale. Stacked transforms are combined.
       Shear, and clipping in the middle of a stack, can be off.
   * - Normal and Screen blend modes
     - Supported
     - Other blend modes become Normal.
   * - Crop
     - Approximate
     - An animated rectangular crop comes across. Rounded, circular, or colored
       padding becomes a plain rectangle. A crop after rotation can follow the
       unrotated source.
   * - Gaussian blur
     - Approximate
     - Blurring only color, or only alpha, is kept. Any other channel choice
       blurs the whole image.
   * - Chroma key
     - Approximate
     - Key color, variance, and similarity come across, including animation on
       variance and similarity. An animated key color becomes one color. Other
       key settings use Shrimply's defaults.
   * - Saturation
     - Supported
     - Imported as color correction, including animation.
   * - Hue shift
     - Supported
     - Imported as color correction, including animation.
   * - Lift / gamma / gain
     - Approximate
     - The red channel becomes brightness, gamma, and value. Differences
       between channels are dropped.
   * - Fade from or to black
     - Supported
     - Imported as a clip fade in or fade out.
   * - Selective color correction
     - Not imported
     - Dropped.
   * - Every other video effect
     - Not imported
     - Dropped.

Effect masks
------------

Rectangle and ellipse Alpha Shapes masks are **Approximate**. They can cover
Crop, Gaussian Blur, Chroma Key, Saturation, Hue Shift, and Lift / Gamma /
Gain. A group of those effects shares one Shrimply mask.

Other mask shapes, other mask modes, nested groups, and groups that contain
any other effect are left out.

Audio effects
-------------

.. list-table::
   :widths: 30 16 54
   :header-rows: 1

   * - Kdenlive audio feature
     - Status
     - What you get
   * - Clip fade in and fade out
     - Supported
     - Imported as audio fades.
   * - Constant gain
     - Supported
     - Imported as a decibel Gain modifier.
   * - Animated volume
     - Supported
     - Level keyframes become a Gain modifier.
   * - Every other audio effect
     - Not imported
     - Dropped.
   * - Track audio effects and mixing
     - Not imported
     - Left out. Only effects on the clip are imported.

Captions
--------

.. list-table::
   :widths: 30 16 54
   :header-rows: 1

   * - Kdenlive caption feature
     - Status
     - What you get
   * - Subtitles on the active sequence (ASS)
     - Approximate
     - Timing, text, line breaks, and hard spaces come across.
   * - Hidden subtitles
     - Supported
     - A hidden subtitle track stays disabled.
   * - Fonts, outlines, position, and other styling
     - Approximate
     - Replaced with one centered caption at the bottom.
   * - Captions in nested sequences, and other subtitle formats
     - Not imported
     - Left out.

When import stops
-----------------

Import writes nothing if the project structure is invalid, or if a supported
effect or an attached file it needs is malformed. That includes a broken
project file, a missing frame rate or timeline, bad values in a supported
effect, and an unreadable PDF, layered image, subtitle file, or generator
file.

A missing media file is not an error during import. It shows up when Shrimply
tries to play that clip.
