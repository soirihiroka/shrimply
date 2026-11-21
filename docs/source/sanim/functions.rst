Functions, animation values and lists
=====================================

Pure functions and capabilities
-------------------------------

Functions are pure by default. They can read their arguments, immutable outer ``let`` values and pure helpers. Local scratch mutation is allowed because it does not change anything outside the call. Mutable parameters, live signals, external reads and observable effects require explicit capabilities:

.. code-block:: text

   let offset = 2
   fn double(value: Number): Number { return value * 2 + offset }

   mut count = 0
   fn<ref> current_count(): Number { return count }
   fn<mut, ref> increment(): Unit { count = count + 1 }
   fn<sig> twice(sig value: Number): sig Number { return value * 2 }
   fn introduction(): Animation { return wait(2s) }

``mut`` permits mutable parameters and observable writes. ``sig`` permits live signal parameters and returns. ``ref`` permits reading outer mutable/signal state or external resources. An outer ``let`` holding a live signal is still live state, rather than an immutable snapshot of its sampled value.

Regular ``assert(condition: Bool, message=...)`` is pure input validation. It returns ``Unit`` on success and fails immediately on false; it does not register deferred verification or advance the timeline. Read-only data queries are pure as well. Assertions over visual/timeline predicates retain their explicit verification effects.

Capabilities belong to function types. A pure function cannot call an impure helper, including through an alias, generic or callback; an impure function may call a pure helper. A pure callback type is ``fn (value: Number): Number``. Explicit capabilities appear in callable types as well, such as ``fn<ref> (): Number``. A caller must permit the capabilities its callees require.

Pure-call reuse includes immutable captures in its semantic inputs. Reflected value overrides and changed data inputs therefore cannot reuse results from different captured values. Returned stateful callable identities are not shared through value memoization.

Anonymous functions
-------------------

Use the same typed parameters and return syntax as named functions:

.. code-block:: text

   let double = fn (value: Number): Number { return value * 2 }
   let answers = [1, 2, 3].map(fn (value: Number): Number {
       return double(value)
   })

Function values can capture lexical bindings, be returned by helpers, and accept defaults or ``sig`` parameters. A callable annotation uses ``fn (value: Number): Number``. Captured immutable values share persistent storage; captured mutable cells retain their live identity. Bounded execution and explicit capabilities apply to anonymous functions too.

Animation values
----------------

The entry script collects unassigned ``Animation`` expression results into a sequence. Bound or assigned plans are not collected. ``return animation`` appends a final plan and stops entry execution. A single collected plan stays the root directly; several become an auto-sized sequence. Constructing a plan inside a function or import has no playback side effect: functions must return their plans. Ordinary functions carry value types and capabilities; they do not have timeline-duration effects. Only the resulting root tree is lowered into shared native tracks.

.. code-block:: text

   fn introduction(): Animation { return wait(2s) }
   let title = <Label>Introduction</Label>
   return <Animation target={title} flow="sequence">
       <FadeIn duration={400ms}/>
       {introduction()}
       <FadeOut duration={400ms}/>
   </Animation>

Temporal layout uses the same bounded exact solver for duration discovery, editor resizing and rendering. ``duration="auto"`` measures content; a fixed Duration sets the box size; ``duration="match_parent"`` fills its parent. Children are parallel by default, or sequential with ``flow="sequence"``. Alignment, gaps, padding and offsets place children inside that temporal box. ``position_mode="absolute"`` excludes a child from its parent's content size. The editor supplies the root time viewport; resizing reuses the immutable plan and recalculates layout without evaluating source.

Guarded function clauses
------------------------

A function can have several disjoint clauses. Literal strings and numeric refinements select a body without an ``if``:

.. code-block:: text

   fn delay(time: "1s"): Animation { return wait(1s) }
   fn delay(time: "2s"): Animation { return wait(2s) }

   fn repetitions(count: Int < 1): Animation { return wait(0s) }
   fn repetitions(count: Int >= 1): Animation { return wait(count * 1s) }

   return Animation(children=[delay("1s"), repetitions(3)], flow="sequence")

``Int`` is an integer refinement of ``Number``; it uses the same exact numeric values. ``Number < 1`` also accepts non-integers below one. Comparisons use matching units; a duration constraint can use ``Duration <= 1s``. Open and closed bounds remain distinct, and the existing type union/intersection/difference operators compose numeric domains.

Clauses share parameter names, kinds and generic/default structure. Their domains must be provably disjoint. Overlap is an error rather than source-order priority. An input outside every clause produces a diagnostic identifying the argument and available domains. Empty lists and ``None`` are real values shared by different container types, so ``List<Number>`` and ``List<String>`` alone cannot distinguish two clauses.

