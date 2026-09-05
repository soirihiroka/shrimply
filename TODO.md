# AppKit renderer and GTK interaction parity

Goal: shared Slang rendering works on Metal while remaining functional on CUDA; AppKit preview and timeline interactions match GTK.

Implementation owner: primary agent. Other agents review and verify only; no implementation delegation.

## Current priorities

- [ ] Consolidate toolkit-independent Skia timeline state, rendering, cursor policy, input/edit handling, refresh decisions and job/subscription cleanup in `timeline-core`. GTK, Qt and AppKit adapters provide rendering contexts, native event translation and popup/callback integration. Audit existing GTK workflows before retiring duplicate runtime code.

- [x] Keep the AppKit launcher and editor as separate processes. `make dev-mac` starts `shrimply-appkit`; selected/created projects spawn the sibling `shrimply-editor-appkit`, while the hidden launcher observes the child from a background thread.
- [x] Flatten the AppKit preview-ready and guides toolbar buttons while preserving their native symbols and behavior.
- [x] Launch the sibling AppKit editor directly. Remove the repeated binary file preflight and let process spawning report a missing or unexecutable editor while retaining project-path validation.
- [x] Preserve playback while dragging the timeline playhead. Shared player state now owns the scrub hint while seeks retain `playing`; timeline-core/AppKit and GTK begin/end the same lifecycle, including cancellation.
- [x] Clear GTK's shared scrub hint if the timeline is unrealized during a held ruler seek, matching AppKit cancellation and preventing a stale fast-scrub state.
- [x] Keep this AppKit/Metal pass on macOS validation. Preserve Linux compile gates and backend separation without running Linux builds or platform checks.
- [x] Connect AppKit's preview FPS label to the completed Metal frame duration and format it with the same shared calculation used by GTK. Timing stays attached to the submitted frame through presentation.
- [x] Fix AppKit fullscreen sizing end to end. The preview host now leaves the editor split hierarchy, fills all four edges of the window content view, and returns to the preview stack on exit. User runtime verification confirms the full-window preview sizing.
- [x] Wrap the AppKit fullscreen control in the same circular native Liquid Glass used by the transport controls and add toolkit-local spacing around both sides of the progress slider.
- [ ] Reproduce the reported blank preview at 00:00:50.43 followed by recovery at 00:14:15.37. The Metal worker now reports slow-frame, shared-Slang-module and pipeline timing; AppKit uses GTK-equivalent loading semantics and the shared 1/24-second anti-flicker delay. First-use shader compilation remains possible but unproven. Read-only sampling after recovery found both preview workers idle.
- [x] Remove blocking shader compilation, media decoding, rendering and readback from the UI thread.
- [x] Verify the reported project loads and renders through the asynchronous Metal path.
- [ ] Complete remaining source/effect orchestration using existing rendering logic, including Morph transitions and remaining modifiers.
- [ ] Complete remaining preview/timeline interaction parity and verify native gestures.

## Shared rendering

