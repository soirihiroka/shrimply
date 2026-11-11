Development
===========

Requirements
------------

A source build needs about 70 GB free. The setup targets Fedora:

.. code-block:: console

   $ make deps-fedora
   $ make dev

The Qt launcher is ``make dev-qt``. The dev log is ``target/shrimply-dev.log``.

The build downloads Slang. You need ``curl``, ``tar``, and ``sha256sum``.
To use a copy you already have, set ``SLANG_LIBRARY_DIR`` and
``SLANG_INCLUDE_DIR``.

CUDA Toolkit 12.9. GTX 900 through RTX 50 should work; that range is not
fully tested. The Linux GTK binary also includes an Intel backend for preview
rendering and video playback. Building the shared binary still requires the
CUDA toolkit; running the Intel backend does not require an NVIDIA driver.

macOS uses the same Makefile targets. ``make check`` there skips the Python
and documentation checks. Other platforms are unsupported.

Nix development environment
---------------------------

The flake provides Rust, the CUDA toolkit, Slang, and the native dependencies
for the GTK and Qt apps on ``x86_64-linux``.

.. code-block:: console

   $ git submodule update --init --recursive
   $ nix develop --accept-flake-config
   $ make dev

The flake uses the `NixOS CUDA binary cache
<https://wiki.nixos.org/wiki/CUDA#Setting_up_CUDA_Binary_Cache>`__. A multi-user
Nix install may need that cache in the system Nix configuration before the
daemon will trust it.

Build the GTK app:

.. code-block:: console

   $ nix build --accept-flake-config
   $ ./result/bin/shrimply

The NVIDIA backend needs an NVIDIA driver. This package ships CUDA kernels for
compute capability 8.6 (GeForce RTX 30-series).

.. _development-intel-preview:

Intel GPU preview on Linux GTK
------------------------------

Install Mesa's Intel Vulkan and OpenGL drivers and Intel's VA-API media driver.
FFmpeg must include Vulkan, VA-API, and DRM support. GTK's OpenGL context must
use the same Intel GPU as the Vulkan renderer.
Use FFmpeg 8 or newer for its Vulkan scalar-layout support. The Flatpak build
compiles FFmpeg 9.0.2 from source with Vulkan, VA-API, DRM, NVDEC, and NVENC.

The GTK binary automatically keeps CUDA on machines with an available NVIDIA
device and selects Intel otherwise. Choose a specific device in Preferences,
Performance, Acceleration device, then restart the editor. Startup applies
native PRIME settings before GTK creates its OpenGL context. Override the saved
choice with ``SHRIMPLY_GPU=intel`` or ``SHRIMPLY_GPU=cuda``.
``SHRIMPLY_INTEL_DEVICE=/dev/dri/renderD128`` selects a particular Intel DRM
render node when needed.

Decoded NV12/P010 video surfaces are imported through DMA-BUF and processed
with the shared Slang shaders. Source full/limited range and BT.601/BT.709
matrix metadata are respected. Other color matrices fail explicitly; the
compositor currently processes eight-bit RGBA even for Main10 input/output.
An unchanged, full-canvas BT.709 limited-range video can pass its native VA-API
surface directly to the encoder when the pixel format matches. This avoids
RGBA conversion and preserves native ten-bit samples for compatible Main10
exports. Edited layers and other color formats use the shared compositor.
GTK imports the Vulkan output texture directly
into OpenGL. Playback does not download pixels to the CPU; copying or saving
a preview image explicitly requests a readback.

For NVIDIA preview, the OpenGL texture is registered with the CUDA context
matching GTK's display GPU. When the selected rendering GPU differs, CUDA
transfers the frame between the two contexts. Direct peer transfer is used
where supported; the driver may stage through host memory otherwise. Shrimply
does not process or read back those pixels on the CPU.

The Intel backend supports video, images, generated vector graphics,
backgrounds, groups, layered images, and the shared raster effects. H.264,
H.265, and H.265 Main10 export use VA-API with GPU color conversion and
DMA-BUF surface writes. Intel export offers constant QP, CBR, and VBR;
the GTK dialog displays the controls supported by the selected backend.
GIF export uses its CPU encoder and explicitly reads back RGBA pixels. Manim,
Gaussian and OBJ rendering, and raster morph estimation use the shared
GPU implementations. Intel's 3D renderer traces rays through a software BVH
on the GPU, including on devices without hardware ray-tracing instructions.
SAM2, stabilization, cache baking, and transparent fill use the same analysis
and cache workflows as CUDA. SAM2 requires a configured analysis server;
its acceleration depends on that server's device.

Check the Intel crates and measure the decode/composite/presentation-export
pipeline on the target machine:

.. code-block:: console

   $ make intel-check
   $ make intel-benchmark VIDEO=/path/to/video.mp4 FRAMES=120
   $ make intel-gl-probe
   $ SHRIMPLY_GPU=intel make gpu-playback-benchmark PROJECT=/path/to/project.shrimp FRAMES=120
   $ SHRIMPLY_GPU=intel make gpu-export-benchmark VIDEO=/path/to/video.mp4 OUTPUT=/tmp/intel.mp4 FORMAT=hevc10
   $ SHRIMPLY_GPU=intel make gpu-export-benchmark PROJECT=/path/to/project.shrimp OUTPUT=/tmp/project.mp4 FORMAT=h264

