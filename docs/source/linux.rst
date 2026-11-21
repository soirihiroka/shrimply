Linux
=====

Requirements
------------

* **OS & display**: x86_64 Linux (Fedora 44 or equivalent) running **Wayland**.
* **GPU**: a compatible NVIDIA GPU (Turing through Blackwell; GTX 900+ may
  work) with NVIDIA drivers, **or** an Intel GPU with Mesa's Intel
  Vulkan/OpenGL drivers and Intel's VA-API media driver.
* **Audio**: **PipeWire** or **PulseAudio** (ALSA alone is not supported).

Install and launch
------------------

Shrimply provides pre-alpha GTK builds via Flatpak. Install from the
prerelease repository:

.. code-block:: console

   $ flatpak install --user https://soirihiroka.github.io/shrimply/shrimply-prerelease.flatpakref
   $ flatpak run dev.shrimply.Shrimply

Update
------

The prerelease repository receives builds from the ``main`` branch:

.. code-block:: console

   $ flatpak update --user dev.shrimply.Shrimply

Install a downloaded bundle
---------------------------

Alternatively, download ``shrimply-gtk-x86_64.flatpak`` from the `GitHub releases page
<https://github.com/soirihiroka/shrimply/releases>`__ and run from the download
directory:

.. code-block:: console

   $ flatpak install --user ./shrimply-gtk-x86_64.flatpak

The bundle also receives updates from the prerelease repository.

GPU selection
-------------

Shrimply uses NVIDIA automatically when available, and Intel otherwise. To
choose a specific GPU, open
:menuselection:`Preferences --> Performance --> Acceleration device` and restart
the editor, or set ``SHRIMPLY_GPU=intel`` / ``SHRIMPLY_GPU=cuda`` (and
``SHRIMPLY_INTEL_DEVICE=/dev/dri/renderD128`` if you have multiple Intel
devices).

For 3D ray-tracing denoising on Intel Xe and newer GPUs, install Intel's Level
Zero GPU driver to use Open Image Denoise; older Intel GPUs use the built-in
fallback.

Terminal command
----------------

To use the ``shrimply`` CLI from your host terminal, open
:menuselection:`Preferences --> External` and select :guilabel:`Install` next
to :guilabel:`CLI`. See :doc:`integrations/cli` for details.

To build Shrimply from source (or via Nix), see :doc:`development`.
