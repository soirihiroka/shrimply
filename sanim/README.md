# Sanim

Sanim is a standalone, headless animation scripting language for Shrimply. It
has its own Rust workspace so language development and tests do not build the
editor, GPU kernels, Python, or GUI dependencies.

```text
let tint = reflect(Color("orange3"))

fn make_marker(identity: String, radius: Length = 24pt): Element {
    return Circle(id= identity, radius= radius, fill= tint, position=Vec2(0%, 0%))
}

let marker = make_marker("marker")
FadeIn(marker, duration=250ms)
repeat 2 {
    MoveTo(marker, Vec2(30%, 0%), duration=1s)
    MoveTo(marker, Vec2(-30%, 0%), duration=1s)
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
make -C sanim run ARGS='check examples/hello.sanim --set hello.sanim::repetitions=4'
make -C sanim run ARGS='check examples/data.sanim --set data.sanim::bar_color="\"#FF8800\""'
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
| `sanim-cli` | Headless commands and event-driven watch sessions |
| `sanim-runtime` | Background preparation, filesystem dependency watches and immutable published revisions |

## Language

The root file is the entry point. Blocks use braces; indentation is cosmetic.
Statements end with a newline or semicolon. Functions use
`fn name(argument: Type = default): ReturnType { ... }`. Defaulted parameters
follow required parameters. Arguments use `name = value`; record values use `{ field = value }`.
Colons declare types (`name: Type`) and separate Python-style dictionary entries
(`{"key": value}`). Named arguments and record values require `=`; colon-style
calls are rejected with a migration diagnostic.

`let` stores an immutable value snapshot. `mut` declares a writable binding, and
`sig` declares a read-only derived value:

```sanim
mut a = 1
sig b = a * 2
let saved = b
a = 2
assert(b == 4)
assert(saved == 2)
```

Ordinary strings interpolate expressions and Python-style format specifications;
no `f` prefix is needed. Raw `r"..."` strings disable interpolation while retaining
normal escapes such as `\n` and `\"`. Double braces write literal braces.

```sanim
let num = 42
let binary = "bits={num:08b}"
let fraction = "share={0.125:.1%}"
let literal = r"{this is literal text}"
mut count = 1
sig caption = "count={count}"
count = 2
assert(caption == "count=2")
```

`let` captures the current value; `sig` tracks the interpolation's dependencies.
Nested format widths and precisions (`"{value:{width}.{precision}f}"`) and
`!s`, `!r`, and `!a` conversions use the same bounded formatter as `str`.

Triple quotes (`"""…"""` or `'''…'''`) preserve multiline text, indentation,
and line endings. They use the same interpolation and escape rules. Prefix
with `r` to keep braces literal, such as `r"""{literal text}"""`.
The source formatter leaves string contents unchanged.

`read<String>("notes.txt")` reads verbatim UTF-8 with any filename extension,
including newlines. File contents are never interpreted as string interpolation.
Demanded text files become watched hot-reload dependencies like CSV/JSON inputs.
Use `read<Data>` followed by `decode<String>` to decode a structured JSON string
instead of reading its source text verbatim.

Only `mut` bindings accept assignment, and their inferred or declared type stays
fixed. Use an annotation for defaults such as `mut maybe:Option<Number> = None`.
Assignments evaluate their right side against the current state before replacing
the value. `let`, mutable initializers, function arguments and return values take
snapshots; they do not become references to another reactive binding.

Signals evaluate lazily and reuse their value until a mutable dependency changes.
Dependencies include reads through other signals and pure helper functions, and
follow the branch actually evaluated. Helpers may assign captured mutable bindings;
function-local bindings are fresh on each invocation. Signal expressions must be
pure: helpers may use bounded local mutation, but cannot assign captured
bindings, schedule animation, or run assertions. Dependency cycles produce diagnostics.

Imported module members read the module's current values. A helper that updates
its module's mutable binding is visible through the next member read, while a
previous `let saved = module.value` keeps its snapshot. Mutations run during
bounded script preparation; signals do not install arbitrary per-frame callbacks.
Hot-reload checkpoints preserve the reactive state
along with the animation timeline, and inspection reports resolved values.

Supported constructor properties retain expressions that depend on mutable cells
or signals, including reads through pure helpers. Fixed values remain fixed;
unsupported live arguments fail explicitly. A bounded
background job animates a mutable input without advancing the main script cursor:

```sanim
mut phase = 0deg
sig size = 100pt + sin(phase) * 25pt
let rectangle = Rect(width=size, height=size, radius=size / 6)
let circle = Circle(radius=size / 2)
add(rectangle)
add(circle)
background {
    animate(phase, to=1800deg, duration=10s, ease="linear")
}
wait(10s)
```

