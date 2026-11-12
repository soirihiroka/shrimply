# Sanim

Sanim is a standalone, headless animation scripting language for Shrimply. It
has its own Rust workspace so language development and tests do not build the
editor, GPU kernels, Python, or GUI dependencies.

```text
reflect tint: Color = Color("orange3")

fn make_marker(identity: String, radius: Length = 24pt): Element {
    return Circle(id= identity, radius= radius, fill= tint)
}

let marker = make_marker("marker")
play(FadeIn(marker), duration= 250ms)
repeat 2 {
    play(MoveTo(marker, Vec2(30%, 0%)), duration= 1s)
    play(MoveTo(marker, Vec2(-30%, 0%)), duration= 1s)
}
assert(always(inside_canvas(marker)))
```

## Build and run

Use the Makefiles. Run builds and checks outside the repository sandbox.

```sh
make -C sanim check
make -C sanim bench
make -C sanim run ARGS='check examples/hello.sanim --json'
make -C sanim run ARGS='inspect examples/data.sanim'
make -C sanim run ARGS='render examples/query.sanim --time 600ms --output /tmp/query.png'
make -C sanim run ARGS='render examples/hello.sanim --frame 60 --output /tmp/sanim.png'
make -C sanim run ARGS='watch examples/hello.sanim --output /tmp/sanim.png'
make -C sanim run ARGS='duration examples/animations.sanim --json'
make -C sanim run ARGS='video examples/animations.sanim --output /tmp/animations.mp4'
```

Reflected inputs accept typed JSON overrides. For example:

```sh
make -C sanim run ARGS='check examples/hello.sanim --set repetitions=4'
make -C sanim run ARGS='check examples/data.sanim --set bar_color="\"#FF8800\""'
```

The CLI has `check`, `inspect`, `duration`, `render`, `video`, and `watch` commands.
Standalone output defaults to 1920×1080, 30 FPS and 96 DPI. Video streams
headless Skia frames to an external FFmpeg encoder for H.264 MP4 output. `--help` describes
frame/time selection, output sizing, and bounded watch sessions. `inspect`
exports source dependencies, reflected values, resolved types and symbols,
scheduling effects, verification evidence, and performance measurements as JSON.
`--progress` emits observed preparation phases/counts as JSON on stderr, leaving
the command's stdout protocol unchanged. Counts are phase-local; unknown totals
remain unknown rather than becoming an estimated overall percentage.

## Crates

| Crate | Responsibility |
| --- | --- |
| `sanim-core` | Shared type/value model, checked rational arithmetic, scene tracks, relative layout, geometric verification |
| `sanim-data` | Bounded schema-driven CSV, JSON, TOML, YAML, NPY and NPZ decoding; no filesystem access |
| `sanim-lang` | Direct-source handwritten AST parser, no lexer/token stream, type checking, preparation, reflection, caching and hot-reload sessions |
| `sanim-render` | Skia vector recordings, bounded geometry/frame caches, explicit CPU PNG/video output |
| `sanim-cli` | Headless commands and polling watch loop |
| `sanim-runtime` | Background preparation, filesystem dependency watches and immutable published revisions |

## Language

The root file is the entry point. Blocks use braces; indentation is cosmetic.
Statements end with a newline or semicolon. Functions use
`fn name(argument: Type = default): ReturnType { ... }`. Defaulted parameters
follow required parameters. Arguments use `name = value`; record values use `{ field = value }`.
Colons declare types (`name: Type`) and separate Python-style dictionary entries
(`{"key": value}`). Older colon-style call/record value syntax remains readable
for source compatibility, but new source uses `=`.

Bindings are immutable. `reflect` adds a runtime input with an inferred or
explicit type, a checked default, and a stable key. Root keys are binding names;
imported-module keys are `module/path.sanim::name`. Renaming a key is an API
change. Reflected inputs and ordinary bindings are both known during preparation.
Inputs cannot be declared conditionally or inside helpers.

Standard names are already available and cannot be redefined in any scope.
Shadowing enclosing bindings is also an error. Case matters: `color` is valid;
`Color` is protected. Records use structural checking with exact field sets.
Generic functions and record aliases use `<T>`, support defaults such as `<T = Data>`,
and constraints such as `<T: Numeric>` or `<T: Readable = Data>`.
`Unknown` is internal inference state and cannot be declared as a source type. Enums support payloads and
exhaustive `match` with `case Variant(binding) { ... }` arms.

