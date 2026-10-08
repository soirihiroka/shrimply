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
