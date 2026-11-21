Functions, Signals, and Lists
=============================

Pure functions and capabilities
-------------------------------

Functions are pure by default: they read their arguments, immutable outer
``let`` values, and pure helpers. Local scratch mutation is allowed. Mutable
parameters, live signals, external reads, and observable effects require
explicit capabilities:

.. code-block:: sanim

   let offset = 2
   fn double(value: Number): Number { return value * 2 + offset }

   mut count = 0
   fn<ref> current_count(): Number { return count }
   fn<mut, ref> increment(): Unit { count = count + 1 }
   fn<sig> twice(sig value: Number): sig Number { return value * 2 }
   fn introduction(): Animation { return wait(2s) }

* ``mut`` permits mutable parameters and observable writes.
* ``sig`` permits live signal parameters and returns.
* ``ref`` permits reading outer mutable/signal state or external resources.

Capabilities belong to function types (e.g., ``fn (value: Number): Number`` or
``fn<ref> (): Number``). A caller must permit the capabilities its callees
require. Regular boolean ``assert(condition, message=...)`` and read-only data
queries are pure.

Anonymous functions
-------------------

Anonymous functions use the same typed parameter and return syntax and can
capture lexical bindings:

.. code-block:: sanim

   let double = fn (value: Number): Number { return value * 2 }
   let answers = [1, 2, 3].map(fn (value: Number): Number {
       return double(value)
   })

Signal parameters and returns
-----------------------------

Parameters are immutable value snapshots unless marked ``sig`` or ``mut``:

.. code-block:: sanim

   fn<sig> twice(sig value: Number): sig Number {
       return value * 2
   }

   mut count = 1
   let doubled = twice(count)
   let saved = doubled
   count = 2
   assert(doubled == 4)
   assert(saved == 2)

* ``sig value: Number`` accepts a read-only live input (an ordinary value is
  lifted to a constant signal).
* ``: sig Number`` returns a live signal. A ``let`` receiving a signal factory
  directly keeps that signal; a ``let`` reading an existing signal takes a
  snapshot. ``sig derived = twice(count)`` also keeps the live result.
* ``mut value: Number`` creates a local mutable copy without mutating the
  caller's variable.
* Captured signal expressions must be pure. To return a live collection,
  qualify the whole return type (``: sig List<Number>``).

See `signal_functions.sanim <https://github.com/soirihiroka/shrimply/blob/main/sanim/examples/signal_functions.sanim>`__.

Guarded function clauses
------------------------

A function can have multiple disjoint clauses selected by literal strings or
numeric refinements:

.. code-block:: sanim

   fn delay(time: "1s"): Animation { return wait(1s) }
   fn delay(time: "2s"): Animation { return wait(2s) }

   fn repetitions(count: Int < 1): Animation { return wait(0s) }
   fn repetitions(count: Int >= 1): Animation { return wait(count * 1s) }

   return Animation(children=[delay("1s"), repetitions(3)], flow="sequence")

``Int`` is an integer refinement of ``Number``. Comparisons use matching units
(e.g., ``Duration <= 1s``). Clause domains must be provably disjoint—overlap is
an error rather than source-order priority.

List lengths in types
---------------------

``List<T, N>`` describes exactly ``N`` elements. Omitting ``N`` or writing
``_`` leaves the length unspecified:

.. code-block:: sanim

   let values = [1, 2, 3]                 # List<Number, 3>
   let triple: List<Number, 3> = values
   let arbitrary: List<Number> = values
   let also_arbitrary: List<Number, _> = values
   type Pair<T> = List<T, 2>

The checker enforces known lengths in assignments, calls, returns, and typed
reads (``read<List<Row, 3>>("rows.csv")``). ``map``, ``scan``, ``sort``, and
``reverse`` preserve length; ``zip`` takes the shorter length; ``concat`` adds
lengths.

Typed persistent-list operations
--------------------------------

List methods return new immutable lists and accept named, generic, guarded, or
anonymous callbacks:

.. code-block:: sanim

   fn square(value: Number): Number { return value * value }
   fn positive(value: Number): Bool { return value > 0 }
   fn combine(total: Number, value: Number): Number { return total + value }

   let values = [-2, 1, 3, 3]
   let squares = values.map(square)
   let positives = values.filter(positive)
   let total = values.fold(0, combine)
   let running = values.scan(0, combine)
   let distinct = values.sort().unique()
   let insertion = distinct.bisect_right(3)

.. list-table:: List methods
   :header-rows: 1
   :widths: 40 60

   * - Method
     - Behavior
   * - ``map(callback)`` / ``flat_map(callback)``
     - Transform each item; flat-map concatenates returned lists.
   * - ``filter(callback)``
     - Keep items whose predicate returns true.
   * - ``reduce(callback)``
     - Combine from the first item; an empty list is an error.
   * - ``fold(initial, callback)``
     - Combine from an explicit initial value; empty input returns it.
   * - ``scan(initial, callback)``
     - Return the accumulator after each item; output length equals input length.
   * - ``any`` / ``all`` / ``find`` / ``position``
     - Short-circuit a predicate. ``find`` and ``position`` return ``Option``.
   * - ``enumerate()`` / ``zip(other)``
     - Return records ``{index, value}`` or ``{left, right}``; ``zip`` stops at the shorter input.
   * - ``take(count)`` / ``drop(count)`` / ``slice(start=..., end=...)``
     - Select sublists; ``end`` is exclusive.
   * - ``reverse()``
     - Return reversed values without mutating the input.
   * - ``sort(key=..., reverse=false)``
     - Stable sort; optional scalar keys are evaluated once per item.
   * - ``unique(key=...)``
     - Keep each scalar key's first occurrence in input order.
   * - ``bisect_left(value, lo=..., hi=...)``
     - Insertion position before equal keys in ascending sorted input.
   * - ``bisect_right(value, lo=..., hi=...)``
     - Insertion position after equal keys in ascending sorted input.
