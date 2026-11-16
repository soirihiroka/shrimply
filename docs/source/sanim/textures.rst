Textures
========

``Texture`` is an immutable, shared straight-RGBA8 pixel value. The same value
can be used by a 2D ``Image`` and by a ``VoxelMaterial``. There is one constructor,
one watched import path, and one pixel store. Texture operations return new
values while preserving their input. Sanim has no texture save or filesystem
write operation; outputs are rendered through the CLI or editor.

Creating and reading
--------------------

.. code-block:: text

   let canvas = Texture(size=[256,256])
   let solid = Texture(size=[32,32],fill=Color("blue3"))
   let exact = Texture(size=[2,1],pixels=[Color("red"),Color("#0000ff80")])
   let photo = read<Texture>("photo.jpg")
   let image = <Image src={photo} width={400pt}/>
   let material = VoxelMaterial(texture=solid)

The default fill is transparent. Explicit pixels are row-major and must contain
exactly ``width*height`` Color values. PNG and JPEG imports use the existing
source, asset cache, dependency tracking, and remote URL policy. ``read<Texture>``
decodes pixels for processing; ``Image(src="path.png")`` retains the ordinary
lazy media path. Animated PNG is rejected rather than silently losing frames.
Dimensions are positive integers, bounded to 4096 in each axis. Byte, pixel,
work and cache budgets may reject a smaller image before allocation.

Procedural generation
---------------------

.. code-block:: text

   let grain = Noise(seed=42,axes="xy",scale=4,octaves=3).remap(-1,1,0,1)
   let texture = Texture(size=[64,64],field=grain,
       colors=[Color("#514635"),Color("#A78A56")],space="oklab")
   let leaf = Texture(size=[16,16],field=grain,
       colors=[Color("#264E36"),Color("#71904C")],
       mask=Noise(seed=9,axes="xy",scale=1).add(0.4))
   let translucent = Texture(size=[64,64],fill=Color("white"),
       alpha=Density(axis="x").remap(0,64,0,1))

Generation runs natively over texel centers ``[x+0.5,y+0.5,0]``. Values 0..1
select evenly spaced color-ramp stops; values outside that range clamp to the
end colors. A one-color ramp is valid. The default interpolation space is
OKLab; ``"oklch"``, ``"linear-rgb"``, and ``"srgb"`` are also available. Palette
conversion is prepared once. Alpha-aware color mixing uses the shared color
library. A positive ``mask`` includes a texel; zero or negative values make it
transparent and skip unnecessary color sampling. ``alpha`` multiplies palette
or fill alpha by a scalar field clamped to 0..1.

Use either ``pixels``, ``fill``, or ``field`` with ``colors``. A mask and alpha
field may accompany fill or field generation. Native ``Noise`` and ``Density``
patterns compose with arithmetic, ``floor()``, ``fract()``, ``mod()``,
``sin()``, ``cos()``, ``smoothstep()``, domain scaling and warping. Pixel lists
remain available for arbitrary bounded Sanim algorithms. Seeds and stateless
``Random`` indexing make repeated generation reproducible.

Editing
-------

.. code-block:: text

   let crop = photo.crop(min=[10,20],size=[160,90])
   let small = crop.resize(size=[80,45],filter="lanczos3")
   let mirrored = small.flip(axes="x")
   let turned = mirrored.rotate(angle=30deg,expand=true,fill=Color("transparent"))
   let padded = turned.pad(size=[128,128],fill=Color("dark3"))
   let repeated = texture.tile(size=[256,256])
   let softened = photo.blur(radius=2)
   let crisp = softened.sharpen(radius=2,amount=1,threshold=3)
   let adjusted = crisp.brightness(factor=1.15).contrast(factor=1.05).saturation(factor=0.8)
   let gray = adjusted.grayscale()
   let negative = adjusted.invert()
   let shifted = adjusted.hue(angle=20deg)
   let dimensions = adjusted.size()
   let swatch = adjusted.pixel(at=[0,0])

``crop`` and ``pixel`` use strict, nonnegative integer coordinates. ``pad``
centers its source unless ``at=[x,y]`` is supplied. Signed pad and composite
offsets clip pixels outside the target. Padding copies original pixels,
including their alpha. ``tile`` repeats the original pixel pattern.

