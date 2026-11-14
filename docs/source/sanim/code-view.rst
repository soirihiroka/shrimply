Native CodeView
===============

CodeView is a rendered element with a cached, immutable syntax tree. Displayed code is never executed.

.. code-block:: text

   let code = <CodeView src="test.py" font_size={18pt}/>
   add(code)
   wait(1s)
   code.patch(src="test.py.patch", animation="morph", duration=700ms)
   wait(1s)

Source and patch files are watched like other Sanim resources. Language is detected from the source filename; ``lang="python"`` overrides it. Inline source uses a literal body:

.. code-block:: text

   let code = <CodeView lang="python">
       def greet(name):
           print(f"Hello {name}!")
   </CodeView>

Outer blank lines and the common indentation are removed. Relative indentation, braces, quotes and foreign operators remain literal: ``{name}`` above belongs to Python. Use ``code={expression}`` for a Sanim string or signal instead. Provide exactly one source: ``src``, a body, or ``code``.

.. code-block:: text

   mut count = 1
   sig snippet = "count = {count:d}"
   let code = <CodeView code={snippet} lang="python"/>
   add(code)
   wait(1s)
   count = 2
   wait(1s)

Code, language, font, font size, line height, line numbers, tab size, themes, spans and ordinary element properties accept signals. Source paths are watched resources; use inline ``code`` for reactive text. A changing source clears selection offsets that no longer describe the same revision. Explicit ``.patch`` cannot overwrite signal-owned code; assign its mutable input instead. Likewise ``.style`` cannot overwrite signal-owned themes or spans.

Patches and animation
---------------------

``.patch(src="change.patch", animation="morph", duration=700ms)`` accepts a unified diff or a complete replacement source file. For a scoped edit, use ``code.range(...).edit(src="change.patch", ...)`` or ``code.patch(src="change.patch", range=selection, ...)``. Diff context may surround a selection, but actual changes outside it are rejected. A non-diff source file replaces the selected interval; ``selection.edit(code="replacement", ...)`` supplies replacement text directly. Source outside the selection remains unchanged. Unified diffs validate exact hunk positions, context, counts, filenames and EOF newline markers. A multi-file patch requires ``file="path.py"``. Invalid context fails with a readable error rather than guessing.

- ``morph``: surviving text moves between layouts without fading. Edits inside a token preserve its unchanged prefix and suffix; only inserted and removed text fades. Syntax recoloring interpolates without replacing the surviving glyphs.
- ``fade``: the old and new code crossfade without token movement.
- ``typewriter``: removed text is backspaced before inserted text is revealed along Pango cursor boundaries. The final glyph outlines are identical to the fully shaped source, including ligatures and combining characters.

All operations have bounded timeline durations and support ordinary scheduling/easing parameters. Revisions retain shared native syntax trees; edits use native Tree-sitter copy-on-write updates. Unchanged source is parsed once per language/grammar revision across views and paths. Style, font, selection and fold changes reuse that tree.

Syntax and folding
------------------

Bundled native grammars cover Python, Rust, JavaScript, TypeScript, TSX, C, C++, JSON, Bash, TOML, YAML and SQL. Sanim uses its own direct AST parser and shared semantic highlighting; no second Sanim lexer or Tree-sitter parser is introduced. ``lang="plain"`` displays arbitrary text without syntax scopes. Invalid displayed code remains renderable and carries syntax diagnostics. Displayed Sanim has a fixed 200,000-step syntax budget, separate from ``Engine.limits.max_parse_steps`` for executable program modules. Foreign syntax trees have fixed 200,000-node and 256-level limits. Fixed display policies allow shared syntax artifacts across engines; source size, retained memory and execution work keep their independent budgets.

Code ranges and independent folding
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

``view.range(...)`` returns one typed selection; ``view.ranges(...)`` returns a list. A singular query reports ambiguity instead of choosing an unrelated match. Add the CodeView to the timeline before using actions on its ranges.

.. code-block:: text

   add(code)
   let push: CodeRange = code.range(name="WorkQueue.push")
   let pop_range = code.range(name="WorkQueue.pop")
   push.fold(duration=400ms)
   pop_range.fold(duration=400ms)
   push.unfold(duration=400ms)  # pop stays folded

Folding changes exactly one syntax scope. Folding a parent temporarily hides its children and preserves each child's independent fold choice. No implicit level-wide fold operation exists. A closing line containing source outside the selected node stays visible, so folding one function never hides its sibling's header. A node with no complete body lines to hide reports an error; put a shared-line sibling declaration on its own line to make that scope foldable.

.. list-table:: Selector forms
   :header-rows: 1
   :widths: 35 65

   * - Query
     - Meaning
   * - ``code.range(name="WorkQueue.push")``
     - A qualified declaration; a unique bare name also works.
   * - ``code.ranges(level=2)``
     - All syntax scopes at depth two, returned for explicit iteration.
   * - ``code.range(line=24)``
     - One source line. ``end_line=28`` extends the selection.
   * - ``code.range(line=24, column=8, length=5)``
     - A Unicode-column position and optional text length.
   * - ``code.range(character=120, length=8)``
     - A global 1-based Unicode character position and length.
   * - ``code.range(regex="return [0-9]+", occurrence=2)``
     - A regex match; occurrence is 1-based.
   * - ``code.ranges(level=2, within=code.range(name="WorkQueue"))``
     - Restrict a query to another selected interval.

