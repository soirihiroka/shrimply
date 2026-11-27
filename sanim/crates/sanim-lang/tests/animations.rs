use sanim_core::{Align, Fraction, Geometry, Property, RevealKind};
use sanim_lang::{Engine, MemorySources};
use std::collections::BTreeMap;

fn prepare(source: &str) -> Result<sanim_lang::Prepared, sanim_core::Diagnostic> {
    let mut files = MemorySources::default();
    files.insert("main.sanim", format!("{}\nAnimation()", source));
    Engine::default().prepare("main.sanim", &files, &BTreeMap::new())
}

#[test]
fn reveal_constructors_have_exact_endpoints_and_distinct_modes() {
    for (name, kind, start, end) in [
        ("create", RevealKind::Create, 0, 1),
        ("write", RevealKind::Write, 0, 1),
        ("uncreate", RevealKind::Create, 1, 0),
        ("unwrite", RevealKind::Write, 1, 0),
    ] {
        let prepared = prepare(&format!(
            "let shape=Box(opacity=0.6, width=2*(32pt), height=2*(32pt), radius=\"full\", fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)\nshape.{name}(duration=1s,ease=\"linear\")\nwait(1s)"
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
    let prepared=prepare("let shape=Box(width=2*(32pt), height=2*(32pt), radius=\"full\", fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)\n<Animation absolute flow=\"sequence\">{shape.create(duration=1s, ease=\"linear\") }</Animation>; <Animation absolute flow=\"sequence\">{shape.move_to([20pt, 0pt], duration=1s, ease=\"linear\") }</Animation>; shape.fade_in(duration=1s, ease=\"linear\")\nshape.unwrite(duration=1s, ease=\"linear\")\nshape.write(duration=1s, ease=\"linear\")").unwrap();
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
        prepare("let shape=Box(width=2*(32pt), height=2*(32pt), radius=\"full\", fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)\n<Animation absolute flow=\"sequence\">{shape.create(duration=1s, ease=\"linear\") }</Animation>; shape.write(duration=1s, ease=\"linear\")").unwrap_err();
    assert!(error.to_string().contains("overlapping writers"), "{error}");
}

#[test]
fn center_growth_preserves_original_scale_and_anchoring() {
    let prepared=prepare("let shape=Box(scale=2, width=64pt, height=64pt, fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)\nshape.grow_from_center(duration=1s, ease=\"linear\")\nshape.shrink_to_center(duration=1s, ease=\"linear\")").unwrap();
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
        let bounds = sanim_core::verify::bounds(&elements[0]).unwrap();
        assert_eq!(
            std::array::from_fn::<_, 2, _>(|axis| (bounds.min.values[axis]
                + bounds.max.values[axis])
                / 2.),
            [150., 100.]
        );
        assert_eq!(elements[0].layout.x_align, Align::Center);
    }
    assert!(
        prepare("let shape=Box(x_align=Align.Start, width=64pt, height=64pt, fill=Color(\"light1\"), y_align=Align.Center)\nshape.grow_from_center(duration=1s, ease=\"linear\")")
            .unwrap_err()
            .to_string()
            .contains("Align.Center")
    );
    assert!(
        prepare("let shape=Box(width=64pt, height=64pt, fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)\nAnimation(children=[Box(flex_direction=\"row\",children=[shape])])\nshape.grow_from_center(duration=1s, ease=\"linear\")")
            .unwrap_err()
            .to_string()
            .contains("flow layout controls")
    );
}

#[test]
fn group_reveals_keep_child_geometry_and_layout_unchanged() {
    let prepared =
        prepare("let row=Box(flex_direction=\"row\",children=[Box(width=2*(32pt), height=2*(32pt), radius=\"full\", fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center),Box(width=64pt, height=64pt, fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)],gap=10pt)\nrow.write(duration=1s, ease=\"linear\")")
            .unwrap();
    let start = prepared
        .animation
        .layout(Fraction::from(0), Fraction::from(300), Fraction::from(200))
        .unwrap();
    let end = prepared
        .animation
        .layout(Fraction::from(1), Fraction::from(300), Fraction::from(200))
        .unwrap();
    let Geometry::Box { children: a, .. } = start[0].geometry.as_ref() else {
        panic!("group")
    };
    let Geometry::Box { children: b, .. } = end[0].geometry.as_ref() else {
        panic!("group")
    };
    assert_eq!(a, b);
    assert_eq!(start[0].position, end[0].position);
}

#[test]
fn animation_methods_use_the_protected_element_registry() {
    for name in [
        "create",
        "write",
        "uncreate",
        "unwrite",
        "grow_from_center",
        "shrink_to_center",
    ] {
        let qualified = format!("Element.{name}");
        assert!(
            sanim_lang::prelude::builtin(&qualified).is_some(),
            "{qualified}"
        );
        assert!(sanim_lang::prelude::protected(&qualified), "{qualified}");
    }
    assert_eq!(prepare("let Element=1").unwrap_err().code, "S101");
}

#[test]
fn ambiguous_hierarchical_reveals_and_rotated_growth_fail_cleanly() {
    for motions in [
        "<Animation absolute flow=\"sequence\">{group.write(duration=1s, ease=\"linear\") }</Animation>; child.uncreate(duration=1s, ease=\"linear\")",
        "<Animation absolute flow=\"sequence\">{child.uncreate(duration=1s, ease=\"linear\") }</Animation>; group.write(duration=1s, ease=\"linear\")",
    ] {
        let error = prepare(&format!(
            "let child=Box(width=64pt, height=64pt, fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)\nlet group=Box(children=[child])\nAnimation(children=[group])\n{motions}"
        ))
        .unwrap_err();
        assert!(
            error.to_string().contains("ancestor/descendant reveals"),
            "{error}"
        );
    }
    for source in [
        "let shape=Line(end=[40pt, 0pt],rotation=90deg)\nshape.grow_from_center(duration=1s, ease=\"linear\")",
        "let shape=Box(width=64pt, height=64pt, fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)\nshape.rotate_to(90deg, duration=1s, ease=\"linear\")\nshape.grow_from_center(duration=1s, ease=\"linear\")",
        "let shape=Box(width=64pt, height=64pt, fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)\n<Animation absolute flow=\"sequence\">{shape.grow_from_center(duration=1s, ease=\"linear\") }</Animation>; shape.rotate_to(90deg, duration=1s, ease=\"linear\")",
    ] {
        let error = prepare(source).unwrap_err();
        assert!(error.to_string().contains("zero rotation"), "{error}");
    }
}

#[test]
fn parent_child_parallel_writes_are_independent_of_argument_order() {
    let mut animations = Vec::new();
    for targets in ["[group,child]", "[child,group]"] {
        let prepared = prepare(&format!(
            r#"let child=Box(id="child", width=2*(32pt), height=2*(32pt), radius="full", fill=Color("light1"), x_align=Align.Center, y_align=Align.Center)
let group=Box(id="group",children=[child]);
({targets}).fade_in(duration=1s,ease="linear")"#
        ))
        .unwrap();
        assert_eq!(prepared.animation.layers.len(), 1);
        assert_eq!(prepared.animation.element_count(), 2);
        animations.push(prepared.animation);
    }
    assert_eq!(animations[0], animations[1]);
}
