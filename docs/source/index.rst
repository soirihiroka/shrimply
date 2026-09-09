Shrimply Documentation
======================

A simple yet powerful cross platform video editor.

Shrimply is a free and open-source video editor for creating videos from start
to finish, whether you are making a quick edit or something fancy.

Visit the `Shrimply repository on GitHub
<https://github.com/soirihiroka/shrimply>`__ to browse the source code, report
issues, and contribute.

Getting started
---------------

* :doc:`Getting started <getting-started>` explains how to launch Shrimply,
  create a project, edit the timeline, and export it.
* :doc:`Editor <guides/editor>` describes the main workspaces and shortcuts.

.. toctree::
   :maxdepth: 2
   :hidden:
   :caption: Getting started

   getting-started
   guides/editor

Installation
------------

* :doc:`Flatpak <flatpak>` covers installing and updating the Linux package.

.. toctree::
   :maxdepth: 2
   :hidden:
   :caption: Installation

   Flatpak <flatpak>

Editing
-------

* :doc:`Importing and creating media <guides/media>` lists accepted sources
  and generated content, with guides for Blender, Kdenlive, and Manim.
* :doc:`Effects and animation <guides/effects>` covers keyframes, visual and
  audio effects, and 3D scenes.
* :doc:`Expressions <guides/expressions>` covers values and functions for
  procedural and audio-reactive properties.
* :doc:`Audio and captions <guides/audio-captions>` explains caption editing,
  audio processing, and lip sync.
* :doc:`Export <guides/export>` describes output formats and encoder options.

.. toctree::
   :maxdepth: 2
   :hidden:
   :caption: Editing

   guides/media
   guides/effects
   guides/expressions
   guides/audio-captions
   guides/export

Compute and automation
----------------------

* :doc:`Compute server <server/index>` covers setup and optional AI features.
* :doc:`MCP integration <integrations/mcp>` documents live editor automation.

.. toctree::
   :maxdepth: 2
   :hidden:
   :caption: Compute and automation

   server/index
   integrations/mcp

Project information
-------------------

* :doc:`Development <development>` contains the supported repository workflow.
* :doc:`Licenses <licenses>` explains project and third-party licensing.

.. toctree::
   :maxdepth: 2
   :hidden:
   :caption: Project information

   development
   licenses

.. image:: img/editor-overview.png
   :alt: Shrimply editor showing the inspector, video preview, and multitrack timeline
   :width: 100%
