Development
===========

Requirements
------------

A source build needs about 70 GB free. The setup targets Fedora:

.. code-block:: console

   $ make deps-fedora
   $ make dev

The Qt launcher is ``make dev-qt``. The dev log is ``target/shrimply-dev.log``.

The build downloads Slang. You need ``curl``, ``tar``, and ``sha256sum``.
To use a copy you already have, set ``SLANG_LIBRARY_DIR`` and
``SLANG_INCLUDE_DIR``.

CUDA Toolkit 12.9. GTX 900 through RTX 50 should work; that range is not
fully tested. You still need an NVIDIA driver to run Shrimply.

macOS uses the same Makefile targets. ``make check`` there skips the Python
and documentation checks. Other platforms are unsupported.

Nix development environment
---------------------------

The flake provides Rust, the CUDA toolkit, Slang, and the native dependencies
for the GTK and Qt apps on ``x86_64-linux``.

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

You still need an NVIDIA driver. This package ships CUDA kernels for compute
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
without launching. The binary is ``target/debug/shrimply-qt``.

Build this site with ``make docs``.

Docker [Experimental]
---------------------

.. code-block:: console

   $ git submodule update --init --recursive
   $ docker buildx build --target export --output type=local,dest=dist/stage .

Contributing
------------

Read the `contribution terms
<https://github.com/soirihiroka/shrimply/blob/main/CONTRIBUTING.md>`__ before
submitting a change.
