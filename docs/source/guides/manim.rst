Manim
=====

`3b1b's Manim <https://github.com/3b1b/manim>`__ is a Python engine for making
precise, programmatic animations. It is commonly used for explanatory videos
containing text, equations, shapes, graphs, and 3D scenes. Shrimply can place a
scene directly on the timeline, render it at the project's canvas size and
frame rate, and composite its transparent background with other video tracks.

Manim sources require approval before execution. See :doc:`/security` for
file and folder trust settings.

Create a scene
--------------

Shrimply includes its own copy of Manim. Scene files import that copy through
``manimlib`` and define at least one ``Scene`` subclass:

.. code-block:: python

   from manimlib import *


   class HelloShrimply(Scene):
       def construct(self):
           square = Square()
           circle = Circle()
           circle.set_fill(BLUE, opacity=0.5)

           self.play(ShowCreation(square))
           self.wait()
           self.play(ReplacementTransform(square, circle))
           self.wait()

The scene's ``play`` and ``wait`` calls determine its duration. Shrimply updates
the timeline item to that duration after the scene loads. A scene with no
``play`` or ``wait`` call becomes a single still frame.

Import and edit a scene
-----------------------

Import the ``.py`` file onto a video track with **Import Media…**, or drag it
onto the timeline. Select the item to open its Manim controls in the inspector.

If the file defines multiple scene classes, choose one with the **Scene**
control. Changing the scene clears parameter values from the previously
selected scene. Use **Anti-aliasing** to trade rendering cost for smoother
edges.

After editing the Python source, click the reload button in the Manim inspector
to rebuild the scene and refresh its scene list and parameters. Python errors
appear in the same inspector.

Expose parameters in the inspector
----------------------------------

The ``shrimply_manim`` reflection API lets a scene author turn Python values
into editable inspector controls. Each function returns the current value: its
default when first imported, or the value chosen in Shrimply on later renders.
``shrimply_manim`` is supplied by Shrimply while it loads the scene; it is not
part of upstream Manim.

Call the functions inside ``construct`` so each scene exposes only its own
controls:

.. code-block:: python

   from fractions import Fraction

   from manimlib import *
   from shrimply_manim import use_color, use_fraction, use_float, use_option


   class ReflectedScene(Scene):
       def construct(self):
           radius = use_float(
               1.0,
               min=0.25,
               max=3.0,
               step=0.25,
               key="radius",
               label="Radius",
           )
           color = use_color("blue3", key="color", label="Color")
           entrance = use_option(
               ["Draw", "Fade"],
               key="entrance",
               label="Entrance",
           )
           hold = use_fraction(
               Fraction(1, 2),
               key="hold",
               label="Hold time",
           )

           circle = Circle(radius=radius)
           circle.set_fill(color, opacity=0.5)
           if entrance == "Draw":
               self.play(ShowCreation(circle))
           else:
               self.play(FadeIn(circle))
           self.wait(hold)

Available controls
~~~~~~~~~~~~~~~~~~

.. list-table::
   :header-rows: 1
   :widths: 24 32 44

   * - Function
     - Inspector control
     - Useful options
   * - ``use_int(default=0)``
     - Integer field
     - ``min``, ``max``, and positive ``step``
   * - ``use_float(default=0.0)``
     - Decimal field
     - ``min``, ``max``, and positive ``step``
   * - ``use_fraction(default=Fraction(0))``
     - Exact decimal field
     - Use for durations and other values that should remain exact
   * - ``use_color(default="blue3")``
     - Color picker
     - A ``#RRGGBB`` value or an Adwaita color from ``blue1`` through
       ``blue5``, and likewise for green, yellow, orange, red, purple, brown,
       light, and dark
   * - ``use_option(options, default=None)``
     - Choice menu
     - A nonempty sequence of unique strings; the first is the default when
       ``default`` is omitted
   * - ``use_bool(default=False)``
     - Switch
     - ``True`` or ``False``
   * - ``use_string(default="")``
     - Single-line text field
     - Any string

Every function also accepts keyword-only ``key`` and ``label`` arguments.
``label`` is the name shown in the inspector. ``key`` is the stable identity
used to save the value in the project.

Parameter guidelines
~~~~~~~~~~~~~~~~~~~~

* Give every parameter an explicit, unique ``key``. An automatically generated
  key depends on call order, so adding or moving a parameter can associate saved
  values with the wrong control.
* Keep reflected calls unconditional and in a stable order. This keeps the
  inspector predictable when another parameter changes.
* Treat a key's type as permanent. If a parameter's type changes, use a new key
  or reset its parameters in the inspector.
* Prefer ``use_fraction`` for ``wait`` durations and timing calculations when
  exact timing matters.
* Validate any reflected string before using it as a file path, class name, or
  other structured value. The text field accepts arbitrary input.

Parameters may also be reflected at module scope, including from a sibling
Python module imported by the scene. Such controls are evaluated for every
scene in the file, so scene-specific values normally belong in ``construct``.
