use sanim_core::{Fraction, Property, Unit, Vec2, Viewport};
use sanim_lang::{Engine, MemorySources};

fn prepare(source: &str) -> Result<sanim_lang::Prepared, sanim_core::Diagnostic> {
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    Engine::new(Viewport {
        width: 200,
        height: 120,
        dpi: 72,
        ..Default::default()
    })
    .prepare("main.sanim", &files, &Default::default())
}
const CONTENT: &str = r#"let first=Rect(width=20pt,height=20pt,x_align=Align.Start,y_align=Align.Start)
let last=Rect(width=20pt,height=20pt,position=Vec2(0pt,80pt),x_align=Align.Start,y_align=Align.Start)
let spacer=Box(width=20pt,height=3*20pt)
let clip=Box(overflow="auto",width=100pt,height=60pt,children=[first,spacer,last])
add(clip)
"#;

#[test]
fn bounded_scroll_methods_use_normal_tracks_and_resolve_percentages_after_resize() {
    let prepared = prepare(&format!(
        "{CONTENT}clip.scroll_to(to=Vec2(0pt,500pt),duration=1s,ease=\"linear\")\nwait(1s)\n"
    ))
    .unwrap();
    let clip = prepared.animation.find("clip").unwrap();
    assert_eq!(
        prepared
            .animation
            .sample_property(clip, Property::Scroll, Fraction::new(1u64, 2u64))
            .unwrap()
            .vec2()
            .unwrap(),
        Vec2 {
            values: [0., 20.],
            unit: Unit::Length
        }
    );
    let sampled = prepared
        .animation
        .layout(1.into(), 200.into(), 120.into())
        .unwrap();
    assert_eq!(sampled[0].layout.scroll.unwrap().values, [0., 40.]);
    assert_eq!(prepared.animation.duration, 2.into());
    let relative = CONTENT.replace("height=60pt", "height=50%");
    let percent = prepare(&format!(
        "{relative}clip.scroll_to(to=Vec2(0%,100%),duration=1s)\n"
    ))
    .unwrap();
    assert_eq!(
        percent
            .animation
            .layout(1.into(), 200.into(), 120.into())
            .unwrap()[0]
            .layout
            .scroll
            .unwrap()
            .values,
        [0., 40.]
    );
    assert_eq!(
        percent
            .animation
            .layout(1.into(), 200.into(), 160.into())
            .unwrap()[0]
            .layout
            .scroll
            .unwrap()
            .values,
        [0., 20.]
    );
}

