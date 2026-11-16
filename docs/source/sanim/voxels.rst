Voxel scenes
============

Sanim's experimental voxel renderer is independent of the Skia 2D renderer. It
uses wgpu and embedded WGSL shaders; no external shader compiler is required.
Voxel scenes use the existing language, immutable values, signals, reflection,
animation plans, source watches, and exact timeline sampler.

Constructing blocks
-------------------

``VoxelGrid`` stores sparse, immutable 8×8×8 bricks. Empty space has no brick
allocation. Uniform bricks use one material value. Editing a grid returns a new
grid sharing unchanged storage; previous snapshots remain valid.

.. code-block:: text

   let grass = VoxelMaterial(color=Color("green3"))
   let ground = VoxelGrid(size=[32,8,32]).fill(
       min=[0,0,0], max=[100%,25%,100%], material=grass
   )
   let scene = VoxelScene(volume=ground)
   scene.fade_in()
   wait(2s)

Coordinates use voxel cells. Bounds are minimum-inclusive and maximum-exclusive.
The axes are +X right, +Y up, and +Z toward the front camera. ``fill``, ``set``,
``erase``, and ``get`` validate coordinates. Cells accept a ``Color`` directly or
a shared ``VoxelMaterial``. Partial transparency is unsupported; textures may
contain opaque or fully transparent cutout texels.

Three-dimensional layout
------------------------

``VoxelBox`` is a layout container, not a rendered cuboid. ``VoxelPart`` places a
voxel grid inside it. Children can flow along any of the three axes. Containers
support intrinsic sizing, definite cell sizes, parent-relative percentages,
gaps, symmetric per-axis padding, cross-axis alignment, and space distribution.
This layout is independent of camera orientation and output pixel dimensions.

.. code-block:: text

   let layout = <VoxelBox
       size={[80,40,60]} flow="x" gap={4} padding={[2,1,2]}
       justify="between" align={["start","start","center"]}
   >
       <VoxelPart volume={house}/>
       <VoxelBox flow="z" gap={3}>
           <VoxelPart volume={farm}/>
           <VoxelPart volume={tower}/>
       </VoxelBox>
   </VoxelBox>
   let scene = VoxelScene(layout=layout)
   scene.fade_in()

``size`` has three components, each a cell count, percentage, or ``"auto"``.
Animated sizes, gaps, and padding are floored to whole cells at each sampled
frame. Grid dimensions and cell indices themselves remain strictly integral.
Percentages use the parent's inner extent after padding. An automatic parent
cannot derive its size from a child that needs that same parent's percentage
size: give that axis a definite size. Layout overflow is an error.

``justify`` supports ``"start"``, ``"center"``, ``"end"``, ``"between"``,
``"around"``, and ``"evenly"``. ``align`` gives independent X/Y/Z alignment;
flow-axis placement comes from ``justify``. Padding is either one cell count or
three symmetric per-axis counts.

Absolute children do not consume flow space. Their ``at`` coordinates reference
the parent, while ``anchor`` references the child. The final origin is floored
once to the voxel lattice. The child must fit completely inside the container.
Numeric and percentage anchors and offsets retain their exact fractional
values until that final snap, including alignment and space distribution.

.. code-block:: text

   <VoxelPart volume={house} position_mode="absolute"
       at={[50%,0%,50%]} anchor={[50%,0%,50%]} offset={[0,4,0]}/>

Layout configuration accepts signals. Camera-only animation preserves the
resolved voxel data; changing layout recomputes placement. ``VoxelScene`` accepts
exactly one of ``volume`` and ``layout``. Use its camera properties rather than
2D position, rotation, or scale.

Placement is planned before rendering. Native layout scenes retain a shared asset
plus an integer origin for each instance; they do not flatten or copy all cells.
A bounded spatial index resolves overlaps in authored order, and cutout holes
continue to the next instance. Camera and lighting changes retain the same scene
source. Placement changes update metadata and changed page-table entries while
reusing uploaded asset bricks. Cache entries do not retain revision history.

Explicit grid composition with ``grid.place_many(...)`` produces a new immutable
grid when callers need voxel edits on the combined result. This is distinct from
``Instances(...)``, which retains reusable assets for rendering.

