CodeView
========

Display source with XML. Code is highlighted, never executed:

.. code-block:: sanim

   let code = <CodeView src="test.py" width={600pt} height={320pt}/>
   <Animation>{code}</Animation>
   code.patch(src="test.py.patch", duration=700ms)

Choose one source: ``src="file.py"``, ``code={string_or_signal}``, or a literal
body. ``lang`` defaults to filename detection; inline source needs an explicit
language. Outer indentation is removed; braces inside the body stay literal.

.. code-block:: sanim

   let code = <CodeView lang="python">
       print(f"Hello {name}!")
   </CodeView>

Files and patches are watched. Text, font, styles and presentation attributes
accept signals. Fixed dimensions create a scrollable viewport; see
:doc:`scrolling` and :doc:`text-wrapping`.

Patch and edit
--------------

``CodeStyle``, ``CodeTheme`` and ``CodeSpan`` name the records returned by their
same-named constructors. ``?`` marks an optional argument. Timed methods return ``Animation`` and also
accept ``duration: Duration = 300ms`` and ``ease: String = "spring"``.

.. code-block:: text

   code.patch(src: String, animation: String = "morph", file?: String, range?: CodeRange): Animation
   selection.edit(code?: String, src?: String, animation: String = "morph", file?: String): Animation

``src`` can contain a unified diff or replacement source. ``edit`` requires
exactly one of ``code``/``src`` and limits changes to the selection. Multi-file
diffs require ``file``. Invalid patch context fails.

* ``morph`` moves surviving text; only inserted/removed text fades.
* ``fade`` crossfades the old/new source.
* ``typewriter`` backspaces removed text, then types inserted text.

Select a range
--------------

.. code-block:: text

   code.range(name?: String, level?: Number, line?: Number, end_line?: Number,
              column?: Number, character?: Number, length?: Number,
              regex?: String, occurrence?: Number, within?: CodeRange): CodeRange
   code.ranges(name?: String, level?: Number, line?: Number, end_line?: Number,
               column?: Number, character?: Number, length?: Number,
               regex?: String, within?: CodeRange): List<CodeRange>

``range`` needs a selector and a unique match. ``ranges`` returns all matches,
possibly empty; ``occurrence`` is only available on ``range``.

.. list-table::
   :header-rows: 1

   * - Selector
     - Meaning
   * - ``name="WorkQueue.push"``
     - Qualified declaration name; a unique bare name also works.
   * - ``level=2``
     - Syntax depth.
   * - ``line=24, end_line=28``
     - Source line interval.
   * - ``line=24, column=8, length=5``
     - Text within a source line.
   * - ``character=120, length=8``
     - Text at a global character offset.
   * - ``regex="return [0-9]+", occurrence=2``
     - A regex match; use raw strings for regex braces.
   * - ``within=code.range(name="WorkQueue")``
     - Restrict another selector to this interval.

Lines, columns, characters and occurrences start at 1. The ``end_line`` selector
includes that line; returned ``CodeRange`` endpoints are exclusive.
Bound ranges belong to one view/revision. Query again after a patch; inline
chains resolve when their animation is placed.

Fold and style
--------------

.. code-block:: text

   selection.fold(): Animation
   selection.unfold(): Animation
   selection.select(): Animation
   selection.scroll_to(align: String = "nearest"): Animation
   selection.style(style: CodeStyle): Animation
   code.select(ranges?: List<CodeRange>): Animation
   code.style(spans?: List<CodeSpan>, theme?: CodeTheme,
              line_numbers?: Bool, scope_indicators?: Bool): Animation

Folds affect one scope independently. A line/regex selection chooses its
innermost containing scope; unrelated scopes cannot be folded together.

The default appearance uses GNOME SourceView colors and a monospace font.
``CodeStyle`` accepts fill, background, opacity, bold, italic, underline and
strike; ``CodeTheme`` applies named capture rules, and ``CodeSpan`` overrides
explicit source ranges. Later spans win. ``code.select()`` clears selection;
``code.style(spans=[])`` clears span overrides.

Line numbers and scope indicators default to hidden. Animate them with
``code.style(line_numbers=true, scope_indicators=true)``. Scope chevrons rotate
with folding. A signal-owned property must be changed through its input,
instead of a second style/patch writer.

Bundled languages: Sanim, Python, Rust, JavaScript, TypeScript, TSX, C/C++, JSON,
Bash, TOML, YAML, SQL and plain text. Invalid displayed code remains visible
with syntax diagnostics.
