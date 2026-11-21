macOS
=====

Requirements
------------

* **OS**: macOS 15 or later.
* **Hardware**: Apple Silicon Mac.
* **Package manager**: `Homebrew <https://brew.sh>`__.

Install and launch
------------------

Install Shrimply from the `Homebrew tap
<https://github.com/soirihiroka/homebrew-tap>`__, then launch it:

.. code-block:: console

   $ brew install --cask soirihiroka/tap/shrimply
   $ open -a Shrimply

You can also open Shrimply from your Applications folder.

Shrimply is ad-hoc signed. The cask automatically removes its quarantine
attribute on installation and upgrades, bypassing the Gatekeeper quarantine
check.

Update
------

The tap tracks prereleases with completed macOS builds:

.. code-block:: console

   $ brew update
   $ brew upgrade --cask soirihiroka/tap/shrimply

Terminal command
----------------

To install the ``shrimply`` CLI on your ``PATH``, open
:menuselection:`Settings --> Integrations` and select :guilabel:`Install` next
to :guilabel:`CLI`. See :doc:`integrations/cli` for details.