Geometry and framing assertions
-------------------------------

Assertions are pure and fail with a source location, layout paths, offending
cell or projected corner, and a suggested repair.

.. code-block:: text

   let house_part = <VoxelPart volume={house}/>
   let tree_part = <VoxelPart volume={tree} absolute at={[40,0,0]}/>
   let world = <VoxelBox size={[80,40,60]}>{house_part}{tree_part}</VoxelBox>
   world.assert_disjoint(house_part, tree_part)
   world.assert_within(house_part, world)
   let scene = VoxelScene(layout=world, check_framing=true, frame_margin=2%)
   scene.assert_framed(margin=2%)
   scene.fade_in()

``assert_disjoint`` tests occupied cells after a bounding-box broad phase, so a
part inside an empty room is valid. ``assert_within`` checks allocated bounds.
A selector matches every occurrence of the same immutable part recipe; use
distinct placement recipes when occurrences need separate checks.

``assert_framed`` checks the constructor's camera snapshot against the external
output viewport. ``check_framing=true`` additionally enforces the contract at
every sampled camera during voxel export and inspection. Margin is measured
as a percentage of each image dimension. This catches animated cropping instead
of assuming that a successful initial snapshot proves the whole animation.

The static linter reports ``V101`` for provably repeated absolute part recipes
in one container, without loading assets or evaluating the scene. Voxel parts
merge into one cell field, so overlapping parts overwrite cells instead of
creating depth-buffer z-fighting. Use explicit disjointness assertions for
broader intersections that static syntax cannot prove.

Block texture packs
-------------------

Voxel materials use the same :doc:`Texture <textures>` values as ordinary 2D
images. Textures can be authored entirely in Sanim:

.. code-block:: text

   let grain = Noise(seed=42, axes="xy", scale=3, octaves=3).remap(-1,1,0,1)
   let texture = Texture(size=[16,16], field=grain,
       colors=[Color("#514635"),Color("#A78A56")])
   let leaves = Texture(size=[16,16], field=grain,
       colors=[Color("#264E36"),Color("#71904C")],
       mask=Noise(seed=9,axes="xy",scale=1).add(0.4))
   let exact = Texture(size=[2,1], pixels=[Color("red"),Color("transparent")])
   let wood = VoxelMaterial(texture=texture)

``Texture`` supports continuous RGBA for 2D images. Voxel surfaces require
opaque or binary cutout alpha; material construction reports the offending
face and texel if partial alpha remains. Use ``texture.cutout(threshold=0.5)``
after filtering foliage, or material ``transmission`` for physical glass/water.

Read PNG textures through the ordinary watched asset API. Materials can use one
texture, separate top/side/bottom textures, or individual left/right/front/back
textures. This supports the directory structure used by Minecraft resource
packs without depending on Minecraft or its assets.

.. code-block:: text

   let grass = VoxelMaterial(
       top=read<Texture>("assets/minecraft/textures/block/grass_block_top.png"),
       side=read<Texture>("assets/minecraft/textures/block/grass_block_side.png"),
       bottom=read<Texture>("assets/minecraft/textures/block/dirt.png")
   )

Textures use nearest sampling. A material's color tints its texture; the default
is white. ``emission`` adds emitted light in path-traced mode. A changed PNG is
a source dependency and triggers the existing hot-reload machinery. The first
renderer version requires a consistent tile size within a scene.

Inspection and rendering
------------------------

.. code-block:: console

   sanim voxel inspect base.sanim --time 2s --json
   sanim voxel inspect base.sanim --time 2s --views front,right,top,isometric --output /tmp/base-views
   sanim voxel pick base.sanim --time 2s --view front --width 640 --height 360 --pixel 320,180 --json
   sanim voxel render base.sanim --time 2s --mode flat --output /tmp/base.png
   sanim voxel video base.sanim --mode path-traced --samples 32 --bounces 4 --output /tmp/base.mp4
   sanim voxel watch base.sanim --time 2s --mode flat --output /tmp/base-preview.png

