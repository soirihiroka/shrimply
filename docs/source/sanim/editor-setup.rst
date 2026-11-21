Editor Setup (LSP, VS Code, Zed)
================================

Sanim includes a built-in Language Server Protocol (LSP) server via
``shrimply sanim lsp`` (with a standalone ``sanim-lsp`` fallback). It provides
diagnostics, lint warnings, completion (including XML tags and utility
attributes), typed hover, signature help, document symbols, inferred type and
timeline duration inlay hints, semantic tokens, jump to definition (including
generated read-only standard library declarations), and document formatting.

Prerequisites
-------------

Install Shrimply and enable its terminal CLI in
:menuselection:`Preferences --> External` (macOS:
:menuselection:`Settings --> Integrations`). Both extensions check ``shrimply``
on ``PATH``, ``~/.local/bin/shrimply``, and standard macOS app locations before
falling back to standalone ``sanim-lsp``.

Live project values on hover
~~~~~~~~~~~~~~~~~~~~~~~~~~~~

When using ``shrimply sanim lsp``, hovering a ``reflect(...)`` input in a saved
``.sanim`` file shows the resolved value for each matching clip in active
Shrimply project sessions. Projects in ancestor directories are discovered
automatically, or pass ``--project /path/to/project.shrimp`` explicitly.
Unsaved buffers keep static features while withholding runtime values. The
server never launches a GUI or headless project owner.

VS Code
-------

The VS Code extension (``sanim/editors/vscode``) provides ``.sanim`` TextMate
syntax highlighting and connects to ``shrimply sanim lsp``.

* Double-click an inferred type inlay hint to insert its explicit ``: Type``
  annotation.
* Set ``sanim.server.path`` to override the executable path, or
  ``sanim.server.arguments`` (for example
  ``["sanim", "lsp", "--project", "/path/to/project.shrimp"]``) to pin a
  project context. Reload the VS Code window after changing server settings.
* The extension is disabled in untrusted workspaces.

To build and install the VSIX locally:

.. code-block:: console

   $ cd sanim/editors/vscode
   $ npm ci && npm run package
   $ code --install-extension sanim-0.1.0.vsix

Zed
---

The Zed extension (``sanim/editors/zed``) connects directly to
``shrimply sanim lsp`` over stdio. To point at a custom binary or project, add
to Zed's ``settings.json``:

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

Build and install locally from the repository root:

.. code-block:: console

   $ make -C sanim/editors/zed target
   $ make -C sanim/editors/zed install

Other LSP editors
-----------------

Any editor with a standard stdio LSP client can run ``shrimply sanim lsp`` (or
a standalone ``sanim-lsp`` built with ``make -C sanim lsp-build``). Standard
output carries only ``Content-Length`` framed JSON-RPC messages.
