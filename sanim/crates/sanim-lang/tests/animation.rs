use sanim_core::{Fraction, Property, Value, verify::Evidence};
use sanim_lang::{Engine, MemorySources, Prepared};
use std::collections::BTreeMap;

fn prepare(text: &str) -> Prepared {
    prepare_in(text, 1920, 1080, Fraction::from(30))
}
fn prepare_in(text: &str, width: u32, height: u32, rate: Fraction) -> Prepared {
    let mut files = MemorySources::default();
    files.insert("main.sanim", text);
    Engine::new(sanim_core::Viewport {
        width,
        height,
        rate,
        dpi: 72,
    })
    .prepare("main.sanim", &files, &BTreeMap::new())
    .unwrap_or_else(|e| panic!("{e}\n{text}"))
}

#[test]
fn direct_seek_has_no_playback_history() {
    let p = prepare(
        "let marker = Circle(id: \"marker\", position: Vec2(-40%, 0%))\nplay(MoveTo(marker, Vec2(40%, 0%)), duration: 2s)\n",
    );
    let later = p.animation.sample(Fraction::from(2)).unwrap();
    let middle = p.animation.sample(Fraction::from(1)).unwrap();
    let first = p.animation.sample(Fraction::from(0)).unwrap();
    assert_eq!(middle[0].position.values[0], 0.0);
    assert!(first[0].position.values[0] < later[0].position.values[0]);
    assert_eq!(later, p.animation.sample(Fraction::from(2)).unwrap());
}
#[test]
fn final_values_hold_past_motion_end() {
    let p = prepare(
        "let marker = Circle(id: \"marker\", position: Vec2(-40%, 0%))\nplay(MoveTo(marker, Vec2(40%, 0%)), duration: 1s)\nwait(2s)\n",
    );
    assert_eq!(
        p.animation.sample(Fraction::from(2)).unwrap()[0]
            .position
            .values[0],
        0.4
    );
}
#[test]
fn multiple_tracks_run_together() {
    let p = prepare(
        "let marker = Circle(id: \"marker\")\nplay(FadeIn(marker), MoveTo(marker, Vec2(40%, 0%)), duration: 1s)\n",
    );
    assert_eq!(p.animation.tracks["marker"].len(), 2);
    assert_eq!(p.animation.duration, Fraction::from(1));
}
#[test]
fn fade_default_duration() {
    let p = prepare("let marker = Circle()\nplay(FadeIn(marker))\n");
    assert_eq!(p.animation.duration, Fraction::from(1));
}
#[test]
fn not_visible_before_added() {
    let p = prepare("wait(1s)\nlet marker = Circle()\nplay(FadeIn(marker), duration: 1s)\n");
    assert!(p.animation.sample(Fraction::from(0)).unwrap().is_empty());
    assert_eq!(
        p.animation.sample(Fraction::from(1)).unwrap()[0].opacity,
        Fraction::from(0)
    );
}
#[test]
fn parent_relative_positions_resize() {
    let p =
        prepare("let marker = Circle(id: \"marker\", position: Vec2(-25%, 25%))\nadd(marker)\n");
    let a = p
        .animation
        .layout(Fraction::from(0), Fraction::from(400), Fraction::from(200))
        .unwrap();
    let b = p
        .animation
        .layout(Fraction::from(0), Fraction::from(800), Fraction::from(600))
        .unwrap();
    assert_eq!(a[0].position.values[0], 100.0);
    assert_eq!(b[0].position.values[0], 200.0);
    assert_eq!(b[0].position.values[1], 450.0);
}
#[test]
fn row_layout_wraps_when_parent_shrinks() {
    let p = prepare(
        "let a = Rect(id: \"a\", width: 60pt, height: 20pt)\nlet b = Rect(id: \"b\", width: 60pt, height: 20pt)\nadd(Row(children: [a, b], gap: 10pt))\n",
    );
    let rows = p
        .animation
        .layout(Fraction::from(0), Fraction::from(100), Fraction::from(100))
        .unwrap();
    let sanim_core::Geometry::Group { children } = &rows[0].geometry else {
        panic!()
    };
    assert_eq!(children[1].position.values[1], 40.0);
}
#[test]
fn nested_group_positions_remain_local() {
    let p = prepare(
        "let marker = Circle(id: \"marker\", position: Vec2(0%, 0%))\nadd(Group(children: [marker], width: 200pt, height: 100pt, position: Vec2(20pt, 30pt)))\n",
    );
    let group = p
        .animation
        .layout(Fraction::from(0), Fraction::from(800), Fraction::from(600))
        .unwrap();
    let sanim_core::Geometry::Group { children } = &group[0].geometry else {
        panic!()
    };
    assert_eq!(children[0].position.values[0], 100.0);
    assert_eq!(group[0].position.values[1], 30.0);
}
#[test]
fn rational_frame_rate_is_exact() {
    let p = prepare_in("wait(1s)\n", 320, 180, Fraction::new(30000u64, 1001u64));
    assert_eq!(
        p.animation.frame_time(30_000).unwrap(),
        Fraction::from(1001)
    );
}
#[test]
fn visual_assertions_name_objects() {
    let p = prepare(
        "let circle = Circle(id: \"circle\", radius: 10pt, position: Vec2(0%, -30%))\nlet rect = Rect(id: \"rect\", position: Vec2(0%, 30%))\nassert(circle.above(rect))\n",
    );
    assert_eq!(p.checks[0].evidence, Evidence::Proved);
    assert!(p.checks[0].message.contains("circle"));
}
#[test]
fn continuous_inside_proof_covers_translation() {
    let p = prepare_in(
        "let marker = Circle(id: \"marker\", radius: 5pt, position: Vec2(-40%, 0%))\nplay(MoveTo(marker, Vec2(40%, 0%)), duration: 1s, ease: \"cubic_out\")\nassert(always(inside_canvas(marker)))\n",
        200,
        100,
        Fraction::from(30),
    );
    assert_eq!(p.checks[0].evidence, Evidence::Proved);
}
#[test]
fn unsupported_continuous_proof_is_unknown() {
    let p = prepare(
        "let marker = Rect(id: \"marker\")\nplay(RotateTo(marker, 90deg), duration: 1s)\ncheck(always(inside_canvas(marker)))\n",
    );
    assert_eq!(p.checks[0].evidence, Evidence::Unknown);
}
#[test]
fn failed_visual_assertion_has_counterexample() {
    let mut files = MemorySources::default();
    files.insert("main.sanim","let circle = Circle(id: \"circle\", position: Vec2(0%, 40%))\nlet rect = Rect(id: \"rect\", position: Vec2(0%, -40%))\nassert(circle.above(rect))\n");
    let e = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap_err();
    assert_eq!(e.code, "S350");
    assert!(e.message.contains("circle") && e.message.contains("rect") && e.message.contains("0s"));
    assert_eq!(e.span.line, 3);
}
#[test]
fn overlapping_writers_fail_atomically() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        "let marker = Circle(id: \"marker\")\nplay(FadeIn(marker), FadeOut(marker), duration: 1s)\n",
    );
    let e = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap_err();
    assert_eq!(e.code, "S307");
    assert!(e.message.contains("overlapping"));
}
#[test]
fn sequential_properties_have_indexed_tracks() {
    let p = prepare(
        "let marker = Circle(id: \"marker\")\nrepeat 4 { play(FadeIn(marker), duration: 1s) }\n",
    );
    assert_eq!(p.animation.tracks["marker"][&Property::Opacity].len(), 4);
    assert!(matches!(p.bindings["marker"], Value::Element(_)));
}

