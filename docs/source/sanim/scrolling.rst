Scrolling
=========

Give a ``Box`` fixed dimensions and ``overflow="auto"``. Display it with XML,
then animate its scroll position:

.. code-block:: sanim

   let viewport = <Box overflow="auto" width={300pt} height={120pt}>
       <Box width={300pt} height={600pt} fill="light1"/>
   </Box>
   <Animation>{viewport}</Animation>
   viewport.scroll_to(y=200pt, duration=1s)

Scroll API
----------

.. code-block:: text

   view.scroll_to(
       to?: List<Length, 2> | List<Percent, 2>,
       x?: Length | Percent, y?: Length | Percent,
       line?: Number, range?: CodeRange,
       align: "nearest" | "start" | "center" | "end" = "nearest",
       duration: Duration = 300ms, ease: String = "spring"
   ): Animation

``?`` means optional. Choose one target form: ``to``, ``x``/``y``, ``line``, or
``range``. Points are content offsets; percentages are a fraction of overflow,
so ``y=100%`` reaches the bottom after resizing too. Offsets are clamped.
``line``/``range`` and ``align`` apply to CodeView; lines start at 1.

Overflow policies: ``visible`` does not clip; ``hidden`` clips without a bar;
``auto`` shows a bar when needed; ``scroll`` always shows it. ``overflow_x``
and ``overflow_y`` set axes separately. A CodeView with fixed dimensions
defaults to ``auto``.

Shared reactive offset
----------------------

.. code-block:: sanim

   mut offset: List<Length, 2> = [0pt, 0pt]
   let viewport = <Box overflow="auto" width={300pt} height={120pt} scroll={offset}>
       <Box width={300pt} height={600pt} fill="light1"/>
   </Box>
   <Animation>{viewport}</Animation>
   animate(offset, to=[0pt, 200pt], duration=1s)

A bound ``scroll`` has one writer: animate its input instead of also calling
``scroll_to``. See :doc:`code-view` for range selection.