#[test]
fn scroll_signal_and_code_line_targets_are_supported() {
    let source = r#"mut offset=Vec2(0pt,0pt)
let shape=Rect(width=20pt,height=20pt,position=Vec2(0pt,80pt),x_align=Align.Start,y_align=Align.Start)
let spacer=Box(width=20pt,height=80pt)
let clip=Box(overflow="auto",width=100pt,height=60pt,children=[spacer,shape],scroll=offset)
add(clip)
animate(offset,to=Vec2(0pt,40pt),duration=1s,ease="linear")
"#;
    let prepared = prepare(source).unwrap();
    assert_eq!(
        prepared
            .animation
            .layout(Fraction::new(1u64, 2u64), 200.into(), 120.into())
            .unwrap()[0]
            .layout
            .scroll
            .unwrap()
            .values,
        [0., 20.]
    );
    let code = prepare(
        r#"let view=CodeView(code=r"""a=1
b=2
c=3
d=4
e=5
f=6
g=7
h=8
i=9
j=10
""",lang="python",width=100pt,height=40pt,font_size=10pt,line_height=12pt,line_numbers=false)
add(view)
view.scroll_to(line=6,align="start",duration=1s,ease="linear")
"#,
    )
    .unwrap();
    let track = code.animation.find("view").unwrap();
    assert_eq!(
        code.animation
            .sample_property(track, Property::Scroll, 1.into())
            .unwrap()
            .vec2()
            .unwrap()
            .values,
        [0., 60.]
    );
}

#[test]
fn invalid_viewports_and_ambiguous_scroll_targets_fail_with_helpful_errors() {
    for (source, expected) in [
        (
            "let c=Circle()\nc.scroll_to(to=Vec2(0pt,1pt))",
            "requires a container",
        ),
        (
            "let v=Box(overflow=\"auto\",width=10pt,height=10pt,children=[])\nv.scroll_to(to=Vec2(0pt,0pt),y=2pt)",
            "one target",
        ),
        (
            "let v=Box(overflow=\"auto\",width=10pt,height=10pt,children=[])\nv.scroll_to(line=2)",
            "require CodeView",
        ),
    ] {
        let error = prepare(source).unwrap_err();
        assert!(error.message.contains(expected), "{source}: {error}");
    }
}

#[test]
fn ordinary_container_overflow_shorthand_and_axis_overrides_are_explicit() {
    use sanim_core::Overflow;
    let prepared = prepare(r#"let pane=<Rect width={100pt} height={60pt} overflow="scroll" overflow_x="hidden" overflow_y="auto">
        <Rect width={200pt} height={120pt}/>
    </Rect>
    let plain=<Box width={100pt} height={60pt}><Circle/></Box>
    let clipped=<Box width={100pt} height={60pt} overflow-hidden overflow-y-scroll><Circle/></Box>
    add(pane)
    add(plain)
    add(clipped)
    wait(1s)"#).unwrap();
    let pane = &prepared.animation.find("pane").unwrap().layout;
    assert_eq!(pane.overflow_x, Overflow::Hidden);
    assert_eq!(pane.overflow_y, Overflow::Auto);
    assert!(pane.scroll.is_some());
    let plain = &prepared.animation.find("plain").unwrap().layout;
    assert_eq!(plain.overflow_x, Overflow::Visible);
    assert_eq!(plain.overflow_y, Overflow::Visible);
    assert!(plain.scroll.is_none());
    let clipped = &prepared.animation.find("clipped").unwrap().layout;
    assert_eq!(clipped.overflow_x, Overflow::Hidden);
    assert_eq!(clipped.overflow_y, Overflow::Scroll);
}

#[test]
fn overflow_errors_explain_required_container_and_dimensions() {
    for (source, reason) in [
        (
            r#"let x=Circle(width=20pt,height=20pt,overflow="hidden")"#,
            "no scrollable children",
        ),
        (
            r#"let x=Box(children=[],width=20pt,height=20pt,scroll=Vec2(0pt,1pt))"#,
            "scroll requires overflow",
        ),
        (
            r#"let x=Box(children=[],width=20pt,height=20pt,overflow="overlay")"#,
            "use visible, hidden, auto or scroll",
        ),
        (
            r#"let x=ScrollView(children=[],width=20pt,height=20pt)"#,
            "ScrollView",
        ),
    ] {
        let error = prepare(source).unwrap_err();
        assert!(error.message.contains(reason), "{source}: {error}");
    }
}

#[test]
fn heterogeneous_flow_uses_intrinsic_paint_and_explicit_leaf_boxes() {
    use sanim_core::{Geometry, math::to_f64, verify};
    let prepared=prepare(r#"let dot_shape=<Circle radius={10pt} width={40pt} height={40pt} padding={4pt}/>
let label=<Label font_size={12pt} padding={4pt}>Label</Label>
let rect=<Rect width={40pt} height={40pt}/>
let code=<CodeView lang="python" font_size={10pt}>value = 1</CodeView>
let xs=LinearScale(domain=Vec2(0,1),range=Vec2(0pt,40pt))
let ys=LinearScale(domain=Vec2(0,1),range=Vec2(0pt,40pt))
let plot=Scatter(data=[{x=0.5,y=0.5}],x_scale=xs,y_scale=ys,radius=2pt)
let plot_box=<Box width={40pt} height={40pt}>{plot}</Box>
let row=<Box flex flex-row width={600pt} height={60pt} gap={8pt} wrap={false}>{dot_shape}{label}{rect}{code}{plot_box}</Box>
let column=<Box flex flex-col width={100pt} height={120pt} gap={8pt} position={Vec2(0pt,70pt)}><Circle radius={6pt}/><Label font_size={10pt}>Next</Label></Box>
add(row)
add(column)
wait(1s)"#).unwrap();
    let laid = prepared
        .animation
        .layout_with(
            0.into(),
            800.into(),
            300.into(),
            sanim_data::resolve_pending,
        )
        .unwrap();
    let row = laid.iter().find(|item| item.id == "row").unwrap();
    let children = row.geometry.children().unwrap();
    let mut right = f64::NEG_INFINITY;
    for child in children {
        let bounds = verify::bounds(child).unwrap();
        assert!(
            bounds.min.values[0] >= right,
            "{} overlaps preceding painted item",
            child.id
        );
        right = bounds.max.values[0];
    }
    let dot_shape = &children[0];
    let Geometry::Circle { radius } = dot_shape.geometry.as_ref() else {
        panic!("circle")
    };
    assert_eq!(
        *radius,
        10.into(),
        "layout boxes do not resize intrinsic circle geometry"
    );
    assert_eq!(to_f64(dot_shape.layout.width.unwrap().value).unwrap(), 40.);
    let column = laid.iter().find(|item| item.id == "column").unwrap();
    let children = column.geometry.children().unwrap();
    assert!(
        verify::bounds(&children[1]).unwrap().min.values[1]
            > verify::bounds(&children[0]).unwrap().max.values[1]
    );
}

#[test]
fn unstyled_box_fills_available_width_and_measures_block_height() {
    let prepared = prepare(
        r#"let pane=<Box padding={3pt} gap={7pt}>
    <Rect width={20pt} height={20pt}/>
    <Rect width={30pt} height={30pt}/>
</Box>
let empty=<Box/>
add(pane)
add(empty)
wait(1s)"#,
    )
    .unwrap();
    for width in [200u64, 320] {
        let laid = prepared
            .animation
            .layout_with(
                0.into(),
                width.into(),
                120.into(),
                sanim_data::resolve_pending,
            )
            .unwrap();
        let pane = laid.iter().find(|item| item.id == "pane").unwrap();
        assert_eq!(pane.layout.width.unwrap().value, width.into());
        assert_eq!(pane.layout.height.unwrap().value, 63.into());
        assert_eq!(
            pane.fill,
            sanim_core::Color::parse("transparent").unwrap().into()
        );
        let children = pane.geometry.children().unwrap();
        assert_eq!(
            sanim_core::verify::bounds(&children[0]).unwrap().min.values,
            [3., 3.]
        );
        assert_eq!(
            sanim_core::verify::bounds(&children[1]).unwrap().min.values,
            [3., 30.]
        );
        let empty = laid.iter().find(|item| item.id == "empty").unwrap();
        assert_eq!(empty.layout.height.unwrap().value, 0.into());
    }
}

#[test]
fn automatic_height_percent_cycle_fails_with_the_parent_constraint_reason() {
    let prepared = prepare(
        r#"let pane=<Box><Box height={50%}/></Box>
add(pane)
wait(1s)"#,
    )
    .unwrap();
    let error = prepared
        .animation
        .layout_with(
            0.into(),
            200.into(),
            120.into(),
            sanim_data::resolve_pending,
        )
        .unwrap_err();
    assert!(
        error.contains("automatic-height Box") && error.contains("set the parent's height"),
        "{error}"
    );
}

#[test]
fn flex_resizing_remeasures_nested_percentage_constraints() {
    let prepared = prepare(
        r#"let left=<Box flex_grow={1} flex_shrink={1}><Rect width={100%} height={20pt}/></Box>
let right=<Box flex_grow={1} flex_shrink={1}><Rect width={100%} height={30pt}/></Box>
let pane=<Box flex_direction="row" width={200pt}>{left}{right}</Box>
add(pane)
wait(1s)"#,
    )
    .unwrap();
    let laid = prepared
        .animation
        .layout_with(
            0.into(),
            200.into(),
            120.into(),
            sanim_data::resolve_pending,
        )
        .unwrap();
    let pane = &laid[0];
    assert_eq!(pane.layout.height.unwrap().value, 30.into());
    for child in pane.geometry.children().unwrap() {
        assert_eq!(child.layout.width.unwrap().value, 100.into());
        let rect = &child.geometry.children().unwrap()[0];
        assert_eq!(
            rect.layout.width.unwrap().value,
            100.into(),
            "percentage width must use the allocated flex width"
        );
        let sanim_core::Geometry::Rectangle { width, .. } = rect.geometry.as_ref() else {
            panic!("rectangle")
        };
        assert_eq!(*width, 100.into());
    }
}

#[test]
fn box_dimension_signals_accept_zero_and_keep_a_zero_sized_clip() {
    let prepared = prepare(
        r#"mut height=20pt
let pane=<Box w-100 height={height} overflow-hidden><Rect width={20pt} height={20pt}/></Box>
add(pane)
wait(100ms)
height=0pt
wait(100ms)"#,
    )
    .unwrap();
    let time = Fraction::new(1u64, 10u64);
    let pane = prepared.animation.find("pane").unwrap();
    assert_eq!(
        prepared
            .animation
            .sample_property(pane, Property::Height, time)
            .unwrap()
            .number()
            .unwrap()
            .value,
        0.into()
    );
    let laid = prepared
        .animation
        .layout_with(time, 200.into(), 120.into(), sanim_data::resolve_pending)
        .unwrap();
    let viewport = sanim_core::scrolling::viewport(&laid[0]).unwrap();
    assert_eq!(viewport[1], viewport[3]);
    assert!(!laid[0].geometry.children().unwrap().is_empty());
}
