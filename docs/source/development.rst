Development
===========

Requirements
------------

A source build needs about 70 GB free. The setup targets Fedora:

.. code-block:: console

   $ git submodule update --init --recursive
   $ make deps-fedora
   $ make dev

The Qt launcher is ``make dev-qt``. The dev log is ``target/shrimply-dev.log``.

* **Slang**: downloaded automatically during build (requires ``curl``, ``tar``,
  and ``sha256sum``). To use a local copy, set ``SLANG_LIBRARY_DIR`` and
  ``SLANG_INCLUDE_DIR``.
* **SPIR-V Tools**: portable Vulkan OBJ shaders require ``spirv-opt`` and
  ``spirv-val`` at build time. Set ``SPIRV_OPT`` and ``SPIRV_VAL`` if they are
  outside ``PATH``.
* **CUDA Toolkit 12.9**: required to build the shared Linux GTK binary even
  when running on the Intel backend.
* **FFmpeg**: native and packaged builds use the revision in
  ``external/ffmpeg`` (with Vulkan, VA-API, DRM, NVDEC, and NVENC on Linux).
  ``make linux-ffmpeg`` builds the local libraries and command-line tools.
* **Python 3.14 & uv**: required for the compute server, the Manim worker, and
  building this documentation site. Always run Python environments with
  ``uv run --locked`` so repository lockfiles and overrides are respected.

macOS uses the same Makefile targets. ``make check`` there skips the Python
and documentation checks.

Nix development environment
---------------------------

The flake provides Rust, the CUDA toolkit, Slang, and the native dependencies
for the GTK and Qt apps on ``x86_64-linux``. It requires Nix 2.27 or newer
to include the FFmpeg submodule.

.. code-block:: console

   $ git submodule update --init --recursive
   $ nix develop --accept-flake-config
   $ make dev

The flake uses the `NixOS CUDA binary cache
<https://wiki.nixos.org/wiki/CUDA#Setting_up_CUDA_Binary_Cache>`__. A multi-user
Nix install may need that cache in the system Nix configuration before the
daemon will trust it.

Build the GTK app:

.. code-block:: console

   $ nix build --accept-flake-config
   $ ./result/bin/shrimply

The published flake also builds with
``nix build github:soirihiroka/shrimply --accept-flake-config``. For a fork,
use ``git+https://github.com/OWNER/shrimply.git?submodules=1`` to fetch its
source and submodules directly. The package ships CUDA kernels for compute
capability 8.6 (GeForce RTX 30-series).

Build and check
---------------

.. code-block:: console

   $ make dev
   $ make build
   $ make check
   $ make test
   $ make release

``make dev-qt`` builds and launches the Qt app. ``make qt-build`` builds it
without launching. The binary is ``target/qt/debug/shrimply`` on Linux and
``target/debug/shrimply.exe`` on Windows.

Build this documentation site with ``make docs``, and validate links and
warnings with ``make docs-check``.

Packaging
---------

Docker [Experimental]
~~~~~~~~~~~~~~~~~~~~~

.. code-block:: console

   $ git submodule update --init --recursive
   $ docker buildx build --target export --output type=local,dest=dist/stage .

Compute server Docker images can be built locally with:

.. code-block:: console

   $ docker build -t shrimply-server:cuda12.6 server
   $ docker build --build-arg CUDA_VERSION=13.0 -t shrimply-server:cuda13.0 server

The CUDA 13.0 build compiles PyCOLMAP using two compiler jobs by default; pass
``--build-arg BUILD_JOBS=8`` to speed up a local build with sufficient memory.

Contributing
------------

Read the `contribution terms
<https://github.com/soirihiroka/shrimply/blob/main/CONTRIBUTING.md>`__ before
submitting a change. For Sanim workspace internals and benchmarks, see
`sanim/ARCHITECTURE.md
<https://github.com/soirihiroka/shrimply/blob/main/sanim/ARCHITECTURE.md>`__.
For the compute server wire protocol, see `server/PROTOCOL.md
<https://github.com/soirihiroka/shrimply/blob/main/server/PROTOCOL.md>`__.