Values include `Number`, `Bool`, `String`, `Color`, `Length`, `Duration`, `Angle`,
`Rate`, `Percent`, `Fraction`, `Vec2`, `Vec3`, `Vec4`, `Matrix4`, `Element`, `Motion`, `Font`, `List<T>`, `Option<T>`,
`Table<T>`, `Array<T>`, `Dict`, `Data`, records, and enums. Vector component units can be made
explicit with `Vec2<Length>` or `Vec2<Percent>`. A vector inferred from
`Vec2(25%, 50%)` retains its percentage units. Accessing `.x`/`.y` requires known
component units. List/array indexing and `get` return `Option<T>`; `unwrap(value, fallback)`
handles missing items without unchecked indexing.

`Color("blue3")`, `Color("orange3")` and `Color("window_bg_dark")` use the existing
shared Shrimply Adwaita palette. All shared palette names are available, including
accent and light/dark semantic colors, case-insensitively with their underscores.
The definitions live once in `shrimply-math-color`; Sanim does not copy the palette.
`Color()` defaults to Adwaita `red3`. Basic CSS-style names such as `"blue"` keep
their literal meaning, and `#RRGGBB`/`#RRGGBBAA` remain available for custom colors.
Palette choices are explicit and independent of the desktop theme for portable output.

Arithmetic and decimal literals use checked rational values. Time never uses
floating point. Units must agree; `96px` is explicitly converted to `72pt`.
Numeric overflow and division by zero are preparation errors.

`Fraction(numerator, denominator)` is an explicit exact rational value with
`.numerator` and `.denominator` inspection. Vectors support component access,
matching-unit addition/subtraction, scalar multiplication/division, and dimensionless
`dot(a, b)`. Plain `Vec2`/`Vec3`/`Vec4` annotations mean dimensionless vectors;
use `<Length>` or another unit when needed. Matrices are row-major: `Matrix4()`
is identity, and `Matrix4([[...], ...])` checks a 4×4 shape. Matrix arithmetic,
`transpose`, `inverse`, and multiplication by `Vec4` use finite `f64` arithmetic.
Vector and matrix storage defaults to `f64`; timeline time and explicit `Fraction`
values remain rational. Non-finite components, division by zero and floating
overflow fail explicitly. Matrix inversion uses partial pivoting.
`Vec3` is never silently extended to homogeneous coordinates. These native types
retain their schemas in reflection and typed JSON/TOML/NPY data loading.

`Circle`, `Rect`/`Rectangle`, `Line`, `Polygon`, `Text`, `Group`, `Row` and `Column`
construct immutable elements. `MoveTo`, `Shift`, `FadeIn`, `FadeOut`, `RotateTo`,
`ScaleTo`, `FillTo`, `Create`, `Write`, `Uncreate`, `Unwrite`, `GrowFromCenter`,
`ShrinkToCenter`, `ExpandX` and `StretchXTo` construct motions. `play` schedules one or more motions
in parallel, then advances the script cursor; `wait` advances it without motion.
`add` inserts a still element. Functions may schedule animation. There is no
per-frame script execution.

`Create` exposes vector path segments with a trace and progressively filled path.
`Write` traces the outline during the first half, then fills it during the second.
`Uncreate` and `Unwrite` reverse those progressions from the current reveal state.
These use the actual Skia paths, including prepared Pango glyph outlines; they are
independent of opacity. A group reveals its children in parallel without changing
layout. Overlapping reveals on an ancestor and descendant are rejected: reveal
the group or use separate plays. Reveal traces remain inside the original bounds.

`GrowFromCenter` scales from zero to the element's declared scale;
`ShrinkToCenter` scales the current state to zero. They require center alignment,
zero rotation throughout the motion, and a target whose size is not controlled
by a parent flow layout. Errors suggest `ScaleTo` or animating the containing
group. `ScaleTo` remains a positive-scale animation using the existing anchor.
`ExpandX(element, origin=50%, offset=0pt)` expands a group from zero width to full
width without changing vertical coordinates or layout dimensions. Its origin is
relative to the group's layout width, with an optional local pt offset (for
example the median of a box plot). Primitive defaults use their local origin;
groups default to 50%. `StretchXTo(element, to, ...)` continues from the current
horizontal scale. Stretching uses native Skia transforms; continuous bounds
proofs conservatively report unknown for this transform.
See [`examples/animations.sanim`](examples/animations.sanim) for the full supported set.

