Flatpak Installation
====================

The Flatpak provides pre-alpha GTK builds for x86_64 Linux. Install Flatpak
and either a compatible NVIDIA driver or Intel's Mesa and VA-API drivers.
See :ref:`development-intel-preview` for the Intel preview backend's scope.

Install and launch
------------------

Install Shrimply from the prerelease repository:

.. code-block:: console

   $ flatpak install --user https://soirihiroka.github.io/shrimply/shrimply-prerelease.flatpakref
   $ flatpak run dev.shrimply.Shrimply

Update
------

The prerelease repository receives builds from the ``main`` branch. Install
available updates with:

.. code-block:: console

   $ flatpak update --user dev.shrimply.Shrimply

Install a downloaded bundle
---------------------------

Alternatively, download ``shrimply-gtk-x86_64.flatpak`` from the `GitHub releases page
<https://github.com/soirihiroka/shrimply/releases>`__ and run this command from
the download directory:

.. code-block:: console

   $ flatpak install --user ./shrimply-gtk-x86_64.flatpak

The bundle also receives updates from the prerelease repository.

Known limitations
-----------------

The Flatpak is missing some features, including MCP support. To build Shrimply
from source, see :doc:`development`.

Build a local bundle with ``make flatpak-gtk``. This passes the checkout's full
Git commit to Docker. When building from a source archive without Git metadata,
set ``SHRIMPLY_BUILD_GIT_HASH`` to the archive's full source commit SHA.