- [x] Compile the shared Slang compute modules for Metal using the Slang C++ API and native Metal API; the inventory now includes background generation (nine modules, 72 entries).
- [x] Share the shader inventory between CUDA and Metal; preserve existing Slang formulas.
- [x] Use the existing Slang compositor for sampling, transforms, alpha, layer order and blending.
- [x] Rename CUDA-specific video rendering to `video-cuda` with `git mv`; keep CUDA outside the AppKit dependency graph.
- [x] Verify CUDA-source export names and reflected layouts against previous kernels.
- [x] Share evaluation and ordered dispatch plans for invert, color correction, Gaussian blur, mirror, posterize and vignette between CUDA and Metal.
- [x] Extend the same path to pixelate/mosaic, sharpen, chromatic aberration, emboss, luma key, directional blur, zoom blur and radial blur. Slang kernels are unchanged; obsolete CUDA output-swap helper removed.
- [x] Extend shared evaluation/dispatch to film grain, scanlines/CRT, halftone, alpha outline, erode/dilate and Kuwahara. Slang kernels remain unchanged.
- [x] Preserve two-pass alpha morphology/outline and Kuwahara's eight-float statistics buffer, plus existing zero-radius skips before canvas materialization.
- [x] Actual Metal checks pass grain determinism, CRT/halftone parameter layouts, both Kuwahara modes, outline color/extent, erosion/dilation and zero-radius bypass (`target/metal-six-effects-probe.log`).
- [x] All six modifiers and their operation variants pass real-project evaluation and complete Metal rendering; prior invert and Gaussian/color-correction checks also pass (`target/preview-six-effects-probe.log`). No project commits.
- [x] Read-only source/ABI/resource review and Linux shared check/Clippy pass (`target/linux-six-effects-check.log`). Full CUDA runtime verification remains open.
- [x] Connect channel mixer, colorize/duotone, threshold and edge detection through shared evaluation and Slang dispatch; these retain the same Slang kernels on CUDA and Metal.
- [x] Preserve packed CUDA matrices while packing Metal matrix columns using Slang reflection. Dense scalar color parameters retain shader field order.
- [x] Actual Metal checks pass matrix identity/channel permutation, color field order, threshold/duotone endpoints and source alpha (`target/metal-color-effects-probe.log`). All four also pass real-project evaluation and complete rendering (`target/preview-color-effects-probe.log`).
- [x] Read-only matrix/layout/alpha review and Linux shared check/Clippy pass the four color adapters (`target/linux-four-color-effects-check.log`). Six-modifier GPU diagnostics also still pass after scalar parameter encoding changes.
- [x] Connect bulge/pinch, twirl, wave/ripple, displacement, fisheye, lens distortion and kaleidoscope through shared evaluation and dispatch; 31 raster modifiers now share CUDA/Metal plans. Existing clamps, units and canvas materialization are preserved.
- [x] Actual Metal checks pass zero-strength identities, rectangular-canvas radius scaling, distortion and transparent lens bounds (`target/metal-geometry-effects-probe.log`). All seven pass real-project evaluation/rendering (`target/preview-geometry-effects-probe.log`). No project commits.
- [x] Read-only geometry/ABI review and Linux shared check/Clippy pass (`target/linux-seven-geometry-effects-check.log`).
- [x] Connect chroma key through shared evaluation and the existing Slang matte kernel; the shared plans retain the existing CUDA materialization behavior. Preserve key color, clamps, scalar ABI and input-alpha behavior.
- [x] Actual Metal chroma-key checks pass key removal, non-key color/alpha preservation, soft edges and spill suppression (`target/metal-chroma-key-probe.log`). Real-project evaluation/rendering and earlier invert/blur checks pass (`target/preview-chroma-key-probe.log`). Read-only ABI/semantic review and Linux shared check/Clippy pass (`target/linux-chroma-playback-check.log`).
- [x] Move drop shadow and glow/bloom to shared evaluation and two-pass Slang plans, preserving shadow alpha scratch and bloom's four-float color scratch.
- [x] Actual Metal shadow/bloom checks pass offset/color, foreground preservation, blur alpha, threshold, zero intensity and scratch layouts (`target/metal-shadow-bloom-probe.log`). Both pass complete real-project rendering (`target/preview-shadow-bloom-probe.log`).
- [x] Read-only parameter/ABI/pass/resource review and Linux shared check/Clippy pass (`target/linux-shadow-bloom-check.log`). There are now 34 raster modifiers with shared CUDA/Metal plans.
- [x] Latest AppKit build, all-target check and Clippy with warnings denied pass after shadow/bloom extraction and playback-control updates (`target/appkit-metal-check.log`).
- [x] GPU checks pass the eight new modifiers, including sharpen's three passes, mosaic block centers, complementary luma-key alpha and sampled-blur parameter layouts (`target/metal-more-effects-probe.log`).
- [x] Real-project evaluation and complete Metal rendering pass for the eight new modifiers (`target/preview-more-effects-probe.log`); candidates remained in memory without project commits.
- [x] Preserve CUDA canvas materialization behavior, including identity sampling bypass; retain GPU resources and propagate intermediate command failures.
- [x] Actual Metal GPU checks pass for compositor and representative blur, geometry and color kernels.
- [x] Actual Metal effect adapter checks pass for identity Mitchell sampling/opacity, mirror, posterize and vignette (`target/metal-effects-identity-probe.log`).
- [x] Real-project invert and Gaussian/color-correction chain checks pass after the latest adapter changes; alpha and placement preserved across 2,073,600 opaque pixels (`target/preview-effects-probe.log`).
- [x] Share item-transition timing, easing, opacity and spatial evaluation between CUDA and Metal (`video-core/transition.rs`).
- [x] Share transition mask, two-pass blur and pixelate dispatch plans, including CUDA clip-mask callers; preserve item-effect → transition-transform → transition-effect order.
- [x] Real-project Metal diagnostic passes fade, slide, slide-fade, zoom, spin, wipe, iris, clock wipe, dissolve, triangular fold, streak wipe, blur and pixelate. Fade preserves color with eased alpha (`target/preview-transitions-probe.log`).
- [x] Share clip-pair selection, source Hold policy, clip-transition spatial evaluation and fade-color layers between CUDA and Metal. Ordered stages preserve item effects → item transitions → clip transitions.
- [x] Metal fixture checks pass crossfade, fade-through-color, wipe, iris, clock wipe, dissolve, slide, push and zoom, including transparent outgoing fade color and interval boundaries (`target/preview-clip-transitions-probe.log`).
- [x] Real-video decoding passes before/at/after the transition cut with both held-source intervals; all fixture changes remained in memory (`target/preview-clip-transitions-probe.log`).
- [ ] Connect Morph clip transitions, remaining modifiers, alpha masks, generators and source types currently rejected by Metal. Reuse existing orchestration and Slang kernels.
- [x] Share vector Morph endpoint selection and opacity interpolation between Metal and CUDA in video-core. Retain endpoint audio analysis gathered before visibility/source filtering and avoid caching audio-pending Morph scenes.
- [ ] Finish deduplicating host orchestration: modifier-chain/vector-raster ordering, transition-stage assembly, motion inverse filtering/fallback and spatial-state materialization. Ported effect evaluation, shader formulas and dispatch plans are already shared; GPU allocation/ABI/execution remain backend-specific.
- [x] Share Corner Pin evaluation and Slang dispatch between CUDA and Metal. Preserve corner order, clamps, exact identity bypass and degenerate-geometry errors. Native uniform matrices now use one reflected packing helper shared with Channel Mixer.
- [x] Actual Metal Corner Pin checks pass identity, transparent bounds/alpha, normalized homography, bilinear sampling, projective/bilinear perspective modes, corner clamps and invalid geometry at either perspective endpoint (`target/preview-corner-probe.log`). Channel Mixer and existing color-effects regression rendering pass (`target/preview-color-effects-probe.log`). Candidate project edits remained in memory.
- [x] Read-only semantic/ABI review, AppKit build/check/Clippy and Linux shared check/Clippy pass Corner Pin and fallible raster evaluation (`target/linux-corner-pin-check.log`). Full CUDA runtime remains unverified.
- [x] Share CUDA's rational motion-blur sampling and per-sample audio evaluation in video-core. Native vector blur uses the existing Skia operation; raster blur dispatches the existing Slang compositor with packed 36-byte inverse matrices and retains the original denominator when singular samples are filtered.
- [x] Retain native forward transforms through raster modifiers, transitions and output scaling, matching CUDA's sampling boundaries. Tiny transforms may recover before sampling; blur cannot revive pixels discarded by a prior pixel materialization. Read-only review passes ordering, alpha, ABI and resource lifetime.
- [x] Actual Metal motion checks pass matrix stride, translated samples, straight-alpha averaging, invalid-sample weighting and validation (`target/metal-motion-probe.log`). Complete rendering passes vector/raster agreement, accurate/fast sample counts, modifier ordering, tiny-scale recovery and prior-materialization transparency (`target/preview-motion-probe.log`).
- [x] Motion blur over the real project renders its first asynchronous frame in 902 ms, retains an image for all 60 scrub requests, and keeps maximum UI draw at 2.8 ms. Revision/exclusion handoff passes (`target/preview-motion-async-probe.log`). Candidate project edits remained in memory.
- [x] AppKit build/check/Clippy and Linux shared check/Clippy pass motion blur and forward spatial state (`target/linux-motion-blur-check.log`). CUDA runtime verification remains open.
- [x] Motion sampling diagnostic passes exact rational shutter times, distinct per-sample audio transforms, sample/angle/phase clamps and item bounds (`target/motion-audio-probe.log`). Shape and folded-sequence regression rendering pass after forward-state changes (`target/preview-shape-probe.log`, `target/preview-sequence-probe.log`).
- [x] Move the existing background shader into render-core and share expression/parameter preparation in video-core. Vulkan/CUDA consumes the shared SPIR-V ABI; Metal packs named values using its own reflection.
- [x] Route all ten background generators and fade-color layers through shared Slang on Metal, preserving CUDA baked source transforms and effect/transition order.
- [x] Actual Metal rendering passes all ten generators, straight-alpha solid color, canvas bounds, baked transforms and deterministic animated-noise epochs (`target/preview-backgrounds-probe.log`). Noise epoch arithmetic now uses rational time in math-media.
- [x] Clip-transition GPU diagnostics still pass with Slang-generated background/fade-color layers, including transparent outgoing layers, boundary colors and real-media held intervals (`target/preview-clip-transitions-probe.log`).
- [x] Source/ABI review passes; Linux shared check/Clippy and Slang C++ API CUDA source generation pass (`target/linux-background-check.log`, `target/linux-background-cuda-api-check.log`). NVCC runtime remains unverified.
- [x] Connect the existing shared frame audio sampler to Metal render evaluation: visibility, transforms, backgrounds, modifiers and compositing use actual frame analysis. CUDA/inspector retain the same shared 48 kHz analysis rate.
- [x] Keep rendering active while mouth analysis is pending, retain the previous image for accurate requests, and propagate analysis failures. Audio revision invalidation refreshes same-time edits.
- [x] Actual Metal output passes audio-reactive opacity, visibility after track muting, same-time scene invalidation and Slang invert (`target/preview-audio-probe.log`). The procedural-audio fixture remained in memory; no project commits.
- [x] Read-only source review and Linux shared check/Clippy pass for frame audio integration (`target/linux-frame-audio-check.log`).
- [x] Shared PreparedText freezes layout/appearance before audio readiness, preserving its layout anchor separately from decoration offset, color glyphs, text-mask interception with unmasked background, and word/glyph morph behavior. Read-only implementation review passes.
- [x] Move existing text drawing/mask/transition/morph code into video-core with a thin CUDA adapter; connect native generated text sources to the same Metal Skia surface and Slang compositor.
- [x] Extract the existing paint preparation/drawing/texture/Drawing-transition implementation into video-core; CUDA and native Metal delegate to it. Native paint caches remain worker-local and clear on project invalidation; drawing errors propagate through the generated surface boundary.
- [x] Actual Metal paint diagnostic passes stroke/fill placement, straight alpha, Immediate/Picture equivalence, Slang invert after Rasterize, all three Drawing fill modes, PathOffset and missing-texture errors (`target/preview-paint-probe.log`).
- [x] Paint-over-real-media asynchronous check passes: first frame 966 ms, 60 scrubs retain an image, maximum UI call 4.2 ms, revision/exclusion removal/restoration pass (`target/preview-paint-async-probe.log`). Candidate changes stayed in memory.
- [x] Read-only paint review, AppKit build/check/Clippy and Linux shared check/Clippy pass (`target/linux-paint-check.log`). Cache clearing on invalidation addresses the review's resource-lifetime finding. Native paint tools and CUDA runtime verification remain open.
- [x] Move existing SVG DOM drawing/morph and SVG transitions into video-core, preserving checked cross-thread DOM ownership. CUDA keeps residency and Vectorize wrappers; native SVGs remain vector sources through modifiers/transitions, with color overrides in source identity. Remove native early SVG rasterization.
- [x] Connect native PDF pages through the existing shared PDF renderer on the media worker. Page identity, source snapshots, actual page dimensions, opaque white background and failures use existing behavior.
- [x] Read-only SVG/PDF source review passes shared drawing, CUDA caller preservation, ownership and native IO/cache behavior.
- [x] Actual Metal SVG/PDF checks pass SVG placement/alpha, color overrides, Immediate/Picture equivalence, Rasterize plus Slang invert, ten SVG transitions and root sizing; PDF sizing, white background, page selection and invalid-page errors pass (`target/preview-document-probe.log`).
- [x] Asynchronous source changes pass SVG color overrides, SVG-to-PDF replacement and PDF page changes: first SVG 118 ms, subsequent updates 24–30 ms, maximum UI call 3.2 ms (`target/preview-document-async-probe.log`). Candidate project changes stayed in memory.
- [x] AppKit build/check/Clippy and Linux shared check/Clippy pass SVG/PDF integration (`target/linux-document-check.log`).
- [x] Actual Metal text checks pass glyph placement/alpha, horizontal/vertical alignment, custom render sizes, Slang invert, all text-mask directions/modes, unmasked backgrounds, decorations, empty text, multiline/RTL/CJK/vertical layouts, eleven supported generated transitions, color emoji and audio-reactive font size (`target/preview-text-probe.log`).
- [x] Text-over-real-media asynchronous check passes: first frame 1.16 s, 60 scrubs retain an image, maximum UI call 2.5 ms, revision/exclusion removal/restoration pass (`target/preview-text-async-probe.log`). No project commits.
- [x] AppKit build/check/Clippy and Linux shared check/Clippy pass shared text and reuse of the existing transition validator (`target/linux-text-check.log`). Shape regression checks pass with valid source/side transition fixtures.
- [x] Extract shape geometry/decorations, generated drawing, path transitions, vector morph data and vector modifier evaluation into video-core; CUDA delegates to the shared implementation.
- [x] Connect shape sources to a worker-owned Metal Skia surface and existing Slang composition/effects. Preserve the explicit vector/raster boundary, lazy pixel transitions and render-canvas correction. The Skia-to-Slang bridge currently performs synchronous worker-side readback into shared Metal storage.
- [x] Actual Metal shape checks pass geometry/anchor/bounds, straight alpha, outlines/shadows, Picture/Immediate equivalence, custom canvas sizes, explicit Rasterize plus Slang invert, vector opacity/repeat and ten supported generated transitions (`target/preview-shape-probe.log`).
- [x] Resolve shape properties before the frame audio-readiness gate; shape draw/morph no longer creates late asynchronous analysis queries. Generated-only best-effort frames rerender on accurate settling.
- [x] Shape-over-real-media asynchronous verification passes: first frame 777 ms, 60 scrubs retain an image, maximum UI call 2.6 ms, frame revision/exclusion removal/restoration pass (`target/preview-shape-async-probe.log`).
- [x] Read-only source review, AppKit build/check/Clippy and Linux shared check/Clippy pass the shape extraction (`target/linux-shape-check.log`). CUDA runtime remains unverified.
- [x] Ordered raster transform/opacity/sampling uses the same evaluation as CUDA. Actual Metal checks pass transform placement, multiplicative opacity, blur-before/after-scale order, singular transforms, sampling overrides and fast-scrub fallback. Unsupported sampling is rejected when consumed; invisible final layers are skipped as on CUDA.
- [x] AppKit build/check/Clippy and Linux shared check/Clippy pass the ordered raster adapter. Folded-sequence rendering and independent real-media instance decoding still pass (`target/preview-sequence-probe.log`).
- [x] Share active-track traversal and folded source-time/cycle/reference resolution with CUDA. Metal recursively composites transparent groups through existing Slang before host state/effects; native media requests, completions and decoder caches use full item addresses. Preserve root audio analysis and nested exclusions.
- [x] Correct native track order to match CUDA and the shared shader: later stored tracks paint over earlier tracks.
- [x] Read-only folded traversal/state/ABI/resource review, AppKit build/check/Clippy and Linux shared check/Clippy pass (`target/linux-folded-sequence-check.log`).
- [x] Actual Metal folded rendering passes parent opacity/effects/transform bounds, nested groups, rational offset/speed and child transitions, empty groups, sibling reuse, independent real-media instance decoding, nested exclusion and explicit cycle/missing-reference failures (`target/preview-sequence-probe.log`). Fixtures remained in memory; no project commits.
- [x] Folded real-project asynchronous diagnostic passes: first frame 726 ms, 60 scrubs retain an image, maximum UI call 3.2 ms, revision/exclusion removal/restoration pass (`target/preview-sequence-async-probe.log`). Ordinary-project regression also passes: first frame 746 ms and maximum UI call 3.4 ms (`target/preview-async-probe.log`).
- [ ] Connect tracked-camera sampling for native rendering and preview geometry.
- [x] Mirror GTK's accepted-frame audio handoff through Metal frame plans, GPU pending work and presentation. Audio and time stay attached to the submitted GPU frame; AppKit no longer initializes overlays with silent analysis.
- [x] Add shared nonblocking audio preparation: queue missing subset queries on the existing worker, defer incomplete providers, retry pending volume/mouth analysis and surface failures. Keep providers/extensions/expression caches on the UI thread; preserve accepted gestures.
- [x] Retain immutable project snapshots for delayed volume queries and let canceled responses retire without stopping the shared worker. Runtime checks pass same-time audio edits, old-frame isolation, discarded requests, deferred geometry, successful retries and explicit failures (`target/preview-audio-handoff-probe.log`).
- [x] Require matching accepted-frame time/revision for idle native preparation. Read-only review passes snapshot lifetime, expression retries, active gestures and frame metadata ownership.
- [x] Linux shared check/Clippy passes the nonblocking handoff (`target/linux-frame-audio-handoff-check.log`).
- [x] Actual Metal verification observes 97 completions during 100 scrub requests with exact audio/time/image pairing; maximum UI draw 2.3 ms (`target/preview-audio-frame-probe.log`).
- [x] Real-project asynchronous verification still passes: first frame 834 ms, 60 scrubs retain an image, maximum UI draw 3.4 ms, revision/exclusion removal and restoration pass (`target/preview-async-probe.log`). Provider hit/drag/cancel verification also passes with deferred preparation (`target/preview-controller-probe.log`).
- [ ] Verify idle handles during playback: strict current-time matching can defer them while rendering trails the playhead. Native visual/input verification remains open.
- [ ] Verify asynchronous mouth-analysis pending/failure behavior with a supported speech fixture.
- [ ] Verify full CUDA runtime behavior; available validation environment lacks NVCC/OptiX.

