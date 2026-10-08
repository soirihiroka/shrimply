From After Effects
==================

After Effects expressions are JavaScript, including older ExtendScript.
Shrimply uses `Rhai <https://rhai.rs/book/language/>`__. Pasting an After
Effects expression will not work.

``value`` is the property before the expression. ``time`` is seconds. ``t``
and ``local_t`` are the same as ``time``. The last value in the script is
the result. See :doc:`expression-basics` for the full list.

Continuous motion
-----------------

This rotates by 40 degrees per second:

.. tabs::

   .. code-tab:: javascript After Effects

      time * 40

   .. code-tab:: rust Shrimply

      time * 40 // or t * 40

Variables
---------

Declare a variable with ``let`` or ``const``. Later assignments drop the
keyword:

.. tabs::

   .. code-tab:: javascript After Effects

      speed = 40;
      speed = 20;
      time * speed

   .. code-tab:: rust Shrimply

      let speed = 40;
      speed = 20;
      t * speed

Wiggle and shake
----------------

``wiggle(frequency, amount)`` varies around the current value. Multiply
``time`` by the frequency, multiply ``shake`` by the amount, and add it to
the value:

.. tabs::

   .. code-tab:: javascript After Effects

      wiggle(5, 20)

   .. code-tab:: rust Shrimply

      value + shake(t * 5) * 20

For a 2D property, give each axis its own seed:

.. code-block:: rust

   [
     x + shake(t * 5, 0) * 20,
     y + shake(t * 5, 1) * 20
   ]

Oscillation
-----------

.. tabs::

   .. code-tab:: javascript After Effects

      value + Math.sin(time * 4) * 20

   .. code-tab:: rust Shrimply

      value + sin(t * 4) * 20

Remapping values
----------------

``lerp`` with a clamped progress value replaces ``linear``. This moves from
0 to 100 over two seconds:

.. tabs::

   .. code-tab:: javascript After Effects

      linear(time, 0, 2, 0, 100)

   .. code-tab:: rust Shrimply

      lerp(0, 100, clamp(t / 2, 0, 1))

Conditions
----------

.. tabs::

   .. code-tab:: javascript After Effects

      time < 1 ? 0 : 100

   .. code-tab:: rust Shrimply

      if t < 1 { 0 } else { 100 }

What Shrimply does not have
---------------------------

There is no ``thisComp``, ``thisLayer``, or link to another property.
Expressions cannot read keyframes, so ``loopIn()``, ``loopOut()``, and
``valueAtTime()`` have nothing to call.

To loop a clip, open :menuselection:`Playback --> Repeat` in the inspector
and set :guilabel:`Strategy` to :guilabel:`Repeat` or :guilabel:`Ping Pong`.
