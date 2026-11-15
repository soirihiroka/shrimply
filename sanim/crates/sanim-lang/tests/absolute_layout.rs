use sanim_core::{Element, Fraction, PositionMode};
use sanim_lang::{Engine, MemorySources};

fn prepare(source: &str) -> sanim_lang::Prepared {
    let mut files = MemorySources::default();
    files.insert("main.sanim", format!("{}\nAnimation()", source));
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
fn center(element: &Element) -> [f64; 2] {
    let bounds = sanim_core::verify::bounds(element).unwrap();
    std::array::from_fn(|axis| (bounds.min.values[axis] + bounds.max.values[axis]) / 2.)
}

#[test]
fn absolute_children_keep_parent_relative_coordinates_without_displacing_block_siblings() {
    let prepared = prepare(
        r#"
let first=<Box width={10pt} height={10pt} fill="light1" x_align={Align.Center} y_align={Align.Center}/>
let overlay=<Box absolute width={6pt} height={8pt} position={[75%, 50%]} x_align={Align.Start} y_align={Align.Start} fill="light1"/>
let second=<Box width={10pt} height={20pt} fill="light1" x_align={Align.Center} y_align={Align.Center}/>
let panel=<Box width={100pt} height={40pt}>{first}{overlay}{second}</Box>
Animation(children=[panel])
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
let first=<Box width={10pt} height={10pt} fill="light1" x_align={Align.Center} y_align={Align.Center}/>
let overlay=<Box absolute width={80pt} height={80pt} position={[0pt, 200pt]} x_align={Align.Start} y_align={Align.Start} fill="light1"/>
let second=<Box width={10pt} height={20pt} fill="light1" x_align={Align.Center} y_align={Align.Center}/>
let panel=<Box width={100pt}>{first}{overlay}{second}</Box>
Animation(children=[panel])
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
let first=<Box grow width={20pt} height={10pt} fill="light1" x_align={Align.Center} y_align={Align.Center}/>
let overlay=<Box absolute grow width={80pt} height={10pt} fill="light1" x_align={Align.Center} y_align={Align.Center}/>
let second=<Box grow width={20pt} height={10pt} fill="light1" x_align={Align.Center} y_align={Align.Center}/>
let panel=<Box flex flex-row width={100pt} height={40pt}>{first}{overlay}{second}</Box>
Animation(children=[panel])
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
let marker=<Box absolute position={[-20%, 10%]} width={2*(3pt)} height={2*(3pt)} rounded-full fill="light1" x_align={Align.Center} y_align={Align.Center}/>
let stage=<Box w-full h-full x_align={Align.Center} y_align={Align.Center}>{marker}</Box>
Animation(children=[stage])
"#,
    );
    for (width, height) in [(100, 100), (200, 160)] {
        let roots = prepared
            .animation
            .layout(0.into(), width.into(), height.into())
            .unwrap();
        assert_eq!(roots[0].position.values, [0., 0.]);
        let marker = center(child(&roots[0], "marker"));
        assert!((marker[0] - f64::from(width) * 0.3).abs() < 1e-8);
        assert!((marker[1] - f64::from(height) * 0.6).abs() < 1e-8);
    }
}

#[test]
fn live_position_modes_invalidate_flow_plans_and_absolute_paint_expands_scroll_range() {
    let prepared = prepare(
        r#"
type Placement="relative"|"absolute"
mut placement:Placement="relative"
let moving=<Box position_mode={placement} width={10pt} height={10pt} position={[50pt, 20pt]} x_align={Align.Start} y_align={Align.Start} fill="light1"/>
let second=<Box width={10pt} height={20pt} fill="light1" x_align={Align.Center} y_align={Align.Center}/>
let panel=<Box width={100pt}>{moving}{second}</Box>
Animation(children=[panel])
wait(1s)
animate(placement,to="absolute",duration=0s)
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
let overlay=<Box absolute width={20pt} height={20pt} position={[0pt, 150pt]} x_align={Align.Start} y_align={Align.Start} fill="light1"/>
let panel=<Box width={100pt} height={60pt} overflow="auto" scroll={[0%, 100%]}>{overlay}</Box>
Animation(children=[panel])
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
let marker=<Box absolute position={[0pt, 0pt]} width={2*(3pt)} height={2*(3pt)} rounded-full fill="light1" x_align={Align.Center} y_align={Align.Center}/>
let stage=<Box w-full h-full x_align={Align.Center} y_align={Align.Center}>{marker}</Box>
Animation(children=[stage])
marker.move_to([20pt, 0pt], duration=1s, ease="linear")
"#,
    );
    let roots = prepared
        .animation
        .layout(Fraction::new(1u64, 2u64), 100.into(), 100.into())
        .unwrap();
    assert_eq!(center(child(&roots[0], "marker")), [60., 50.]);
    let overridden = prepare(
        "let marker=<Box absolute position_mode=\"relative\" width={2*(32pt)} height={2*(32pt)} rounded-full fill=\"light1\" x_align={Align.Center} y_align={Align.Center}/>\nAnimation(children=[marker])",
    );
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
    files.insert("main.sanim", "let marker=Box(position_mode=\"fixed\", width=2*(32pt), height=2*(32pt), radius=\"full\", fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)\nAnimation()");
    let error = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap_err();
    assert_eq!(error.code, "S106");
}

#[test]
fn absolute_intrinsics_are_measured_once_in_flex_flow() {
    let prepared = prepare(
        r#"
let first=<Box grow width={20pt} height={10pt} fill="light1" x_align={Align.Center} y_align={Align.Center}/>
let overlay=<Box absolute grow width={80pt} height={10pt} fill="light1" x_align={Align.Center} y_align={Align.Center}/>
let second=<Box grow width={20pt} height={10pt} fill="light1" x_align={Align.Center} y_align={Align.Center}/>
let panel=<Box flex flex-row width={100pt} height={40pt}>{first}{overlay}{second}</Box>
Animation(children=[panel])
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
let overlay=<Box absolute width={20pt} height={20pt} position={[70pt, 100pt]} x_align={Align.Start} y_align={Align.Start} fill="light1"/>
let panel=<Box flex flex-row width={100pt} height={80pt} x_align={Align.Start} y_align={Align.Start} overflow="auto" fill="light1">{overlay}</Box>
Animation(children=[panel])
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
let overlay=<Box absolute position={[10pt, 0pt]} width={2*(3pt)} height={2*(3pt)} rounded-full fill="light1" x_align={Align.Center} y_align={Align.Center}/>
let panel=<Box width={100pt} height={100pt} children={items}/>
Animation(children=[panel])
animate(items,to=append(items,overlay),duration=1s,ease="linear")
wait(1s)
"#,
    );
    let roots = prepared
        .animation
        .layout(Fraction::new(1u64, 2u64), 100.into(), 100.into())
        .unwrap();
    let overlay = child(&roots[0], "overlay");
    assert_eq!(center(overlay), [60., 50.]);
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
mut offset:List<Length,2>=[0pt, 0pt]
let marker=Box(position_mode=placement,{position}width=6pt,height=6pt,radius="full",fill=Color("light1"),x_align=Align.Center,y_align=Align.Center)
let panel=Box(width=100pt,height=100pt,children=[marker])
Animation(children=[panel])
wait(1s)
animate(placement,to="{next}",duration=0s)
marker.move_to([20pt, 0pt], duration=1s, ease="linear")
"#
            );
            let mut files = MemorySources::default();
            files.insert("main.sanim", format!("{}\nAnimation()", source));
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
let marker=<Box absolute width={2*(3pt)} height={2*(3pt)} rounded-full fill="light1" x_align={Align.Center} y_align={Align.Center}/>
mut items:List<Element>=[marker]
let panel=<Box width={100pt} height={100pt} children={items}/>
Animation(children=[panel])
marker.move_to([20pt, 0pt], duration=1s, ease="linear")
"#,
    );
    let roots = prepared
        .animation
        .layout(Fraction::new(1u64, 2u64), 100.into(), 100.into())
        .unwrap();
    assert_eq!(center(child(&roots[0], "marker")), [60., 50.]);
}

