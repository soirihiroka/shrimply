use sanim_core::{Align, Fraction, Geometry, Property, RevealKind};
use sanim_lang::{Engine, MemorySources};
use std::collections::BTreeMap;

fn prepare(source: &str) -> Result<sanim_lang::Prepared, sanim_core::Diagnostic> {
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    Engine::default().prepare("main.sanim", &files, &BTreeMap::new())
}

#[test]
fn reveal_constructors_have_exact_endpoints_and_distinct_modes() {
    for (name, kind, start, end) in [
        ("Create", RevealKind::Create, 0, 1),
        ("Write", RevealKind::Write, 0, 1),
        ("Uncreate", RevealKind::Create, 1, 0),
        ("Unwrite", RevealKind::Write, 1, 0),
    ] {
        let prepared = prepare(&format!(
            "let shape=Circle(opacity:0.6)\nplay({name}(shape),duration:1s)\nwait(1s)"
        ))
        .unwrap();
        let animation = &prepared.animation;
        for (time, progress) in [
            (Fraction::from(0), Fraction::from(start)),
            (Fraction::new(1u64, 2u64), Fraction::new(1u64, 2u64)),
            (Fraction::from(2), Fraction::from(end)),
        ] {
            let sampled = animation.sample(time).unwrap();
            assert_eq!(sampled[0].reveal.kind, kind, "{name}");
            assert_eq!(sampled[0].reveal.progress, progress, "{name}");
            assert_eq!(sampled[0].opacity, Fraction::new(3u64, 5u64));
        }
        assert_eq!(animation.duration, Fraction::from(2));
    }
}

#[test]
fn reveals_compose_with_motion_but_reject_competing_reveals() {
    let prepared=prepare("let shape=Circle()\nplay(Create(shape),MoveTo(shape,Vec2(20pt,0pt)),FadeIn(shape),duration:1s)\nplay(Unwrite(shape),duration:1s)\nplay(Write(shape),duration:1s)").unwrap();
    assert_eq!(
        prepared.animation.tracks.values().next().unwrap()[&Property::Reveal].len(),
        3
    );
    assert_eq!(
        prepared.animation.sample(Fraction::from(3)).unwrap()[0]
            .reveal
            .progress,
        Fraction::from(1)
    );
    let error =
        prepare("let shape=Circle()\nplay(Create(shape),Write(shape),duration:1s)").unwrap_err();
    assert!(error.to_string().contains("overlapping writers"), "{error}");
}

#[test]
fn center_growth_preserves_original_scale_and_anchoring() {
    let prepared=prepare("let shape=Rect(scale:2)\nplay(GrowFromCenter(shape),duration:1s)\nplay(ShrinkToCenter(shape),duration:1s)").unwrap();
    for (time, scale) in [(0, 0), (1, 2), (2, 0)] {
        let elements = prepared
            .animation
            .layout(
                Fraction::from(time),
                Fraction::from(300),
                Fraction::from(200),
            )
            .unwrap();
        assert_eq!(elements[0].scale, Fraction::from(scale));
        assert_eq!(elements[0].position.values, [150.0, 100.0]);
        assert_eq!(elements[0].layout.x_align, Align::Center);
    }
    assert!(
        prepare("let shape=Rect(x_align:Align.Start)\nplay(GrowFromCenter(shape))")
            .unwrap_err()
            .to_string()
            .contains("Align.Center")
    );
    assert!(
        prepare("let shape=Rect()\nadd(Row(children:[shape]))\nplay(GrowFromCenter(shape))")
            .unwrap_err()
            .to_string()
            .contains("flow layout controls")
    );
}

#[test]
fn group_reveals_keep_child_geometry_and_layout_unchanged() {
    let prepared =
        prepare("let row=Row(children:[Circle(),Rect()],gap:10pt)\nplay(Write(row),duration:1s)")
            .unwrap();
    let start = prepared
        .animation
        .layout(Fraction::from(0), Fraction::from(300), Fraction::from(200))
        .unwrap();
    let end = prepared
        .animation
        .layout(Fraction::from(1), Fraction::from(300), Fraction::from(200))
        .unwrap();
    let Geometry::Group { children: a } = start[0].geometry.as_ref() else {
        panic!("group")
    };
    let Geometry::Group { children: b } = end[0].geometry.as_ref() else {
        panic!("group")
    };
    assert_eq!(a, b);
    assert_eq!(start[0].position, end[0].position);
}

#[test]
fn animation_names_are_protected_by_registration() {
    for name in [
        "Create",
        "Write",
        "Uncreate",
        "Unwrite",
        "GrowFromCenter",
        "ShrinkToCenter",
    ] {
        assert!(prepare(&format!("let {name}=1")).is_err(), "{name}");
    }
}

#[test]
fn ambiguous_hierarchical_reveals_and_rotated_growth_fail_cleanly() {
    for motions in [
        "Write(group),Uncreate(child)",
        "Uncreate(child),Write(group)",
    ] {
        let error = prepare(&format!(
            "let child=Rect()\nlet group=Group(children:[child])\nplay({motions},duration:1s)"
        ))
        .unwrap_err();
        assert!(
            error.to_string().contains("ancestor/descendant reveals"),
            "{error}"
        );
    }
    for source in [
        "let shape=Line(end:Vec2(40pt,0pt),rotation:90deg)\nplay(GrowFromCenter(shape))",
        "let shape=Rect()\nplay(RotateTo(shape,90deg))\nplay(GrowFromCenter(shape))",
        "let shape=Rect()\nplay(GrowFromCenter(shape),RotateTo(shape,90deg))",
    ] {
        let error = prepare(source).unwrap_err();
        assert!(error.to_string().contains("zero rotation"), "{error}");
    }
}

#[test]
fn parent_child_parallel_writes_are_independent_of_argument_order() {
    let mut animations = Vec::new();
    for motions in ["Write(group),FadeIn(child)", "FadeIn(child),Write(group)"] {
        let prepared = prepare(&format!(
            r#"let child=Circle(id:"child")
let group=Group(id:"group",children:[child])
play({motions},duration:1s)"#
        ))
        .unwrap();
        assert_eq!(prepared.animation.layers.len(), 1);
        assert_eq!(prepared.animation.element_count(), 2);
        animations.push(prepared.animation);
    }
    assert_eq!(animations[0], animations[1]);
}
