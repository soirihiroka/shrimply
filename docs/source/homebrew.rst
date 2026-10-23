Homebrew Installation
=====================

The `Homebrew tap <https://github.com/soirihiroka/homebrew-tap>`__ provides
pre-alpha builds for Apple Silicon Macs running macOS 15 or later.
`Homebrew <https://brew.sh>`__ must be installed on your system.

Install and launch
------------------

Install Shrimply from the tap, then launch it:

.. code-block:: console

   $ brew install --cask soirihiroka/tap/shrimply
   $ open -a Shrimply

You can also open Shrimply from your Applications folder.

Shrimply is ad-hoc signed. The cask automatically removes its quarantine
attribute on installation and upgrades, bypassing the Gatekeeper quarantine
check.

Update
------

The tap tracks prereleases with completed macOS builds. Install available
updates with:

.. code-block:: console

   $ brew update
   $ brew upgrade --cask soirihiroka/tap/shrimply