Named views are front, back, left, right, top, bottom, and isometric. Inspection
without an output directory reports metadata without initializing a GPU. With
an output directory it renders the requested views and writes ``scene.json``.
All views use one sampled animation state. Reports include cameras, layout
boxes, voxel dimensions, occupied cells, resident bytes, and source location.
Structural inspection shows the sampled blocks at full opacity, so an entry
fade does not hide them; the report preserves the actual presentation opacity.
Picking reports the hit voxel, face normal, material, and spatial distance; it
uses the same camera convention and cutout rules as rendering.

Flat mode is the default structural preview. Path-traced mode uses bounded
Monte Carlo diffuse bounces, sunlight, environment lighting, and emissive
materials. Samples and bounces control its cost; it is not a different scene
representation. Both modes retain unchanged voxel buffers and textures.
Watch mode reuses the existing native event watcher and keeps its GPU renderer
alive across edits. Failed edits preserve the last published PNG.

Procedural fields and reusable assets
-------------------------------------

``Noise`` and ``Density`` build immutable scalar fields. Composing a field does
not generate a world or execute a Sanim callback for every cell. Native bulk
operations evaluate one shared, bounded field program and publish an immutable
grid. Reusing a subexpression evaluates it once per sample.

.. code-block:: text

   let hills = (Noise(seed=42, kind="fbm", scale=24, octaves=4, axes="xz")
       .multiply(3).add(12))
   let basin = (Distance(center=[32,0,32], axes="xz")
       .divide(24).pow(2).multiply(8).add(5))
   let heights = hills.min(basin)
   let terrain = (VoxelGrid(size=[64,32,64])
       .heightmap(heights, Color("dark3"))
       .surface(Color("green3"), depth=2))
   let tree = (VoxelGrid(size=[5,8,5])
       .fill([2,0,2], [3,5,3], Color("orange5"))
       .fill([0,4,0], [5,8,5], Color("green4")))
   let trees = Instances(
       asset=tree, positions=terrain.surface_points(step=16), size=[64,32,64]
   )
   let world = <VoxelBox size={[64,32,64]}>
       <VoxelPart volume={terrain} absolute/>
       {trees}
   </VoxelBox>
   let scene = VoxelScene(layout=world)
   scene.fade_in()
   wait(2s)

Noise kinds are ``"value"``, ``"gradient"``, ``"fbm"``, ``"ridged"``, and
``"cellular"``. Samples are continuous and lie within -1 to 1; ``scale`` is
the feature size in cells. Defaults are seed 0, fBm, scale 32, four octaves,
persistence 0.5, and lacunarity 2. Fractal modes support one to eight octaves.
``axes`` accepts ``"xy"``, ``"xz"``, ``"yz"``, or ``"xyz"`` (the default).
Planar noise uses a two-dimensional kernel and rotates fractal octaves within
that plane. Use ``axes="xz"`` for height maps and climate masks: changing Y
then cannot change the sampled value. Value, gradient, and cellular modes use
one base octave; ``octaves``, ``persistence``, and ``lacunarity`` control fBm
and ridged fractals. Seeds are stateless and sample order does not affect them.
``Distance`` returns Euclidean distance from ``center`` and defaults to all
three axes; ``axes="xz"`` creates a horizontal radial mask.

``Density(value=...)`` creates a constant field. ``Density(axis="y")`` creates
a coordinate field, with optional ``scale`` and ``offset``. Fields support
``add``, ``subtract``, ``multiply``, ``divide``, ``min``, ``max``, ``pow``,
``abs``, ``negate``, ``clamp``, ``smoothstep``, and ``remap``. Binary operations accept a Number
or another field. ``translate([x,y,z])``, ``scale(number_or_three_components)``,
and ``warp(seed=..., scale=..., strength=..., axes="xyz")`` transform the sample domain.
Warp strength is displacement in cells. A planar warp uses planar noise and
leaves the excluded coordinate unchanged; for example, ``axes="xz"`` bends
a shoreline without changing height. ``smoothstep(min=0,max=1)`` maps values
to 0..1 with flat endpoint tangents and saturates outside finite ``min < max``.
It is useful for blending elevation fields and scatter probabilities:

.. code-block:: text

   let moisture = (Noise(seed=7, axes="xz", scale=48)
       .warp(seed=23, axes="xz", scale=64, strength=12)
       .smoothstep(min=-0.25, max=0.3))

