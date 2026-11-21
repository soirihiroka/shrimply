Getting Started
===============

Run Shrimply
------------

Shrimply is currently pre-alpha software. See your platform's page for system
requirements and installation:

* :doc:`linux`
* :doc:`macos`
* :doc:`windows`
* Build from source: :doc:`development`

Create a project
----------------

Start Shrimply without a project path to open the launcher. Select
:guilabel:`Create Project`, then choose a name, canvas width, canvas height,
and frame rate. A new project is a ``.shrimp`` file and starts with a caption
track, a video track, and an audio track.

Open a project
--------------

Select :guilabel:`Open Project` to open a ``.shrimp``, ``.sjson`` (or legacy
``.json``), ``.otio``, or ``.kdenlive`` project.

On Linux, double-click a ``.shrimp`` file to open it. If a file manager already
has a different default, use :guilabel:`Open With` and pick Shrimply or
Shrimply Qt.

Build a timeline
----------------

Open the application menu and choose :menuselection:`New Track` to add a
caption, video, audio, or comment track. Drop or paste media onto a compatible
track, then select a clip to edit it in the inspector.

Essential shortcuts:

* :kbd:`Space`: play or pause
* :kbd:`S`: split every clip at the playhead
* :kbd:`D`: delete the selection
* :kbd:`Ctrl+Z` / :kbd:`Ctrl+Shift+Z`: undo / redo

In the GTK app, press :kbd:`Ctrl+?` (or open **Keyboard Shortcuts** from the
application menu) to browse all shortcuts. See :doc:`guides/editor` for
timeline editing tools, :doc:`guides/media` for supported formats, and
:doc:`guides/modifiers` for keyframes, effects, and transitions.

Save and export
---------------

Use :menuselection:`Save` or :menuselection:`Save As` to write the project.
Select :guilabel:`Export` to render a video or GIF, export captions, or write
project data. See :doc:`guides/export` for formats, encoders, and command-line
rendering.
