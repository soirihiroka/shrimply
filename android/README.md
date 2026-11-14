# Adaptive Android layout

Android 13+ application using the shared project backend and a Vulkan / Skia
timeline. Normal launches show the project launcher, then open the selected real
`.shrimp` project in the editor. Project storage, history, timeline actions, and
exact playback state are shared with the desktop. Android uses the existing FFmpeg,
audio engine, Pango/Cairo, OpenCV, Poppler, and Vulkan compositor crates, while
excluding GTK and CUDA host code. Video export uses the shared export pipeline.

The Rust `shrimply-timeline-skia::scene::Scene` owns the same timeline drawing,
selection, editing, media jobs and input behavior as GTK, AppKit and Qt. Compose owns
Android's native launcher, header, transport, menus, and adaptive panes.

## Build

Use JDK 21 or newer supported by Gradle 9.8, Android SDK 37, build-tools 37.0.0,
and NDK 28.2.13676358. Override `ANDROID_SDK_ROOT` if necessary. All Rust commands
are routed through the repository Makefile.

```sh
make android-build
make android-install ANDROID_SERIAL=PHONE_IP:CONNECTION_PORT
make android-check
make android-build ANDROID_ABI=x86_64
```

The default ABI is arm64-v8a. x86_64 is allowed for debug emulator builds only.
Install and benchmark targets require an explicit `ANDROID_SERIAL` so another
connected device is not selected accidentally. They do not restart ADB.
Development builds and benchmarks use the debug profile. Distribution signing
is left to the release environment.

