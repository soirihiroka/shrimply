Development
===========

Requirements
------------

Building Shrimply from source requires around 70GB or more of free disk space
and a reasonably modern machine. Due to the complexity of the development
setup, using a coding agent to help with setup is recommended.

The current development setup targets Fedora and uses the Rust toolchain in
``rust-toolchain.toml``. Install the native dependencies with:

.. code-block:: console

   $ make deps-fedora

The ``shrimply-slang-build`` crate's ``build.rs`` downloads the pinned Slang
binary release and verifies its SHA-256 checksum. Downloads are locked and
extracted atomically into a versioned cache under Cargo's build directory
(``target/``, ignored by Git), shared across crate rebuilds. Slang is never
compiled from source. The download requires ``curl``, ``tar``, and ``shasum``
(macOS) or ``sha256sum`` (Linux). To use an existing binary distribution,
set ``SLANG_LIBRARY_DIR`` and ``SLANG_INCLUDE_DIR`` to its library and header
directories. Slang's prebuilt library compiles the compositor shaders to CUDA,
and ``nvcc`` packages the CUDA artifacts. The supported CUDA
Toolkit version is 12.9. In theory, NVIDIA GeForce GTX 900-series through RTX
50-series GPUs should work, but this full range has not been verified.

Nix development environment
---------------------------

The checked-in flake provides the pinned Rust nightly, CUDA toolkit, Slang, and
native dependencies for the GTK and Qt applications on ``x86_64-linux``.
Initialize the required source submodules, then enter the shell:

.. code-block:: console

   $ git submodule update --init --recursive
   $ nix develop --accept-flake-config
   $ make dev

The flake requests the `NixOS CUDA binary cache
<https://wiki.nixos.org/wiki/CUDA#Setting_up_CUDA_Binary_Cache>`__. Multi-user
Nix installations may require adding that cache to the system Nix
configuration before the daemon will trust it.

Build the default GTK package with the pinned submodule sources:

.. code-block:: console

   $ nix build --accept-flake-config
   $ ./result/bin/shrimply

A compatible NVIDIA driver is still required at runtime. The package currently
embeds CUDA kernels for compute capability ``sm_86``.

The development shell sets the tool and library paths expected by the existing
Makefile, so commands such as ``make dev`` and ``make dev-qt`` work unchanged. It
also sets ``SLANG_LIBRARY_DIR`` and ``SLANG_INCLUDE_DIR`` to the same prebuilt
Slang release the Cargo build scripts would otherwise download, so builds stay
offline and reproducible.

Build and check
---------------

Use the Makefile for repository operations:

.. code-block:: console

   $ make dev
   $ make build
   $ make check
   $ make test
   $ make release

To build and run the optional Qt 6 launcher instead of the GTK launcher, use
``make dev-qt``. This produces the separate ``shrimply-qt`` development binary;
it does not replace the normal launcher or installation.
``make qt-build`` performs that debug build without launching it and writes the
binary to ``target/debug/shrimply-qt``.

``make check`` selects checks for the host platform. On Linux, it verifies native
dependencies and CUDA artifacts, formatting, source size, the selected Rust
binaries, Clippy, the server and Manim Python code, and this documentation site.
On macOS, it runs the AppKit build, Rust checks, and Clippy, plus formatting and
source-size checks; it skips Python and documentation checks. Other platforms
are unsupported. The development launcher writes its log to
``target/shrimply-dev.log``.

Build the documentation on its own with:

.. code-block:: console

   $ make docs


Docker Build Environment [Experimental]
---------------------------------------

You can build Shrimply using the provided Dockerfile. 
This will generate the necesary binaries to run Shrimply on your machine.

Remember to pull the Git Submodules before building Shrimply

.. code-block:: console

   $ docker buildx build --target export --output type=local,dest=dist/stage .

Python environments
-------------------

Python dependencies are managed with uv. The documentation, local compute
server, and Manim worker each have their own ``pyproject.toml`` and committed
``uv.lock``. Use their Makefile targets or ``uv run --project`` from the
repository root; do not install their dependencies globally.

Repository layout
-----------------

``crates/binaries``
   Launcher and editor applications.

``crates/timeline`` and ``crates/project``
   Timeline behavior, project state, storage, and project importers.

``crates/preview``, ``crates/video``, and ``crates/export``
   Playback, decoding, compositing, visual effects, and export.

``crates/audio``
   Audio rendering, modifiers, transcription, text-to-speech, and lip sync.

``crates/3d``, ``crates/paint``, and ``crates/layered-image``
   Specialized content and rendering pipelines.

``crates/math`` and ``crates/project/property-model``
   Shared math and core data types.

``crates/integrations/mcp`` and ``crates/integrations/compute-client``
   Live editor automation and compute-server communication.

``server``
   The uv-managed local AI compute server.

Contributing
------------

Read the repository's `contribution terms
<https://github.com/soirihiroka/shrimply/blob/main/CONTRIBUTING.md>`__ before
submitting a change.
