MCP Integration
===============

``shrimply-mcp`` lets an MCP client drive an open project.

Configure the adapter
---------------------

A development build is at ``target/debug/shrimply-mcp``. Point the client at
that binary:

.. code-block:: toml

   [mcp_servers.shrimply]
   command = "/absolute/path/to/shrimply/target/debug/shrimply-mcp"

An installed release can use ``shrimply-mcp`` as the command. After
``make dev``, register the development adapter with Codex:

.. code-block:: console

   $ make install-codex-mcp-dev

The Flatpak build does not include MCP.

Create or connect to a project
------------------------------

``create_project`` takes an absolute path ending in ``.shrimp``. Shrimply
creates that file, opens it, and connects the session. The file must not
already exist. The name, width, height, and frame rate default to Untitled
Project, 1920, 1080, and 30 fps. The frame rate has to be one Shrimply
supports.

For a project that already exists, open it in Shrimply and call
``connect_project`` with its path.

The connection fails if the project is closed, the lock is stale, or that
window has a different project open. If the editor does not launch, the new
``.shrimp`` file is still there.

The client lists the tools and their arguments. It can read the timeline,
move the playhead, add and edit clips, import media, run transcription and
text-to-speech, and render a frame.