#[test]
fn external_viewports_revalidate_required_contracts() {
    let p = prepare_in(
        "let marker=Circle(id: \"marker\",radius: 20pt)\nadd(marker)\nassert(inside_canvas(marker))\n",
        200,
        100,
        Fraction::from(30),
    );
    assert!(p.animation_for_viewport(400, 200).is_ok());
    let e = p.animation_for_viewport(10, 10).unwrap_err();
    assert_eq!(e.code, "S350");
    assert!(e.message.contains("10 × 10") && e.message.contains("marker"));
}

#[test]
fn taffy_flex_layout_aligns_distributes_and_resizes() {
    let p = prepare(
        "let a=Circle(id:\"a\",radius:10pt)\nlet b=Rect(id:\"b\",width:40pt,height:40pt)\nadd(Row(children:[a,b],height:100pt,padding:10pt,justify:Justify.SpaceBetween,align:Align.Center,wrap:false))",
    );
    for (width, expected) in [(200, 170.), (300, 270.)] {
        let layout = p
            .animation
            .layout(
                Fraction::from(0),
                Fraction::from(width),
                Fraction::from(100),
            )
            .unwrap();
        let sanim_core::Geometry::Group { children } = &layout[0].geometry else {
            panic!()
        };
        assert_eq!(children[0].position.values, [20., 50.]);
        assert_eq!(children[1].position.values, [expected, 50.]);
    }
}
#[test]
fn column_layout_supports_spacing_and_cross_axis_alignment() {
    let p = prepare(
        "let a=Rect(width:20pt,height:20pt)\nlet b=Rect(width:40pt,height:40pt)\nadd(Column(children:[a,b],width:100pt,height:200pt,padding:10pt,justify:Justify.SpaceEvenly,align:Align.Center))",
    );
    let layout = p
        .animation
        .layout(Fraction::from(0), Fraction::from(100), Fraction::from(200))
        .unwrap();
    let sanim_core::Geometry::Group { children } = &layout[0].geometry else {
        panic!()
    };
    assert_eq!(children[0].position.values, [50., 60.]);
    assert_eq!(children[1].position.values, [50., 130.]);
}
#[test]
fn non_flow_groups_apply_padding_to_the_child_viewport() {
    let p = prepare(
        "let marker=Circle(radius:5pt,position:Vec2(0%,0%),x_align:Align.End,y_align:Align.End)\nadd(Group(children:[marker],width:100pt,height:100pt,padding:10pt))",
    );
    let layout = p
        .animation
        .layout(Fraction::from(0), Fraction::from(100), Fraction::from(100))
        .unwrap();
    let sanim_core::Geometry::Group { children } = &layout[0].geometry else {
        panic!()
    };
    assert_eq!(children[0].position.values, [85., 85.]);
}
