use sanim_core::{Element, Fraction, PositionMode};
use sanim_lang::{Engine, MemorySources};

fn prepare(source: &str) -> sanim_lang::Prepared {
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap()
}
fn child<'a>(root: &'a Element, id: &str) -> &'a Element {
    root.geometry
        .children()
        .unwrap()
        .iter()
        .find(|child| child.id == id)
        .unwrap()
}
fn low(element: &Element) -> [f64; 2] {
    sanim_core::verify::bounds(element).unwrap().min.values
}

#[test]
fn absolute_children_keep_parent_relative_coordinates_without_displacing_block_siblings() {
    let prepared = prepare(
        r#"
let first=<Rect width={10pt} height={10pt}/>
let overlay=<Rect absolute width={6pt} height={8pt} position={Vec2(75%,50%)} x_align={Align.Start} y_align={Align.Start}/>
let second=<Rect width={10pt} height={20pt}/>
let panel=<Box width={100pt} height={40pt}>{first}{overlay}{second}</Box>
add(panel)
wait(1s)
"#,
    );
    let roots = prepared
        .animation
        .layout(0.into(), 100.into(), 100.into())
        .unwrap();
    assert_eq!(low(child(&roots[0], "first")), [0., 0.]);
    assert_eq!(low(child(&roots[0], "second")), [0., 10.]);
    assert_eq!(low(child(&roots[0], "overlay")), [75., 20.]);
    assert_eq!(
        child(&roots[0], "overlay").layout.position_mode,
        PositionMode::Absolute
    );
}

#[test]
fn absolute_children_do_not_set_automatic_height_or_take_flex_space() {
    let auto = prepare(
        r#"
let first=<Rect width={10pt} height={10pt}/>
let overlay=<Rect absolute width={80pt} height={80pt} position={Vec2(0pt,200pt)} x_align={Align.Start} y_align={Align.Start}/>
let second=<Rect width={10pt} height={20pt}/>
let panel=<Box width={100pt}>{first}{overlay}{second}</Box>
add(panel)
"#,
    );
    let roots = auto
        .animation
        .layout(0.into(), 100.into(), 100.into())
        .unwrap();
    assert_eq!(roots[0].layout.height.unwrap().value, Fraction::from(30));
    assert_eq!(low(child(&roots[0], "overlay")), [0., 200.]);
    let flex = prepare(
        r#"
let first=<Rect grow width={20pt} height={10pt}/>
let overlay=<Rect absolute grow width={80pt} height={10pt}/>
let second=<Rect grow width={20pt} height={10pt}/>
let panel=<Box flex flex-row width={100pt} height={40pt}>{first}{overlay}{second}</Box>
add(panel)
"#,
    );
    let roots = flex
        .animation
        .layout(0.into(), 100.into(), 100.into())
        .unwrap();
    for id in ["first", "second"] {
        assert_eq!(
            child(&roots[0], id).layout.width.unwrap().value,
            Fraction::from(50)
        );
    }
    assert_eq!(
        child(&roots[0], "overlay").layout.width.unwrap().value,
        Fraction::from(80)
    );
    assert_eq!(low(child(&roots[0], "second"))[0], 50.);
}

#[test]
fn full_size_stages_preserve_centered_coordinates_when_the_host_resizes() {
    let prepared = prepare(
        r#"
let marker=<Circle absolute radius={3pt} position={Vec2(-20%,10%)}/>
let stage=<Box w-full h-full x_align={Align.Center} y_align={Align.Center}>{marker}</Box>
add(stage)
"#,
    );
    for (width, height) in [(100, 100), (200, 160)] {
        let roots = prepared
            .animation
            .layout(0.into(), width.into(), height.into())
            .unwrap();
        assert_eq!(roots[0].position.values, [0., 0.]);
        let marker = child(&roots[0], "marker");
        assert!((marker.position.values[0] - f64::from(width) * 0.3).abs() < 1e-8);
        assert!((marker.position.values[1] - f64::from(height) * 0.6).abs() < 1e-8);
    }
}

