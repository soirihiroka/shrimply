# Sanim for Zed

The adapter launches the standalone `sanim-lsp` over stdio. Diagnostics,
completions, hover information, navigation, semantic highlighting, and duration
hints use Sanim's handwritten parser and compiler. No Tree-sitter grammar or
animation runtime is built into this extension.

Put `sanim-lsp` on the project environment's `PATH`, or configure a binary path:

```jsonc
{
  "languages": {
    "Sanim": {
      "language_servers": ["sanim-lsp"],
      "semantic_tokens": "full",
      "inlay_hints": { "enabled": true }
    }
  },
  "lsp": {
    "sanim-lsp": {
      "binary": { "path": "/absolute/path/to/sanim-lsp" }
    }
  }
}
```

The adapter respects optional `binary.arguments` and `binary.env` as well. It
does not download or start any other server.

Build and install from this directory:

```sh
make target  # install Rust's wasm32-wasip2 target once
make check
make install-check  # verify real installs and updates in an isolated directory
make install
```

`make install` stages the complete prebuilt adapter, then installs it in Zed's
standard user extension directory. Zed watches this directory and rebuilds its
own index. Existing settings and other extensions are preserved. Run it again
after changing the adapter; update `sanim-lsp` separately after compiler changes.

This package is a separate workspace with only the official Zed extension SDK.
`make check` checks Rust and metadata; `make build` checks the emitted WebAssembly
contains the Zed API version section.
