Sanim for Zed
=============

The adapter prefers ``shrimply sanim lsp`` over stdio and falls back to a standalone ``sanim-lsp``. Diagnostics, completions, hover information, navigation, semantic highlighting, and duration hints use Sanim's handwritten parser and compiler. No Tree-sitter grammar or animation runtime is built into this extension.

Install Shrimply and enable its CLI in Preferences. The adapter checks PATH, ``~/.local/bin/shrimply`` and standard macOS application locations before looking for ``sanim-lsp`` on PATH. For a custom installation, configure the executable:

.. code-block:: json

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
         "binary": { "path": "/absolute/path/to/shrimply" }
       }
     }
   }

Shrimply gets ``["sanim", "lsp"]`` arguments automatically; standalone ``sanim-lsp`` gets no arguments. The adapter respects optional ``binary.arguments`` and ``binary.env``. It does not download a server or open Shrimply's GUI.

The Shrimply server adds resolved reflected input values to hovers using active project sessions. It discovers projects in source-file ancestor directories, or you can set ``binary.arguments`` to ``["sanim", "lsp", "--project", "/path/to/project.shrimp"]``. Values identify each matching clip and are withheld for unsaved source. Static language features remain available without an active project. The standalone server has no project runtime connection.

Build and install from the repository root:

.. code:: sh

   make -C sanim/editors/zed target  # install Rust's wasm32-wasip2 target once
   make -C sanim/editors/zed check
   make -C sanim/editors/zed install-check  # verify real installs and updates in an isolated directory
   make -C sanim/editors/zed install

``make install`` stages the complete prebuilt adapter, then installs it in Zed's standard user extension directory. Zed watches this directory and rebuilds its own index. Existing settings and other extensions are preserved. Run it again after changing the adapter; update Shrimply to update its compiler and server.

This package is a separate workspace with only the official Zed extension SDK. ``make check`` checks Rust and metadata; ``make build`` checks the emitted WebAssembly contains the Zed API version section.