Directly bound constructors infer their ID from the variable name (`rectangle`
and `circle` here). Repeated function/loop instances receive a stable instance
suffix, imported module globals are qualified, and an explicit `id=` overrides
inference. Copying an existing element preserves its identity.

This scene lasts ten seconds. Both shapes sample the same retained formula once
per frame. Playback uses a bounded native expression graph and exact cell tracks,
without evaluating source syntax or calling user functions. Scale, dimensions,
position, rotation, fill and opacity use the scene's property sampler. `Rect` also
accepts an animated corner `radius`, defaulting to zero; drawing clamps it to half
the shorter edge. Circle radii remain strictly positive. A `let`
snapshot of `size` passed instead would keep its fixed value. Overlapping cell
writers and a property controlled by both a signal and a motion are errors.
Continuous geometric proofs conservatively report unknown for retained formulas;
explicit frame checks evaluate their actual sampled geometry.
Background assignments preserve their scheduled time for scalar values and typed
configuration values, including fonts and code documents. Fixed child lists can
contain shapes with reactive properties; arbitrary replacement of shape topology
requires an explicit list edit or a component supporting changing content.

`Label` and `Text` accept signals for text, font, font size, fill, alignment and
transforms. Inline formatted text stays reactive:

```sanim
mut count = 0
mut typeface = Font("sans-serif")
let title = <Label font={typeface} font_size={24pt}>Count = {count:.0f}</Label>
add(title)
animate(count, to=10, duration=1s, ease="linear")
typeface = Font("monospace")
wait(1s)
```

Changed text, font or size resolves through the shared native outline cache before
flow layout, alignment and picking. Equal inputs reuse their geometry; reverse
seeks reuse sampled frames and outlines. Continuously changing font size creates
new shaped values; animate `scale` when the text layout can remain fixed. Native
hosts use `Animation::layout_with` to resolve pending content; pure core bounds
checks report unresolved shaping rather than using stale text dimensions.

Flow layouts recompute child extents for animated dimensions. They own child
positions, so retained position writers on those children are rejected.

`reflect(value)` is an expression that exposes a typed
runtime input with a checked default, a config key, and a display label:

```sanim
let color1 = reflect(Color(), label="Display Color")
let accent = reflect(Color("blue3"), key="palette.accent", label="Accent Color")
let grid_line_width = reflect(2pt)
let title = reflect("title") + "test"
let optional_count:Option<Number> = reflect(None)
let style = {fill=reflect(Color()), radius=reflect(12pt)}
add(Circle(id="marker", fill=reflect(Color(), key="marker.fill")))
```

The first positional argument is always the default value. Named `key` and
`label` metadata must be static string literals. Reflection preserves the
default's type, including native colors, compound values and contextually typed
`None`. In the title example, the input default is `"title"`; the resulting
binding is `"titletest"`. Overriding the input replaces only the wrapped value,
then the surrounding expression runs normally.

Default keys are `relative/file.sanim::variable_name`, including the entry file,
relative to the animation source root. Nested record fields and named constructor
arguments add their field path: the style above exposes `style.fill` and
`style.radius` after the file prefix. A sole reflection under an operator keeps
the binding name; multiple operator inputs gain distinct `left`/`right` paths.
Anonymous constructors use a semantic path such as `Circle.fill`; use explicit
keys when repeated anonymous shapes need independent controls or when identity
must survive code restructuring. Default labels split snake_case, camelCase, acronym
boundaries and numeric suffixes (`grid_line_width` → “Grid Line Width”, `color1`
→ “Color 1”). Labels can change without changing config identity. Explicit keys
are used exactly as written and remain stable when the variable is renamed.
Duplicate keys anywhere in the import graph fail with both declaration locations.
Overrides accept only the canonical keys reported by reflection metadata. Bare
variable names do not act as aliases; explicit keys are used exactly as written.

Reflection metadata includes the variable name, label, source span, complete type,
default and current value. The compiler and hosts share `reflection::Schema` for
key resolution and validation, including records, lists and optional values.
Resetting an input removes its override; JSON null remains a valid Option/Data
value. Reflected inputs and ordinary bindings are both known during preparation.
Inputs can control bounded loops and exact duration planning. Reflection must be
evaluated once at module scope: it cannot appear inside helpers, loops, branches,
short-circuited operands, signal initializers or another reflection. Defaults cannot schedule animation
or run assertions; bind the input first, then use it in the animation script.

Standard names are already available and cannot be redefined in any scope.
Shadowing enclosing bindings is also an error. Case matters: `color` is valid;
`Color` is protected. Records use structural checking with exact field sets.
Record types compose with `&` and subtract fields with `-`:

```sanim
type RunInfo = {run:String, n:Number}
type Measurements = {x:Number, y:Number}
type Sample = RunInfo & Measurements
type Pair = RunInfo & {left:Sample, right:Sample}
type Coordinates = Sample - RunInfo
```

