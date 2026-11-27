Images and textures
===================

Display files, URLs or generated pixels with XML:

.. code-block:: sanim

   let image = <Image src="./photo.png" width={300pt}/>
   image.fade_in()
   wait(1s)

``src`` accepts ``String | Texture``. One dimension preserves aspect ratio;
local files are watched. SVG is displayed as native vectors. Remote HTTP/HTTPS
assets use the shared cache; SVG external resources are not fetched.

Create pixels
-------------

``?`` marks an optional argument in these signatures.

.. code-block:: text

   read<Texture>(path: String): Texture
   Texture(
       size: List<Number, 2>, pixels?: List<Color>, fill?: Color,
       field?: Density, colors?: List<Color>, mask?: Density, alpha?: Density,
       space: "oklab" | "oklch" | "linear-rgb" | "srgb" = "oklab"
   ): Texture

``read<Texture>`` decodes static PNG/JPEG. Sizes are integer pixels, up to
4096 per axis within host budgets. Choose ``pixels`` (row-major), ``fill``, or
``field`` with ``colors``; the default is transparent. A field value 0..1
indexes the palette. ``mask > 0`` keeps a pixel; ``alpha`` multiplies opacity
by a value clamped to 0..1. See :doc:`colors` for interpolation spaces.

Geometry and filters
--------------------

Call these methods on a ``Texture``:

.. code-block:: text

   texture.size(): List<Number, 2>
   texture.pixel(at: List<Number, 2>): Color
   texture.crop(min: List<Number, 2>, size: List<Number, 2>): Texture
   texture.resize(size: List<Number, 2>, filter: "nearest" | "triangle" | "catmull-rom" | "gaussian" | "lanczos3" = "lanczos3"): Texture
   texture.flip(axes: "x" | "y" | "xy" = "x"): Texture
   texture.rotate(angle: Angle = 90deg, expand: Bool = true, filter: "nearest" | "triangle" = "triangle", fill: Color = Color("transparent")): Texture
   texture.pad(size: List<Number, 2>, at?: List<Number, 2>, fill: Color = Color("transparent")): Texture
   texture.tile(size: List<Number, 2>): Texture
   texture.blur(radius: Number = 2): Texture
   texture.sharpen(radius: Number = 2, amount: Number = 1, threshold: Number = 0): Texture

Coordinates are pixels. ``crop``/``pixel`` require nonnegative integers;
``pad`` centers by default. Rotation is clockwise. Filtering handles alpha in
linear light.

Color and composition
---------------------

.. code-block:: text

   texture.grayscale(): Texture
   texture.invert(): Texture
   texture.brightness(factor: Number = 1): Texture
   texture.contrast(factor: Number = 1): Texture
   texture.saturation(factor: Number = 1): Texture
   texture.hue(angle: Angle): Texture
   texture.blend(other: Texture, amount: Number = 0.5, space: "oklab" | "oklch" | "linear-rgb" | "srgb" = "oklab"): Texture
   texture.composite(over: Texture, at: List<Number, 2> = [0, 0], mode: String = "normal", opacity: Number = 1): Texture
   texture.mask(mask: Texture, channel: "alpha" | "luma" = "alpha"): Texture
   texture.with_alpha(alpha: Number | Texture, channel: "alpha" | "luma" = "alpha"): Texture
   texture.cutout(threshold: Number = 0.5): Texture
   texture.draw(content: Element): Texture

Factors are nonnegative; 1 preserves the original. ``blend`` requires equal
sizes and uses the same spaces as ``Color.mix``. ``composite`` places an overlay
at a signed pixel offset. ``mask`` multiplies alpha; ``with_alpha`` replaces it;
``cutout`` makes alpha binary for voxel foliage. ``draw`` bakes a 2D snapshot
using Skia; SVG inputs are rejected.

All operations return immutable values and accept signals. Pixels are shared
between ``Image`` and ``VoxelMaterial``; voxel texture faces must have equal
sizes and binary alpha. Export through the CLI/editor; Sanim has no save API.