Supported easings are `linear`, `cubic_in`, `cubic_out`, and `smooth`. Tracks hold
their final value. Duplicate property writers in one `play` are errors. Position
track endpoints use matching units: both percentages or both pt. A zero offset
adopts the other endpoint’s units because it has the same meaning in either mode.
Explicit element IDs are recommended for stable identity across source edits;
otherwise IDs derive from source locations and bounded invocation paths.

`for item in collection` traverses finite collections. `range(end, start: 0,
step: 1)` constructs a bounded nonnegative integer range. `repeat count { ... }`
executes sequentially. `timer count every period { ... }` starts iterations at
fixed intervals and pads the remaining time; an overrun is an error. Counts,
nesting, total preparation operations, source size, and scene size are bounded.
Recursive call graphs and copying functions into values/containers are rejected.
Every expression, statement and function carries a timeline effect. `Engine::duration`
and CLI `duration` resolve exact total length without building unrelated geometry.
Timing-dependent reads are evaluated; unrelated data and visual validation can be
skipped, so a duration report does not replace full preparation before rendering.
State-observing timing can fall back to full preparation with a `P004` warning.
All animations are finite: resolved duration above two hours is an error;
exactly two hours is valid. Overflow uses structured diagnostics, never an unbounded loop.

## Relative layout

`pt` is the logical layout unit. Positions are parent-local `Vec2` offsets.
There is no separate point or position wrapper type. Horizontal and vertical
alignment are independent element fields, `x_align` and `y_align`, using the same
`Align.Start`, `Align.Center`, and `Align.End` values. On the horizontal axis these
mean left, center and right; on the vertical axis they mean top, center and bottom.
Alignment matches the corresponding element and parent edges, then adds the offset.

```text
Rect(
    width= 100pt,
    height= 40pt,
    x_align= Align.End,
    y_align= Align.Center,
    position= Vec2(-12pt, 0pt),
)
```

Ordinary shapes default to center alignment and zero offset. Groups default to
start/start alignment. Percentage offsets resolve against parent dimensions.
`Row` lays children out with gaps and wraps when its available width shrinks;
`Column` stacks them. Flow controls immediate child positions: conflicting position
motions fail explicitly. Groups used as flow children need explicit dimensions.
Rotated shapes need explicit placement when their flow extent is unavailable.

Canvas dimensions, frame rate and DPI belong to the host, never to the script.
CLI `--width`, `--height`, `--fps` and `--dpi` supply that viewport; logical pt size
is pixels × 72 / DPI. Browser/CSS reference DPI is 96, so `96px` equals `72pt`.
Scripts can set `background(color)`. A host can call `Animation::layout` with new
logical dimensions without reparsing. `Row`/`Column` use Taffy flexbox with padding,
gaps, wrapping, `Justify` and independent cross-axis `Align`; Taffy layout uses f32
geometry while timeline arithmetic remains exact.

## Data and modules

```text
import "./theme" as theme
type Sample = { id: String, value: Number }
let samples = read<Table<Sample>>("samples.csv")
// Equivalent contextual inference:
let other: Table<Sample> = read("samples.csv")
```

Module imports are relative, qualified and cycle-checked. They cannot execute
timeline effects during import. Imported helpers may schedule when explicitly
called by the root. Reads occur during preparation, with one immutable snapshot
per resource. File sources enforce a root, including canonical symlink checks.
Hosts can implement `Sources` or use `MemorySources` without filesystem access.

CSV requires a record row schema and an exact header; quoted fields, escaped
quotes, CRLF, and multiline fields are supported. Empty optional cells decode to
`None`. JSON, TOML and YAML validate fields recursively. Both `.yaml` and `.yml` work;
YAML aliases consume expanded byte/value/depth budgets and duplicate keys fail. NPY supports C-order scalar
numeric/boolean arrays and retains shape; NPZ maps named arrays to record fields.
Object/pickle arrays, structured dtypes, Fortran order and unsupported formats
fail explicitly. NPY floating data must be finite and representable. Limits
cover decoded values, nesting, bytes, cumulative binding storage and NPZ expansion. Fonts load through
`read<Font>("font.ttf")`; Text requires an explicit font resource.

## Verification and diagnostics

```text
assert(circle.above(rect))
check(circle.disjoint(rect), message= "Objects should not overlap")
assert(always(inside_canvas(circle)))
```

