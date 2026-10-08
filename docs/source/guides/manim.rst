Manim
=====

Shrimply can put a `3b1b Manim <https://github.com/3b1b/manim>`__ scene on the
timeline. The scene renders at the project canvas size and frame rate, with a
transparent background.

Manim files need approval before they run. See :doc:`/security`.

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
selected scene. **Anti-aliasing** smooths edges and takes longer.

After editing the Python source, click the reload button in the Manim inspector
to rebuild the scene and refresh its scene list and parameters. Python errors
appear in the same inspector.

Expose parameters in the inspector
----------------------------------

``shrimply_manim`` turns Python values into inspector controls. Each call
returns the value set in Shrimply, or the default if it has not been set.
Shrimply provides ``shrimply_manim`` while loading the scene. It is not part
of upstream Manim.

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

* Give every parameter an explicit, unique ``key``. A generated key follows
  call order, so adding or moving a parameter can attach a saved value to the
  wrong control.
* Keep the calls unconditional and in a stable order, so the inspector stays
  put when another parameter changes.
* Treat a key's type as fixed. If the type changes, pick a new key or reset
  the parameter in the inspector.
* A parameter at module scope, including one in a sibling module the scene
  imports, shows up on every scene in the file. Scene-specific values belong
  in ``construct``.
