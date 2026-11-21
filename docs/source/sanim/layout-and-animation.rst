Layout, Elements, and Animation
===============================

Elements and native markup
--------------------------

``Box``, ``Line``, ``Polygon``, ``Text``/``Label``, ``Image``, and ``CodeView``
construct immutable elements. Bound constructors infer their element ID from
the variable name (override with ``id="..."``).

Native XML-style notation lowers to the same constructors:

.. code-block:: sanim

   let name = reflect("Sanim")
   let card = <Box w-60% h-40% card flex flex-col items-center justify-center gap-16>
       <Label text-xl text-light1>Hello {name}</Label>
       <Box w-200 h-44 pill fill={LinearGradient(from=Color("blue3"), to=Color("purple3"))}>
           <Label font_size={16pt}>Native gradient</Label>
       </Box>
   </Box>
   Animation(children=[card])
   wait(2s)

Attributes accept quoted strings, ``{typed_expression}``, or bare native
utilities:

* **Presets & surfaces**: ``card``, ``raised``, ``pill``, ``suggested``,
  ``destructive``, ``bg-COLOR``, ``text-COLOR`` (on labels), ``border[-N]``,
  ``border-COLOR``, ``rounded[-sm/lg/xl/full/none]``, ``rounded-N``.
* **Flex & sizing**: ``flex``, ``flex-row``, ``flex-col``, ``flex-wrap``,
  ``items-start/center/end``, ``justify-start/center/end/between/around``,
  ``grow``, ``shrink``, ``flex-1``, ``flex-none``, ``basis-N``, ``gap-N``,
  ``p-N``, ``w-N``, ``h-N`` (numbers in ``pt``, plus percentages and ``full``).
* **Text wrapping & overflow**: ``whitespace-normal/nowrap/pre/pre-wrap/pre-line``,
  ``wrap-normal/break-word/anywhere``, ``overflow-auto/hidden/scroll/visible``
  (see :doc:`text-wrapping` and :doc:`scrolling`).
* **Gradients**: ``bg-linear-to-r`` (and the other seven compass directions),
  ``from-COLOR``, optional ``via-COLOR`` / ``via-30%``, and ``to-COLOR``.

Explicit constructor arguments override utilities. Combining gradient utilities
with an explicit ``fill`` is an error.

Fills and gradients
~~~~~~~~~~~~~~~~~~~

``fill`` accepts a ``Color`` or ``LinearGradient(from=..., to=..., angle=0deg)``
(or ``stops=[{offset=0, color=...}, ...]`` with 2–64 ordered stops over 0–1;
``0deg`` points right, ``90deg`` down). Gradient stop interpolation is not
supported yet; opacity and transform animations work.

Relative and flex layout
------------------------

``pt`` is the logical layout unit (``96px`` equals ``72pt`` at 96 DPI).
Positions are parent-local two-element Lists such as ``[12pt, 0pt]``.
``x_align`` and ``y_align`` use ``Align.Start``, ``Align.Center``, and
``Align.End`` (left/center/right and top/center/bottom).

.. code-block:: sanim

   Box(
       width=100pt, height=40pt,
       x_align=Align.End, y_align=Align.Center,
       position=[-12pt, 0pt], fill=Color("light1"),
   )

* Ordinary shapes default to center alignment and zero offset.
* ``Box`` defaults to start/start alignment, transparent fill, full available
  width, and automatic height measuring children in vertical block flow.
* Percentage offsets resolve against parent dimensions. An automatic-height
  parent with a percentage-height child fails with a constraint-cycle
  diagnostic.
* Flow layouts own immediate child positions; use ``absolute``
  (``position_mode="absolute"``) when a child needs explicit ``position`` and
  alignment.

Canvas dimensions, frame rate, and DPI belong to the host (CLI flags or the
Shrimply canvas). Use ``bg(Color(...))`` to set the background (default is
Adwaita ``window_bg_dark``; use ``bg(Color("transparent"))`` for transparency).

Animation trees and timing
--------------------------

Animation methods (``.move_to()``, ``.fade_in()``, ``.create()``, ``.write()``)
return immutable ``Animation`` values—calling them does not mutate the scene.

* Unassigned top-level ``Animation`` expressions collect into a sequence.
  ``return animation`` appends a final plan and stops entry execution.
* Functions and imported modules never play anything automatically; functions
  return ``Animation`` values explicitly.
