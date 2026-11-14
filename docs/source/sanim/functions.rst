Functions, durations and lists
=============================

Guarded function clauses
------------------------

A function can have several disjoint clauses. Literal strings and numeric refinements select a body without an ``if``:

.. code-block:: text

   fn delay(time: "1s"): Unit { wait(1s) }
   fn delay(time: "2s"): Unit { wait(2s) }

   fn delay(count: Int < 1): Unit { wait(0s) }
   fn delay(count: Int >= 1): Unit { wait(count * 1s) }

   delay("1s")
   delay(3)

``Int`` is an integer refinement of ``Number``; it uses the same exact numeric values. ``Number < 1`` also accepts non-integers below one. Comparisons use matching units; a duration constraint can use ``Duration <= 1s``. Open and closed bounds remain distinct, and the existing type union/intersection/difference operators compose numeric domains.

Clauses share parameter names, kinds and generic/default structure. Their domains must be provably disjoint. Overlap is an error rather than source-order priority. An input outside every clause produces a diagnostic identifying the argument and available domains. Empty lists and ``None`` are real values shared by different container types, so ``List<Number>`` and ``List<String>`` alone cannot distinguish two clauses.

Known literal arguments can select a clause during checking. Other calls dispatch once using their actual values. ``sig`` inputs select a body from their current sample and retain their live handles inside that body; a later signal change does not switch an already-running invocation to another body. A new call samples and selects again. Ordinary arguments remain snapshots and ``mut`` arguments remain local copies.

Symbolic timeline effects
-------------------------

Value types and timeline effects are separate. A function may have an exact fixed duration, a formula, a finite set of alternatives, or an unresolved effect:

.. code-block:: text

   fn steps(count: Int): Unit {
       repeat count { wait(250ms) }
   }

   fn rows(items: List<Number>): Unit {
       for item in items { wait(100ms) }
   }

   fn choose(short: Bool): Unit {
       if short { return wait(1s) }
       return wait(2s)
   }

The effects are respectively ``count * 250ms``, ``len(items) * 100ms``, and ``1s | 2s``. A union is a set of possible timeline durations, not an instruction to add both branches. Early returns stop their path. Sequential work adds durations; independent branch choices in repeated work can produce mixed sums, so repeating ``1s | 2s`` twice permits ``2s | 3s | 4s``.

Actual inputs resolve formulas. A list read from CSV contributes its parsed record count; quoted CSV fields are handled by the regular reader. Reflected inputs use the current runtime override, rather than baking their source defaults into the plan. If per-item values affect time, the duration pass evaluates those required values. It does not assume every loop costs ``length * one constant``.

The duration pass uses fixed-body multiplication for bounded loops, preserves actual control flow when needed, and skips unrelated value work and rendering. Formulas and alternative sets have fixed complexity budgets. Unsupported or overly complex formulas remain unresolved and use bounded selective evaluation; they never become an invented zero duration. Exact fractions, iteration/work limits and the two-hour total limit apply to the resolved result.

Manual duration contracts
-------------------------

Declare a contract when a helper needs an explicit duration interface:

.. code-block:: text

   @duration(step)
   fn pause(step: Duration): Unit {
       wait(step)
   }

   @duration(1s)
   fn preset(name: "quick"): Unit { wait(1s) }

   @duration(2s)
   fn preset(name: "slow"): Unit { wait(2s) }

The annotation is a pure ``Duration`` expression in the function's parameter scope. It does not schedule work. Checking rejects a provable mismatch; full evaluation checks the declared duration against the actual call before publishing its result. A mismatch is an error, including when the body takes less time than declared.

Fast duration-only planning may use this declared contract without evaluating an otherwise unnecessary body. It is therefore a promise that full preparation must validate, not a proof obtained from the body. Use ``check`` for complete validation. The separate call option ``duration=...`` retains its ordinary envelope behavior: it may pad a shorter call and rejects a body exceeding the envelope.

Typed persistent-list operations
-------------------------------

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

Callbacks execute once per visited item through the same function runtime as ordinary calls. Timed callbacks advance sequentially, so a fixed 100ms callback mapped over three items contributes 300ms. Short-circuit predicates may visit fewer items. Sorting and uniqueness run each key callback once, rather than once per comparison. Callback mutations and errors are preserved, and list work obeys the configured iteration and memory budgets.
