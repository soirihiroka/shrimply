# Stabilization vision primitives

This crate translates the scalar routines used by Shrimply's stabilization from
OpenCV 4.13.0 into Rust. It does not depend on the OpenCV library or bindings.
The original source is https://github.com/opencv/opencv/tree/4.13.0.

The same sparse tracking algorithm also runs on the GPU. `src/gpu.rs` packs
base grayscale images and shared pyramid geometry. Its preparation-step list
runs exact integer Gaussian downsampling and Scharr derivatives on the GPU,
then `shaders/tracking.slang` tracks each feature with a cooperative 64-thread
workgroup. Window samples use shared memory; lane zero retains the scalar
reference's row-major reductions. No per-feature global scratch is needed.
CUDA, Vulkan, and Metal bridges only bind buffers and execute the shared plan;
feature ranking and the small affine fit stay in Rust. Windows without a GPU
backend, and windows larger than the shared-memory capacity of 20 pixels, use
the scalar reference. Interpolation weights explicitly use nearest-even
rounding on every backend. Tracking disables CUDA multiply-add contraction,
uses Metal safe math, and enables precise Vulkan compilation.

The source mapping is:

- `RgbaImage::gray`: `modules/imgproc/src/color_rgb.simd.hpp` (`RGB2Gray<uchar>`)
  and `color.simd_helpers.hpp` (15-bit luminance coefficients).
- `good_features`: `modules/imgproc/src/featureselect.cpp`
  (`goodFeaturesToTrack`, `greaterThanPtr`), `corner.cpp`
  (`cornerEigenValsVecs`, `calcMinEigenVal`), and the 3-tap Sobel and
  unnormalized box filters in `filter.simd.hpp`, `box_filter.simd.hpp`, and
  `deriv.cpp` used by those routines.
- `track`: `modules/video/src/lkpyramid.cpp` (`calcScharrDeriv`,
  `buildOpticalFlowPyramid`, `LKTrackerInvoker::operator()`, criteria clamping)
  and `modules/imgproc/src/pyramids.cpp` (`pyrDown`, 5-tap Gaussian).
- GPU tracking: `shaders/tracking.slang` translates the same
  `LKTrackerInvoker::operator()` loops. `vision_pyr_down` and `vision_scharr`
  translate the same integer filters used by the CPU reference; the Rust
  preparation plan shares its pyramid stopping rules and geometry with that
  reference. Cooperative window loading follows
  `opencv_contrib` 4.13.0 `modules/cudaoptflow/src/cuda/pyrlk.cu`
  (`sparseKernel`, per-feature workgroups and shared-memory windows), from
  https://github.com/opencv/opencv_contrib/blob/4.13.0/modules/cudaoptflow/src/cuda/pyrlk.cu.
  The integer interpolation, Scharr derivatives and convergence criteria stay
  those of the CPU reference rather than the CUDA texture-sampling variant.
- RGBA quarter-turn rotation: `modules/videoio/src/cap_interface.hpp`
  (`VideoCaptureBase::applyMetadataRotation`) and `modules/core/src/matrix_transform.cpp`
  (`rotate`). Display-matrix metadata stays in Shrimply's shared video decoder.
- Shared frame-number decoding: `shrimply-video-decoder::VideoDecoderSession::frame_by_index`
  adapts `modules/videoio/src/cap_ffmpeg_impl.hpp` from OpenCV 4.13.0:
  `seek(int64_t)` (lines 2270–2344), `grabFrame` timestamp normalization
  (lines 1720–1807), and `get_duration_sec`, `get_fps`, `get_total_frames`,
  `dts_to_frame_number`, and `dts_to_sec` (lines 2170–2235). It reuses Shrimply's
  decoder, retaining the source's first-frame origin, adaptive preroll,
  frame-count clamp, and ordinal walk after a keyframe seek. Counters use
  `trunc(fps * seconds + 0.5)`; display-matrix angles use nearest-even `cvRound`.
- Affine fitting: `modules/calib3d/src/ptsetreg.cpp`, `levmarq.cpp`,
  `precomp.hpp`, `modules/core/src/lapack.cpp`, and
  `modules/core/include/opencv2/core/operations.hpp`; see `src/math.rs`.

The scope is packed 8-bit grayscale/RGBA images, Shi–Tomasi corners using a
3-tap Sobel gradient, and sparse pyramidal Lucas–Kanade with square windows,
no initial-flow flag, and photometric residual errors. Shrimply does not use
Harris corners, masks, multichannel tracking, or the alternate LK error flag.
Those unused modes are not exposed.

Corner detection stages the separable Sobel rows once, then uses a bounded
row ring and the original double-precision `RowSum`/`ColumnSum` updates for the
unnormalized covariance box filter. Candidate selection can stop comparing a
3×3 neighbourhood as soon as a larger response excludes that candidate.

The CPU tracker prepares reflected pixel borders and zero derivative borders
once, uses separable integer Gaussian/Scharr passes, and performs integer
interpolation with portable SIMD. Scoped standard-library workers partition
independent features; each feature retains the scalar source's floating-point
accumulation and pyramid order, and the result vector retains input order.

The translation retains reflected image borders, zero derivative padding,
14-bit interpolation weights, 5-bit patch intensity scaling, OpenCV's corner
tie ordering and grid suppression, pyramid stopping criteria, eigenvalue
threshold, convergence/oscillation tests, and error normalization. Floating
point reductions follow the scalar reference; SIMD and fused-instruction
implementations can have small rounding differences.

OpenCV's Apache-2.0 license and the original per-source legacy notices are
retained in LICENSE. No original image/video decoding is vendored here;
Shrimply supplies decoded image buffers through its existing media pipeline.

GPU convergence uses fp32 thresholds; the CPU reference preserves OpenCV’s
double-precision dot product for that comparison. This follows GPU precision
constraints, so exact agreement on the validated inputs is not a guarantee of
bit-identical results for every threshold boundary.

The shared stabilization tracker warms the GPU pipeline, measures both paths on
the first usable frame pair, and retains the faster backend for that analysis
chunk. The GPU measurement includes job packing, upload, preparation, tracking,
and result readback; no device-specific performance rule is embedded.
