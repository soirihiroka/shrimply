Sanim Command-Line Tools
========================

Run Sanim development tools through ``shrimply sanim`` (or the standalone
``sanim`` binary—both use the same implementation):

.. code-block:: console

   $ shrimply sanim lint scene.sanim --json
   $ shrimply sanim check scene.sanim --deny-warnings
   $ shrimply sanim fmt scene.sanim --write
   $ shrimply sanim duration scene.sanim --json
   $ shrimply sanim inspect scene.sanim
   $ shrimply sanim profile scene.sanim --samples 60 --json
   $ shrimply sanim render scene.sanim --time 500ms --output preview.png
   $ shrimply sanim watch scene.sanim --output preview.png
   $ shrimply sanim video scene.sanim --output animation.mp4
   $ shrimply sanim video indefinite.sanim --duration 10s --output animation.mp4

Discover commands with ``shrimply cli search "sanim"`` and inspect schemas with
``shrimply schema sanim profile``. (For editing reflected parameters on a clip
inside a ``.shrimp`` project, see ``shrimply sanim get`` / ``set`` in
:doc:`editor-and-presenter` and :doc:`/integrations/cli`.)

Subcommands
-----------

* ``lint``: parses and type-checks the entry file and its imports, reporting
  static performance warnings without loading data assets or building frames.
* ``check``: fully evaluates values, validates visual contracts, and forces
  native geometry checks.
* ``fmt FILE``: prints canonical formatted source without evaluating it. Use
  ``--check`` to exit 1 if changes are needed, ``--write`` to update in place
  atomically, or ``-`` for stdin. Layout flags: ``--indent-width 4``,
  ``--line-width 100``, ``--tabs``. Combine ``lint``/``check --check-format``
  with ``--deny-warnings`` to enforce formatting (code ``F101``).
* ``duration``: computes exact timeline duration without rendering. Reports
  ``forever`` (with the finite prefix) for explicit indefinite animations.
* ``inspect``: exports source dependencies, reflected inputs, resolved types
  and symbols, temporal layout, verification evidence, and diagnostics as JSON.
* ``render`` / ``video`` / ``watch``: render a single frame (``--time`` or
  ``--frame``), stream H.264 MP4 video via FFmpeg, or watch for filesystem
  changes and re-render automatically. Video export requires a finite
  ``--duration`` for indefinite scenes.
* ``profile``: measures preparation and cold/warm frame passes without
  rasterizing or encoding PNGs.

Reflected input overrides
-------------------------

Pass typed JSON values to ``reflect(...)`` inputs with ``--set``:

.. code-block:: console

   $ shrimply sanim check examples/hello.sanim --set hello.sanim::repetitions=4
   $ shrimply sanim check examples/data.sanim --set data.sanim::bar_color='"#FF8800"'

Defaults, flags, and exit codes
-------------------------------

* Defaults: ``1920×1080``, ``30`` FPS, and ``96`` DPI (override with
  ``--width``, ``--height``, ``--fps``, ``--dpi``).
* ``--progress`` emits phase-local JSON progress on stderr without altering
  stdout.
* ``--warn-operations N`` and ``--warn-prepare-ms N`` adjust warning thresholds
  (not execution limits).
* Exit status: ``0`` for success, ``1`` for validation/evaluation errors or
  warnings rejected by ``--deny-warnings``, ``2`` for invalid arguments.