Known literal arguments can select a clause during checking. Other calls dispatch once using their actual values. ``sig`` inputs select a body from their current sample and retain their live handles inside that body; a later signal change does not switch an already-running invocation to another body. A new call samples and selects again. Ordinary arguments remain snapshots and ``mut`` arguments remain local copies.

Explicit indefinite playback
----------------------------

``wait(forever)`` constructs an indefinite hold. It does not end language execution: the script still builds and returns its value normally. In sequential temporal layout, children after an indefinite in-flow hold cannot be reached. ``forever`` is a protected sentinel rather than a nonfinite number; finite calculations and ordinary loops retain their two-hour and operation bounds.

``animation.repeat(forever)`` retains one positive finite cycle and samples it with exact periodic clocks. It does not rerun source, allocate successive cycles, or execute an unbounded language loop. A zero-time cycle or competing writer is rejected. Use an absolute child for a background cycle while an in-flow sibling establishes a finite root duration:

.. code-block:: text

   mut phase = 0deg
   sig size = 64pt + 16pt * sin(phase)
   let shape = <Box width={size} height={size} fill="blue3"/>
   let pulse = animate(phase, to=360deg, duration=2s, ease="linear").repeat(forever)
   return <Animation>
       {shape}
       <Animation position_mode="absolute">{pulse}</Animation>
       {wait(10s)}
   </Animation>

Default temporal overflow is hidden, so the absolute recurring child is cropped to the finite root viewport. A relative indefinite child instead gives the root an intrinsic indefinite extent. Reflow uses the retained plan and preserves animation curves when clipping; it does not stretch a cropped curve into a new easing interval.

List lengths in types
---------------------

``List<T, N>`` describes exactly ``N`` elements. Omitting the length, or writing ``_``, leaves it unspecified:

.. code-block:: text

   let values = [1, 2, 3]                 # List<Number,3>
   let triple: List<Number,3> = values
   let arbitrary: List<Number> = values
   let also_arbitrary: List<Number,_> = values
   type Pair<T> = List<T,2>

The checker rejects known length mismatches in assignments, arguments and returns. ``read<List<Row,3>>("rows.csv")`` validates the row count when the file is loaded. An empty list has length zero. Generic identity functions preserve known lengths, and nested lists retain each dimension when it is uniform. Ragged dimensions become unspecified rather than inventing a size.

``map``, ``scan``, sorting and reversal preserve length. ``zip`` takes the shorter length, concatenation adds lengths, and known slicing bounds determine the result length. ``filter`` and ``unique`` usually have unspecified result lengths because their sizes depend on values. Exact lengths also constrain mapped Animation lists; complete preparation still validates loaded data.

Inferred mutable, signal and reflected lists allow resizing. An explicit ``List<T,N>`` annotation enforces that size. Signal guards describe the sample at invocation, so their body cannot assume that a live signal retains its initial length. Length parameters currently accept nonnegative integer literals or ``_``.

Typed persistent-list operations
--------------------------------

List methods accept ordinary named functions, including generic and guarded functions:

.. code-block:: text

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
     - Short-circuit a predicate. Find and position return optional results.
   * - ``enumerate()`` / ``zip(other)``
     - Return records ``{index, value}`` or ``{left, right}``; zip stops at the shorter input.
   * - ``take(count)`` / ``drop(count)`` / ``slice(start=..., end=...)``
     - Select persistent sublists; the end is exclusive.
   * - ``reverse()``
     - Return reversed values without mutating the input.
   * - ``sort(key=..., reverse=false)``
     - Stable sorting; optional scalar keys are evaluated once in input order.
   * - ``unique(key=...)``
     - Keep each scalar key's first occurrence in input order.
   * - ``bisect_left(value, lo=..., hi=...)``
     - Insertion position before equal keys in ascending sorted input.
   * - ``bisect_right(value, lo=..., hi=...)``
     - Insertion position after equal keys in ascending sorted input.

Bisection requires already-sorted input and uses bounded indexed probes. It does not rescan the entire list to verify sorting on each query. ``lo`` and ``hi`` bound the search interval, with an exclusive ``hi``. List operations retain typed schemas for empty results and share persistent input storage where possible; they do not mutate their source.

Callbacks execute once per visited item through the same function runtime as ordinary calls. A callback returning an Animation builds a value; place the resulting list in an Animation with ``flow="sequence"`` or ``flow="parallel"`` to choose its temporal layout. Function-internal expression statements never emit root animations. Short-circuit predicates may visit fewer items. Sorting and uniqueness run each key callback once, rather than once per comparison. Callback capabilities and errors are preserved, and list work obeys the configured iteration and memory budgets.

Visual relationship assertions take ``at=1s`` to choose a frame explicitly, or ``always(predicate)`` to verify an interval. Omitting ``at`` samples time zero. Constructing a preceding ``wait(...)`` plan does not change assertion time: plan construction has no timeline cursor. Boolean assertions validate immediately and reject ``at``.
