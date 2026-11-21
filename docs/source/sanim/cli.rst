Sanim command line tools
========================

Sanim development tools run through either executable:

.. code:: sh

   shrimply sanim lint scene.sanim --json
   shrimply sanim check scene.sanim --deny-warnings
   shrimply sanim profile scene.sanim --samples 60 --json
   shrimply sanim render scene.sanim --time 500ms --output preview.png
   shrimply sanim watch scene.sanim --output preview.png
   shrimply sanim video scene.sanim --output animation.mp4
   shrimply sanim video indefinite.sanim --duration 10s --output animation.mp4

``sanim lint ...`` uses exactly the same implementation as ``shrimply sanim lint ...``. No project or separately installed Sanim executable is required. Discover commands with ``shrimply cli search "sanim performance"`` and inspect their input schema with ``shrimply schema sanim profile``.

``lint`` parses and type-checks the entry and its imports and reports static performance warnings without opening data assets or building animation frames. ``check`` fully evaluates values, validates visual contracts, and forces native geometry validation. Use it to catch errors in values that an editor preview has not needed yet. ``inspect`` publishes the complete typed preparation result.

``render``, ``video``, ``watch``, and ``profile`` use editor-style demand preparation. ``profile`` measures preparation and two passes over requested frame times: cold frame state/geometry/Skia recording work, then warm cache reuse. It does not rasterize, encode PNG, or read pixels back from a GPU. Build mode, sample count, scene complexity, and cache budgets affect its results.

Structured diagnostics contain source, span, code, severity, and message. Runtime performance warnings and actual evaluation/cache statistics are included in JSON results. ``--warn-operations N`` adjusts the per-statement warning budget; ``--warn-prepare-ms N`` adjusts the total preparation warning budget. These are warning thresholds, not execution limits.

Exit status is ``0`` for success, ``1`` for evaluation/validation failures or warnings rejected by ``--deny-warnings``, and ``2`` for invalid CLI arguments. A denied warning prevents render/video output from being created. Watch uses native change events and reports each accepted or failed revision without filesystem polling.

Defaults are 1920×1080, 30 FPS, and 96 DPI. Timeline values and frame rates remain exact; durations such as ``0.5s``, ``125ms``, and ``4m1s`` are accepted.

``fmt FILE`` prints canonical source without evaluating it. ``fmt FILE --check`` returns 1 if formatting would change; ``--write`` atomically updates an existing ``.sanim`` file while preserving permissions and refusing concurrent changes. ``fmt -`` accepts standard input. Raw/formatted strings and ``#`` comments keep their contents. Named constructor arguments use ``=``; ``:`` remains for type annotations. Layout options are ``--indent-width 4``, ``--line-width 100``, and ``--tabs``. ``lint/check --check-format`` report F101 when source needs formatting; combine with ``--deny-warnings`` for enforcement without modifying the file.

``duration`` reports ``forever`` for an explicit indefinite root Animation, together with the finite compiled prefix. Its JSON keeps exact finite ``seconds``/``frames`` and adds ``indefinite``. Video export requires a finite ``--duration`` for these sources. Single-frame preview can sample beyond the compiled prefix. In the editor, indefinite sources can be extended while retaining their final state or repeating child tracks; explicit clip Repeat/PingPong controls still repeat the finite prefix.

``inspect`` exports the compiled animation, temporal/source layout, types, reflection, dependencies and diagnostics. Internal builder plans and evaluation bindings remain in memory for host APIs; they are not recursively expanded into the JSON export.
