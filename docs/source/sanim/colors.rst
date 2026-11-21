Colors and perceptual ramps
===========================

``Color`` is shared by 2D elements, generated textures and voxel materials. Named
Adwaita colors use Shrimply's palette. String parsing also uses the shared CSS
name/hex/rgb/hsl parser, including ``Color("rebeccapurple")`` and ``Color("#abc")``.
``Color("transparent")`` is transparent black.

Typed constructors avoid the byte quantization of legacy CSS string parsing:

.. code-block:: text

   let rgb = Rgb(0.2, 0.5, 0.8, alpha=0.75)
   let hsl = Hsl(210deg, 0.6, 0.5)
   let hsv = Hsv(210deg, 0.6, 0.8)
   let lab = Oklab(0.7, -0.05, -0.1)
   let accent = Oklch(0.7, 0.15, 250deg)

RGB channels, saturation, lightness, HSV value and alpha use 0..1. Oklab ``a`` and
``b`` are signed; OKLCH chroma is nonnegative. Hue is a typed angle, wraps around
360 degrees and accepts negative angles. Nonfinite values and invalid ranges
produce source diagnostics rather than silently clamping arguments.

Conversion methods return typed records, including their alpha:

.. code-block:: text

   let rgb_channels = accent.to_rgb()       # red, green, blue, alpha
   let hsl_channels = accent.to_hsl()       # hue, saturation, lightness, alpha
   let hsv_channels = accent.to_hsv()       # hue, saturation, value, alpha
   let lab_channels = accent.to_oklab()     # lightness, a, b, alpha
   let lch_channels = accent.to_oklch()     # lightness, chroma, hue, alpha

Mixing and adjustments
----------------------

``mix`` defaults to OKLab. Explicit spaces are ``"oklab"``, ``"oklch"``,
``"linear-rgb"`` and ``"srgb"``. RGB and OKLab channels are interpolated with
premultiplied alpha; OKLCH lightness and chroma are premultiplied while hue is
interpolated around the selected angular path. Gray or fully transparent
endpoints do not introduce an arbitrary hue. Endpoints are preserved exactly.

.. code-block:: text

   let warm = Color("orange3")
   let cool = Color("blue3")
   let middle = warm.mix(cool, 0.5)
   let rainbow = warm.mix(cool, 0.5, space="oklch", hue="longer")
   let encoded = warm.mix(cool, 0.5, space="srgb")
   let adjusted = middle.with_lightness(0.75).with_chroma(0.1).rotate_hue(20deg)
   let translucent = adjusted.with_alpha(0.4)

OKLCH hue paths are ``"shorter"`` (default), ``"longer"``, ``"increasing"`` and
``"decreasing"``. ``with_hue(angle)`` sets hue directly. ``over(background)``
composites in linear light, preserving alpha. ``luminance()`` returns relative
linear-light luminance; ``contrast(other)`` returns a contrast ratio and requires
opaque colors. Composite translucent colors first. ``distance(other)`` measures
Euclidean OKLab distance, excluding alpha.

The alpha and hue rules follow the standard distinction described in
`CSS Color 4 interpolation <https://www.w3.org/TR/css-color-4/#interpolation>`_.

Display gamut
-------------

OKLab/OKLCH constructors, mixing and perceptual adjustments accept
``gamut="chroma"`` (default), ``"clip"`` or ``"error"``. Chroma mapping uses a
bounded radial search, reducing chroma while preserving lightness and hue.
It is not CSS's local-MINDE mapping algorithm. Clip clamps display RGB channels;
error reports that the requested color lies outside display sRGB.

Signals and reuse
-----------------

Color constructors, methods and conversion-record fields use the same retained
native graph as other animated properties. Sampling never interprets source
functions again. Multiple fields of one conversion share a frame-local result;
projecting a literal record depends only on the selected field.

.. code-block:: text

   mut hue = 30deg
   sig tint = Oklch(0.72, 0.12, hue)
   sig lightness = tint.to_oklch().lightness
   let swatch = <Box w-160 h-80 fill={tint}/>
   Animation(children=[swatch])
   animate(hue, to=390deg, duration=4s)

Generated texture ramps use the same color interpolation. Palette stops are
converted once per prepared ramp, with bounded allocation and work accounting.
