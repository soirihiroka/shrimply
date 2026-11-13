Signal parameters and returns
=============================

Parameters are immutable value snapshots unless marked ``sig`` or ``mut``.

.. code-block:: text

   fn twice(sig value: Number): sig Number {
       return value * 2
   }

   mut count = 1
   let doubled = twice(count)
   let saved = doubled
   count = 2
   assert(doubled == 4)
   assert(saved == 2)

``sig value: Number`` takes a read-only live input. Passing an ordinary value lifts it to a constant signal. ``: sig Number`` returns a live result whose captured inputs remain owned after the function finishes. An ordinary parameter takes a snapshot even if the function returns a signal. ``mut value: Number`` creates a local mutable copy; it does not assign the caller's variable.

Reading a signal observes its current value. A ``let`` directly receiving a signal factory result keeps that signal; a ``let`` reading an existing signal snapshots its current value. ``sig derived = twice(count)`` also keeps the live result. Containers hold snapshots; to return a live collection, qualify the whole return type (``: sig List<Number>``) rather than its individual fields or items.

Captured expressions must be pure. Evaluate scheduling, mutation, assertions, or reflected input declarations before the capture. A signal factory can perform bounded setup before returning its pure expression, and that setup runs once.

Shape helpers can accept live sizes with ``sig size: Length``. Their visual properties lower to the existing native expression DAG; playback does not run the language interpreter. Shapes receiving ordinary parameters keep snapshots. Signals use bounded timeline tracks, hold final values after animation ends, and retain historical state for reverse seeking. No lifetime annotation is needed.

See `signal_functions.sanim <https://github.com/soirihiroka/shrimply/blob/main/sanim/examples/signal_functions.sanim>`__.