Composition produces one flat record schema. Equal overlapping fields merge;
conflicting field types fail with the field name and both types. Subtraction
removes named fields and rejects names absent from the left side. Parentheses
make grouping explicit. The resulting schemas work with generic aliases and
typed data loading; there is no runtime class hierarchy.
Generic functions and record aliases use `<T>`, support defaults such as `<T = Data>`,
and constraints such as `<T: Numeric>` or `<T: Readable = Data>`.
`Unknown` is internal inference state and cannot be declared as a source type. Enums support payloads and
exhaustive `match` with `case Variant(binding) { ... }` arms.

String literal types describe closed choices:

```sanim
type Variant = "before" | "after"
fn select_variant(variant:Variant):Variant { return variant }
let selected = select_variant("before")
```

These schemas work in parameters, record fields, signals, reflection and typed
reads. Immutable discriminants narrow inside equality branches. Mutable and
reflected string defaults infer `String`; annotate a closed union to restrict
their values. An unresolved `before` in this argument suggests `"before"`.
Editor completion inserts correctly escaped choices. `read<Variant>("mode.txt")`
validates verbatim UTF-8, including any newline; it does not trim the input.

Values include `Number`, `Bool`, `String`, `Color`, `Length`, `Duration`, `Angle`,
`Rate`, `Percent`, `Fraction`, `Vec2`, `Vec3`, `Vec4`, `Matrix4`, `Element`, `Font`, `List<T>`, `Option<T>`,
`Array<T>`, `Dict`, `Data`, records, and enums. Vector component units can be made
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
Animations default to `window_bg_dark`; use `bg(Color("transparent"))`
for a transparent background or `bg(...)` to choose another color.
Palette choices are explicit and independent of the desktop theme for portable output.

Arithmetic and decimal literals use checked rational values. Time never uses
floating point. Units must agree; `96px` is explicitly converted to `72pt`.
Numeric overflow and division by zero are preparation errors.
Explicit approximate math functions bound their rational result denominator to
one billion, retaining headroom for follow-up arithmetic and suppressing tiny
trigonometric roundoff. Source literals, explicit fractions and time remain exact.

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

`Circle`, `Rect`/`Rectangle`, `Line`, `Polygon`, `Text`/`Label`, `Box` and `CodeView`
construct immutable elements. `MoveTo`, `Shift`, `FadeIn`, `FadeOut`, `RotateTo`,
`ScaleTo`, `FillTo`, `Create`, `Write`, `Uncreate`, `Unwrite`, `GrowFromCenter`,
`ShrinkToCenter`, `ExpandX` and `StretchXTo` schedule directly and advance the script
cursor. They return `Unit`, with a default duration of 300ms. Give a list of targets
to animate them together: `FadeIn([heading, subtitle], duration=400ms)`. Sequential
calls advance in order; use `background { ... }` for independent concurrent work.
When animating a box and its children in separate calls, `add(box)` first to
establish the hierarchy before scheduling child animations.
`wait` advances time without changing the scene.
`add` inserts a still element. Functions may schedule animation. There is no
per-frame script execution.

`Create` exposes vector path segments with a trace and progressively filled path.
`Write` traces the outline during the first half, then fills it during the second.
`Uncreate` and `Unwrite` reverse those progressions from the current reveal state.
These use the actual Skia paths, including prepared Pango glyph outlines; they are
independent of opacity. A group reveals its children in parallel without changing
layout. Overlapping reveals on an ancestor and descendant are rejected: reveal
the group or use sequential calls. Reveal traces remain inside the original bounds.

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

The default easing is `spring`: a critically damped physical response that stays
between its endpoints. `spring_bouncy` adds restrained overshoot for unconstrained
motion. Both are stateless closed-form curves with exact bounded start/end times
and a smooth final settle; seeking never runs a physics integration loop.
List reflow uses the spring, while entry/exit use a monotonic fade and a subtle
92%–100% centered scale instead of shrinking whole items to a dot.

Other easings are `linear`, `cubic_in`, `cubic_out`, `smooth` (cubic),
`smootherstep` (quintic, matching ManimGL's `smooth`), and `overshoot` (ManimGL's
title entrance curve for positions/fades). Calls accept `ease=...`, for example
`MoveTo(title, Vec2(0pt, 0pt), ease="spring_bouncy")`. Use `ease="linear"` for
constant-speed phase or clock inputs such as a sine oscillator. Tracks hold
their final value. Overlapping writers of the same property are errors. Position
track endpoints use matching units: both percentages or both pt. A zero offset
adopts the other endpoint’s units because it has the same meaning in either mode.
Bound variables infer stable element IDs automatically; explicit `id=` is optional.
Unbound constructors use source locations and bounded invocation paths.

