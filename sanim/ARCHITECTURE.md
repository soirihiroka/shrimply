# Sanim Architecture & Contributor Notes

Internal workspace layout, caching/performance architecture, headless Rust API,
and benchmark commands for Sanim contributors.

## Workspace crates

| Crate | Responsibility |
| --- | --- |
| `sanim-core` | Shared type/value model, checked rational arithmetic, scene tracks, relative layout, geometric verification |
| `sanim-data` | Bounded schema-driven CSV, JSON, TOML, YAML, NPY and NPZ decoding; syntax grammars; no filesystem access |
| `sanim-lang` | Direct-source handwritten AST parser (no lexer/token stream), type checking, preparation, reflection, caching and hot-reload sessions |
| `sanim-render` | Skia vector recordings, bounded geometry/frame caches, explicit CPU PNG/video output |
| `sanim-cli` | Headless commands and event-driven watch sessions |
| `sanim-runtime` | Background preparation, filesystem dependency watches and immutable published revisions |
| `sanim-lsp` | Standalone Language Server Protocol binary over stdio |
| `sanim-presenter` | Native 2D deck presenter |
| `sanim-voxel` | Experimental sparse voxel data model, layout, procedural fields, and assertions |
| `sanim-voxel-render` | Experimental `wgpu`/WGSL voxel flat and path-traced renderer |

## Build, check, and benchmark commands

Run builds and checks outside the repository sandbox using `make`:

```sh
make -C sanim check
make -C sanim bench
make -C sanim headless-check      # verifies default-features = false boundary (no Pango/Cairo/Skia)
make -C sanim headless-bench      # repeated duration edits over 10k-row data source
make -C sanim lsp-build
make -C sanim lsp-check
make -C sanim presenter-release
make -C sanim text-wrapping-check
make -C sanim edit-latency-bench
make -C sanim runtime-hot-bench
make -C sanim voxel-check
make -C sanim voxel-gpu-check
make -C sanim voxel-procedural-check
make -C sanim voxel-procedural-bench
make -C sanim voxel-bench
make -C sanim voxel-storage-bench
make -C sanim voxel-valley-bench
```

From the repository root, additional Sanim performance checks include
`make performance-check`, `make perf-reload-bench`, `make demand-bench`,
`make hot-reload-memory-bench`, `make plot-bench`, and `make markup-bench`.

Voxel profiling environment overrides: `SANIM_VOXEL_PROFILE_SOURCE`,
`SANIM_VOXEL_PROFILE_SMALL=1` (64×36 sweep), and
`SANIM_VOXEL_PROFILE_PREVIEW=1` (flat preview at 1280×720).

## Headless Rust API

```rust
use std::collections::BTreeMap;
use sanim_lang::{Engine, FileSources, Reload, Session};

let sources = FileSources::new("examples").unwrap();
let mut engine = Engine::default();
let prepared = engine.prepare("hello.sanim", &sources, &BTreeMap::new()).unwrap();
assert!(!prepared.animation.layers.is_empty());

let mut session = Session::new("hello.sanim");
match session.poll(&sources) {
    Reload::Updated { revision, prepared, changed_elements } => {
        // Publish this complete revision to the host's playback system.
    }
    Reload::Failed { diagnostic, last_good_revision } => {
        // Display the diagnostic and mark the last-good animation as stale.
    }
    Reload::Unchanged => {}
}
```

## Preparation, caching, and runtime internals

- **Feature boundaries**: `sanim-lang` and `sanim-data` build with `default-features = false`, excluding Pango, Cairo, and Skia while retaining parsing, type checking, data decoders, SQLite, syntax trees, checkpoints, and duration evaluation. Default builds enable `native-text` and `media`.
- **Preparation modes**: `Engine::preparation` selects `Preparation::Full` (headless default; evaluates all bindings) or `Preparation::Demand` (interactive runtime; defers zero-time immutable `read`/`decode`/`query` expressions and unread signals until needed).
- **Checkpoints & persistent collections**: Parsed modules, pure expression results, decoded assets, and root-statement execution checkpoints are cached with independent entry and byte budgets. `imbl` maps/vectors share tree nodes; restoring a checkpoint does not copy large tables or geometry. Reference-counted payloads release ownership when dropped (no tracing GC).
- **Time map & diffing**: `Prepared::time_map` maps source spans to timeline ranges using compact winner runs and indexed periodic clocks bounded by `Limits::max_time_map_bytes`. `TimelineDiff` computes conservative visual invalidation intervals so layout, Skia command recording, and raster LRU caches preserve unchanged intervals across edits.
- **Background runtime**: `sanim-runtime::Runtime::open(path)` prepares on a worker and uses native filesystem notifications (debounced, no polling). `Runtime::snapshot()` never blocks on I/O on the caller thread. `set_waker` notifies the host outside state locks.
- **Grammar registry**: `sanim-data` exposes `Grammar::new`, `parse_with`, and native parser registration while keeping native Tree-sitter trees directly.
- **Performance diagnostics**: `P001` (per-statement interpreted operation budget), `P002` (preparation latency), `P003` (expensive SQLite query), `P004` (slow native bulk operation). Hard limits and thresholds are configured through `Limits` (including `Limits.max_native_work`).
