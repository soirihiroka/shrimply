Sanim
=====

Sanim is a standalone declarative animation language for Shrimply. Write a
``.sanim`` file, drop it onto a Shrimply video track for live GPU playback, or
render it headlessly with the CLI or native presenter.

.. code-block:: sanim

   let tint = reflect(Color("orange3"))

   fn make_marker(identity: String, radius: Length = 24pt): Element {
       return Box(
           id=identity, fill=tint, position=[0%, 0%],
           width=2 * radius, height=2 * radius, radius="full",
           x_align=Align.Center, y_align=Align.Center,
       )
   }

   let marker = make_marker("marker")
   marker.fade_in(duration=250ms)
   repeat 2 {
       marker.move_to([30%, 0%], duration=1s)
       marker.move_to([-30%, 0%], duration=1s)
   }
   assert(always(inside_canvas(marker)))

Current boundaries
------------------

Sanim is not a drop-in clone of Manim. It does not include audio, physics,
arbitrary per-frame callbacks, LaTeX typesetting, or general path morphing
(``Transform`` / ``ReplacementTransform``). Text uses Pango vector outlines
(bitmap/color glyphs fail explicitly). Timeline arithmetic is exact; layout
and rasterization use floating geometry, so bit-identical output across
platforms is not promised.

.. toctree::
   :maxdepth: 1
   :caption: Getting started

   quickstart
   editor-and-presenter

.. toctree::
   :maxdepth: 1
   :caption: Everyday workflows

   layout-and-animation
   text-wrapping
   scrolling
   code-view
   data-and-charts
   dataframes
   images-and-textures
   colors
   curves

.. toctree::
   :maxdepth: 1
   :caption: Language reference

   language
   functions

.. toctree::
   :maxdepth: 1
   :caption: Tooling

   cli
   editor-setup

.. toctree::
   :maxdepth: 1
   :caption: Experimental

   voxels
