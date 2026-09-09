Flatpak Installation
====================

The Flatpak provides pre-alpha GTK builds for x86_64 Linux. Flatpak and a
compatible NVIDIA driver must be installed on your system.

Install and launch
------------------

Install Shrimply from the prerelease repository:

.. code-block:: console

   $ flatpak install --user https://soirihiroka.github.io/shrimply/shrimply-prerelease.flatpakref
   $ flatpak run dev.shrimply.Shrimply

Continue with :doc:`getting-started` to create a project and start editing.

Update
------

The prerelease repository receives builds from the ``main`` branch. Install
available updates with:

.. code-block:: console

   $ flatpak update --user dev.shrimply.Shrimply

Install a downloaded bundle
---------------------------

Alternatively, download ``shrimply-gtk.flatpak`` from the `GitHub releases page
<https://github.com/soirihiroka/shrimply/releases>`__ and run this command from
the download directory:

.. code-block:: console

   $ flatpak install --user ./shrimply-gtk.flatpak

The bundle also receives updates from the prerelease repository.

Known limitations
-----------------

The Flatpak is missing some features, including MCP support, and its project
lockfile support is currently broken. To build Shrimply from source, see
:doc:`development`.
