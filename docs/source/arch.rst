Arch Linux (AUR)
================

Shrimply is available on the Arch User Repository (AUR) as `shrimply-git <https://aur.archlinux.org/packages/shrimply-git>`__, maintained by `korigamik <https://aur.archlinux.org/account/korigamik>`__.

The package builds native GTK 4 / Libadwaita binaries for ``x86_64`` Linux with CUDA acceleration and desktop integration.

Install with an AUR helper
--------------------------

Install using ``paru``:

.. code-block:: console

   $ paru -S shrimply-git

Or with ``yay``:

.. code-block:: console

   $ yay -S shrimply-git

Manual build with makepkg
-------------------------

Clone the AUR repository and build with ``makepkg``:

.. code-block:: console

   $ git clone https://aur.archlinux.org/shrimply-git.git
   $ cd shrimply-git
   $ makepkg -s -i

Hardware and drivers
--------------------

Shrimply requires an NVIDIA GPU with a compatible driver (provided by ``nvidia-utils``) and CUDA support.