``sample([x,y,z])`` queries one point without rasterizing a grid.
Periodic patterns use the same fields: ``floor()``, ``fract()``, ``sin()``,
``cos()``, and ``mod(value)``. Fract returns the nonnegative fractional part;
mod uses a nonnegative Euclidean remainder and rejects a zero divisor.
Trigonometric fields use radians. For example,
``Density(axis="x").divide(4).floor().mod(2)`` alternates four-cell stripes.

``heightmap(field, material)`` fills from Y=0 up to each clamped, floored height,
sampling X/Z at cell centers. ``density(field, material, threshold=0)`` fills
cells where the field meets the threshold, optionally within ``min``/``max``
bounds. ``carve(field, threshold=0, min=..., max=...)`` removes cells under
the same selection rule and uses that same chunk-batched rasterizer. This
supports caves, arches, and channels without editing one voxel per DSL call.
``flood(level=..., material=...)`` fills only empty cells below an
integer waterline, preserving terrain and leaving cells at or above it unchanged.
It fills enclosed empty spaces too; it does not simulate connected fluid flow.
``surface(material, depth=1, mask=...)`` recolors existing top layers without
filling caves. Its optional Density mask selects columns only when sampling
``[x+0.5, top, z+0.5]`` is positive; omitting it selects every occupied column.
For example, ``mask=Density(axis="y").subtract(17)`` keeps beds below the
17-cell waterline unpainted.
``scatter(material, density=..., seed=..., step=1)`` places one
cell immediately above selected surface columns; density accepts a Number or
field and is clamped to 0..1. This is useful for densely scattered foliage
without constructing one layout node per plant. Cross-card foliage is not a
support surface, so layered flower scatters replace a grass card in the same
cell instead of stacking plants in the air. Surface painting and point
selection share this definition of the supporting terrain surface.

``surface_points(step=4, seed=0, jitter=0, density=1, mask=..., max_slope=...)``
returns cell coordinates immediately above supporting cube columns, ignoring
cross-card foliage. It scans
one temporary raster height table and applies all selection natively, rather
than sampling terrain fields repeatedly in a DSL loop. Density accepts a
Number or field and is clamped to probability 0..1. A mask must be positive to
include a point; both fields sample the selected cell center at its top boundary.
Jitter 0 keeps the tile origin; 1 distributes points throughout each step-sized
tile, preserving unique, in-bounds columns. Seeds are independent of traversal
and mask rejection. ``max_slope`` limits local height rise per horizontal cell,
using the four adjacent raster columns, so isolated peaks and cliff edges can
be excluded. The slope test includes the X/Z gradient magnitude. Output is
bounded to 65,536 candidate points and all field/neighbor work is prepaid.
``Instances`` places one shared asset at each supplied origin
and returns an absolute layout container. ``size`` can be inferred from numeric
positions, but must be explicit for empty or percentage-based positions. Assets
must fit completely inside it. Rendered instances retain their original asset
geometry and per-asset material mapping. ``place_many`` instead explicitly
composes a grid when a materialized grid is needed.

``Random(seed=...)`` is stateless: ``rng.number(index, min=0, max=1)``,
``rng.integer(index, min=..., max=...)``, and
``rng.chance(index, probability=...)`` always produce the same indexed result.
Ranges include ``min`` and exclude ``max``. Inserting an unrelated random call
does not move the sequence. Seeds and indices are nonnegative 64-bit integers.

Fields are limited to 128 shared instructions; bulk operations validate work
budgets before expensive sampling. Native algorithms share a separate cumulative
64-million-unit preparation budget; voxel cells do not count as interpreted
DSL steps. Hosts configure ``Limits.max_native_work``; the CLI exposes
``--max-native-work``. Restoring an execution prefix also restores its native
work accounting. Profiles report executed native work as ``native_work`` and
include it in the total ``operations`` count; ``interpreted_operations`` excludes
native loops. P001 warnings use interpreted work only. P004 reports a native
call when its measured time exceeds the configured preparation warning time.
Native height plans use two temporary bytes
per X/Z column, then share uniform bricks and unchanged grid storage. Surface
point lists are limited to 65,536 entries; an ``Instances`` group has at most
4,095 children and also follows the scene's spatial-candidate budgets. Use
native scatter for dense ground cover.

