Editor and Timeline
===================

Workspace
---------

Shrimply's main window has three areas: the **preview canvas**, the
**multitrack timeline**, and the **inspector**. Use the application menu to
show or hide the inspector and timeline.

Select any clip on the timeline to inspect and edit it:

* **Info tab**: shows the selected clip's timing, source dimensions, and file
  details.
* **Inspector controls**: change clip transforms, playback speed/repeat,
  modifiers, transitions, keyframes, property expressions, or source-specific
  settings (such as Blender cameras, Manim/Sanim parameters, or caption text).

Tracks
------

New projects start with a caption track, a video track, and an audio track.
Add more tracks from :menuselection:`New Track` in the application menu or the
timeline's empty-track menu:

* **Video tracks**: hold video, images, text, shapes, paint, backgrounds, 3D
  scenes, Blender, Manim, Sanim, and video-generation clips. Higher tracks
  composite over lower ones.
* **Audio tracks**: hold audio clips, recordings, procedural audio generators,
  and text-to-speech items.
* **Caption tracks**: hold styled, timed captions that render in the preview
  and export to YTT, ASS, SRT, VTT, or plain text (see :doc:`export`).
* **Comment tracks**: optional note tracks for timeline annotations. Add a
  comment with the track's **+** button or by double-clicking an empty spot on
  the track, then drag its edges to adjust its range and set its text and
  color in the inspector. Comments snap, split, and undo like clips, and stay
  out of the preview, caption exports, and rendered duration.

Tracks can be toggled on or off without deleting their clips. Disabled tracks
are excluded from preview playback and export.

Editing on the timeline
-----------------------

Drag clips to reposition them across time or compatible tracks, and drag a
clip's left or right edge to trim its in/out points.

If an import collides with existing clips, Shrimply lets you overwrite them,
cancel the import, or place the new clip on a new track.

Keyboard shortcuts
~~~~~~~~~~~~~~~~~~

In the GTK app, press :kbd:`Ctrl+?` (or choose **Keyboard Shortcuts** from the
application menu) to view the full shortcuts reference window. Common timeline
shortcuts:

* :kbd:`Space`: play or pause
* :kbd:`S`: split every clip at the playhead
* :kbd:`Shift+S`: split and select the clips on the left
* :kbd:`Q`: ripple-trim the selected clip to the playhead
* :kbd:`D`: delete the selection
* :kbd:`Shift+D`: ripple cut
* :kbd:`Ctrl+X`: cut
* :kbd:`Z`: toggle timeline zoom
* :kbd:`Ctrl+Z`: undo
* :kbd:`Ctrl+Shift+Z`: redo

Clip actions and grouping
-------------------------

Right-click a clip (or multi-selection) for context actions:

* **Clipboard**: copy, cut, paste, or paste modifiers onto another clip.
* **Group / Ungroup**: keep related clips moving together on the timeline.
* **Fold Sequence**: collapse selected clips into a reusable folded sequence
  that behaves like a single clip on the parent timeline.
* **Media & AI actions** (depending on clip type): save a preview frame, export
  audio, remove silence, transcribe speech to captions (:doc:`../server/transcription`),
  or generate speech from captions (:doc:`../server/text-to-speech`).

Next steps
----------

* :doc:`media` — import files, create layers, and record screen/microphone.
* :doc:`modifiers` — animate with keyframes, apply video/audio effects, and add
  transitions.
* :doc:`expressions` — drive individual clip properties with Rhai expressions.
* :doc:`export` — render video, GIF, or captions.