Resize filters are ``"nearest"``, ``"triangle"``, ``"catmull-rom"``,
``"gaussian"``, and ``"lanczos3"`` (default). Rotation is clockwise, defaults
to 90 degrees, and supports nearest or triangle sampling. ``expand=false`` keeps
the original canvas. Right-angle turns use the exact pixel permutation.

Filtering and resampling premultiply alpha in linear light, then restore
straight sRGBA. Transparent texels do not create dark or colored fringes.
``blur`` uses the image library's three-box Gaussian approximation, with bounded
work independent of an unbounded kernel allocation. ``sharpen`` applies an
unsharp mask; threshold uses byte-channel units 0..255. Color-adjustment factors
are nonnegative: 1 preserves the original; brightness, contrast and saturation
operate in linear RGB, with contrast centered on alpha-weighted mean luminance.
Hue rotation uses the shared HSV conversion.

Compositing and alpha
---------------------

.. code-block:: text

   let mixed = photo.blend(other=another,amount=0.35,space="oklab")
   let assembled = canvas.composite(over=small,at=[-4,12],mode="normal",opacity=0.8)
   let faded = photo.mask(mask=alpha_texture,channel="alpha")
   let replaced = photo.with_alpha(alpha=alpha_texture,channel="luma")
   let half = photo.with_alpha(alpha=0.5)
   let foliage = leaf.blur(radius=0.5).cutout(threshold=0.5)

``blend`` mixes equal-sized images using shared premultiplied color ramps.
``composite`` uses source-over compositing in linear light and the shared layer
blend modes, including normal, multiply, screen, overlay and difference.
``mask`` multiplies existing alpha; ``with_alpha`` replaces it. A mask texture
must have the same size and can supply its alpha or linear luminance channel.
``cutout`` thresholds alpha to 0 or 255 for voxel foliage. Voxel material
construction is the boundary that rejects continuous coverage; 2D images retain
smooth RGBA alpha.

Signals, caching and budgets
----------------------------

.. code-block:: text

   mut tint = Color("blue3")
   sig image_pixels = Texture(size=[256,256],fill=tint).blur(radius=2)
   let image = <Image src={image_pixels} width={240pt}/>
   image.fade_in()
   wait(10s)

Constructors and transforms use one core operation registry for preparation,
type checking and retained playback. Arguments may be signals. Unchanged input
values reuse the same immutable output and revision across frames through the
common byte/entry-bounded resource cache. Keys use existing persistent content
proofs and texture revisions rather than rescanning pixel data. The cache holds
outputs, not source ASTs, input histories, editor objects or GPU resources.

Output buffers, temporary filtering planes, palette scratch and Vec-to-Arc
publication overlap are preflighted before allocation. Retained sampling also
bounds aggregate native work and retained texture bytes. Cache hits still obey
host output-byte limits. All sampling and edits remain deterministic; eviction
may require recomputation but does not change results.

Drawing labels and vector elements
----------------------------------------

``texture.draw(content)`` returns a new texture with a 2D element composited over
its pixels. It uses the ordinary Sanim Skia renderer, layout and font handling;
there is no second text or shape renderer, intermediate PNG, or GPU readback.
The standard CLI/editor build includes this native drawing backend. A minimal
headless language build reports a clear unsupported-feature error unless its
host installs a drawing backend.

.. code-block:: text

   let lettering = Texture(size=[256,128]).draw(
       <Box w-full h-full x_align={Align.Center} y_align={Align.Center}>
           <Label font_size={22pt} fill="light1">Cedar Falls</Label>
       </Box>
   )
   let decal = Texture(size=[256,128],fill=Color("brown3")).composite(lettering)
   let preview = <Image src={decal}/>
   let material = VoxelMaterial(texture=decal)

Draw accepts an immutable element snapshot, including nested Boxes, Labels,
bitmap Images and CodeViews; its separate animation timeline is not baked. SVG
inputs are rejected because their embedded resources cannot be bounded by the
texture decoder budget. Use the
existing element layout and paint APIs for vector paths, borders and gradients.
Bake static lettering or overlays once, then composite them with changing
backgrounds. Family-font changes invalidate both preparation and raster memos.
Unknown font-environment revisions disable that raster memo conservatively.

The texture viewport is its pixel size at the standard 96 DPI. RGBA compositing
preserves continuous alpha for 2D; an opaque sign background also makes the result
suitable for voxel materials. All faces in a voxel texture array must have
matching dimensions, as described in :doc:`voxels`.