Set ``RATE_CONTROL=cbr`` or ``RATE_CONTROL=vbr`` on the export benchmark to
exercise bitrate-controlled encoding. The default is ``RATE_CONTROL=cqp``.

``make gtk-gpu-preferences-validation`` exercises the Performance ComboRow on
a hybrid Intel/NVIDIA machine, restarts for every device selection, and checks
the actual GTK preview framebuffer after native GPU texture sharing. Set
``XDG_CONFIG_HOME`` to a new, empty directory so validation cannot overwrite
existing preferences. Only the final pixel verification reads back to the CPU.
``make cuda-peer-copy-validation SOURCE_GPU=0 DESTINATION_GPU=1`` checks native
CUDA transfers, including managed allocations and padded image rows. Omit the
ordinals for a single-GPU mechanics check; cross-GPU validation needs two
CUDA-visible NVIDIA devices.

Ray-tracing denoising
~~~~~~~~~~~~~~~~~~~~~~~~

The material inspector lists GPU denoisers supported by the selected backend:
OptiX and Open Image Denoise on CUDA, Open Image Denoise and the basic a-trous
fallback on Intel Vulkan, and Metal SVGF on macOS. Auto selects OptiX on CUDA,
Metal SVGF on Metal, and Open Image Denoise on supported Intel GPUs, falling
back to a-trous otherwise. Off disables filtering. Saved boolean settings
remain compatible: true selects Auto and false selects Off.

Open Image Denoise uses the existing CUDA stream or imports Vulkan allocations
by file descriptor on the same physical GPU. It never selects a CPU device.
The runtime is bundled with Linux packages; Intel's Level Zero GPU driver
must also be installed for the SYCL backend. Interactive rendering uses FAST
quality and final rendering uses HIGH. Use ``make oidn-gpu-validation`` to
check native CUDA interop and measure filtering on the first NVIDIA GPU.

The Intel Vulkan fallback uses a basic spatial edge-avoiding a-trous denoiser.
Three compute passes filter HDR ray-traced color at increasing tap spacing,
using depth, normal, color, and alpha to limit edge bleeding. Background and
coverage stay exact, and tone mapping follows filtering. All image buffers
stay on the GPU. This filter has no temporal history or neural inference;
NVIDIA keeps its existing GPU OptiX denoiser as the default. There is no CPU
denoising fallback.

The implementation follows the algorithm in
`Dammertz et al., HPG 2010 <https://doi.org/10.2312/EGGH/HPG10/067-075>`_.
Other open-source GPU implementations include
`Falcor's SVGF pass <https://github.com/NVIDIAGameWorks/Falcor/tree/master/Source/RenderPasses/SVGFPass>`_.

The open-source alternatives evaluated for Intel are:

* `Open Image Denoise <https://www.openimagedenoise.org/>`_: supports GPU beauty,
  albedo, and normal inputs, but its Intel GPU backend requires Xe or newer.
  The validation machine's UHD 630 is unsupported.
* `NRD <https://github.com/NVIDIA-RTX/NRD>`_: supports Vulkan and is a potential
  cross-vendor solution. Integration needs motion vectors, view-space depth,
  normal/roughness guides, and appropriately separated lighting signals. The
  current renderer does not produce the complete inputs or temporal history.
* `FidelityFX Denoiser <https://gpuopen.com/fidelityfx-denoiser/>`_: Vulkan
  implementations target individual shadow masks and reflections, rather than
  the renderer's combined beauty output.

Additional denoiser backends require GPU support and hardware validation;
CPU implementations are not used as a fallback.

Use ``PROFILE=release`` with either benchmark to measure optimized performance.
The default development profile leaves Rust code unoptimized.
The playback benchmark reports cold and warm passes separately. Its default
``ACCURACY=playback`` uses normal continuous playback, which can reuse the latest
decoded frame. ``ACCURACY=accurate`` waits for each requested source frame and
uses full content quality. These render-throughput measurements exclude GTK's
display refresh limit; use the export benchmark when comparing transcoding.
Native project exports use the full timeline duration. ``FRAMES`` limits the
synthetic single-video benchmark. Compare both unchanged video and a project
with visible edits when measuring export performance: the former can reuse
decoder surfaces, while the latter exercises GPU rendering and conversion.

Build and check
---------------

.. code-block:: console

   $ make dev
   $ make build
   $ make check
   $ make test
   $ make release

``make dev-qt`` builds and launches the Qt app. ``make qt-build`` builds it
without launching. The binary is ``target/debug/shrimply-qt``.

Build this site with ``make docs``.

Linux audio uses native PipeWire, with direct PulseAudio as a fallback. It does
not fall back to ALSA. If neither audio server is available, playback reports an
error. The audio startup log includes the selected backend.

Docker [Experimental]
---------------------

.. code-block:: console

   $ git submodule update --init --recursive
   $ docker buildx build --target export --output type=local,dest=dist/stage .

Contributing
------------

Read the `contribution terms
<https://github.com/soirihiroka/shrimply/blob/main/CONTRIBUTING.md>`__ before
submitting a change.