Relationships include `above`, `below`, `left_of`, `right_of`, `contains`,
`overlaps`, `disjoint`, and `touches`, with optional nonnegative `gap` for the
directional relations. Their current geometric meaning is axis-aligned layout
bounds, not rendered visibility. Plain relationships are checked at the current
script time. `always` requests a conservative continuous proof over the prepared
timeline. Supported fixed-geometry translations use monotonic interval envelopes.
Animated scale/rotation and hierarchical motion produce
`Unknown`; endpoint sampling never masquerades as a continuous proof.

`assert` requires proof and fails on `Refuted` or `Unknown`. `check` records
evidence without rejecting the animation. Boolean assertions check known values.
Diagnostics carry stable codes, module paths, byte spans, line and column. Visual
failures identify objects, expected relation, bounds and counterexample time
where available. CLI `--json` preserves structured diagnostics.

## Caching, hot reload and performance

`Session::poll` compares content hashes for all source/data dependencies, including
failed reads, so creating a previously missing resource recovers automatically.
Every successful revision is prepared atomically. Circular imports fail at the
closing import with the full cycle and guidance to extract shared declarations. Failed edits return diagnostics
and preserve a clearly identified last-good revision. Dependencies are rechecked
before accepting a prepared revision so files changed during preparation cannot
silently produce a mixed update.

Parsed modules, pure expression results and decoded assets are cached separately.
Keys cover semantic version, expression structure, inputs, invocation identity,
data schema, content and relevant limits. Expression and asset/query caches each
have an aggregate byte cap as well as an entry cap. Tables and arrays share
immutable storage across bindings and warm preparations. Unchanged dependencies are reused;
source/schema/input changes invalidate affected expression results. Scheduling
effects and user calls before a valid execution checkpoint are skipped entirely.
Checkpoints store persistent bindings, scene/timeline indexes, cursor, reflected
inputs, assertions, resource/function versions and consumed logical budgets.
`imbl` maps/vectors share tree nodes; immutable data, strings, geometry, fonts and
runtime schemas share their payloads. Restoring a retained boundary does not copy
large tables or geometry. The current checkpoint granularity is a root statement;
an edited loop or function invocation resumes before its enclosing root statement.
Changed code, inputs or files roll back before their earliest dependent use.
Hoisted declarations and types are checked afresh, and deferred visual assertions
are verified against the completed new timeline. Failed suffixes preserve usable
prefix states. Checkpoints have independent entry/estimated-byte accounting under
the host cache limits; `Engine::execution_state(boundary)` directly returns a
retained immutable state root. Statistics report restored statements/time separately
from work actually executed on this attempt.

`Prepared::time_map` links source spans and repeated helper calls to exact timeline
ranges and root boundaries. `at_frame(frame)` uses a direct frame table (O(1));
source overlap queries use an augmented interval index. The table is built with a
boundary sweep, and its size is checked against `Limits::max_time_map_bytes`
before allocating. There is no timeline scan during frame lookup.

Scene changes identify affected element IDs. `TimelineDiff` computes conservative
visual invalidation intervals; recording/layout/raster caches preserve unchanged
intervals across edits, including an earlier prefix and later reconverged state.
Computed layouts, Skia command recordings and raster outputs use separate
byte/entry-bounded LRU caches. Identical hold frames share a canonical state.
Renderer surfaces, compositing and path work have independent budgets.
`Renderer::prepare` caches paths and contour lengths by immutable geometry content,
so color, opacity, timing and viewport edits reuse geometry. This cache is bounded
by entries and estimated path bytes. Completed contours are appended without
remeasurement; a partial contour is measured when sampled. `Scene::new` is the
convenience API for a standalone scene; retain a Renderer for hot-edit reuse. Render
results expose measured rasterization time; CLI render/watch events also report
the complete render-and-write time.

Preparation reports wall time, operation counts, parsed/reused modules,
evaluated/reused expressions and decoded/reused assets. Source-located `P001`
warnings identify statements exceeding an operation budget; `P002` reports
preparation latency, and source-located `P003` reports expensive SQLite queries. Hosts configure thresholds and hard limits through `Limits`.
The benchmark reports measured cold/warm preparation and direct frame sampling.
These are local measurements, not portable latency guarantees.

## Headless Rust API

