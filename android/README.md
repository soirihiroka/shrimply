# Android tablet milestone

Android 13+ shell with a Vulkan 1.1 / Skia timeline. The preview is explicitly
labeled as deferred. The APK has no project loading, decoding, editing, playback,
audio, or export actions.

The Rust `shrimply-timeline-paint-skia` crate supplies the ruler/grid, track labels,
clip fill/borders/text, dividers, and playhead used by Android and the desktop
adapter. Its presentation inputs retain `Time` and `Fraction`; the Android graph
contains no project document, editor runtime, FFmpeg, GTK/GIO, or Pango/Cairo.
Desktop media, recording, waveform, transition, and drag orchestration stays in
`timeline-skia`.

## Build

Use JDK 21 or newer supported by Gradle 9.8, Android SDK 37, build-tools 37.0.0,
and NDK 28.2.13676358. Override `ANDROID_SDK_ROOT` if necessary. All Rust commands
are routed through the repository Makefile.

```sh
make android-build
make android-install
make android-check
make android-build ANDROID_ABI=x86_64
make android-build ANDROID_PROFILE=release
```

The default ABI is arm64-v8a. x86_64 is allowed for debug emulator builds only.
Release APKs are unsigned unless `ANDROID_DEV_SIGN=true` is explicitly supplied;
`android-install` uses the local debug key for device development, including
release builds. Distribution signing is left to the release environment.

`android-check` runs Android Rust clippy, lint, dependency isolation, ELF LOAD
segment checks, and APK ZIP alignment checks for 16 KiB pages. The native linker
uses `-z max-page-size=16384`; NDK r28 supplies aligned native runtime libraries.
See [Android's page-size guidance](https://developer.android.com/guide/practices/page-sizes).
`make android-layout-check` compiles and lints Kotlin without a native build.
`make android-host-check` type-checks and lints the shared painter and portable
Vulkan code on the desktop. Run `make check` for the existing desktop checks.

## Ownership and interaction

Compose owns the toolbar, left inspector, preview placeholder, and dividers.
The inspector collapses below 720 dp. Drag either divider to resize it. The
initial timeline height is 40% of the available content. AndroidExternalSurface
hosts the timeline; all Skia and Vulkan objects belong to one Rust thread.

Drag the track area to pan both axes, pinch to zoom around the gesture centroid,
and tap or drag the ruler to position the playhead. JNI accepts batches of six
floats per gesture (`kind, x, y, dx, dy, zoom`, in dp). Rust retains the viewport,
selection fixture, and playhead. A bounded mailbox coalesces pending inputs
before drawing. A GPU fence limits work to one frame in flight; presentation
semaphores belong to swapchain images. Unchanged state blocks on the mailbox.

Pause drops the renderer. Resume recreates it on the retained window when valid.
Surface replacement, resize, and destruction synchronously acknowledge completion
before Kotlin returns from its lifecycle callback. Renderer teardown waits for
the GPU, destroys Skia, the swapchain, device, and Vulkan surface, then releases
the native window. Unsupported Vulkan capabilities fail explicitly.

## Benchmarks and device checks

```sh
make android-benchmark-install
make android-benchmark
make android-benchmark STRESS=true
```

The release benchmark pans through 1,200 frames, then becomes idle. Normal data
contains 16 tracks × 24 clips; stress data contains 128 tracks × 2,048 clips.
Both include all four track kinds, overlaps, selection, long labels, and scrolling
in both axes. Reports are saved to `target/android-benchmark-{normal,stress}.txt`.

Logs report submission/acquire/present wall-time p50/p95, frame-interval p50/p95,
estimated missed 60 Hz intervals, JNI-input-to-submission latency (only when input
was supplied), and RSS initial/final/min/max. Drag during a benchmark to collect
input latency. These are CPU-side observations, not GPU timestamp queries or
input-to-photon measurements. `dumpsys gfxinfo` is captured as complementary shell
telemetry; its Compose frames do not count native surface frames.

Check portrait/landscape, tablet density and text, clip boundaries, both dividers,
pan/zoom/playhead, multi-window resize, background/resume, and repeated surface
recreation. Run release measurements on the reference tablet before claiming
sustained 60 Hz. Emulator layout checks and the connected Android 15 phone are
smoke checks only. Further video work requires MediaCodec/AImageReader,
AHardwareBuffer import and synchronization; it is outside this milestone.

## Validation recorded for this implementation

The shared painter was compared with the pre-extraction implementation in 30
raster cases: five viewport/zoom combinations, light and dark themes, and
1×/1.5×/2× density. Rulers, clip borders, labels, selection, and playheads had
zero differing pixel channels. Desktop coordinate calculations retain their
previous arithmetic, and visual tick spacing retains the previous rounding.

Kotlin debug compilation/lint, the portable Vulkan host check/clippy, and Android
dependency isolation passed on Linux. macOS and Windows native builds require
those hosts. The native Android cross-build, APK execution, lifecycle stress, 16 KiB APK inspection, and hardware
benchmarks have not been completed: the release build was stopped and shared ADB
access was left available for the concurrent keyboard work. No performance or
cross-platform runtime certification is implied by the source checks.

The full `make check` reached the existing font-system tests, then failed while
linking with `No space left on device`. This is an incomplete full-suite result,
not a passing check. The interrupted Android source write was restored and
formatted; only this task's Android release/intermediate artifacts were removed.
