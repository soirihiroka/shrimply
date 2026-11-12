# Sanim

Sanim is a standalone, headless animation scripting language for Shrimply. It
has its own Rust workspace so language development and tests do not build the
editor, GPU kernels, Python, or GUI dependencies.

```text
reflect tint: Color = Color("orange")

fn make_marker(identity: String, radius: Length = 24pt): Element {
    return Circle(id: identity, radius: radius, fill: tint)
}

let marker = make_marker("marker")
play(FadeIn(marker), duration: 250ms)
repeat 2 {
    play(MoveTo(marker, Vec2(30%, 0%)), duration: 1s)
    play(MoveTo(marker, Vec2(-30%, 0%)), duration: 1s)
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
```

Reflected inputs accept typed JSON overrides. For example:

```sh
make -C sanim run ARGS='check examples/hello.sanim --set repetitions=4'
make -C sanim run ARGS='check examples/data.sanim --set bar_color="\"#FF8800\""'
```

The CLI has `check`, `inspect`, `render`, and `watch` commands. `--help` describes
frame/time selection, output sizing, and bounded watch sessions. `inspect`
exports source dependencies, reflected values, resolved types and symbols,
scheduling effects, verification evidence, and performance measurements as JSON.

## Crates

| Crate | Responsibility |
| --- | --- |
| `sanim-core` | Shared type/value model, checked rational arithmetic, scene tracks, relative layout, geometric verification |
| `sanim-data` | Bounded schema-driven CSV, JSON, TOML, YAML, NPY and NPZ decoding; no filesystem access |
| `sanim-lang` | Handwritten lexer/parser, one AST, type checking, preparation, reflection, caching and hot-reload sessions |
| `sanim-render` | CPU rasterization and PNG output; no SVG or GUI toolkit |
| `sanim-cli` | Headless commands and polling watch loop |
| `sanim-runtime` | Background preparation, filesystem dependency watches and immutable published revisions |

## Language

The root file is the entry point. Blocks use braces; indentation is cosmetic.
Statements end with a newline or semicolon. Functions use
`fn name(argument: Type = default): ReturnType { ... }`. Defaulted parameters
follow required parameters. Arguments can be named using `name: value`.

Bindings are immutable. `reflect` adds a runtime input with an inferred or
explicit type, a checked default, and a stable key. Root keys are binding names;
imported-module keys are `module/path.sanim::name`. Renaming a key is an API
change. Reflected inputs and ordinary bindings are both known during preparation.
Inputs cannot be declared conditionally or inside helpers.

Standard names are already available and cannot be redefined in any scope.
Shadowing enclosing bindings is also an error. Case matters: `color` is valid;
`Color` is protected. Records use structural checking with exact field sets.
Generic functions and record aliases use `<T>`. Enums support payloads and
exhaustive `match` with `case Variant(binding) { ... }` arms.

Values include `Number`, `Bool`, `String`, `Color`, `Length`, `Duration`, `Angle`,
`Rate`, `Percent`, `Fraction`, `Vec2`, `Vec3`, `Vec4`, `Matrix4`, `Element`, `Motion`, `Font`, `List<T>`, `Option<T>`,
`Table<T>`, `Array<T>`, `Dict`, `Data`, records, and enums. Vector component units can be made
explicit with `Vec2<Length>` or `Vec2<Percent>`. A vector inferred from
`Vec2(25%, 50%)` retains its percentage units. Accessing `.x`/`.y` requires known
component units. List/array indexing and `get` return `Option<T>`; `unwrap(value, fallback)`
handles missing items without unchecked indexing.

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
`ScaleTo`, and `FillTo` construct motions. `play` schedules one or more motions
in parallel, then advances the script cursor; `wait` advances it without motion.
`add` inserts a still element. Functions may schedule animation. There is no
per-frame script execution.

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

## Relative layout

`pt` is the logical layout unit. Positions are parent-local `Vec2` offsets.
There is no separate point or position wrapper type. Horizontal and vertical
alignment are independent element fields, `x_align` and `y_align`, using the same
`Align.Start`, `Align.Center`, and `Align.End` values. On the horizontal axis these
mean left, center and right; on the vertical axis they mean top, center and bottom.
Alignment matches the corresponding element and parent edges, then adds the offset.

```text
Rect(
    width: 100pt,
    height: 40pt,
    x_align: Align.End,
    y_align: Align.Center,
    position: Vec2(-12pt, 0pt),
)
```

Ordinary shapes default to center alignment and zero offset. Groups default to
start/start alignment. Percentage offsets resolve against parent dimensions.
`Row` lays children out with gaps and wraps when its available width shrinks;
`Column` stacks them. Flow controls immediate child positions: conflicting position
motions fail explicitly. Groups used as flow children need explicit dimensions.
Rotated shapes need explicit placement when their flow extent is unavailable.

Output pixels and logical viewport dimensions are independent. `--width` and
`--height` change raster resolution; `--logical-width` and `--logical-height`
recompute layout for a different container. A host can call `Animation::layout`
with new logical dimensions without recompiling the script.

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
check(circle.disjoint(rect), message: "Objects should not overlap")
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
effects and user calls are replayed conservatively while their pure subexpressions
can reuse cached results. This is incremental preparation, not an in-place AST
patcher. Scene changes identify affected element IDs. Renderer frame caching is
separate and keyed by immutable scene identity, time and viewport.
Renderer surfaces, compositing and glyph work have independent budgets. Render
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

This implementation is a language/runtime foundation, not complete Manim feature
parity. There is no native Sanim GPU rasterizer, audio, physics, arbitrary
per-frame callbacks, automatic layout constraint solver, LaTeX typesetting, or
path morphing. Text rasterization uses explicit fonts and glyph order; it does
not implement complex-script shaping. Timeline evaluation remains exact; layout and rasterization use
floating geometry; bit-identical output across all platforms is
not promised. Unsupported verification is explicitly reported as unknown.


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
    tables: {samples: samples},
    params: {"minimum": 10},
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
changed element IDs and reflected-input acknowledgement. `set_inputs` accepts a
complete JSON override snapshot; values are checked against the newly compiled
source schema, so a type change can be repaired with matching input values.

The editor accepts `.sanim` files through timeline drag/import. The editor canvas
is the logical viewport, overriding standalone canvas dimensions for layout.
Required visual contracts are revalidated for that viewport. Native Sanim
clips persist their source path and reflected overrides. Shared Vulkan/Metal
preview and CUDA export use the same runtime and rasterizer; paused previews
poll inexpensive runtime state so data-only edits refresh. Exports reject initial preparation errors and pin one prepared source/data
revision per clip for the export session; later file edits do not mix revisions
within that clip. The timeline clip envelope remains an editor setting after
reload; extending source content does not automatically move neighboring clips.
The editor's source bridge exports straight RGBA and skips PNG encoding. No Python process or
intermediate video is required. Reflected values are available through the
headless schema API; dedicated Sanim inspector controls are not implemented yet.