Native chart elements share one explicit coordinate mapping:

```sanim
let xs = LinearScale(domain=Vec2(0.78, 1.28), range=Vec2(-275pt, 309pt))
let ys = LinearScale(domain=Vec2(0, 4), range=Vec2(200pt, -200pt))
let ticks = [0.8, 0.9, 1.0, 1.1, 1.2]
let grid = Grid(scale=xs, ticks=ticks, spans=[Vec2(-200pt, 200pt)], highlight=[1.0])
let axis = Axis(scale=xs, ticks=ticks, format="percent", font=read<Font>("font.ttf"))
type Observation = {x:Number, y:Number}
let samples = read<List<Observation>>("samples.csv")
let points = Scatter(data=samples, x_scale=xs, y_scale=ys, radius=2pt)
let interval_box = BoxPlot(scale=xs, summary={low=0.8, q1=0.9, middle=1.0, q3=1.1, high=1.2})
add(grid)
add(axis)
background { FadeIn(points, duration=400ms, ease="smootherstep") }; ExpandX(interval_box, duration=400ms, ease="smootherstep")
```

`Axis`, `Grid`, `Scatter` and `BoxPlot` produce ordinary elements with stable child
IDs and no separate rendering path. Scales have typed `domain`, `range`, and
`clamp` fields; `scale_value(scale, value)` returns a local length. Out-of-domain
values fail unless clamping is explicit. Reversed ranges support y axes.
Axes provide bounded automatic nice ticks, custom labels, percent formatting,
vertical orientation and overlap diagnostics. `show_labels=false` retains only
axis geometry when labels need a separate entrance. Grids support disjoint
spans and reference ticks. Scatter takes a statically checked `List<{x:Number,
y:Number}>`; optional unique `keys` preserve point identities through row reorder,
and `point_opacity` controls individual overlapping marks. Mark and tick budgets
apply before allocation. Box plots validate the five-number ordering, preserve
all whiskers and caps, and give `ExpandX` a median anchor. `BoxSummary(values)`
uses linear empirical quantiles; quartiles describe sample spread, not confidence
intervals. Colors default to the shared Adwaita palette.


Native curves and distributions use those same scales and Adwaita defaults:

```sanim
let xs = LinearScale(domain=Vec2(-4, 4), range=Vec2(-240pt, 240pt))
let ys = LinearScale(domain=Vec2(0, 0.7), range=Vec2(100pt, -100pt))
let measured = [-2, -1.5, -1, -0.4, 0, 0.2, 0.7, 1, 1.7, 2]
let bars = Histogram(values=measured, bins=16, normalization="density", x_scale=xs, y_scale=ys)
let estimate = DensityPlot(values=measured, bandwidth=0.5, x_scale=xs, y_scale=ys, fill=Color("orange3"))
let model = NormalCurve(mean=0, std_dev=1, x_scale=xs, y_scale=ys, shade=false)
Create([bars, estimate, model], duration=1s)
wait(2s)
```

- `LinePlot(data=List<{x:Number,y:Number}>, ...)` draws a single open path.
  Linear interpolation preserves data order; `interpolation="monotone"` requires
  increasing x and adds a bounded, shape-preserving cubic interpolation. It does
  not invent extrema between measurements. `subdivisions` controls resolution.
- `AreaPlot` fills a curve to `baseline=0`; `BandPlot` fills explicit
  `List<{x:Number,low:Number,high:Number}>` bounds. Bands require increasing x
  and `low <= high`; they do not infer a statistical confidence interval.
- `Histogram` accepts a bin count or explicit increasing `edges`. Bins are
  left-inclusive/right-exclusive, with the final right edge included. `count`
  gives sample counts, `probability` gives bin probabilities, and `density`
  divides by each bin's width so the area is one. Empty inputs give empty bars.
  Every sample must lie inside the bin edges; no measurements silently disappear.
- `DensityPlot` is an exact Gaussian kernel density estimate, with optional
  positive `bandwidth`. The automatic rule is Silverman's robust rule using the
  smaller of sample standard deviation and IQR/1.34. Constant datasets need an
  explicit bandwidth. The estimate is normalized over the real line; cropping
  its displayed domain does not renormalize it. `points` and host operation
  limits bound the sample-by-grid work.
- `NormalCurve` samples the analytic normal density with explicit `mean` and
  `std_dev`; it shares `points`, `shade`, and `fill_opacity` with `DensityPlot`.
- `ECDFPlot` produces right-continuous empirical steps and merges duplicate
  values. `DataRange(values, padding=0.05, include_zero=false)` computes a finite
  scale domain, including a sensible nonzero range for constant data.