## Preview interactions

- [x] Move GTK caption drawing and split hit testing into preview-interaction-core and reexport for GTK. AppKit draws the shared overlay in preview coordinates, before guides/handles, using caption preferences and current playback time.
- [x] Connect caption hover/caret and primary-click splitting to existing timeline edits; preserve guide priority, select the right caption and refresh caption/inspector state. Caption overlays stay separate from video export pixels.
- [x] Read-only caption review and Linux shared check/Clippy pass (`target/linux-caption-check.log`). Native clicks use successful guide-press priority; cached guide cursor affects hover only.
- [x] Shared caption diagnostic passes active intervals/track visibility, UTF-8 split hit/caret, strict split endpoints, live font/inset appearance, and existing markup-preserving split. Metal video pixels exclude the host caption overlay (`target/preview-caption-probe.log`). Candidate changes stayed in memory. Native caption gestures remain part of UI verification.
- [x] Extract GTK provider preparation, edit sinks, pointer/keyboard dispatch, cancellation, refresh accumulation, frame acceptance and overlay drawing into `preview-interaction-core`; GTK delegates.
- [x] Connect AppKit pointer, hover, keyboard, modifier-key and scroll events to the shared controller.
- [x] Preserve actual presented-frame revision/exclusion for overlay handoff and retain the last completed image during scrubbing.
- [x] Carry shared frame accuracy through Metal submission/presentation. Paused seeks and scrub refinement remain loading until a fully accurate frame arrives; playback alone uses the one-frame lag tolerance. Keep GTK's exact shared 1/24-second indicator delay and report slow Metal/Slang phases.
- [x] Keep loading-indicator timing in preview core while sizing each toolkit's native indicator locally. AppKit uses a compact 16-point spinner inside the existing preview toolbar slot.
- [x] Connect shared guides; cancel stale guide snapshots, synchronize visibility, and save only completed edits.
- [x] Initialize paint interaction state with GTK defaults; balance native cursor visibility and suppress unchanged release samples.
- [x] Shared-controller diagnostic on the real project passes hit testing, transform drag, cancellation rollback and suppressed mouse-up without a project commit (`target/preview-controller-probe.log`).
- [x] Asynchronous real-project diagnostic: first frame 736 ms, 60 scrubs retained an image, maximum UI draw 2.8 ms; frame revision and exclusion handoff passed (`target/preview-async-probe.log`).
- [ ] Verify native item transforms, overlays, guides, pointer/keyboard/trackpad input and checkerboard visually.
- [x] Use the existing shared playback-speed label in AppKit and connect Space/L capture shortcuts to shared player state. GTK and AppKit now share the 200 ms frame-step repeat interval. Native buttons act on press and repeat without an extra release step; source/API review and Linux shared check/Clippy pass (`target/linux-chroma-playback-check.log`).
- [x] Wrap the existing back/play/forward AppKit controls in individual circular native Liquid Glass views without replacing their actions, repeat behavior or SF Symbols. Keep the timecode in the native monospaced system font.
- [ ] Verify native frame-step hold/cancellation and playback-speed shortcuts.
- [ ] Complete paint tools. AppKit FPS and delayed loading status are connected.
- [x] Match GTK's preview Copy/Save Image context actions, including Control-click. Capture the already presented image before a picker, encode off-thread, preserve image dimensions/alpha, use preview folder preferences and reuse native scoped/atomic file and clipboard handling. Read-only review passes selected-frame export preservation and callback borrow lifetimes.
- [ ] Verify native preview context gestures, clipboard and save picker.
- [x] Extract GTK fullscreen pointer reveal policy and the three-second timeout into shared preview code; GTK and AppKit consume them. Preserve idle baseline, per-axis movement threshold and control motion behavior.
- [x] AppKit fullscreen reparents the existing playbar into a bottom glass overlay, hides tools/toolbar, restores prior visibility and layout, handles Escape before preview tools, hides on timeout/playback and updates caption inset. Drag events refresh the timeout; failed fullscreen entry restores layout.
- [x] Fullscreen policy diagnostic matches the original GTK implementation over 10,000 motion/hide/show/reset events, including one-pixel boundaries, repeated motion and NaN coordinates (`target/fullscreen-policy-probe.log`). Read-only source review, AppKit build/check/Clippy, Linux shared check/Clippy, formatting and source-size checks pass (`target/linux-fullscreen-check.log`).
- [ ] Verify remaining native fullscreen controls/dragging, hide/reveal, Escape and restoration. Full-window preview sizing is user-verified; native computer-use still cannot perform the remaining gestures.

