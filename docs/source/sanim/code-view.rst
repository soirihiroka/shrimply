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

``.patch(src="change.patch", animation="morph", duration=700ms)`` accepts a unified diff or a complete replacement source file. Unified diffs validate exact hunk positions, context, counts, filenames and EOF newline markers. A multi-file patch requires ``file="path.py"``. Invalid context fails with a readable error rather than guessing.

- ``morph``: surviving text moves between layouts without fading. Edits inside a token preserve its unchanged prefix and suffix; only inserted and removed text fades. Syntax recoloring interpolates without replacing the surviving glyphs.
- ``fade``: the old and new code crossfade without token movement.
- ``typewriter``: removed text is backspaced before inserted text is revealed along Pango cursor boundaries. The final glyph outlines are identical to the fully shaped source, including ligatures and combining characters.

All operations have bounded timeline durations and support ordinary scheduling/easing parameters. Revisions retain shared native syntax trees; edits use native Tree-sitter copy-on-write updates. Unchanged source is parsed once per language/grammar revision across views and paths. Style, font, selection and fold changes reuse that tree.

Syntax and folding
------------------

Bundled native grammars cover Python, Rust, JavaScript, TypeScript, TSX, C, C++, JSON, Bash, TOML, YAML and SQL. Sanim uses its own direct AST parser and shared semantic highlighting; no second Sanim lexer or Tree-sitter parser is introduced. ``lang="plain"`` displays arbitrary text without syntax scopes. Invalid displayed code remains renderable and carries syntax diagnostics. Displayed Sanim has a fixed 200,000-step syntax budget, separate from ``Engine.limits.max_parse_steps`` for executable program modules. Foreign syntax trees have fixed 200,000-node and 256-level limits. Fixed display policies allow shared syntax artifacts across engines; source size, retained memory and execution work keep their independent budgets.

.. code-block:: text

   code.fold(level=2, duration=400ms)
   code.unfold(level=2, duration=400ms)
   code.fold(scope="function_definition:main#1", duration=400ms)
   code.unfold(duration=400ms)

``.scopes()`` returns typed records containing ``id``, ``label``, ``start_line``, ``end_line`` and ``depth``. Scope IDs follow syntax ancestry and declaration names, with sibling ordinals. Folding works on real syntax scopes rather than indentation guesses; nested fold choices survive opening a parent. Folded code clips/fades as its surrounding lines move. A stale scope ID produces an explicit diagnostic.

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
   code.select(ranges=[code.find("answer")], duration=300ms)
   code.style(spans=[
       CodeSpan(CodeLines(2), CodeStyle(background=Color("yellow4"), fill=Color("dark5")))
   ], duration=300ms)

``CodeStyle`` supports fill, background, opacity, bold, italic, underline and strike. Unspecified properties inherit the syntax theme. Explicit spans override syntax captures; later explicit spans override earlier ones. ``.style(spans=[])`` clears explicit span overrides; ``.select()`` clears selections. ``CodeTheme`` accepts default and selection styles plus named ``CodeRule`` entries.

``CodeRange(start_line,start_column,end_line,end_column)`` uses 1-based Unicode columns with an exclusive end. ``CodeLines(start,end)`` selects inclusive lines. ``.find(text,occurrence=1)`` returns a source range and reports a helpful error when the requested occurrence is missing. Tabs retain source columns while visual layout respects ``tab_size`` (1–32).

Viewports
---------

Set ``line_numbers=false`` to hide the source-line gutter. It also accepts a signal, so the gutter can change during the animation without reparsing the source. The `scrolling CodeView example <https://github.com/soirihiroka/shrimply/blob/main/sanim/examples/code_view_scroller.sanim>`__ demonstrates scrolling, two syntax folding levels, toggling the gutter, and a small patch transition.

.. code-block:: text

   let code = <CodeView src="long.py" width={600pt} height={320pt}/>
   add(code)
   code.scroll_to(line=40, duration=500ms)

Provide both width and height for a clipped viewport. ``scroll=Vec2(...)`` accepts lengths or percentages of the overflow range. Scrolling shares the native layout, clipping and picking pipeline; offscreen glyph paths are culled.

The headless ``sanim-data`` grammar adapter exposes ``Grammar::new``, ``parse_with`` and native parser registration for custom queries or parsers under a registered language identifier. Adding a new language name currently requires extending the core language registry and its grammar configuration. It keeps the actual native tree instead of copying it into a second generic AST. Syntax, shape and frame caches are byte- and entry-bounded; persistent snapshots share their source, trees, options and event maps.
