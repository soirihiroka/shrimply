Shrimply Editor and Presenter
=============================

Using Sanim in Shrimply
-----------------------

Drag a ``.sanim`` file onto a video track, or import it with
:menuselection:`Add --> Import Media…`.

* **Viewport & live reload**: the Shrimply canvas provides the logical viewport
  (overriding standalone dimensions) and revalidates visual contracts for that
  viewport. Editing the ``.sanim`` file or any imported module or data file
  reloads the clip automatically on a background worker while retaining the
  last-good frame until the new revision is ready.
* **Timeline duration**: the clip's timeline envelope stays under editor
  control after a reload—extending source duration does not push neighboring
  clips. Indefinite sources (``wait(forever)`` / ``repeat(forever)``) can be
  extended on the timeline.
* **Reflected inspector controls**: every ``reflect(...)`` value appears in the
  inspector as a labeled, typed control (colors, booleans, strings, numbers,
  units, or schema-validated JSON) with reset-to-default actions.
* **Export**: exports reject scenes with preparation errors and pin one
  prepared source/data revision per clip for the export session.

Inspecting elements in GTK
~~~~~~~~~~~~~~~~~~~~~~~~~~

In the GTK editor, pause playback, select a Sanim clip, then click a rendered
shape in the preview (or right-click it and choose **Inspect Sanim Element**).
The popover shows the element's name, kind, containing groups, constructor
line, and helper call sites, with copy buttons for ``file:line:column`` or the
source line. Inspection respects clip transforms and preview zoom; disable
external warp/distortion modifiers when inspecting source geometry.

Editing Sanim parameters from the CLI
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Use the project CLI (:doc:`/integrations/cli`) to inspect or update a clip's
reflected overrides inside a ``.shrimp`` project:

* ``shrimply sanim get`` reports canonical keys, labels, defaults, current
  values, and types.
* ``shrimply sanim set`` applies typed JSON ``parameters`` and separate
  ``reset_keys`` (JSON ``null`` is a value, not a reset).

Font assets on macOS
~~~~~~~~~~~~~~~~~~~~

Explicit font files (``read<Font>("font.ttf")``) require a Pango build with
Fontconfig/FreeType enabled (provided by Homebrew's Pango on macOS). CoreText's
font map cannot load private font files.

Sanim Presenter
---------------

Open a finite 2D slide deck directly with ``sanim-presenter deck.sanim``.

* Playback starts on open and pauses at the first ``page()`` stop.
* :kbd:`Right`, :kbd:`Page Down`, or :kbd:`Space` advances to the next stop.
* :kbd:`Left` or :kbd:`Page Up` jumps back to the previous stop (or returns to
  the last reached stop during a transition).
* :kbd:`F11` toggles fullscreen; :kbd:`Escape` exits fullscreen.

Place ``page()`` in the outer animation sequence wherever playback should
pause. Ordinary Sanim rendering treats ``page()`` as zero duration; use
``wait(duration)`` for timed holds. Layers persist across pages, so hide or
animate out earlier elements when advancing:

.. code-block:: sanim

   bg(Color("#121826"))
   let first = <Label font_size={64pt}>First page</Label>
   <Animation>{first}</Animation>
   page()
   first.fade_out(duration=0s)
   let second = <Label font_size={64pt}>Second page</Label>
   <Animation>{second}</Animation>
   page()
   second.fade_out(duration=300ms)
   let third = <Label font_size={64pt}>Third page</Label>
   third.fade_in(duration=300ms)
   page()

Release archives
~~~~~~~~~~~~~~~~

Tags beginning with ``sanim-presenter-v`` publish release archives to GitHub
Releases:

* **Linux (x86_64)**: targets Ubuntu 26.04 with GTK 4 and Pango 1.57
  (``libgtk-4-1``, ``libpango-1.0-0``, ``libpangocairo-1.0-0``).
* **macOS (Apple Silicon)**: includes native libraries in ``lib/``; keep that
  directory beside the executable.
