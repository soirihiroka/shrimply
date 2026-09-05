# AppKit renderer and GTK interaction parity

Goal: shared Slang rendering works on Metal while remaining functional on CUDA; AppKit preview and timeline interactions match GTK.

Implementation owner: primary agent. Other agents review only; no implementation delegation.

## Current priorities

- [ ] Finish Manim runtime integration. Keep compilation, scene state, progress, errors and WGPU rendering in shared Manim crates; keep exact-device interop in `manim-metal` and CUDA interop in `manim-cuda`.
- [ ] Implement AppKit Settings with the shared preferences/schema and GTK's available options; enable the application menu action and Command-comma.
- [ ] Connect external video masks through the shared Slang kernel, including selected-stream EOF behavior.
- [ ] Finish Metal source/effect orchestration for Morph transitions, remaining modifiers, alpha masks, generators and source types by reusing the shared plans and Slang kernels.
- [ ] Move remaining reusable host orchestration into `-core`: modifier/vector ordering, transition assembly, motion fallback and spatial-state materialization.
- [ ] Connect tracked-camera sampling to native rendering and preview geometry.

## Preview

- [ ] Complete paint tools.
- [ ] Fix any remaining AppKit interaction gaps reported during use through the shared preview interaction core.

## Timeline

- [ ] Move remaining reusable context and service orchestration into timeline core; keep GTK and AppKit as native context, menu, dialog and callback bridges.
- [ ] Complete recording, transcription, silence-removal and speech-generation workflows through shared timeline operations.

The Shrimply MCP is unavailable in this session. Existing project files are read through application APIs and are not edited directly.
