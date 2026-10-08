Expression basics
=================

An expression is a small `Rhai <https://rhai.rs/book/language/>`__ script on a
property.

Enable one with the code button beside the property. The current value,
keyframed or not, is available as ``value``. The last value in the script is
the result.

For example, this makes a number pulse over time:

.. code-block:: rust

   value * (0.75 + 0.25 * sin(time * 6))

Property values
---------------

The available components and required result depend on the property:

.. list-table::
   :header-rows: 1
   :widths: 25 35 40

   * - Property
     - Inputs
     - Result
   * - Number
     - ``value`` or ``x``
     - A number
   * - 2D vector
     - ``x``, ``y``
     - ``[x, y]``
   * - 3D vector
     - ``x``, ``y``, ``z``
     - ``[x, y, z]``
   * - Color
     - ``r``, ``g``, ``b``, ``a`` and ``value``
     - A color helper or an array of up to four channels
   * - Boolean
     - ``value``
     - ``true`` or ``false``
   * - Text
     - ``value``
     - A string
   * - Discrete choice
     - ``value``
     - A valid option name as a string

Here is a 2D position that moves up and down:

.. code-block:: rust

   [x, y + sin(time * 4) * 20]

Project values
--------------

``time``, ``t``, ``local_t``
   Time within the current item, in seconds. These names are aliases.

``duration``
   Duration of the current item, in seconds.

``fps``
   Project frame rate.

``canvas_width``, ``canvas_height``
   Project canvas dimensions.

``media_width``, ``media_height``
   Source-media dimensions, or the canvas dimensions when they are not
   available. ``source_width`` and ``source_height`` are aliases.

``seed``
   A stable integer seed for this item at this time.

Functions
---------

Math
~~~~

``sin()``, ``cos()``, and ``tan()`` use the current item time. Pass a value to
use a different input: ``sin(value)``, ``cos(value)``, or ``tan(value)``.

The other math helpers are ``sqrt(value)``, ``pow(value, power)``,
``clamp(value, low, high)``, ``lerp(a, b, progress)``, ``abs(value)``, and
``int(value)``.

``random()`` returns a deterministic random value. ``shake()`` returns smooth
noise based on time; use ``shake(phase)`` to control its speed and
``shake(phase, seed)`` to create independent motion:

.. code-block:: rust

   [
     x + shake(time * 8, 0) * 12,
     y + shake(time * 8, 1) * 12
   ]

Timecode
~~~~~~~~

``timecode(frame, frame_rate, drop_frame)`` formats a nonnegative frame number
as SMPTE ``HH:MM:SS:FF`` text. Pass ``true`` for drop-frame numbering; standard
29.97 and 59.94 fps rates use ``;`` before the frame field and skip the
appropriate frame numbers.

For example, this displays the current project frame as timecode:

.. code-block:: rust

   timecode(int(time * fps), fps, false)

Color
~~~~~

Use ``rgb(r, g, b)``, ``rgba(r, g, b, a)``, ``gray(luminance)``, or
``graya(luminance, alpha)`` with channels from ``0`` to ``1``.

``hsv(hue, saturation, value)`` and ``hsva(hue, saturation, value, alpha)``
use hue in degrees and other channels from ``0`` to ``1``. ``oklab(l, a, b)``
and ``oklaba(l, a, b, alpha)`` are also available.

For example, this cycles through hues:

.. code-block:: rust

   hsv(time * 60, 1, 1)

Audio and lip sync
~~~~~~~~~~~~~~~~~~

``vol()`` returns the current peak amplitude of the complete audio mix from
``0`` to ``1``. Pass zero-based audio-track indices to select tracks, such as
``vol(0)`` or ``vol(0, 2)``.

``mouth()`` returns the current lip-sync mouth shape. It accepts the same
optional track indices. See :doc:`lip-sync` for the available shapes and an
example.

Errors
------

The editor highlights syntax errors and can show an expression's output or
error. If an expression is empty, disabled, fails, or returns the wrong type,
Shrimply keeps the normal or keyframed property value instead.
