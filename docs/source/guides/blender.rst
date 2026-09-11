Blender
=======

Shrimply uses a locally installed copy of Blender to read and render ``.blend``
files. On macOS, the default executable is
``/Applications/Blender.app/Contents/MacOS/Blender``. To use another installation,
or configure Blender on other platforms, open the application menu and choose
:menuselection:`Preferences --> External`. Under :guilabel:`Blender`, click
:guilabel:`Choose…` and select the Blender executable. Shrimply checks the
selected executable and remembers it for future projects.

Blender reads the file metadata directly, including compressed ``.blend`` files.
The configured Blender installation must support the file's version. Import
reports an error if Blender cannot open the file.

After selecting Blender, import a ``.blend`` file onto a video track with
:menuselection:`Add --> Import Media…`, or drag it onto the timeline. Select the
item to choose its scene, view layer, camera, and preview settings in the
inspector.

Blender sources require approval before metadata inspection or rendering.
See :doc:`/security` for file and folder trust settings.