#[test]
fn live_position_modes_invalidate_flow_plans_and_absolute_paint_expands_scroll_range() {
    let prepared = prepare(
        r#"
type Placement="relative"|"absolute"
mut placement:Placement="relative"
let moving=<Rect position_mode={placement} width={10pt} height={10pt} position={Vec2(50pt,20pt)} x_align={Align.Start} y_align={Align.Start}/>
let second=<Rect width={10pt} height={20pt}/>
let panel=<Box width={100pt}>{moving}{second}</Box>
add(panel)
wait(1s)
placement="absolute"
wait(1s)
"#,
    );
    for (time, moving, second) in [
        (0, [0., 0.], [0., 10.]),
        (2, [50., 20.], [0., 0.]),
        (0, [0., 0.], [0., 10.]),
    ] {
        let roots = prepared
            .animation
            .layout(time.into(), 100.into(), 100.into())
            .unwrap();
        assert_eq!(low(child(&roots[0], "moving")), moving);
        assert_eq!(low(child(&roots[0], "second")), second);
        assert_eq!(
            roots[0].layout.height.unwrap().value,
            Fraction::from(if time == 2 { 20 } else { 30 })
        );
    }
    let scroll = prepare(
        r#"
let overlay=<Rect absolute width={20pt} height={20pt} position={Vec2(0pt,150pt)} x_align={Align.Start} y_align={Align.Start}/>
let panel=<Box width={100pt} height={60pt} overflow="auto" scroll={Vec2(0%,100%)}>{overlay}</Box>
add(panel)
"#,
    );
    let roots = scroll
        .animation
        .layout(0.into(), 100.into(), 100.into())
        .unwrap();
    assert_eq!(roots[0].layout.scroll_range, [0., 110.]);
    assert_eq!(roots[0].layout.scroll.unwrap().values, [0., 110.]);
}

#[test]
fn absolute_positions_can_animate_and_explicit_mode_overrides_utilities() {
    let prepared = prepare(
        r#"
let marker=<Circle absolute radius={3pt} position={Vec2(0pt,0pt)}/>
let stage=<Box w-full h-full x_align={Align.Center} y_align={Align.Center}>{marker}</Box>
add(stage)
MoveTo(marker,Vec2(20pt,0pt),duration=1s,ease="linear")
"#,
    );
    let roots = prepared
        .animation
        .layout(Fraction::new(1u64, 2u64), 100.into(), 100.into())
        .unwrap();
    assert_eq!(child(&roots[0], "marker").position.values, [60., 50.]);
    let overridden =
        prepare("let marker=<Circle absolute position_mode=\"relative\"/>\nadd(marker)");
    assert_eq!(
        overridden
            .animation
            .find("marker")
            .unwrap()
            .layout
            .position_mode,
        PositionMode::Relative
    );
    let mut files = MemorySources::default();
    files.insert("main.sanim", "let marker=Circle(position_mode=\"fixed\")");
    let error = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap_err();
    assert_eq!(error.code, "S106");
}

#[test]
fn absolute_intrinsics_are_measured_once_in_flex_flow() {
    let prepared = prepare(
        r#"
let first=<Rect grow width={20pt} height={10pt}/>
let overlay=<Rect absolute grow width={80pt} height={10pt}/>
let second=<Rect grow width={20pt} height={10pt}/>
let panel=<Box flex flex-row width={100pt} height={40pt}>{first}{overlay}{second}</Box>
add(panel)
"#,
    );
    let mut visits = std::collections::BTreeMap::<String, usize>::new();
    prepared
        .animation
        .layout_with(0.into(), 100.into(), 100.into(), |element, _| {
            *visits.entry(element.id.clone()).or_default() += 1;
            Ok(())
        })
        .unwrap();
    assert_eq!(
        visits["overlay"], 1,
        "absolute content must not be remeasured for unrelated flex allocations"
    );
}

#[test]
fn absolute_children_use_the_same_containing_block_inside_rects() {
    let prepared = prepare(
        r#"
let overlay=<Rect absolute width={20pt} height={20pt} position={Vec2(70pt,100pt)} x_align={Align.Start} y_align={Align.Start}/>
let panel=<Rect flex flex-row width={100pt} height={80pt} x_align={Align.Start} y_align={Align.Start} overflow="auto">{overlay}</Rect>
add(panel)
"#,
    );
    let roots = prepared
        .animation
        .layout(0.into(), 100.into(), 100.into())
        .unwrap();
    let bounds = low(child(&roots[0], "overlay"));
    assert_eq!(
        [
            bounds[0] + roots[0].position.values[0],
            bounds[1] + roots[0].position.values[1]
        ],
        [70., 100.]
    );
    assert_eq!(roots[0].layout.scroll_range, [0., 40.]);
}