``make -C sanim voxel-procedural-check`` checks these APIs, immutable snapshots,
bounded hot-edit ownership, and error paths. ``voxel-procedural-bench`` compares
native heightmap batching with repeated immutable column edits and verifies
that the resulting grids are identical.

Lighting
--------

The default lighting uses a warm key, a large sun source for soft shadows, and
a cooler sky fill. Environment illumination is independent of the visible
background. ``--sun-diameter 8deg`` controls the angular size;
``--sun-strength 1.1`` and ``--environment-strength 1`` adjust contrast without
changing materials. A zero sun diameter produces hard shadows. These are
artistic studio defaults rather than the physical angular size of Earth's sun.

Shadows and ambient occlusion are independent controls:

* ``--shadows auto|off|hard|soft`` selects key-light visibility. Automatic mode
  leaves flat previews unshadowed and enables soft shadows in path tracing.
  ``--shadow-strength`` blends visibility and ``--shadow-samples`` controls its
  ray count, bounded to eight rays per shading sample.
* ``--ao auto|off|local|ray-traced`` selects ambient occlusion. Automatic mode
  uses inexpensive neighboring-cell AO in flat previews and leaves it off in
  path tracing, whose indirect light already includes occlusion. Explicit AO
  can be used for a stylized path-traced image. ``--ao-strength``,
  ``--ao-distance`` in voxel cells, and ``--ao-samples`` control it; ray AO is
  bounded to sixteen rays per primary sample.
* ``--exposure`` adjusts foreground brightness in stops, without changing the
  visible background. Shadow and AO strengths range from zero to one.

AO affects ambient and indirect light, preserving emitted light and direct
sunlight. Settings validate the combined trace-work budget before rendering.
The Rust renderer exposes these same settings as typed, grouped structures.

.. code-block:: console

   sanim voxel render base.sanim --time 2s --mode flat --shadows soft --shadow-samples 4 --ao ray-traced --ao-samples 8 --ao-distance 6 --output /tmp/stylized-base.png

The soft-shadow sampler uses one uniformly sampled sun-cone direction per path
bounce, with the same direction for visibility and surface cosine. Increasing
the sun diameter does not add shadow rays. See the
`PBRT cone sampling derivation <https://pbr-book.org/4ed/Sampling_Algorithms/Sampling_Multidimensional_Functions#A.5.4>`_
and `Blender's light-size controls <https://docs.blender.org/manual/en/4.4/render/lights/light_object.html>`_.

Physical water, glass, and foliage
----------------------------------

Opaque cubes remain the default. Binary texture alpha cuts holes for leaves,
windows, and plants. Physical transparency uses ``transmission`` rather than
partially transparent coverage:

.. code-block:: text

   let water = VoxelMaterial(
       transmission=1, ior=1.333, roughness=0.025,
       attenuation_color=Color("#65BDCA"), attenuation_distance=12,
       wave_strength=0.055, wave_length=9, wave_speed=0.7
   )
   let glass = VoxelMaterial(transmission=1, ior=1.5, roughness=0.02)
   let grass = VoxelMaterial(texture=read<Texture>("grass.png"),geometry="cross")

Water and glass refract the actual scene and apply Fresnel reflection, total
internal reflection, and Beer absorption through the traveled material depth.
``attenuation_color`` is the transmitted color at ``attenuation_distance`` cells;
white causes no absorption. Roughness controls the microfacet distribution.
Ripples change the coherent surface normal at the sampled timeline time.
Contiguous cells of one material form a medium rather than separate glass boxes.
Direct shadow transmission uses a bounded straight-ray approximation; refractive
caustics are not simulated.

``geometry="cross"`` creates two double-sided diagonal cards within each cell.
Texture holes remain empty for visibility, picking, and shadows. Cards are thin
cutout surfaces and cannot be combined with volume transmission. Textures require
opaque or fully transparent pixels; intermediate alpha is rejected with guidance
to use transmission.

See the physical models in
`PBRT's dielectric implementation <https://github.com/mmp/pbrt-v4/blob/master/src/pbrt/bxdfs.cpp>`_
and the separation of coverage and transmission in
`KHR_materials_transmission <https://github.com/KhronosGroup/glTF/tree/main/extensions/2.0/Khronos/KHR_materials_transmission>`_.