- `Legend(labels=[...], colors=[...])` uses the shared native font, theme colors,
  and line swatches. Omitting colors cycles through Adwaita colors; `vertical`
  switches its flow. Axis `format` also accepts the shared Python-style formatter,
  such as `.2f` or `.0%`, alongside the `number` and `percent` presets.

`Polyline(points=List<Vec2<Length>>, width=2pt)` is the same batched, stroked
primitive used by curves. It supports native reveal animations and animated line
width. Plot positions remain parent-relative, and a plot's group can be animated
without rebuilding its path. Changing measured data creates new immutable
geometry that participates in the same hot-reload diff and path caches.
DataFrame/Series inputs materialize explicitly at the plot boundary: use
`Histogram(values=df["value"].to_list(), ...)` or
`LinePlot(data=df.select(["x", "y"]).to_list(), ...)`. Those conversions use the
existing typed list representation, not another plotting-specific table type.

See [`examples/adwaita_plots.sanim`](examples/adwaita_plots.sanim), and run
`make plot-bench` to compare a 1,000-point path with separate line elements and
measure 10,000-sample histogram/density work.

`for item in collection` traverses finite collections. `range(end, start: 0,
step: 1)` constructs a bounded nonnegative integer range. `repeat count { ... }`
executes sequentially. `timer count every period { ... }` starts iterations at
fixed intervals and pads the remaining time; an overrun is an error. Counts,
nesting, total preparation operations, source size, and scene size are bounded.
Recursive call graphs and copying functions into values/containers are rejected.
Every expression, statement, block and function carries a timeline effect, including
ordinary operations whose duration is zero. Native and user functions use the same
contract; `wait` is an ordinary zero-value function with a duration argument.
Every builtin accepts optional `duration=...` (normally zero; direct animation functions default to 300ms) and `ease=...`. A user function can
also receive a call-duration envelope: its body retains its own timing, remaining
time becomes a hold, and exceeding the requested duration is an error. The envelope
includes concurrent jobs started by that function body, including nested helpers;
unrelated work scheduled before the call does not count against it. A declared
parameter named `duration` remains an ordinary parameter whose body decides timing.
Argument/default-expression effects occur before the call-body envelope.
Assignments interpolate numeric, fraction, color and vector state over the returned
call's interval. Arguments finish first; a zero-time call commits immediately after
its arguments. Other typed state (such as strings, records, arrays and union values)
commits at the interval end. Timing is independent of the returned value's type.

Durations use exact units: `0.5s`, `250ms`, `7m`, `4m1s`, and `1s250ms` are valid.
Compound units appear once in decreasing order (`m`, `s`, `ms`). Durations cannot
be confused with dimensionless numbers or angles.

`Engine::duration`
and CLI `duration` resolve exact total length without building unrelated geometry.
Timing-dependent reads are evaluated; unrelated data and visual validation can be
skipped, so a duration report does not replace full preparation before rendering.
If timing observes scene state, the canonical full evaluator is selected upfront
and a `P004` performance message explains the extra preparation. There is no
speculative execution followed by a second interpreter pass.
All animations are finite: resolved duration above two hours is an error;
exactly two hours is valid. Overflow uses structured diagnostics, never an unbounded loop.

## Animated lists

`append(items, value)`, `insert(items, index, value)`, `pop(items)` (last item),
`popleft(items)` (first item),
`replace_at(items, index, value)`, and `move_item(items, from_index, to_index)` return
new typed lists. Inputs remain unchanged. Empty results retain their element type.
`pop(items, index=...)` removes another position. Removing from an empty list
reports its length.
Indices must be nonnegative integers; invalid edits report the index and list length.
Lists and arrays share persistent copy-on-write vector storage; edits share unchanged
branches instead of copying every item.
Proven native edits share the core edit implementation with scene updates; only
changed item payloads and copied tree paths are validated and charged. Arbitrary
replacement expressions retain full validation.

```sanim
mut items: List<Element> = []
let row = Box(flex_direction="row",children=items, width=400pt, height=100pt, gap=12pt)
add(row)
items = append(items, Rect(width=64pt, height=64pt), duration=0.5s)
items = popleft(items, duration=300ms)
```

Assigning a timed result to a mutable list records a bounded state transition.
Logical state commits at the end; seeking restores the correct snapshot without
replaying edits. A bound flex Box matches stable element identities, fades
entries/exits, and interpolates surviving items between Taffy layouts. Removed
items are temporary overlays and do not occupy destination layout slots. Layout
responds to the host viewport during the transition. A replacement with different
geometry needs a new identity; list indices are never identities. Currently each
mutable element list supports one live view, and overlapping writes to that list
are rejected. Independent lists can animate concurrently in `background` blocks.

The complete [stack and queue example](examples/stack_queue.sanim) uses the same
append operation for arrivals, then removes the last stack item or the first queue
item. It uses no hand-authored item position tracks.

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

