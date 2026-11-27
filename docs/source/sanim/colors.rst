Colors
======

Constructors
------------

In signatures, ``Gamut`` abbreviates ``"chroma" | "clip" | "error"``.

.. code-block:: text

   Color(value: String = "red3"): Color
   Rgb(red: Number, green: Number, blue: Number, alpha: Number = 1): Color
   Hsl(hue: Angle, saturation: Number, lightness: Number, alpha: Number = 1): Color
   Hsv(hue: Angle, saturation: Number, value: Number, alpha: Number = 1): Color
   Oklab(lightness: Number, a: Number, b: Number, alpha: Number = 1, gamut: Gamut = "chroma"): Color
   Oklch(lightness: Number, chroma: Number, hue: Angle, alpha: Number = 1, gamut: Gamut = "chroma"): Color

``Color`` accepts Adwaita/CSS names and CSS color strings, such as ``"blue3"``,
``"#3584e4"`` or ``"transparent"``. RGB, saturation, lightness, value and alpha
are in 0..1. OKLab ``a``/``b`` are signed; chroma is nonnegative. Angles use
units such as ``210deg``.

Mix two colors
--------------

Call on a ``Color`` value:

.. code-block:: text

   color.mix(
       other: Color,
       amount: Number,
       space: "oklab" | "oklch" | "linear-rgb" | "srgb" = "oklab",
       hue: "shorter" | "longer" | "increasing" | "decreasing" = "shorter",
       gamut: "chroma" | "clip" | "error" = "chroma"
   ): Color

``amount`` is the share of ``other``: 0 returns ``color``, 1 returns ``other``;
0.5 blends halfway. It must be in 0..1.

.. list-table:: ``space`` chooses how the intermediate color is calculated
   :header-rows: 1

   * - Value
     - Meaning
   * - ``"oklab"``
     - Blend perceptual lightness and color axes; the default.
   * - ``"oklch"``
     - Blend lightness, chroma and hue; ``hue`` selects the angular path.
   * - ``"linear-rgb"``
     - Blend RGB channels in linear light.
   * - ``"srgb"``
     - Blend encoded display RGB channels directly.

.. code-block:: sanim

   let midpoint = Color("orange3").mix(other=Color("blue3"), amount=0.5)

For colors outside display sRGB, ``gamut="chroma"`` reduces chroma while
preserving lightness and hue, ``"clip"`` clamps RGB channels, and ``"error"``
fails. The same option applies to OKLab/OKLCH constructors and adjustments.

Adjust and measure
------------------

.. code-block:: text

   color.with_alpha(alpha: Number): Color
   color.with_lightness(lightness: Number, gamut: Gamut = "chroma"): Color
   color.with_chroma(chroma: Number, gamut: Gamut = "chroma"): Color
   color.with_hue(hue: Angle, gamut: Gamut = "chroma"): Color
   color.rotate_hue(angle: Angle, gamut: Gamut = "chroma"): Color
   color.over(background: Color): Color
   color.luminance(): Number
   color.contrast(other: Color): Number
   color.distance(other: Color): Number

Adjustments use OKLCH. ``over`` composites in linear light. ``luminance`` is
relative linear-light brightness; ``contrast`` returns a contrast ratio and
requires opaque colors.
``distance`` is Euclidean OKLab distance, excluding alpha.

Conversions
-----------

Each method returns a record with the listed fields:

.. code-block:: text

   color.to_rgb(): {red: Number, green: Number, blue: Number, alpha: Number}
   color.to_hsl(): {hue: Angle, saturation: Number, lightness: Number, alpha: Number}
   color.to_hsv(): {hue: Angle, saturation: Number, value: Number, alpha: Number}
   color.to_oklab(): {lightness: Number, a: Number, b: Number, alpha: Number}
   color.to_oklch(): {lightness: Number, chroma: Number, hue: Angle, alpha: Number}

Color operations accept signals; see :doc:`language`. Texture palettes use the
same mixing rules (:doc:`images-and-textures`).