Multiple lights and environment
-------------------------------

.. code-block:: text

   mut brightness = 200
   sig lamp = PointLight(position=[50%,12,25%],color=Color("#FFC37D"),
       intensity=brightness,radius=1.2)
   let sun = SunLight(direction=[-0.4,0.8,0.5],intensity=2,diameter=5deg)
   let sky = Environment(top=Color("#6D9DD8"),horizon=Color("#C5DDE9"),
       ground=Color("#475B43"),intensity=0.7,visible=true)
   let scene = VoxelScene(volume=world,lights=[sun,lamp],environment=sky)

Up to 32 lights may be supplied. Omitting ``lights`` preserves the default sun;
``lights=[]`` disables it. Light colors and environment intensities are linear
radiometric values after color decoding. Point intensity falls with squared
distance; radius gives a finite emitter and soft shadows. Sun ``diameter`` is an
angular diameter. A luminous voxel uses material ``emission``; place an explicit
point light at it to illuminate nearby surfaces efficiently. Give a point inside
a one-cell bulb a radius of at least 0.9 cells so samples can leave the bulb.
Opaque emissive geometry still blocks rays normally.

Light constructors and environment fields accept signals. Changing them updates
small uniforms while retaining terrain and texture allocations. Path tracing
samples a bounded light distribution per bounce. Flat previews sum every light
without random light selection; soft visibility samples are evaluated per light,
so adding a light does not introduce whole-surface brightness speckles.

Camera recipes and curves
-------------------------

.. code-block:: text

   let route = Curve(points=[[120%,60%,120%],[100%,40%,90%],[20%,30%,80%]])
   let targets = Curve(points=[[50%,25%,50%],[45%,30%,50%],[40%,25%,45%]])
   mut progress = 0
   sig shot = Camera(path=route,progress=progress,target=targets.at(progress),fov=50deg)
   let scene = VoxelScene(volume=world,camera=shot)
   scene.fade_in()
   animate(progress,to=1,duration=6s,ease="smootherstep")

Coordinates are ordinary fixed-size Lists. Numbers denote cells; percentages
resolve against each world dimension. ``Curve(points=...)`` supports 2D and 3D
Lists, linear or centripetal interpolation, and optional closed paths. Normalized
``.at(progress)`` follows distance along the geometric curve. Open paths hold their
endpoints; closed paths wrap. The spatial lookup is computed once per world size,
with two bounded slots for world-space and normalized sampling.

A camera with ``path`` follows its tangent when no target is supplied; an explicit
target can be static or driven by its own curve. ``look_ahead`` chooses a forward
framing distance. ``Camera(position=...,target=...)`` defines a direct shot;
``OrbitCamera(target=...,radius=...,azimuth=...,elevation=...)`` defines an orbit.
Camera recipes are immutable and can be retained as signals.

``.pan([right,up])`` translates the eye and target in screen space,
``.offset([x,y,z])`` translates in world space, and ``.dolly(distance)`` moves the eye
toward its target. ``.zoom(factor)`` changes lens framing, ``.roll(angle)`` rotates
the view, and ``.look_at(target)`` sets framing. ``.dolly_zoom(distance)`` compensates
the lens to retain target-plane size. Dolly motion cannot cross the target.
Camera configuration rejects conflicting legacy fitted-camera arguments such as
``camera=...`` together with ``yaw=...``.

Time-based keyframe curves control how animated values change over time; geometric
curves describe a path through space. This distinction allows independent timing,
framing, and spatial path edits without rebuilding voxel geometry.

Animation and the survival-base example
---------------------------------------

.. code-block:: text

   mut yaw = 0deg
   let scene = VoxelScene(layout=layout, yaw=yaw)
   scene.fade_in(duration=300ms)
   animate(yaw, to=180deg, duration=6s, ease="linear")

The examples generate their original textures directly in Sanim. No external
asset generator is required:

