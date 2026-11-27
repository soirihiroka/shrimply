From After Effects
==================

After Effects expressions use JavaScript or ExtendScript. Shrimply expressions
use :doc:`Sanim <../sanim/language>`. The examples below translate common patterns.

``value`` is the property before the expression. ``time``, ``t``, and ``local_t``
are exact durations. Divide by ``1s`` for a number in seconds. See
:doc:`expressions` for the available inputs and functions.

Continuous motion
-----------------

This rotates by 40 degrees per second:

.. tabs::

   .. code-tab:: javascript After Effects

      time * 40

   .. code-tab:: sanim Shrimply

      time / 1s * 40

Variables
---------

Use ``mut`` for a variable you intend to reassign.

.. tabs::

   .. code-tab:: javascript After Effects

      speed = 40;
      speed = 20;
      time * speed

   .. code-tab:: sanim Shrimply

      mut speed = 40
      speed = 20
      time / 1s * speed

Wiggle and shake
----------------

``wiggle(frequency, amount)`` varies around the current value. Multiply
``time`` by the frequency, multiply ``shake`` by the amount, and add it to
the value:

.. tabs::

   .. code-tab:: javascript After Effects

      wiggle(5, 20)

   .. code-tab:: sanim Shrimply

      value + shake(time / 1s * 5) * 20

For a 2D property, give each axis its own seed:

.. code-block:: sanim

   [
     x + shake(time / 1s * 5, 0) * 20,
     y + shake(time / 1s * 5, 1) * 20
   ]

Oscillation
-----------

.. tabs::

   .. code-tab:: javascript After Effects

      value + Math.sin(time * 4) * 20

   .. code-tab:: sanim Shrimply

      value + sin(time / 1s * 4) * 20

Remapping values
----------------

``lerp`` with a clamped progress value replaces ``linear``. This moves from
0 to 100 over two seconds:

.. tabs::

   .. code-tab:: javascript After Effects

      linear(time, 0, 2, 0, 100)

   .. code-tab:: sanim Shrimply

      lerp(0, 100, clamp(time / 2s, 0, 1))

Conditions
----------

.. tabs::

   .. code-tab:: javascript After Effects

      time < 1 ? 0 : 100

   .. code-tab:: sanim Shrimply

      if time < 1s {
          return 0
      }
      100

What Shrimply does not have
---------------------------

There is no ``thisComp``, ``thisLayer``, or link to another property.
Expressions cannot read keyframes, so ``loopIn()``, ``loopOut()``, and
``valueAtTime()`` have nothing to call.

To loop a clip, open :menuselection:`Playback --> Repeat` in the inspector
and set :guilabel:`Strategy` to :guilabel:`Repeat` or :guilabel:`Ping Pong`.