Ordinary shapes default to center alignment and zero offset. Box defaults to
start/start alignment, transparent fill and no border or radius. Its width fills
the available parent width; its automatic height measures children in vertical
block flow, like an unstyled div. Empty boxes have zero natural height. Percentage
offsets resolve against parent dimensions. An automatic-height parent with a
percentage-height child fails with a constraint-cycle diagnostic.

Opt into flex layout with `<Box flex flex-row>` or `<Box flex flex-col>`.
Padding, gaps, wrapping, justification, alignment and flex factors share the same
layout boundary. Flow controls immediate child positions: conflicting position
motions fail explicitly. Intrinsic leaves keep their geometry inside allocated
layout boxes; allocating more width does not stretch a circle or enlarge a font.

Canvas dimensions, frame rate and DPI belong to the host, never to the script.
CLI `--width`, `--height`, `--fps` and `--dpi` supply that viewport; logical pt size
is pixels × 72 / DPI. Browser/CSS reference DPI is 96, so `96px` equals `72pt`.
Scripts can set `bg(color)`. A host can call `Animation::layout` with new
logical dimensions without reparsing. Box layout uses Taffy with padding,
gaps, wrapping, `Justify` and independent cross-axis `Align`; Taffy layout uses f32
geometry while timeline arithmetic remains exact.

## Data and modules

