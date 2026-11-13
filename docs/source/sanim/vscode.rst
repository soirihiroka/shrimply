Sanim for VS Code
=================

This extension provides ``.sanim`` syntax highlighting and launches ``shrimply sanim lsp`` for compiler diagnostics, lint warnings, completion, hover, signatures, document symbols, inferred type/timeline duration hints, semantic tokens, and jump to definition.

Install Shrimply and enable its CLI in Preferences. The extension looks for ``shrimply`` on PATH, then the user CLI at ``~/.local/bin/shrimply`` and standard macOS application locations. It prefers Shrimply and falls back to ``sanim-lsp`` on PATH. No separate language-server installation is needed with Shrimply.

For a custom installation, set ``sanim.server.path`` to the executable's absolute path. Shrimply gets ``sanim lsp`` arguments automatically; a standalone server gets no arguments. ``sanim.server.arguments`` overrides these arguments, for example ``["sanim", "lsp", "--project", "/path/to/project.shrimp"]`` to select a project's runtime context explicitly. The extension is disabled in untrusted workspaces.

The Shrimply server queries active project sessions for reflected input values and shows them on hover for saved source. Projects in the source file's ancestor directories are discovered automatically. Each matching clip is identified separately because different clips can override the same reflected input. Unsaved source keeps static language features, but runtime values are withheld until it is saved. The server does not open an editor or start a headless project owner.

Alternatively, build the standalone server from the repository root:

.. code:: sh

   CARGO_INCREMENTAL=0 make -C sanim lsp-build

Put ``sanim/target/release/sanim-lsp`` on PATH, or set ``sanim.server.path`` to it. The standalone server provides static analysis without project runtime values.

Build, test the actual TextMate grammar, and package the extension:

.. code:: sh

   cd sanim/editors/vscode
   npm ci
   npm test
   npm run package
   code --install-extension sanim-0.1.0.vsix

The VSIX bundles the production language-client dependencies into its entry point. The native server comes from Shrimply or a standalone installation; the extension does not download or rebuild it. Reload the VS Code window after changing server settings.

Syntax highlighting covers declarations, comments, native calls, type names, colors, point/percent/time units, compound durations, default string interpolation, escaped braces, and ``r"literal strings"``. All semantic language features use the Rust compiler's canonical AST and types rather than an independent parser.

Native XML-style elements have tag/property highlighting and whole-token native utility highlighting, including ``w-180.5``, ``h-50%`` and ``overflow-y-auto``. The language server completes utility values after ``-``, filters them by the element's registered properties, and replaces the entire hyphenated token. Hover shows the resolved typed properties (``w-full`` → ``width=100%``) and unit semantics. Invalid utilities receive compiler diagnostics; repeated or superseded utility properties receive ``U001`` warnings with the later setting linked. Explicit properties such as ``width={220pt}`` intentionally override utilities without an override warning.

Zed receives the same completion, hover, diagnostics and semantic tokens from the same language server; these features use the native parser and utility registry.
