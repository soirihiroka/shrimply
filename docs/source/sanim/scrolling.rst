Container overflow and scrolling
================================

Scrolling is a layout behavior of ordinary ``Box`` and ``Rect`` elements. A Box fills the available width and measures its natural height, like an unstyled div. Set a bounded height and ``overflow="auto"`` to scroll its content. There is no separate scrolling element.

+-----------------------+---------------------------------------------------------+------------------------------+
| Policy                | Clipping                                                | Scrollbar                    |
+=======================+=========================================================+==============================+
| ``visible`` (default) | Content can extend outside the box                      | None                         |
+-----------------------+---------------------------------------------------------+------------------------------+
| ``hidden``            | Content is clipped; programmatic scrolling is supported | None                         |
+-----------------------+---------------------------------------------------------+------------------------------+
| ``auto``              | Content is clipped                                      | Shown when content overflows |
+-----------------------+---------------------------------------------------------+------------------------------+
| ``scroll``            | Content is clipped                                      | Always shown                 |
+-----------------------+---------------------------------------------------------+------------------------------+

``overflow`` sets both axes. ``overflow_x`` and ``overflow_y`` override their respective axes independently. For example, ``overflow_x="hidden" overflow_y="auto"`` shows only the vertical scrollbar when needed. The equivalent bare utilities are ``overflow-auto``, ``overflow-x-hidden`` and ``overflow-y-auto``.

.. code-block:: text

   let content = <Box flex flex-col width={300pt} height={800pt}>
       <Label>First item</Label>
       <Label>Second item</Label>
   </Box>
   let viewport = <Box overflow="auto" width={50%} height={30%}>{content}</Box>
   add(viewport)
   viewport.scroll_to(to=Vec2(0%, 100%), duration=600ms)

Scroll offsets are ``Vec2`` values. Points describe content offsets; percentages describe the current overflow range, so ``Vec2(0%, 100%)`` means the bottom edge even after resizing. Offsets are clamped after layout, including content changes, folding and font changes. The default scroll curve is the normal bounded spring; ``ease="linear"`` is available for constant-speed movement.

``scroll_to`` accepts one target form: ``to=Vec2(...)``, ``x=``/``y=``, or a Code View ``line=``/``scope=``. Code lines use 1-based numbering; scopes accept the IDs returned by ``.scopes()`` or an unambiguous declaration name.

Axis targets preserve their units: ``scroll_to(y=100%)`` stays at the bottom when the viewport resizes. When specifying both ``x=`` and ``y=``, use the same unit for both.

.. code-block:: text

   let code = <CodeView src="./example.py" width={80%} height={50%} />
   add(code)
   code.scroll_to(line=40, align="center", duration=500ms)
   code.scroll_to(scope="main", duration=400ms)

A CodeView with both width and height defaults to ``overflow="auto"``. Without a fixed viewport it uses its intrinsic content size and visible overflow. An explicit ``overflow="visible"`` keeps content visible outside an allocated box.

The default ``align="nearest"`` moves only enough to reveal the requested line. ``start``, ``center`` and ``end`` choose an explicit vertical alignment. A target inside a folded scope reveals its visible header.

For a shared reactive offset, supply ``scroll=`` and animate its mutable input:

.. code-block:: text

   mut offset = Vec2(0pt, 0pt)
   let viewport = <Box overflow="auto" width={300pt} height={120pt} scroll={offset}>
       <Rect width={300pt} height={600pt} x_align={Align.Start} y_align={Align.Start} />
   </Box>
   add(viewport)
   animate(offset, to=Vec2(0pt, 200pt), duration=1s)

A signal-owned offset has one writer: use ``animate(offset, ...)`` rather than ``.scroll_to(...)`` on that view. These controls animate the Sanim scene and do not intercept the editor's wheel or canvas navigation gestures.

Clipping and offset transforms are shared by Skia drawing and item picking. Glyphs outside the viewport defer native path preparation; cached states and recordings support reverse scrubbing normally.

Rendered scrollbars use libadwaita's idle overlay proportions: a narrow rounded thumb, inset ends, and no persistent trough. They remain inside the viewport clip. Hover and dragging belong to the host editor, rather than the headless animation.

All drawables have the same layout metadata and common alignment, box-size and padding inputs. Intrinsic leaves such as Circle, Polygon and Label keep their geometry: a wider layout box does not stretch a circle or change a font size. Rect and media width/height retain their normal geometry-sizing behavior. ``Line.width`` remains stroke thickness. Box children use normal vertical block flow. Opt into flex flow with ``flex flex-row`` or ``flex flex-col``; flex items accept ``flex_grow``, ``flex_shrink`` and ``flex_basis``. An automatic-height parent with a percentage-height child fails with a clear constraint-cycle error.