```text
import "./theme" as theme
type Sample = { id: String, value: Number }
let samples = read<List<Sample>>("samples.csv")
# Equivalent contextual inference:
let other: List<Sample> = read("samples.csv")
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
cover decoded values, nesting, bytes, cumulative binding storage and NPZ expansion.
Fonts are first-class values: `Font("Google Sans, Arial, sans-serif")` uses an ordered
CSS-style family list. `Font("Google Sans", "sans-serif")` also accepts a separate
fallback. `Font()` and `Font("sans-serif")` use the generic
sans family. `Text("Hello")` and chart axis labels use this default automatically.
An explicit `font=` accepts either a family descriptor or `read<Font>("font.ttf")`
for an exact bundled asset. Family selection depends on installed host fonts; the
prepared Pango outlines are portable. Asset fonts remain isolated and report missing
glyphs instead of silently substituting a different installed font.
Sanim, editor text, and 3D text share `shrimply-fonts`: the same native family
resolver, application font registry, and cache invalidation epochs. Fonts activated
by the editor's font picker become available to Sanim on its render thread as well.

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

`Engine::preparation` selects `Preparation::Full` (the headless API default) or
`Preparation::Demand` (interactive runtime). Full preparation evaluates every
published binding so `check` and inspection expose data errors even in unused
values. Demand preparation defers expensive, zero-time immutable data expressions
such as `read`, `decode`, and `query` until their values are needed, and leaves
unread signals untouched. `Prepared::bindings` then contains available values;
`Prepared::deferred_bindings` explicitly lists unforced names with their types and
source locations. Ordinary cheap values, timeline effects, reflection, and scene
identity retain their source ordering. Deferred `let` expressions snapshot any
referenced mutable/signal values at declaration; later assignments cannot change
their results. User-function calls remain ordered because they may capture globals.
Demanded results live in persistent execution state, so prefix restoration and
resource-version invalidation also apply to deferred values. The statistics expose
`deferred_values` and `forced_values`; `make demand-bench` compares a disabled detail
view against the same view enabled, with full and demand preparation.

`str(value, format="08b")` uses the same bounded numeric/string formatter as
interpolation. Supported numeric presentations are `b`, `o`, `x`, `X`, `d`, `c`,
`f`, `F`, `e`, `E`, `g`, `G`, and `%`, with alignment/fill, sign, alternate prefixes,
zero padding, comma/underscore grouping, and precision. Strings support width,
alignment/fill and character-count precision. `conversion="s"`, `"r"`, or `"a"`
converts before string formatting. Width is limited to 16,384 characters, precision
to 1,024, and formatted output to 1 MiB. Locale-specific `n` and fractional-digit
grouping are rejected with explicit diagnostics. `str(value)` keeps its existing
representation; an explicit empty format uses decimal numeric presentation.

`Runtime` uses native filesystem notifications, not a polling timer. A notification
invalidates only the affected resource snapshots before preparation begins. Unchanged
resources reuse immutable bytes and their content hash; manual refresh invalidates
the snapshots explicitly. `Session::poll` checks the entry first and stops at the
first changed dependency. Missing resources remain watched, so creating one recovers
a failed preparation automatically. Custom `Sources` control their own I/O policy.
Every successful revision is prepared atomically. Circular imports fail at the
closing import with the full cycle and guidance to extract shared declarations. Failed edits return diagnostics
and preserve a clearly identified last-good revision. Dependencies are rechecked
before accepting a prepared revision so files changed during preparation cannot
silently produce a mixed update.

Parsed modules, pure expression results and decoded assets are cached separately.
Prepared fingerprints identify the complete semantic inputs, including source/data hashes, overrides, limits and viewport. Nonvisual source edits can publish a new revision while frame caches retain equal animation states.
Keys cover semantic version, expression structure, inputs, invocation identity,
data schema, content and relevant limits. Expression and asset/query caches each
have an aggregate byte cap as well as an entry cap. Lists share immutable storage across bindings and warm preparations. Unchanged dependencies are reused;
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

The runtime has no tracing garbage collector. Reference-counted immutable payloads
and persistent collections release ownership when their last consumer drops. Cache
eviction and dependency pruning release old revisions; an editor may deliberately
retain a last-good revision while a source edit is invalid. Shared native font and
media caches use the same byte/entry-bounded cache implementation as other resources.
`make hot-reload-memory-bench` exercises 2,000 real filesystem edits, checks old
revision release and unchanged prefix recordings, and measures RSS after warmup.

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
(`Transform`/`ReplacementTransform`). Pango shapes native font families or explicit font assets into vector
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
explicit table bindings. SQLite parses the SQL. Plain queries return `List<Dict>`;
an explicit row type opts into result-column and value validation.

```text
type Sample = { category: String, amount: Number }
let samples = read<List<Sample>>("samples.csv")
let minimum = 10
let totals = query(
    "SELECT category, SUM(amount) AS total FROM samples WHERE amount >= {minimum} GROUP BY category ORDER BY category",
    tables= {samples= samples},
)
for row in totals {
    let total = decode<Number>(row["total"])
}
```

Quoted SQL arguments are contextually typed `Template`: literal SQL stays literal
and `{value}` holes are bound SQLite parameters. Use `SqlIdent(name)` explicitly
for a quoted table/column identifier. Ordinary String aliases and concatenated
Strings are rejected; preserve holes using `let statement:Template="..."` or
`let statement=Sql("...")`. There is no public `params` argument.
`tables` is optional for statements that do not read imported lists. Templates
also render automatically when passed to a `String` parameter; converting an
already rendered String back into a SQL template is intentionally unsupported.

`query<Summary>(...)` returns `List<Summary>` and checks exact column names,
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
write bursts are debounced. There is no periodic filesystem polling. Watches are registered before dependency reads and restored when directories are replaced.
`Runtime::snapshot()` performs no source reads or preparation on the caller's
thread. It exposes last-good prepared content, the current diagnostic, revision,
changed element IDs, early planned duration and reflected-input acknowledgement.
Its progress snapshot distinguishes initial loading, reloading with last-good
content, failure and readiness, including the current resource and elapsed time.
Notifications are coalesced. Initial loading and failures use Skia feedback on the compositor's GPU canvas; successful reloads retain the last-good frame until atomic replacement, without a loading overlay. Strict exports return diagnostics instead of
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
CPU→GPU→CPU→GPU pixel round trip. No Python process or intermediate video is required. Reflected values appear as labeled, typed inspector controls with source/type
tooltips and explicit default/reset actions. Colors, booleans, strings, exact
numbers and units use native controls; structured values use schema-validated
JSON. Off-playhead metadata discovery runs on a background worker, while visible
clips reuse read-only preview observations.

In the GTK editor, pause playback, select a Sanim timeline clip, then click a
rendered shape in the preview. Its popover shows the element's name and kind,
containing groups, constructor source line, and helper call sites. Copy buttons
provide a `file:line:column` reference or the original source line. The selected
shape is outlined. Dragging continues to transform the clip normally.
Inspection uses the exact prepared scene and time attached to the displayed
frame; loading, source edits, seeking, and selection changes cannot reuse stale
locations. Source evaluation and hit testing run off the GTK thread.
Clip positioning, rotation, scaling, shear, and preview zoom are respected;
external warp/distortion modifiers are not included in source-element hit tests.
Disable those modifiers when inspecting source geometry.

`shrimply sanim get` reports canonical config keys, display labels, defaults,
current values and full type information. `shrimply sanim set` accepts typed JSON
`parameters` and separate `reset_keys`; JSON null is an input value rather than a
reset instruction. Source metadata is validated before transactional edits,
which use the editor's normal undoable project operations.

Explicit font assets use Pango's file-capable Fontconfig/FreeType backend (or
Win32), independent of the platform's default PangoCairo backend. On macOS,
use a Pango build with Fontconfig/FreeType enabled; Homebrew's Pango provides
this. CoreText's Pango font map cannot load private font files. Shaping stays
asset-based with fallback disabled; unsupported builds fail with a diagnostic
instead of substituting an installed font.

Source formatting uses the canonical Sanim parser without executing data reads or
loading imported modules:

```sh
shrimply sanim fmt scene.sanim
shrimply sanim fmt scene.sanim --check
shrimply sanim fmt scene.sanim --write
shrimply sanim lint scene.sanim --check-format --deny-warnings
```

`fmt` preserves comments and raw/formatted string contents. `--write` atomically
replaces an existing `.sanim` file and preserves its permissions; the default
prints to standard output. Use `-` as the source path to format standard input.
`--indent-width`, `--line-width`, and `--tabs` control layout. The standalone
LSP offers the same formatter through the editor's Format Document action.

`shrimply sanim lsp` serves these language features over stdio without opening
the GUI. The Zed and VS Code extensions prefer this installed command and fall
back to standalone `sanim-lsp`. The Shrimply server can also read reflected input
values from active project sessions through the same IPC bridge as the CLI.
Hover a reflected input in saved source to see each matching clip's resolved
value. Active projects in source-file ancestor directories are discovered
automatically; `shrimply sanim lsp --project /path/to/project.shrimp` selects one
explicitly. The optional `sanim/runtime` LSP request takes `textDocument.uri` for
an open document and returns matching project/clip contexts and their reflection
metadata. This reads saved runtime state, never evaluates unsaved buffers, and
does not open an editor or start a headless project owner.

Comments begin with `#` outside strings and continue to the end of the line.
Hexadecimal colors use `Color("#3584e4")`; `#abcdef` is a comment.