`android-check` runs Android Rust clippy, lint, dependency isolation, ELF LOAD
segment checks, and APK ZIP alignment checks for 16 KiB pages. The native linker
uses `-z max-page-size=16384`; NDK r28 supplies aligned native runtime libraries.
See [Android's page-size guidance](https://developer.android.com/guide/practices/page-sizes).
`make android-layout-check` compiles and lints Kotlin without a native build.
`make android-host-check` type-checks and lints the shared painter and portable
Vulkan code on the desktop. Run `make check` for the existing desktop checks.

## Projects and lifecycle

Normal launches open the project launcher. Its **Create Project**, **Open Project**,
search, and history controls follow the GTK launcher. Creation uses the shared
project presets, resolution limits, exact frame rates, and tag normalization.
A project selection enters the editor; closing it returns to the launcher.
No sample timeline is opened in this flow.

The native backend owns the canonical `Project`, file locking, validation,
transactions, autosave, undo, redo, and project storage used by the desktop.
Edits use the shared history and autosave pipeline. Pause and close persist the
cursor/view state, flush history, and release resources. Reopening loads the same
`.shrimp` document. External edits are watched through native filesystem events;
invalid replacements retain the last valid in-memory project and report an error.
Idle projects do not poll files.

Android's document picker grants access to a project folder. The adapter mirrors
that folder to app-private storage while preserving relative asset paths, and
retains the document-provider grant across restarts. Provider changes and native
saves are synchronized through event-driven, bounded, coalesced queues. Source
fingerprints in Android's SQLite storage detect conflicting external/local changes
before writing. Transfers stream bounded buffers; newer local edits remain
recoverable in the working project. App-owned imports, mirror synchronization, flushes, and exports share one I/O
transaction lock, so a directory notification cannot publish a partially copied
native import. Active writes finish before adapter shutdown.
SAF provider output streams do not promise atomic writes against process death or
other applications writing concurrently. **Export Project**
exports the complete folder, including media, rather than a file with broken asset
references. A different existing destination is never silently overwritten.

File input passes a live Android document-provider descriptor to the native
import transaction. The shared model receives a durable project asset, never a
short-lived content URI or descriptor path. Unsupported import, preview, or encoder
capabilities report specific errors; they do not substitute a sample
project or manufacture output.

## Editor interaction

The editor follows the GTK arrangement: project header and inspector above the
timeline, with step-back, play/pause, step-forward, seek, and exact time display
under the preview area. Playback uses the same extracted `shrimply-player-core`
state and rational frame arithmetic as the desktop. Adwaita symbolic paths and
the existing translation catalog supply controls and text.

Compose owns the native controls and resizable panes. AndroidExternalSurface
hosts the full shared timeline Scene. The project worker owns its canonical
`Rc<RefCell<Project>>`, history and input state, and records immutable Skia
Pictures. The Vulkan worker replays these commands directly on the GPU, without
CPU pixel presentation or per-frame project copies. Surface replacement, resize, pause, and destruction acknowledge
completion before the native window is released. Pending UI snapshots and watch
notifications are coalesced. Unchanged state blocks on its mailbox; waveform,
beat and performance completion wake the worker. Animation and active jobs use
bounded timers. Touchscreen contacts on Android, GTK, AppKit and Qt enter the
same input recognizer. A direct drag moves an item; two contacts pan and pinch
the timeline. Native mouse and keyboard events enter the existing Scene input
and key bindings. The left rail uses the GTK tool descriptions, preferences and
symbolic paths; it scrolls when a narrow window cannot fit all tools.

Inspector categories, cards, controls, effect menus, resets, and header actions
come from `shrimply-inspector-document`. Its reusable native-host bridge resolves
commands against the current selection and stable item/effect identifiers before
calling the existing inspector controllers. Compose supplies native widgets;
numeric conversions, constraints, exact fractions, property changes, expressions,
and history commits stay in the shared crates. The timeline and inspector share
selection and property clipboard state. Specialized controls are still being
integrated; the generic controls do not imply complete inspector parity.

The preview uses the same playback scheduling, cancellation, frame planning, and
Vulkan compositor worker as the desktop. Android's surface adapter borrows completed
Vulkan textures directly into Skia and retains each allocation through its draw
fence. It restores the texture layout before another surface can borrow it. The
timeline and preview share the Vulkan device and queue; their window lifecycle
acknowledgments remain independent of project I/O. Runtime validation of video
decoding and presentation is tracked separately from successful cross-compilation.
Caption overlays use the same Skia drawing entry point as GTK. Android supplies
system font directories and an application cache directory to the shared native
Fontconfig initializer on the project worker. The shared Vulkan device also owns
a pipeline cache, with optional private disk persistence, bounded input, checksum,
and driver/device compatibility validation.

Video export uses `shrimply-export-core` for audio mixing/effects, audio encoding,
frame iteration, muxing, cancellation, asset validation, and output publication.
The Android backend renders accurate frames through the shared Vulkan compositor,
then uses the same Skia/Vulkan surface host as preview to queue completed textures
directly on a MediaCodec encoder input surface. H.264/H.265 export does not read
video pixels back to the CPU. Hardware codec enumeration is shared with decoding;
there is no software-video fallback. Timestamped surfaces require
`VK_GOOGLE_display_timing`. GIF retains the desktop's explicit GPU readback and
shared CPU palette encoder.

GTK and Android share the bounded export worker, cancellation state, progress
labels, percentages, and exact-time ETA. Android stages the complete file in its
private cache before streaming it to the user's document-provider destination.
The staged file is removed after a successful save; a failed copy retains it for
retry. Native dialogs account for keyboard insets and keep their actions visible.

## Benchmarks and device checks

```sh
make android-benchmark-install ANDROID_SERIAL=PHONE_IP:CONNECTION_PORT
make android-benchmark ANDROID_SERIAL=PHONE_IP:CONNECTION_PORT
make android-benchmark STRESS=true ANDROID_SERIAL=PHONE_IP:CONNECTION_PORT
```

The debug benchmark pans through 1,200 frames, then becomes idle. Normal data
contains 16 tracks × 24 clips; stress data contains 128 tracks × 2,048 clips.
Both use actual comment, caption, shape, and generated-audio project items.
Clips obey project frame-grid and non-overlap invariants within each track, with
overlap across tracks and scrolling in both axes. Reports are saved to `target/android-benchmark-{normal,stress}.txt`. Hardware
model, OS version, serial number, and network address are not recorded.

Logs separately report Scene recording p50/p95 and submission/acquire/present wall-time p50/p95, frame-interval p50/p95,
estimated missed 60 Hz intervals, JNI-input-to-submission latency (only when input
was supplied), and RSS initial/final/min/max. Drag during a benchmark to collect
input latency. These are CPU-side observations, not GPU timestamp queries or
input-to-photon measurements. Only structured timing and memory measurements are saved; raw device logs and
platform dumps are not written to reports.

Check portrait/landscape, tablet density and text, clip boundaries, both dividers,
pan/zoom/playhead, multi-window resize, background/resume, and repeated surface
recreation. Measure the full Scene path on hardware before claiming sustained 60 Hz. Emulator layout checks and phone runs are smoke checks only.

## Validation

Kotlin debug compilation and lint have passed for the launcher/editor controls,
project lifecycle adapter, retained session, and schema-generated track menus.
The dependency guard excludes GTK/GDK and the CUDA preview/runtime crates.
FFmpeg, Pango/Cairo, and the shared audio engine are part of the portable editor graph. Sanim's headless duration,
data, SQLite, and hot-reload regressions pass without native text/media features;
requesting an unavailable shaping or decoding operation produces a capability error.

Debug APK checks and the full Linux `make check` have passed. Wireless-device
smoke checks have exercised real project storage, SAF video import, MediaCodec
hardware decoding, Vulkan/Skia preview presentation, scrubbing, and the shared
audio transport. Decoder images are acquired after shared frame selection;
skipped frames never wait for ImageReader delivery. Native image descriptor
pools grow to the format's actual requirements rather than reserving the GPU's
entire sampler limit for each frame.
The generic inspector has also been exercised on the wireless debug device:
project dimensions and video opacity were edited through the shared controllers,
the GPU preview reflected opacity changes, and undo restored them. A vignette
was inserted and edited, then its value edit and insertion were undone separately.
Multilingual captions have rendered through the shared Skia overlay. These are
specific smoke checks, rather than validation of every inspector control.

H.264 hardware export with AAC has produced decodable MP4 files with verified frame
counts, timestamps, source-image timecode, generated audio, and a silent tail.
Saving through the Android document picker preserved the file checksum, and
cancelling a full-rate export removed its partial output. HEVC MP4, H.264 Matroska,
and GIF have also produced fully decodable 34-frame, 1920×1080 exports at 1 fps.
Matroska uses shared deferred header handling for surface encoders that deliver
codec configuration with their first compressed packet. Audio timestamps are
converted only after the container has established its stream time bases.
An export at 30000/1001 fps produced 995 decodable frames, with every PTS
advancing by exactly 1001 ticks at a 1/30000 time base. Export progress updates
are separate from timeline invalidation: both native surface workers remained
idle in a sampled interval while encoding advanced. This is an idle check,
not a sustained frame-rate or input-latency certification.

These checks use controlled fixtures and do not establish complete feature parity
or sustained performance. First-use compositor pipeline compilation still takes
several seconds on the tested path. The complete export settings, inspector, and
remaining media/resource and native interaction paths are still being integrated.
Native macOS/Windows checks and broader device lifecycle validation remain required.
