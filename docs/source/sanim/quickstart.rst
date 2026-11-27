Quickstart
==========

Save as ``hello.sanim``:

.. code-block:: sanim

   let title = <Label font_size={48pt}>Hello, Sanim</Label>
   title.fade_in()
   wait(1s)
   title.fade_out()

Unassigned top-level animations run in order. Drag the file onto a Shrimply
video track; saving changes updates the preview.

To render from the terminal:

.. code-block:: console

   $ shrimply sanim check hello.sanim
   $ shrimply sanim video hello.sanim --output hello.mp4

Output defaults to 1920×1080, 30 FPS and 96 DPI. See :doc:`cli` for preview,
watch and export options.

To edit the text in Shrimply's inspector, replace the first line with:

.. code-block:: sanim

   let title = <Label>{reflect("Hello, Sanim", label="Title")}</Label>

Next: :doc:`layout-and-animation` for composition; :doc:`language` for values,
signals and types.