## Timeline interactions

- [x] Use shared Skia scrollbar hit regions and prevent scrollbar clicks from seeking.
- [x] Route secondary click/control-click to shared context contracts.
- [x] Connect context edits, native speed/gain sliders, audio export and frame-copy/save workflows.
- [x] Connect GTK tool rail and keyboard shortcuts, including copy/cut/paste/properties, grouping, split, ripple trim/delete and zoom toggle.
- [x] Connect shared beat loading/drawing, fresh snapping targets and populated-track confirmation.
- [x] Add clip edge resize, cut-click, rectangle/gap selection and final-release updates.
- [x] Extract GTK nested/root drop destinations and nested drag updates/application into timeline core; GTK and AppKit consume them.
- [x] Preserve grabbed clip focus, resolve concrete root destinations before fallback validation, collapse root multi-selection on a plain click, and update group track targets atomically.
- [x] Extract GTK transition hit priority, focus, creation, fade drag, duration handles, rolling cuts and final application into core; connect AppKit drawing, input and cancellation.
- [x] Review transition semantics and fix the GTK release callback borrow risk.
- [ ] Verify native drag/drop placement and preview invalidation, nested/folder moves, transition handles, trackpad pan/zoom and fixed-width audio meter.
- [x] Extract GTK cut-hover selection/grouping, middle-button pan math, folded-sequence toggling and double-click caption creation into core; connect AppKit handlers.
- [x] Address review findings: middle-click focuses the timeline, multi-item cuts preserve clicked-item focus, double-click drags derive snapping from their active gesture.
- [x] Extract GTK snapping with shared grouped/nested move offsets, resize candidates, natural media boundaries and rational snap radius; source review and Linux shared check/Clippy pass.
- [x] Connect GTK playhead visibility during held scrubbing/paused seeks, restore saved zoom/center, save wheel/pinch zoom, and use expanded-sequence bounds for scrollbars/panning.
- [x] Match GTK rectangle snapping and drag-distance threshold.
- [x] Share timeline hover/drag cursor hit testing between GTK and AppKit, including resize and transition handles; restore the correct cursor after release and Escape.
- [x] Use shared project group expansion for AppKit item selection while preserving Shift's ungrouped selection behavior.
- [x] Reload waveform data only when its shared cache signature changes. Placement-only root, nested and cross-scope moves retain cached waveforms; overwrite trims/splits, speed-scaled move-out, resizing, rolling audio transitions and grouped audio cuts invalidate them. GTK and AppKit use the same rule.
- [x] Move drag/resize waveform-cache invalidation and item-kind refresh mapping into timeline-core by migrating the actual GTK gesture engine; remove the duplicate AppKit gesture implementation. Context edits calculate waveform invalidation from their before/after inputs in core.
- [x] GTK, Qt and AppKit use the same core scene for drawing, grouped/root/nested selection, drag/resize, rectangle selection, cuts, transitions, scrollbar hit testing and cursor policy. Preserve native recording overlays, performance data, text-drop previews, accent color and relative software cursors through the drawing boundary.
- [ ] Finish tightening the native adapter interface and review lifecycle/input migration: core owns media jobs and weak player subscriptions; adapters provide context, event delivery, scheduling and native callbacks. Native context-menu/service integration remains under review.
- [ ] Consolidate GTK file-drop inspection and text-drop placement in the shared scene; preserve immediate previews, snapping and cancellation on leave. Latest gesture-engine AppKit build/check/Clippy passed before this follow-up (`target/appkit-timeline-core-migration-check.log`).
- [x] Read-only review passes snapping parity, weak listener lifetime, seek callbacks, zoom persistence and expanded timeline bounds.
- [ ] Verify seek-follow, restored zoom, middle-button panning, hover cuts and double-click behavior with native input.
- [ ] Complete recording, transcription, silence-removal and speech-generation workflows.
- [ ] Verify frame-copy/save through the shared Metal renderer.