.. code-block:: console

   sanim voxel inspect sanim/examples/voxel_survival.sanim --time 1s --output /tmp/survival-views
   sanim voxel video sanim/examples/voxel_survival.sanim --width 1280 --height 720 --mode path-traced --samples 32 --bounces 4 --output /tmp/survival.mp4
   sanim voxel video sanim/examples/voxel_valley.sanim --width 1280 --height 720 --mode path-traced --samples 64 --bounces 4 --output /tmp/valley.mp4

The independent renderer currently accepts one mounted ``VoxelScene`` per
output. Mixing it with 2D layers is rejected explicitly.

Image reconstruction and sampling
---------------------------------

Ray samples are estimates. Low sample counts need image reconstruction to avoid
visible grain; increasing ray counts alone is expensive. The renderer can filter
diffuse illumination separately from the original material texture:

.. code-block:: console

   sanim voxel render scene.sanim --mode path-traced --samples 4 --reconstruction spatial --spatial-passes 3 --antialias 4 --output /tmp/scene.png
   sanim voxel video scene.sanim --reconstruction temporal --history-frames 16 --antialias 4 --shadows soft --shadow-samples 1 --output /tmp/scene.mp4

The default is ``spatial`` with two filter passes and two primary antialiasing
samples. Deterministic preview lighting skips the filter passes automatically.

``spatial`` uses depth, surface normal, and material identity to stop lighting
filters at geometry boundaries. Textured albedo and final alpha are not blurred.
Emission, the visible background, and transmission bypass diffuse filtering.
``off`` provides an unfiltered reference; antialiasing can still be requested
independently. Path tracing uses its existing path samples for primary coverage
instead of multiplying them by the antialiasing count.

``temporal`` additionally reprojects a bounded previous lighting image through
the previous camera. Surface mismatch, disocclusion, source edits, large lighting
changes, and timeline seeks reject history. Gradual lighting changes shorten the
history window, with neighborhood clipping to limit stale illumination. Use
``spatial`` for history-independent frame renders. Integrations can call
``Renderer::reset_history`` explicitly after a discontinuity.

The history length is a sample window, not a collection of retained frames.
Reconstruction targets are allocated once per image configuration and counted
in the GPU memory budget before allocation. JSON render statistics expose
``reconstruction_bytes``, ``reconstruction_passes``, ``history_frames``,
``history_reset``, and ``history_reactive``.

Texture-array mipmaps are generated once during texture upload. Minification
uses the primary ray footprint; magnification keeps pixel-art texels sharp.
Mip colors are averaged in linear light with alpha weighting, preventing dark
fringes from transparent texels. Visibility tests retain the original cutout
alpha. Palette-only edits reuse an unchanged texture atlas and its mipmaps.

These techniques use independently implemented GPU passes and standard
reconstruction principles described in
`Spatiotemporal Variance-Guided Filtering <https://research.nvidia.com/labs/rtr/publication/schied2017spatiotemporal/>`_.
Frame reconstruction stays on the GPU, without a GPU-to-CPU-to-GPU round trip.
Texture mipmaps are prepared once during upload.

Validation and performance
--------------------------

``make -C sanim voxel-check`` runs the headless model, language, inspection,
watch, and editor-completion regressions. ``voxel-gpu-check`` additionally
exercises a real GPU. ``voxel-bench`` measures retained GPU resources, camera
edits, and held layout; ``voxel-storage-bench`` measures persistent brick edits.
``voxel-valley-bench`` prepares the actual example once and retains one GPU
renderer across day/night, lighting, sample-count, and bounce-count measurements.
It verifies that those changes upload no voxel geometry. Set
``SANIM_VOXEL_PROFILE_SOURCE`` to profile another generated example, or
``SANIM_VOXEL_PROFILE_SMALL=1`` to restrict the sweep to 64×36.
``SANIM_VOXEL_PROFILE_PREVIEW=1`` instead measures flat previews at 1280×720,
with shadows disabled and with four soft-shadow samples per light.

Run GPU timings without concurrent rendering. Export benchmarks include pixel
readback; embedded GPU rendering does not require it.

The GPU checks also compare hard-shadow visibility on the generated survival
scene against independent CPU rays at five orbit positions. They report the
sampled pixels and boundary exclusions explicitly. Surface ray origins use the
exact crossed voxel plane, preventing rounding errors from placing a shadow
ray inside its own surface.
