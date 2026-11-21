Quickstart
==========

Write and run your first Sanim animation in a few minutes.

1. Create a scene
-----------------

Save this as ``hello.sanim``:

.. code-block:: sanim

   bg(Color("#121826"))

   let title = <Label font_size={48pt} fill="light1">Hello, Sanim</Label>
   let badge = <Box w-160 h-44 pill fill="blue3" flex items-center justify-center>
       <Label font_size={16pt} fill="light1">First scene</Label>
   </Box>

   let stack = <Box w-full h-full flex flex-col items-center justify-center gap-16>
       {title}
       {badge}
   </Box>

   stack.fade_in(duration=400ms)
   wait(1.5s)
   stack.fade_out(duration=400ms)

Top-level animation expressions (``fade_in``, ``wait``, ``fade_out``) run in
sequence automatically.

2. Preview or render
--------------------

* **In Shrimply**: drag ``hello.sanim`` onto a video track (or use
  :menuselection:`Add --> Import Media…`). Saving edits to ``hello.sanim``
  hot-reloads the timeline preview automatically.
* **From the command line**:

.. code-block:: console

   $ shrimply sanim check hello.sanim
   $ shrimply sanim render hello.sanim --time 1s --output preview.png
   $ shrimply sanim video hello.sanim --output hello.mp4
   $ shrimply sanim watch hello.sanim --output preview.png

Standalone output defaults to 1920×1080, 30 FPS, and 96 DPI. See :doc:`cli`
for all subcommands and formatting options.

3. Expose live parameters
-------------------------

Wrap any default value in ``reflect(...)`` to turn it into an editable control
in the Shrimply inspector and CLI:

.. code-block:: sanim

   let accent = reflect(Color("blue3"), label="Accent Color")
   let heading = reflect("Hello, Sanim", label="Title")

   let card = <Box w-60% h-40% card flex flex-col items-center justify-center gap-12>
       <Label text-xl fill={accent}>{heading}</Label>
   </Box>

   card.fade_in(duration=400ms)
   wait(2s)

Override reflected inputs from the CLI with ``--set``:

.. code-block:: console

   $ shrimply sanim render hello.sanim --time 1s --set hello.sanim::heading='"Custom"' --output preview.png

Where to go next
----------------

* :doc:`editor-and-presenter` — Shrimply inspector controls, element picking,
  and slide decks with ``sanim-presenter``.
* :doc:`layout-and-animation` — elements, XML utilities, flex layout, timing,
  easings, and animated lists.
* :doc:`language` — types, reactive ``mut``/``sig`` state, units, strings, and
  modules.
* :doc:`editor-setup` — syntax highlighting, completion, and hover in VS Code
  and Zed.