## Verification and limitations

- [x] Latest `make appkit-check` passes after shared source rendering, caption/context/fullscreen changes, motion blur, forward spatial state and Corner Pin: launcher/editor build, all-target checks and Clippy with warnings denied (`target/appkit-metal-check.log`).
- [x] Linux shared timeline/preview/Skia/export checks passed during extraction; latest effects and transition checks/Clippy pass (`target/linux-more-effects-check.log`).
- [x] Linux check/Clippy passes after root click-selection, transitions and the eight-modifier extension (`target/linux-eight-effects-check.log`).
- [x] Linux shared check/Clippy passes after cut/pan/double-click extraction (`target/linux-timeline-input-check.log`). GTK source review found one stale import; removed.
- [x] Linux shared check/Clippy passes after snapping extraction and zoom restoration (`target/linux-timeline-snapping-check.log`).
- [x] Linux shared background checks/Clippy and CUDA source generation pass (`target/linux-background-check.log`, `target/linux-background-cuda-api-check.log`).
- [x] Linux shared check/Clippy passes after frame-accuracy loading and timeline cache/cursor changes; the Slang C++ API still generates all nine CUDA modules and 72 kernel exports (`target/linux-loading-check.log`, `target/linux-loading-cuda-api-check.log`).
- [ ] Full CUDA/GTK/Qt build remains unverified without NVCC/OptiX; Linux validation is outside this AppKit/Metal pass.
- [x] Latest targeted formatting and `git diff --check` pass.
- [x] Source-size check passes (`make SHELL=/bin/zsh source-size-check`).
- [x] Latest AppKit launcher/editor build, all-target check and Clippy with warnings denied pass after frame-accuracy loading, native glass playback controls, shared selection/cursor behavior and waveform invalidation (`target/appkit-selection-waveform-cursor-check.log`).
- [x] AppKit launcher/editor build, all-target check and Clippy with warnings denied pass with toolkit-local spinner sizing (`target/appkit-spinner-toolkit-local-check.log`).
- [x] AppKit launcher/editor build, all-target check and Clippy with warnings denied pass after completed-frame FPS timing, fullscreen fill constraints, the glass fullscreen control and slider spacing (`target/appkit-fps-fullscreen-final-check.log`).
- [x] AppKit launcher/editor build, all-target check and Clippy with warnings denied pass after shared Morph presentation/audio handling and playback-preserving timeline scrubbing (`target/appkit-morph-timeline-scrub-check.log`).
- [x] AppKit launcher/editor build, all-target check and Clippy with warnings denied pass after fullscreen reparents the preview host at the root and pins it to all four window edges (`target/appkit-fullscreen-root-host-check.log`).
- [x] AppKit launcher/editor build, all-target check and Clippy with warnings denied pass with flat preview-toolbar buttons and the reviewed shared scrub lifecycle (`target/appkit-flat-preview-tools-check.log`).
- [ ] Finish native UI verification.

Native UI automation still cannot acquire the verification launcher. A process sample found AppKit waiting inside its saved-window recovery alert before applicationDidFinishLaunching (`target/appkit-native-sample.txt`). A fresh verification bundle identifier appeared in the UI inventory, but getApp rejected that same identifier as invalid. Owned verification processes were stopped. Native gesture verification remains open; diagnostics do not establish full UI parity.

Latest native verification retry failed before app selection: the computer-use tool reported `Sky Computer Use native pipe startup failed`. No UI actions were performed during that retry.

The Shrimply MCP is unavailable in this session. Diagnostics load the existing project through application APIs and modify candidates only in memory. No project file is edited directly or committed for verification.
