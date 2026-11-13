# Native scroll viewports

`ScrollView` is a normal native container with a clipped viewport. Give it a width and height; both accept parent-relative percentages or points. Its children keep their ordinary layout positions.

```sanim
let content = <Column width={300pt} height={800pt}>
    <Label>First item</Label>
    <Label>Second item</Label>
</Column>
let viewport = <ScrollView width={50%} height={30%}>{content}</ScrollView>
add(viewport)
viewport.scroll_to(to=Vec2(0%, 100%), duration=600ms)
```

Scroll offsets are `Vec2` values. Points describe content offsets; percentages describe the current overflow range, so `Vec2(0%, 100%)` means the bottom edge even after resizing. Offsets are clamped after layout, including content changes, folding and font changes. The default scroll curve is the normal bounded spring; `ease="linear"` is available for constant-speed movement.

`scroll_to` accepts one target form: `to=Vec2(...)`, `x=`/`y=`, or a Code View `line=`/`scope=`. Code lines use 1-based numbering; scopes accept the IDs returned by `.scopes()` or an unambiguous declaration name.

Axis targets preserve their units: `scroll_to(y=100%)` stays at the bottom when the viewport resizes. When specifying both `x=` and `y=`, use the same unit for both.

```sanim
let code = <CodeView src="./example.py" width={80%} height={50%} />
add(code)
code.scroll_to(line=40, align="center", duration=500ms)
code.scroll_to(scope="main", duration=400ms)
```

The default `align="nearest"` moves only enough to reveal the requested line. `start`, `center` and `end` choose an explicit vertical alignment. A target inside a folded scope reveals its visible header.

For a shared reactive offset, supply `scroll=` and animate its mutable input:

```sanim
mut offset = Vec2(0pt, 0pt)
let viewport = <ScrollView width={300pt} height={120pt} scroll={offset}>
    <Rect width={300pt} height={600pt} x_align={Align.Start} y_align={Align.Start} />
</ScrollView>
add(viewport)
animate(offset, to=Vec2(0pt, 200pt), duration=1s)
```

A signal-owned offset has one writer: use `animate(offset, ...)` rather than `.scroll_to(...)` on that view. These controls animate the Sanim scene and do not intercept the editor's wheel or canvas navigation gestures.

Clipping and offset transforms are shared by Skia drawing and item picking. Glyphs outside the viewport defer native path preparation; cached states and recordings support reverse scrubbing normally.
