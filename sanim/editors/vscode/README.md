# Sanim for VS Code

This extension provides `.sanim` syntax highlighting and connects to the separate
native `sanim-lsp` process for compiler diagnostics, lint warnings, completion,
hover, signatures, document symbols, inferred type/timeline duration hints, semantic
tokens, and jump to definition.

Build the server from the repository root:

```sh
CARGO_INCREMENTAL=0 make -C sanim lsp-build
```

Put `sanim/target/release/sanim-lsp` on PATH, or set `sanim.server.path` to its
absolute path in VS Code settings. The extension never launches Shrimply's GUI
or the animation runtime. It is disabled in untrusted workspaces because the
language client launches a configurable local executable.

Build, test the actual TextMate grammar, and package the extension:

```sh
npm ci
npm test
npm run package
code --install-extension sanim-0.1.0.vsix
```

The VSIX bundles the production language-client dependencies into its entry point. The native server
is installed separately; the extension does not download or rebuild it. Reload
the VS Code window after changing `sanim.server.path`.

Syntax highlighting covers declarations, comments, native calls, type names,
colors, point/percent/time units, compound durations, default string interpolation,
escaped braces, and `r"literal strings"`. All semantic language features use the
Rust compiler's canonical AST and types rather than an independent parser.

Native XML-style elements have tag/property highlighting and whole-token native
utility highlighting, including `w-180.5`, `h-50%` and `overflow-y-auto`. The
language server completes utility values after `-`, filters them by the element's
registered properties, and replaces the entire hyphenated token. Hover shows the
resolved typed properties (`w-full` → `width=100%`) and unit semantics. Invalid
utilities receive compiler diagnostics; repeated or superseded utility properties
receive `U001` warnings with the later setting linked. Explicit properties such as
`width={220pt}` intentionally override utilities without an override warning.

Zed receives the same completion, hover, diagnostics and semantic tokens from the
standalone server; these features use the native parser and utility registry.
