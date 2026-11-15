# Stabilization vision primitives

This crate translates the scalar routines used by Shrimply's stabilization from
OpenCV 4.13.0 into Rust. It does not depend on the OpenCV library or bindings.
The original source is https://github.com/opencv/opencv/tree/4.13.0.

The same sparse tracking algorithm also runs on the GPU. `src/gpu.rs` reuses
Rust's Gaussian pyramids and Scharr derivatives, then packs a single job for
`shaders/tracking.slang`. Each GPU invocation tracks one feature through all
pyramid levels. CUDA, Vulkan, and Metal bridges only upload, dispatch, and
return results; feature ranking and the small affine fit stay in Rust.
Interpolation weights explicitly use nearest-even rounding on every backend.
The tracking module disables CUDA multiply-add contraction and uses Metal safe
math; Vulkan's `precise` accumulations preserve the scalar operation order.

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
  `LKTrackerInvoker::operator()` loops, sharing CPU-prepared pyramids and
  derivatives through `src/gpu.rs`.
- RGBA quarter-turn rotation: `modules/videoio/src/cap_interface.hpp`
  (`VideoCaptureBase::applyMetadataRotation`) and `modules/core/src/matrix_transform.cpp`
  (`rotate`). Display-matrix metadata stays in Shrimply's shared video decoder.
- Affine fitting: `modules/calib3d/src/ptsetreg.cpp`, `levmarq.cpp`,
  `precomp.hpp`, `modules/core/src/lapack.cpp`, and
  `modules/core/include/opencv2/core/operations.hpp`; see `src/math.rs`.

The scope is packed 8-bit grayscale/RGBA images, Shi–Tomasi corners using a
3-tap Sobel gradient, and sparse pyramidal Lucas–Kanade with square windows,
no initial-flow flag, and photometric residual errors. Shrimply does not use
Harris corners, masks, multichannel tracking, or the alternate LK error flag.
Those unused modes are not exposed.

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
