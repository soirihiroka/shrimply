Images and Textures
===================

Use ``Image`` to display bitmap files, SVGs, or generated ``Texture`` values on
the 2D canvas. Use ``Texture`` when you need to generate, filter, composite, or
draw into an immutable RGBA8 pixel buffer (shared by 2D ``Image`` and 3D
``VoxelMaterial``). Sanim has no texture-save or filesystem-write operation;
outputs are rendered through the CLI or editor.

Displaying images and SVGs
--------------------------

``Image`` accepts local paths, HTTP/HTTPS URLs, or ``Texture`` values. Format
detection inspects file contents (not just extensions):

.. code-block:: sanim

   let logo = <Image src="https://example.org/logo.svg" width={120pt}/>
   let photo = <Image src="./photo.png" width={300pt}/>
   Animation(children=[logo, photo])
   wait(1s)

* Intrinsic size follows 96 pixels per inch (``1px == 0.75pt``). Providing one
  dimension preserves aspect ratio; providing both sets an explicit size.
* Local files are watched for hot reload. Remote URLs use Shrimply's bounded
  HTTP cache (30-second timeout, ``Cache-Control`` / ``ETag`` revalidation) and
  pin the observed bytes for the prepared revision.
* SVGs replay native vectors directly and are self-contained (external linked
  URLs inside an SVG are not fetched).

Creating and reading textures
-----------------------------

.. code-block:: sanim

   let canvas = Texture(size=[256, 256])
   let solid = Texture(size=[32, 32], fill=Color("blue3"))
   let exact = Texture(size=[2, 1], pixels=[Color("red"), Color("#0000ff80")])
   let photo = read<Texture>("photo.jpg")
   let image = <Image src={photo} width={400pt}/>
   let material = VoxelMaterial(texture=solid)

* The default fill is transparent. Explicit ``pixels`` are row-major and must
  contain ``width * height`` ``Color`` values.
* ``read<Texture>`` decodes PNG and JPEG pixels for processing (animated PNG is
  rejected). Use ``Image(src="path.png")`` when you only need to display an
  image without pixel editing.
* Dimensions are positive integers up to 4096 per axis, subject to host byte
  and pixel budgets.

Procedural generation
---------------------

.. code-block:: sanim

   let grain = Noise(seed=42, axes="xy", scale=4, octaves=3).remap(-1, 1, 0, 1)
   let texture = Texture(
       size=[64, 64], field=grain,
       colors=[Color("#514635"), Color("#A78A56")], space="oklab",
   )
   let leaf = Texture(
       size=[16, 16], field=grain,
       colors=[Color("#264E36"), Color("#71904C")],
       mask=Noise(seed=9, axes="xy", scale=1).add(0.4),
   )
   let translucent = Texture(
       size=[64, 64], fill=Color("white"),
       alpha=Density(axis="x").remap(0, 64, 0, 1),
   )

Values ``0..1`` index evenly spaced color-ramp stops (clamping outside
``0..1``). The default interpolation space is ``"oklab"`` (``"oklch"``,
``"linear-rgb"``, and ``"srgb"`` are also supported). Positive ``mask`` values
keep a texel; zero or negative values make it transparent. ``alpha`` multiplies
alpha by a scalar field clamped to ``0..1``.

Use ``pixels``, ``fill``, or ``field`` with ``colors``. ``Noise`` and
``Density`` fields compose with arithmetic, ``floor()``, ``fract()``,
``mod()``, ``sin()``, ``cos()``, ``smoothstep()``, domain scaling, and warping.

Editing
-------

.. code-block:: sanim

   let crop = photo.crop(min=[10, 20], size=[160, 90])
   let small = crop.resize(size=[80, 45], filter="lanczos3")
   let mirrored = small.flip(axes="x")
   let turned = mirrored.rotate(angle=30deg, expand=true, fill=Color("transparent"))
   let padded = turned.pad(size=[128, 128], fill=Color("dark3"))
   let repeated = texture.tile(size=[256, 256])
   let softened = photo.blur(radius=2)
   let crisp = softened.sharpen(radius=2, amount=1, threshold=3)
   let adjusted = crisp.brightness(factor=1.15).contrast(factor=1.05).saturation(factor=0.8)
   let gray = adjusted.grayscale()
   let negative = adjusted.invert()
   let shifted = adjusted.hue(angle=20deg)
   let dimensions = adjusted.size()
   let swatch = adjusted.pixel(at=[0, 0])

* ``crop`` and ``pixel`` require nonnegative integer coordinates. ``pad``
  centers its source unless ``at=[x, y]`` is given.
* Resize filters: ``"nearest"``, ``"triangle"``, ``"catmull-rom"``,
  ``"gaussian"``, and ``"lanczos3"`` (default).
* Rotation is clockwise (default ``90deg``) with ``"nearest"`` or
  ``"triangle"`` sampling; ``expand=false`` keeps the original canvas.
* Filtering premultiplies alpha in linear light to avoid dark fringes.
  ``brightness``, ``contrast``, and ``saturation`` accept nonnegative factors
  (``1`` preserves the original).

Compositing and alpha
---------------------

.. code-block:: sanim

   let mixed = photo.blend(other=another, amount=0.35, space="oklab")
   let assembled = canvas.composite(over=small, at=[-4, 12], mode="normal", opacity=0.8)
   let faded = photo.mask(mask=alpha_texture, channel="alpha")
   let replaced = photo.with_alpha(alpha=alpha_texture, channel="luma")
   let half = photo.with_alpha(alpha=0.5)
   let foliage = leaf.blur(radius=0.5).cutout(threshold=0.5)

``blend`` mixes equal-sized textures; ``composite`` supports standard blend
modes (``normal``, ``multiply``, ``screen``, ``overlay``, ``difference``).
``mask`` multiplies existing alpha; ``with_alpha`` replaces it. ``cutout``
thresholds alpha to 0 or 255 for voxel foliage (see :doc:`voxels`).

Drawing labels and vector elements
----------------------------------

``texture.draw(content)`` composites a 2D element snapshot (nested ``Box``,
``Label``, bitmap ``Image``, ``CodeView``) onto a texture at 96 DPI:

.. code-block:: sanim

   let lettering = Texture(size=[256, 128]).draw(
       <Box w-full h-full x_align={Align.Center} y_align={Align.Center}>
           <Label font_size={22pt} fill="light1">Cedar Falls</Label>
       </Box>
   )
   let decal = Texture(size=[256, 128], fill=Color("brown3")).composite(lettering)
   let preview = <Image src={decal}/>
   let material = VoxelMaterial(texture=decal)

``draw`` does not bake separate animation timelines and rejects SVG inputs.
Bake static overlays once and composite them with changing backgrounds. All
faces in a voxel texture array must share matching dimensions (:doc:`voxels`).

Animated textures
-----------------

Texture constructor and method arguments accept signals; unchanged inputs reuse
cached outputs automatically across frames:

.. code-block:: sanim

   mut tint = Color("blue3")
   sig image_pixels = Texture(size=[256, 256], fill=tint).blur(radius=2)
   let image = <Image src={image_pixels} width={240pt}/>
   image.fade_in()
   wait(10s)
