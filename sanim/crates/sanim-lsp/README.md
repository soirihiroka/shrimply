The Sanim language server is a separate native executable:

```sh
CARGO_INCREMENTAL=0 make -C sanim lsp-build
sanim/target/release/sanim-lsp
```

It communicates over LSP standard input/output. Standard output contains only
`Content-Length` framed JSON-RPC messages. It is not part of Shrimply's normal
binary build and is never loaded into the animation runtime.

The server reuses Sanim's source loader, parser, static checker, type information,
prelude registration, and lint rules. It does not evaluate scripts, read runtime
CSV/JSON/font assets, shape text, render frames, or initialize the GUI. Unsaved
import overlays are analyzed together with their open dependents.

Implemented editor features include incremental UTF-16 document synchronization,
versioned diagnostics, native and local completion, typed hover, signature help,
document symbols, inferred type/timeline duration inlay hints, semantic tokens, and
jump to definition (including imported module members), and source formatting.
Formatting reuses the canonical parser, preserves comments/literal contents, and
returns edits for unsaved text without evaluating or writing files.
Unsupported methods return JSON-RPC MethodNotFound and are not advertised as
capabilities. Semantic tokens come from the canonical AST and use standard editor theme token
names. VS Code also has a TextMate grammar for highlighting incomplete source.

`make -C sanim lsp-check` verifies framing, lifecycle, UTF-16 and CRLF positions,
atomic incremental edits, diagnostics, unsaved imports, and navigation. This
server can also be configured in any editor with a standard LSP client.