Native element notation lowers to the same constructors and render state:

```sanim
let name = reflect("Sanim")
let card = <Box w-60% h-40% card flex flex-col items-center justify-center gap-16>
    <Label text-xl text-light1>Hello {name}</Label>
    <Box w-200 h-44 pill fill={LinearGradient(from=Color("blue3"),to=Color("purple3"))}>
        <Label font_size={16pt}>Native gradient</Label>
    </Box>
</Box>
add(card)
wait(2s)
```

`Label` is `Text`. Attributes use quoted strings, `{typed_expression}`, or bare
boolean flags or native utilities such as `<Box w-180 flex flex-col>`. Literal color attributes use the registered parameter type to
lower `fill="blue3"` to `Color("blue3")`. Text supports the same interpolation,
Python format specifications, `{{`/`}}` escapes and reactive snapshots as strings.
Nested elements are ordinary `List<Element>` children with stable source/binding
identities; rectangles remain painted shapes whose root properties animate.

Bare opening-tag attributes supply bounded native utilities: `card`, `raised`, `pill`, `suggested`,
`destructive`, `flex`, `flex-row`/`flex-col`, `items-start/center/end`, `justify-start/center/end/between/around`,
`flex-wrap`, `grow`, `shrink`, `flex-1`, `flex-none`, `basis-N`, `gap-N`, `p-N`, `w-N`, `h-N`, `rounded[-sm/lg/xl/full/none]`, `rounded-N`,
`bg-COLOR`, `text-COLOR` on labels, and `border[-N]`/`border-COLOR`.
Numbers are logical pt. Width and height also accept percentages and `full`,
resolved against the external parent viewport after signal/animation sampling.
Pill corners remain fully round when resized. Explicit constructor arguments
win over utility defaults; unknown or incompatible utilities fail with guidance.

Native fills accept `Color` or `LinearGradient(from=Color("blue3"),
to=Color("purple3"), angle=0deg)`. A gradient may instead provide an explicit typed
`stops=[{offset=0,color=Color("blue3")}, {offset=1,color=Color("purple3")}]` list.
Gradients support 2–64 ordered stops covering 0–1; 0deg points right, 90deg down.
Gradients are typed `fill` values; utilities do not encode gradient directions or color stops. Skia shaders paint the existing native path directly. Gradient stop
interpolation currently fails explicitly; opacity and transform animations work.
See [the complete native markup animation](examples/native_markup.sanim).
`make markup-bench` verifies identical native IR and measures rendering against
the ordinary constructor form for 100 gradient cards and labels.

### Native source-code views

`<CodeView src="file.py"/>` loads a watched source file directly as an element.
A literal `<CodeView lang="python">…</CodeView>` body preserves foreign syntax;
`code={signal}` accepts live text. Methods animate patches, selections, styles,
syntax scopes and scrolling through native Pango/Skia. Actual syntax trees are
cached by source and grammar and shared across views and timeline revisions.
See [CodeView](docs/code-view.md) and [the example](examples/code_view.sanim).

A container with `overflow="auto"` provides the same clipped viewport for ordinary element trees.
Animate its `scroll` signal or call `scroll_to(y=100%, duration=600ms)`; percentages
stay relative to the current overflow after resizing. CodeView additionally
accepts source-line and syntax-scope targets. See [scrolling](docs/scrolling.md)
and [the scrolling example](examples/scrolling.sanim).