* ``<Animation>`` (or ``Animation(children=[...])``) runs children in parallel
  by default; pass ``flow="sequence"`` to run them sequentially.
* Include elements as children to mount them; ``wait(1s)`` holds time.
* List receivers target all items together:
  ``[heading, subtitle].fade_in(duration=400ms)``.

.. code-block:: sanim

   fn introduction(): Animation { return wait(2s) }
   let title = <Label>Introduction</Label>
   return <Animation target={title} flow="sequence">
       <FadeIn duration={400ms}/>
       {introduction()}
       <FadeOut duration={400ms}/>
   </Animation>

``duration="auto"`` measures content, a fixed ``Duration`` sets the box size,
and ``duration="match_parent"`` fills its parent. ``position_mode="absolute"``
excludes a child from its parent's content duration. Finite intrinsic durations
above two hours are rejected.

Indefinite playback
~~~~~~~~~~~~~~~~~~~

``wait(forever)`` creates an indefinite hold. ``animation.repeat(forever)``
repeats a positive finite cycle periodically without an unbounded loop. Use an
absolute child for a background loop alongside a finite sibling:

.. code-block:: sanim

   mut phase = 0deg
   sig size = 64pt + 16pt * sin(phase)
   let shape = <Box width={size} height={size} fill="blue3"/>
   let pulse = animate(phase, to=360deg, duration=2s, ease="linear").repeat(forever)
   return <Animation>
       {shape}
       <Animation position_mode="absolute">{pulse}</Animation>
       {wait(10s)}
   </Animation>

Motion, reveal, and easing
--------------------------

* **Defaults**: motion duration is ``300ms``. Transforms and sizes default to
  ``spring``; opacity, fill, and reveal default to ``smootherstep``.
* **Easings** (``ease=...``): ``spring``, ``spring_bouncy``, ``linear``,
  ``cubic_in``, ``cubic_out``, ``smooth``, ``smootherstep``, ``overshoot``.
* **Reveals**: ``.create()`` / ``.uncreate()`` trace and fill vector paths
  (including text glyph outlines); ``.write()`` / ``.unwrite()`` trace the
  outline first, then fill. Overlapping reveals on an ancestor and descendant
  are rejected.
* **Scaling & expansion**: ``.grow_from_center()`` / ``.shrink_to_center()``
  require center alignment, zero rotation, and non-flow sizing; use
  ``.scale_to()`` otherwise. ``.expand_x(origin=50%, offset=0pt)`` and
  ``.stretch_x_to(to, ...)`` animate horizontal expansion without changing
  vertical layout.
* **Tracks**: tracks hold their final value; overlapping writers to the same
  property or mixing a signal with a motion on the same property are errors.
  Position endpoints must use matching units (``pt`` or ``%``).

See `examples/animations.sanim <https://github.com/soirihiroka/shrimply/blob/main/sanim/examples/animations.sanim>`__
for the full set of animation methods.

Animated lists
--------------

List helpers ``append``, ``insert``, ``pop``, ``popleft``, ``replace_at``, and
``move_item`` return new immutable lists. Use ``animate(items, to=..., duration=...)``
on a flex ``Box`` bound to a mutable element list to automatically fade entries
and exits and smoothly reflow surviving items by identity:

.. code-block:: sanim

   mut items: List<Element> = []
   let row = Box(flex_direction="row", children=items, width=400pt, height=100pt, gap=12pt)
   Animation(children=[row])
   let filled = append(items, Box(width=64pt, height=64pt, fill=Color("light1")))
   animate(items, to=filled, duration=0.5s)
   animate(items, to=popleft(filled), duration=300ms)

See `examples/stack_queue.sanim <https://github.com/soirihiroka/shrimply/blob/main/sanim/examples/stack_queue.sanim>`__.

Visual verification
-------------------

.. code-block:: sanim

   assert(circle.above(rect))
   check(circle.disjoint(rect), message="Objects should not overlap")
   assert(always(inside_canvas(circle)))

* Relations: ``above``, ``below``, ``left_of``, ``right_of``, ``contains``,
  ``overlaps``, ``disjoint``, ``touches`` (with optional ``gap`` for
  directional relations), plus ``inside_canvas``.
* Plain relations check time ``0s`` unless ``at=1s`` is passed; ``always(...)``
  requests a conservative continuous proof over the timeline.
* ``assert`` fails on ``Refuted`` or ``Unknown``; ``check`` records evidence
  without failing the scene. Boolean assertions validate immediately and do not
  accept ``at``.
