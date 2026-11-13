Text wrapping
=============

Text and Label default to HTML's ``white-space: normal``. CodeView defaults to ``pre``, like ``<pre><code>``: source newlines, spaces and tabs are preserved and long lines overflow. Width comes from the element or its parent's content box, after padding and flex sizing. A width signal causes immediate reflow at the sampled width; it does not add a code transition or crossfade.

.. code-block:: text

   mut width = 400pt
   mut policy: "normal" | "pre-wrap" | "nowrap" = "normal"
   let text = <Label font_size={18pt}>A paragraph with enough words to wrap.</Label>
   let panel = <Box width={width} white_space={policy}>{text}</Box>
   let code = <CodeView lang="python" w-full whitespace-pre-wrap wrap-anywhere>
       print("A long string can wrap without changing its source line number")
   </CodeView>

``white_space`` and ``overflow_wrap`` are typed string choices, accept signals, and inherit through Box children. Utility attributes lower to those same properties; the language server completes and checks them using the same registry.

======================= =============== =============== =============
Utility                 Spaces and tabs Source newlines Soft wrapping
======================= =============== =============== =============
``whitespace-normal``   Collapse        Collapse        Yes
``whitespace-nowrap``   Collapse        Collapse        No
``whitespace-pre``      Preserve        Preserve        No
``whitespace-pre-wrap`` Preserve        Preserve        Yes
``whitespace-pre-line`` Collapse        Preserve        Yes
======================= =============== =============== =============

Collapsible whitespace means ASCII spaces, tabs and segment breaks. Nonbreaking spaces remain nonbreaking. Collapsed leading/trailing spaces are removed; ``pre-wrap`` retains indentation. Tabs in CodeView use its ``tab_size``. ``break-spaces`` is not supported and produces a diagnostic rather than approximating it.

+---------------------+----------------------------------------------------------------------------------+
| Utility             | Behavior at a constrained width                                                  |
+=====================+==================================================================================+
| ``wrap-normal``     | Use native Unicode word/line break opportunities; an unbroken word can overflow. |
+---------------------+----------------------------------------------------------------------------------+
| ``wrap-break-word`` | Also break an otherwise overflowing word at a native grapheme boundary.          |
+---------------------+----------------------------------------------------------------------------------+
| ``wrap-anywhere``   | Permit the same emergency grapheme breaks.                                       |
+---------------------+----------------------------------------------------------------------------------+

The latter two have the same result at an explicit constrained width. Sanim does not expose CSS ``min-content`` sizing, where browsers distinguish their intrinsic width contributions. These are wrapping policies, independent of ``flex-wrap``, which wraps whole children. ``overflow-x-hidden`` clips overflow; ``overflow-x-auto`` can show a scrollbar. Clipping does not enable wrapping.

CodeView keeps its source text and shared syntax tree intact. Soft rows retain original logical line/Unicode-column coordinates; a continuation row does not get an invented line number. With collapsed source newlines, the gutter shows the first source line on that visual row. Native width, style spans, tabs and wrapping are part of the shared Pango layout cache key. Cached frame replay does no new layout or glyph preparation.

Native layouts share normalized source text, style spans and source mappings across widths. Paragraph measurement defers outlines until a renderer requests them; CodeView uses the shared native glyph outlines for its visible pieces.

Run ``make -C sanim text-wrapping-check`` for normalization, native wrapping, source mapping, inherited signals and deterministic cache performance regressions. The `width-loop example <https://github.com/soirihiroka/shrimply/blob/main/sanim/examples/text_wrapping.sanim>`__ repeats narrow/wide cycles under four wrapping policies. Render it from the repository root:

.. code-block:: sh

   make -C sanim run ARGS='video examples/text_wrapping.sanim --output /tmp/text-wrapping.mp4'