#[test]
fn absolute_list_entries_keep_authored_positions_and_share_entry_appearance() {
    let prepared = prepare(
        r#"
mut items:List<Element>=[]
let overlay=<Circle absolute radius={3pt} position={Vec2(10pt,0pt)}/>
let panel=<Box width={100pt} height={100pt} children={items}/>
add(panel)
items=append(items,overlay,duration=1s,ease="linear")
wait(1s)
"#,
    );
    let roots = prepared
        .animation
        .layout(Fraction::new(1u64, 2u64), 100.into(), 100.into())
        .unwrap();
    let overlay = child(&roots[0], "overlay");
    assert_eq!(overlay.position.values, [60., 50.]);
    assert_eq!(overlay.opacity, Fraction::new(1u64, 2u64));
    assert_eq!(overlay.scale, Fraction::new(24u64, 25u64));
}

#[test]
fn dynamic_position_modes_reject_manual_position_writers_in_both_directions() {
    for (initial, next) in [("relative", "absolute"), ("absolute", "relative")] {
        for retained in [false, true] {
            let position = if retained { "position=offset," } else { "" };
            let source = format!(
                r#"
type Placement="relative"|"absolute"
mut placement:Placement="{initial}"
mut offset=Vec2(0pt,0pt)
let marker=Circle(position_mode=placement,{position}radius=3pt)
let panel=Box(width=100pt,height=100pt,children=[marker])
add(panel)
wait(1s)
placement="{next}"
MoveTo(marker,Vec2(20pt,0pt),duration=1s,ease="linear")
"#
            );
            let mut files = MemorySources::default();
            files.insert("main.sanim", source);
            let error = Engine::default()
                .prepare("main.sanim", &files, &Default::default())
                .unwrap_err();
            assert!(error.message.contains("position_mode signal"), "{error}");
            assert!(error.message.contains("constant absolute"), "{error}");
        }
    }
}

#[test]
fn absolute_live_list_items_allow_manual_position_animation() {
    let prepared = prepare(
        r#"
let marker=<Circle absolute radius={3pt}/>
mut items:List<Element>=[marker]
let panel=<Box width={100pt} height={100pt} children={items}/>
add(panel)
MoveTo(marker,Vec2(20pt,0pt),duration=1s,ease="linear")
"#,
    );
    let roots = prepared
        .animation
        .layout(Fraction::new(1u64, 2u64), 100.into(), 100.into())
        .unwrap();
    assert_eq!(child(&roots[0], "marker").position.values, [60., 50.]);
}

#[test]
#[ignore = "manual flow/absolute comparison: make absolute-layout-bench"]
fn absolute_layout_benchmark() {
    use std::{hint::black_box, time::Instant};
    const ITEMS: usize = 1000;
    const FRAMES: usize = 100;
    for mode in ["relative", "absolute"] {
        let elements=(0..ITEMS).map(|i|format!("Rect(id=\"item_{i}\",width=1pt,height=1pt,position_mode=\"{mode}\",position=Vec2({}pt,{}pt))",i%50,i/50)).collect::<Vec<_>>().join(",");
        let prepared = prepare(&format!(
            "let panel=Box(width=200pt,height=200pt,children=[{elements}])\nadd(panel)"
        ));
        prepared
            .animation
            .layout(0.into(), 200.into(), 200.into())
            .unwrap();
        let started = Instant::now();
        for _ in 0..FRAMES {
            black_box(
                prepared
                    .animation
                    .layout(0.into(), 200.into(), 200.into())
                    .unwrap(),
            );
        }
        eprintln!(
            "{mode}: {ITEMS} elements × {FRAMES} cached layouts: {:?}",
            started.elapsed()
        );
    }
    let elements = (0..ITEMS)
        .map(|i| format!("Rect(id=\"stable_{i}\",width=1pt,height=1pt)"))
        .collect::<Vec<_>>()
        .join(",");
    let prepared = prepare(&format!(
        "mut overlay_width=1pt\nlet overlay=Rect(position_mode=\"absolute\",width=overlay_width,height=1pt)\nlet panel=Box(width=200pt,height=200pt,children=[{elements},overlay])\nadd(panel)\nanimate(overlay_width,to=100pt,duration=1s,ease=\"linear\")"
    ));
    for changing in [false, true] {
        let started = Instant::now();
        for frame in 0..FRAMES {
            let time = Fraction::new(if changing { frame as u64 } else { 0 }, FRAMES as u64);
            black_box(
                prepared
                    .animation
                    .layout(time, 200.into(), 200.into())
                    .unwrap(),
            );
        }
        eprintln!(
            "absolute overlay changing={changing}: {ITEMS} stable siblings × {FRAMES} layouts: {:?}",
            started.elapsed()
        );
    }
}