```rust,no_run
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

## Current boundaries

This implementation does not have complete Manim feature parity. It has no audio,
physics, arbitrary per-frame callbacks, LaTeX typesetting, or general path morphing
(`Transform`/`ReplacementTransform`). Pango shapes explicit font assets into vector
outlines, including complex scripts; bitmap/color glyphs fail explicitly.
Skia records vector commands and replays them directly on native GPU canvases.
CPU rasterization is explicit for headless PNG/FFmpeg output. Timeline evaluation
remains exact; layout and rasterization use floating geometry. Bit-identical output
across platforms is not promised. Unsupported verification returns `Unknown`;
groups/hierarchical relations currently require additional evidence.


## Unstructured data and dictionaries

`Dict` has string keys and mixed, unstructured values. It has no type parameter.
Quoted keys distinguish dictionary literals from fixed-field record initialization.
Dictionary access uses brackets; dot access is a type error.

```text
let settings = read<Dict>("settings.yaml")
let title = decode<String>(settings["presentation"]["title"])
let mixed: Dict = {"name": "demo", "count": 4, "nested": {"enabled": true}}
let count = decode<Number>(mixed["count"])
let fallback = decode<String>(unwrap(get(mixed, "subtitle"), Data("Untitled")))
```

Bracket access stops at the first missing key or incompatible container and names
the access path, missing key and available keys. Nested access preserves the
original failure; a missing parent is never turned into a later null/type error.
`get` is the optional lookup, returning `Option<Data>`. `keys` lists dictionary
keys. `Data` represents arbitrary scalar, array or mapping data; `decode<T>`
validates it at the point a known type is needed. Exact scalars and durations
roundtrip through Data using exact integer/rational encodings, without f64 coercion.

## SQLite queries

Queries use actual bundled SQLite, with an in-memory database populated from the
explicit table bindings. SQLite parses the SQL. Plain queries return `Table<Dict>`;
an explicit row type opts into result-column and value validation.

```text
type Sample = { category: String, amount: Number }
let samples = read<Table<Sample>>("samples.csv")
let totals = query(
    "SELECT category, SUM(amount) AS total FROM samples WHERE amount >= :minimum GROUP BY category ORDER BY category",
    tables= {samples= samples},
    params= {"minimum": 10},
)
for row in totals {
    let total = decode<Number>(row["total"])
}
```

`query<Summary>(...)` returns `Table<Summary>` and checks exact column names,
including empty results, and every returned cell. Nullable columns need
`Option<T>`. SQLite uses its own INTEGER/REAL numeric semantics; explicit Fraction
columns are exact rational text for roundtrips; SQLite numeric operators do
not implement rational arithmetic on those text columns. Scalar columns and nullable scalars are supported.
Queries cannot mutate/attach databases, load extensions, access ambient clock or
randomness. Instruction, database, result byte and value budgets bound work.
Time functions are disabled; preprocess dates explicitly when needed. Specify
`ORDER BY` whenever row order matters. Results are cached by SQL, bound table
content, parameters, result schema and limits; data edits invalidate the cache.

## Background runtime and editor

`sanim-runtime::Runtime::open(path)` prepares on a worker and watches source,
module and `read` dependencies through native filesystem notifications. Short
write bursts are debounced; periodic content polling recovers missed events.
`Runtime::snapshot()` performs no source reads or preparation on the caller's
thread. It exposes last-good prepared content, the current diagnostic, revision,
changed element IDs, early planned duration and reflected-input acknowledgement.
Its progress snapshot distinguishes initial loading, reloading with last-good
content, failure and readiness, including the current resource and elapsed time.
Notifications are coalesced; native previews draw that feedback as Skia commands
on the compositor's GPU canvas. Strict exports return diagnostics instead of
rendering loading/error screens.
`set_waker` notifies the host without polling; callbacks run outside state locks. `set_inputs` accepts a
complete JSON override snapshot; values are checked against the newly compiled
source schema, so a type change can be repaired with matching input values.

The editor accepts `.sanim` files through timeline drag/import. The editor canvas
is the logical viewport, overriding standalone canvas dimensions for layout.
Required visual contracts are revalidated for that viewport. Native Sanim
clips persist their source path and reflected overrides. Shared Vulkan/Metal
preview and CUDA export use the same runtime and Skia recordings; the runtime
wakes paused previews when source or data dependencies change. Exports reject initial preparation errors and pin one prepared source/data
revision per clip for the export session; later file edits do not mix revisions
within that clip. The timeline clip envelope remains an editor setting after
reload; extending source content does not automatically move neighboring clips.
The editor replays vector recordings onto its existing GPU canvas, avoiding a
CPU→GPU→CPU→GPU pixel round trip. No Python process or intermediate video is required. Reflected values are available through the
headless schema API; dedicated Sanim inspector controls are not implemented yet.
