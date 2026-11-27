Layout and animation
====================

Build elements
--------------

Use XML for visuals and animation composition. Attributes accept strings,
``{expressions}``, or bare utilities:

.. code-block:: sanim

   let card = <Box w-240 h-100 rounded-lg fill="blue3" flex items-center justify-center>
       <Label>Hello</Label>
   </Box>

``Box`` is an unstyled container: transparent, full available width, automatic
height, vertical child flow. Use ``flex flex-row`` or ``flex flex-col`` for
flex layout. Explicit attributes override utilities.

Animate in order
----------------

Unassigned top-level animations run in order. Assigned animations are stored
for later; functions return animations without playing them.

.. code-block:: sanim

   let title = <Label>Hello</Label>
   title.fade_in()
   title.move_to([120pt, 0pt], duration=1s)
   wait(1s)
   title.fade_out()

Run together
------------

``<Animation>`` runs its children in parallel. Set ``flow="sequence"`` to run
them in order. Include a visual child to display it immediately.

.. code-block:: sanim

   let title = <Label>Hello</Label>
   <Animation>
       {title.fade_in(duration=1s)}
       {title.move_to([120pt, 0pt], duration=1s)}
   </Animation>

Animation methods
-----------------

Methods below are called on an ``Element`` or ``List<Element>`` and return
``Animation``. Each accepts ``duration: Duration = 300ms`` and ``ease: String``.
The default ease is ``"spring"`` for motion/size, ``"smootherstep"`` for
fade/fill/reveal.

.. code-block:: text

   element.fade_in(duration: Duration = 300ms, ease: String = "smootherstep"): Animation
   element.fade_out(duration: Duration = 300ms, ease: String = "smootherstep"): Animation
   element.move_to(to: List<Length, 2> | List<Percent, 2>, duration: Duration = 300ms, ease: String = "spring"): Animation

Other methods take the same timing options:

.. list-table::
   :header-rows: 1

   * - Method and extra arguments
     - Effect
   * - ``shift(to: List<Length, 2> | List<Percent, 2>)``
     - Add a relative offset.
   * - ``rotate_to(to: Angle)`` / ``scale_to(to: Number)``
     - Set rotation or scale.
   * - ``fill_to(to: Color)``
     - Change fill color.
   * - ``create()`` / ``uncreate()``
     - Reveal/erase a vector path.
   * - ``write()`` / ``unwrite()``
     - Trace outlines, then fill/erase.
   * - ``grow_from_center()`` / ``shrink_to_center()``
     - Scale a centered, unrotated element with fixed sizing.
   * - ``expand_x(origin: Percent = 50%, offset: Length = 0pt)``
     - Reveal horizontal extent from an origin.
   * - ``stretch_x_to(to: Number, origin: Percent = 50%, offset: Length = 0pt)``
     - Set horizontal scale around an origin.

.. code-block:: text

   wait(duration: Duration | Forever): Animation
   animate<T>(target: mut T, to: T, duration: Duration, ease: String = "spring"): Animation
   animation.repeat(count: Number | Forever): Animation

``animate`` changes a mutable input and updates its dependent signals.
Alternatively pass ``curve=`` instead of ``to=`` (:doc:`curves`). Overlapping
writers to the same property are errors. Tracks hold their final value.
``wait(forever)`` holds indefinitely; ``repeat(forever)`` repeats a finite,
positive cycle. Other durations must be finite and at most two hours.

Layout and timing options
-------------------------

* Sizes: ``w-N``, ``h-N``, ``w-full``, ``h-full``; numbers are points.
  ``96px == 72pt``. Percentages resolve against the parent.
* Placement: ``position={[x, y]}``, ``x_align``/``y_align`` using
  ``Align.Start``, ``Align.Center``, ``Align.End``. ``Box`` defaults to start;
  other shapes default to center. Use ``absolute`` to opt out of child flow.
* Flex: ``items-start/center/end``, ``justify-start/center/end/between/around``,
  ``gap-N``, ``p-N``, ``grow``, ``shrink``, ``basis-N``.
* Appearance: ``fill``, ``rounded-N``, ``rounded-full``, ``border-N``,
  ``bg-COLOR``; gradient utilities ``bg-linear-to-r from-COLOR to-COLOR``.
* ``<Animation>``: ``duration={...}`` sets its time span; ``"auto"`` measures
  children and ``"match_parent"`` fills the parent. ``gap``/``padding`` use
  durations; ``justify`` distributes spare time. ``position_mode="absolute"``
  excludes a child from intrinsic duration. ``overflow`` clips by default.

The host controls canvas size, FPS and DPI. ``bg(Color(...))`` sets the
background; its default is Adwaita ``window_bg_dark``.

For animated list insertion/removal, bind ``children={items}`` and animate
that mutable list with ``animate``. See :doc:`functions` for list operations,
:doc:`scrolling` for overflow and :doc:`text-wrapping` for text layout.

Visual checks
-------------

.. code-block:: text

   assert(condition: Bool | Predicate, message?: String, at?: Duration): Unit
   check(condition: Bool | Predicate, message?: String, at?: Duration): Unit
   always(condition: Predicate): Predicate
   inside_canvas(element: Element): Predicate

Relations include ``element.above(other, gap=0pt)``, ``below``, ``left_of``,
``right_of``, ``contains``, ``overlaps``, ``disjoint`` and ``touches``. Visual
checks sample time zero unless ``at`` is given; ``always`` checks the whole
timeline. ``assert`` fails on refuted or unknown results; ``check`` records
them. Boolean checks run immediately and do not accept ``at``.