#[test]
#[ignore = "manual flow/absolute comparison: make absolute-layout-bench"]
fn absolute_layout_benchmark() {
    use std::{hint::black_box, time::Instant};
    const ITEMS: usize = 1000;
    const FRAMES: usize = 100;
    for mode in ["relative", "absolute"] {
        let elements=(0..ITEMS).map(|i|format!("Box(id=\"item_{i}\",width=1pt,height=1pt,position_mode=\"{mode}\",position=[{}pt, {}pt],fill=Color(\"light1\"),x_align=Align.Center,y_align=Align.Center)",i%50,i/50)).collect::<Vec<_>>().join(",");
        let prepared = prepare(&format!(
            "let panel=Box(width=200pt,height=200pt,children=[{elements}])\nAnimation(children=[panel])"
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
        .map(|i| format!("Box(id=\"stable_{i}\", width=1pt, height=1pt, fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)"))
        .collect::<Vec<_>>()
        .join(",");
    let prepared = prepare(&format!(
        "mut overlay_width=1pt\nlet overlay=Box(position_mode=\"absolute\", width=overlay_width, height=1pt, fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)\nlet panel=Box(width=200pt,height=200pt,children=[{elements},overlay])\nAnimation(children=[panel])\nanimate(overlay_width,to=100pt,duration=1s,ease=\"linear\")"
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
