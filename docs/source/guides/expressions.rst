Property Expressions
====================

.. toctree::
   :hidden:

   from-after-effects
   lip-sync

Use the code button beside a property to drive it with :doc:`Sanim <../sanim/language>`.
The expression receives the property's normal or keyframed ``value`` and runs
at the current clip time. The final expression supplies the new value.

.. code-block:: sanim

   value * (0.75 + 0.25 * sin(time / 1s * 6))

Use ``return`` when a script branches. Parentheses around conditions are optional.

.. code-block:: sanim

   if time < 1s {
       return value
   } else if time < 2s {
       return value * 2
   }
   value * 3

Property expressions support math, lists, typed functions and local variables.
Scene construction, animation scheduling, resource reads, imports and signals
are available when writing :doc:`Sanim scenes <../sanim/index>`.

Property values
---------------

* Numbers expose ``value`` and ``x``.
* 2D and 3D vectors expose ``value`` as a fixed-size numeric list and provide
  ``x``, ``y``, and, for 3D, ``z``. Return a list of the same size.
* Colors expose a ``Color`` and the channels ``r``, ``g``, ``b``, and ``a`` in
  the range 0 to 1. Return a Sanim color, such as
  ``Rgb(r, g, b, alpha=0.5)`` or ``Hsv(120deg, 0.8, 1)``.
* Booleans and text return ``Bool`` and ``String``. For a discrete choice,
  return its option key as a string.

Add a vector directly to an animated position:

.. code-block:: sanim

   value + [0, sin(time / 1s * 4) * 20]

Project values and time
-----------------------

* ``time``, ``t``, and ``local_t`` represent the clip's expression time.
  These aliases and ``duration`` use Sanim's exact ``Duration`` type.
  Divide by ``1s`` for a dimensionless number of seconds.
* ``fps`` is the effective frame rate and uses the exact ``Rate`` type.
  Multiplying a duration by this rate gives a frame count.
* ``canvas_width`` and ``canvas_height`` describe the project canvas.
  ``media_width`` and ``media_height`` describe the source or fall back to the
  canvas dimensions. ``source_width`` and ``source_height`` are aliases.
* ``seed`` is a deterministic integer for the current item and time.

``frame_index(time, fps)`` returns the index of the containing frame.
``timecode(frame, fps, drop_frame=false)`` formats a nonnegative frame as
SMPTE timecode. ``timecode_parts`` takes the same arguments and returns
``hours``, ``minutes``, ``seconds``, and ``frames``.

.. code-block:: sanim

   timecode(frame_index(time, fps), fps, drop_frame=true)

Math and audio
--------------

``sin``, ``cos``, and ``tan`` take a number in radians or an angle such as
``45deg``. ``lerp(a, b, progress)`` interpolates numbers or matching numeric
lists. Use ``trunc`` to discard the fractional part of a number.

``random()`` gives a deterministic sample from -1 up to 1. ``shake()``
gives smooth noise at the current time. Pass a dimensionless phase to
control its rate and separate seeds for independent motion:

.. code-block:: sanim

   let phase = time / 1s * 8
   value + [shake(phase, 0), shake(phase, 1)] * 12

``vol()`` returns the current peak amplitude of the full audio mix, from
0 to 1. ``vol([0, 2])`` selects zero-based audio tracks. ``mouth()`` and
``mouth([0, 2])`` use the same selection rules. See :doc:`lip-sync`.

Errors
------

When an expression is empty, disabled, fails, or returns the wrong type,
Shrimply uses the normal or keyframed value. Existing Rhai source stays
editable and can be rewritten in Sanim using the patterns in this guide.
