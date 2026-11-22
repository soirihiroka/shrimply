Android
=======

Requirements
------------

* **OS**: Android 13 or later.
* **Hardware**: 64-bit ARM (``arm64-v8a``) device with Vulkan 1.1 support.
* **Updater**: `Obtainium <https://obtainium.imranr.dev>`__ (recommended for
  prerelease updates).

Install with Obtainium
----------------------

With Obtainium installed on your device, open the link below to add Shrimply
with prerelease tracking preconfigured:

`Add Shrimply to Obtainium <https://apps.obtainium.imranr.dev/redirect?r=obtainium://app/%7B%22id%22%3A%22dev.shrimply.app%22%2C%22url%22%3A%22https%3A%2F%2Fgithub.com%2Fsoirihiroka%2Fshrimply%22%2C%22author%22%3A%22soirihiroka%22%2C%22name%22%3A%22Shrimply%22%2C%22additionalSettings%22%3A%22%7B%5C%22includePrereleases%5C%22%3Atrue%2C%5C%22fallbackToOlderReleases%5C%22%3Atrue%2C%5C%22includeZips%5C%22%3Atrue%2C%5C%22apkFilterRegEx%5C%22%3A%5C%22shrimply-android-arm64%5C%22%2C%5C%22versionDetection%5C%22%3Afalse%7D%22%7D>`__

To add the source manually in Obtainium:

1. Select :guilabel:`Add App` and enter
   ``https://github.com/soirihiroka/shrimply``.
2. Enable :guilabel:`Include prereleases`,
   :guilabel:`Fallback to older releases`, and :guilabel:`Include ZIP files`.
3. Set the APK filter regex to ``shrimply-android-arm64``.
4. Disable standard version detection (prerelease tags use commit timestamps
   rather than semantic versions).
5. Select :guilabel:`Add`, then install the APK.

Update
------

Obtainium tracks prereleases with completed Android builds. Open Obtainium to
check for updates, or enable background updates in Obtainium's settings.

Install a downloaded archive
----------------------------

Alternatively, download ``shrimply-android-arm64.zip`` from the `GitHub
releases page <https://github.com/soirihiroka/shrimply/releases>`__, extract
``shrimply-android-arm64.apk``, and install it on your device.
