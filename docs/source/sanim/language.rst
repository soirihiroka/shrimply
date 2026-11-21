Language Reference
==================

Syntax basics
-------------

* Blocks use braces ``{ ... }``; indentation is cosmetic. Statements end with a
  newline or semicolon.
* Comments begin with ``#`` outside strings and run to the end of the line.
  (Hex colors must be quoted: ``Color("#3584e4")``.)
* Named arguments and record values use ``=`` (``Box(width=100pt)``,
  ``{ field = value }``). Colons declare types (``name: Type``) and separate
  dictionary entries (``{"key": value}``).
* Standard names are protected and shadowing enclosing bindings is an error.

Bindings and reactivity
-----------------------

``let`` stores an immutable snapshot, ``mut`` declares a writable cell, and
``sig`` declares a lazily evaluated derived signal:

.. code-block:: sanim

   mut a = 1
   sig b = a * 2
   let saved = b
   a = 2
   assert(b == 4)
   assert(saved == 2)

Constructor properties retain expressions that depend on ``mut`` cells or
``sig`` values, so animating a cell with ``animate(cell, to=..., duration=...)``
updates every dependent property smoothly:

.. code-block:: sanim

   mut phase = 0deg
   sig size = 100pt + sin(phase) * 25pt
   let square = Box(
       width=size, height=size, radius=size / 6,
       fill=Color("light1"), x_align=Align.Center, y_align=Align.Center,
   )
   <Animation>
       {square}
       {animate(phase, to=1800deg, duration=10s, ease="linear")}
       {wait(10s)}
   </Animation>

``Label`` and ``Text`` accept signals for text, font, font size, fill,
alignment, and transforms (animate ``scale`` rather than continuously changing
``font_size`` when layout can stay fixed):

.. code-block:: sanim

   mut count = 0
   mut typeface = Font("sans-serif")
   let title = <Label font={typeface} font_size={24pt}>Count = {count:.0f}</Label>
   Animation(children=[title, animate(count, to=10, duration=1s, ease="linear")])
   typeface = Font("monospace")
   wait(1s)

Signal expressions must be pure. Dependency cycles and overlapping writers to
the same cell are errors.

Strings and formatting
----------------------

Ordinary strings interpolate expressions and Python-style format specifications
without an ``f`` prefix. Raw ``r"..."`` strings keep braces literal while
retaining standard escapes (``\n``, ``\"``). Double braces write literal
braces. Triple quotes (``"""..."""`` / ``'''...'''``) preserve multiline text.

.. code-block:: sanim

   let num = 42
   let binary = "bits={num:08b}"
   let fraction = "share={0.125:.1%}"
   let literal = r"{this is literal text}"
   mut count = 1
   sig caption = "count={count}"
   count = 2
   assert(caption == "count=2")

``str(value, format="08b")`` uses the same formatter (supporting ``b``, ``o``,
``x``, ``X``, ``d``, ``c``, ``f``, ``F``, ``e``, ``E``, ``g``, ``G``, ``%``,
``conversion="s"|"r"|"a"``, alignment/fill, sign, and precision; locale ``n``
is rejected).

Types, records, enums, and unions
---------------------------------

Built-in types include ``Number``, ``Bool``, ``String``, ``Color``, numeric
units, ``Fraction``, ``Element``, ``Font``, ``List<T, N>``, ``Option<T>``,
``Array<T>``, ``Dict``, ``Data``, records, and enums.

Record types are structural and compose with ``&`` and ``-``:

.. code-block:: sanim

   type RunInfo = { run: String, n: Number }
   type Measurements = { x: Number, y: Number }
   type Sample = RunInfo & Measurements
   type Coordinates = Sample - RunInfo

Generic functions and aliases support defaults (``<T = Data>``) and constraints
(``<T: Numeric>``). Enums support payloads and exhaustive ``match`` with
``case Variant(binding) { ... }``.

String literal unions define closed choices:

.. code-block:: sanim

   type Variant = "before" | "after"
   fn select_variant(variant: Variant): Variant { return variant }
   let selected = select_variant("before")

Units, exact math, and matrices
-------------------------------

* **Exact arithmetic**: numbers and time use checked rational arithmetic (no
  floating-point drift in timelines). ``Fraction(num, den)`` exposes
  ``.numerator`` and ``.denominator``.
* **Durations**: ``0.5s``, ``250ms``, ``7m``, ``4m1s``, ``1s250ms`` (compound
  units in decreasing ``m``, ``s``, ``ms`` order).
* **Layout & angles**: ``pt``, ``px`` (``96px == 72pt``), ``%``, ``deg``.
  Mismatched units fail explicitly.
* **Vectors & matrices**: numeric Lists support pointwise arithmetic, ``dot``,
  ``norm``, ``cross``, and ``concat(a, b)``. Row-major nested Lists
  (``List<List<Number, 3>, 2>``) support matrix ``*``, ``transpose``, and
  ``inverse`` (up to 64×64).

Reflected inputs
----------------

``reflect(default, key=..., label=...)`` exposes a typed runtime input at
module scope:

.. code-block:: sanim

   let color1 = reflect(Color(), label="Display Color")
   let accent = reflect(Color("blue3"), key="palette.accent", label="Accent Color")
   let optional_count: Option<Number> = reflect(None)
   let style = { fill=reflect(Color()), radius=reflect(12pt) }

* Default keys are ``relative/file.sanim::variable_name`` (plus nested field
  paths such as ``style.fill``). Explicit ``key="..."`` stays stable across
  variable renames; duplicate keys fail.
* ``reflect`` must run once at module scope (not inside helpers, loops,
  branches, or signal initializers).

Loops and modules
-----------------

* ``for item in collection { ... }`` and ``repeat count { ... }`` run bounded
  preparation loops; recursion is rejected.
* ``import "./theme" as theme`` imports relative, cycle-checked modules.
  Imported modules never play animations automatically.

See :doc:`functions` for function capabilities, anonymous functions, guarded
clauses, and list methods.