Levels select syntax depth; they do not trigger a fold. ``range`` requires a unique match unless ``occurrence`` selects one. ``ranges`` returns all matches in source order, including an empty list when no match exists. Regex compilation, search work, match counts and returned data are bounded. Backslashes in regex strings use Sanim's ordinary string escaping (``"\\s+"`` for the regex whitespace escape). Use a raw string for regex braces, such as ``r"[0-9]{2}"``, so they are not string interpolations.

Every operation consumes the same range representation:

.. code-block:: text

   let section = code.range(name="WorkQueue.push")
   section.select(duration=200ms)
   section.scroll_to(align="center", duration=400ms)
   section.style(style=CodeStyle(background=Color("blue5")), duration=200ms)
   section.fold(duration=400ms)
   section.unfold(duration=400ms)
   section.edit(src="push.patch", animation="morph", duration=700ms)

Name and level queries select whole syntax scopes. For line, character or regex selections, ``fold`` resolves the innermost syntax scope containing the selected text. An interval spanning unrelated scopes fails with a diagnostic.

Bound ranges belong to one CodeView and one source revision. They can be reused for styling and folding while that source remains unchanged. After an edit or patch, query a new range; reusing an old selection reports that its source is stale. A range from another CodeView is rejected. Bound highlights are cleared when an edit invalidates their source revision.

``CodeRange(start_line,start_column,end_line,end_column)`` and ``CodeLines(start,end)`` construct the same coordinate representation without an owner. Coordinates use 1-based Unicode positions and an exclusive end. These unbound coordinates can be passed to ``code.select(ranges=[...])`` or ``CodeSpan`` for an explicit target view. Tabs retain source columns while visual layout respects ``tab_size`` (1–32).

GNOME styles and precise overrides
----------------------------------

The default theme follows GNOME SourceView's Adwaita semantic colors. Code uses the shared native font pipeline; default is monospace. Configure exact semantic styles and source spans without replacing the syntax tree:

.. code-block:: text

   let theme = CodeTheme(rules=[
       CodeRule("keyword", CodeStyle(fill=Color("orange2"), bold=true)),
       CodeRule("function", CodeStyle(fill=Color("blue1"), underline=true))
   ])
   let code = <CodeView src="test.py" theme={theme}/>
   add(code)
   code.range(regex="answer").select(duration=300ms)
   code.style(spans=[
       CodeSpan(CodeLines(2), CodeStyle(background=Color("yellow4"), fill=Color("dark5")))
   ], duration=300ms)

``CodeStyle`` supports fill, background, opacity, bold, italic, underline and strike. Unspecified properties inherit the syntax theme. Explicit spans override syntax captures; later explicit spans override earlier ones. ``.style(spans=[])`` clears explicit span overrides; ``.select()`` clears selections. ``CodeTheme`` accepts default and selection styles plus named ``CodeRule`` entries.


Viewports
---------

Line numbers and scope controls are both hidden by default. Set ``line_numbers=true`` or ``scope_indicators=true`` to opt in independently. Both constructor inputs also accept signals. For a timed reveal or hide, use the ordinary presentation style operation:

.. code-block:: text

   code.style(line_numbers=true, scope_indicators=true, duration=300ms)
   let block = code.range(name="main")
   block.fold(duration=600ms)
   block.unfold(duration=600ms)
   code.style(line_numbers=false, duration=300ms)

Gutter changes fade the controls while source text moves to its new inset and remains opaque. Scope chevrons point right when collapsed and rotate downward when expanded, using the fold/unfold operation's timing and easing. They mark real multiline syntax scopes, once per original header line even when that header wraps. Nested controls disappear with their folded parent and return when it opens.

Immediate signal assignments still commit immediately; use a timed ``.style`` call to animate static presentation flags. A property controlled by a signal has one writer, so ``.style`` cannot also write that property. The `scrolling CodeView example <https://github.com/soirihiroka/shrimply/blob/main/sanim/examples/code_view_scroller.sanim>`__ demonstrates scrolling, rotating scope controls, animated gutters and a small patch transition.

.. code-block:: text

   let code = <CodeView src="long.py" width={600pt} height={320pt}/>
   add(code)
   code.scroll_to(line=40, duration=500ms)

Provide both width and height for a clipped viewport. ``scroll=Vec2(...)`` accepts lengths or percentages of the overflow range. Scrolling shares the native layout, clipping and picking pipeline; offscreen glyph paths are culled.

The headless ``sanim-data`` grammar adapter exposes ``Grammar::new``, ``parse_with`` and native parser registration for custom queries or parsers under a registered language identifier. Adding a new language name currently requires extending the core language registry and its grammar configuration. It keeps the actual native tree instead of copying it into a second generic AST. Syntax, shape and frame caches are byte- and entry-bounded; persistent snapshots share their source, trees, options and event maps.
