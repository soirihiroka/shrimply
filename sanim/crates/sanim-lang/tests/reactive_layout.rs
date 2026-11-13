use sanim_core::{Fraction, Geometry};
use sanim_lang::{Engine, MemorySources};
use std::collections::BTreeMap;

#[test]
fn retained_row_width_reflows_its_children_and_supports_current_width_getters() {
    let mut files = MemorySources::default();
    files.insert("main.sanim","mut container_width=100pt\nlet child=Circle(id=\"child\",radius=10pt)\nlet row=Row(id=\"row\",width=container_width,height=40pt,children=[child],justify=Justify.End)\nadd(row)\nbackground {animate(container_width,to=200pt,duration=2s)}\nwait(2s)");
    let p = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    let (width, height) = p.animation.logical_size();
    let before = p
        .animation
        .layout(Fraction::from(0), width, height)
        .unwrap();
    let after = p
        .animation
        .layout(Fraction::from(2), width, height)
        .unwrap();
    assert_eq!(
        before[0].layout.width,
        Some(sanim_core::Number {
            value: Fraction::from(100),
            unit: sanim_core::Unit::Length
        })
    );
    assert_eq!(
        after[0].layout.width,
        Some(sanim_core::Number {
            value: Fraction::from(200),
            unit: sanim_core::Unit::Length
        })
    );
    let Geometry::Group { children: before } = before[0].geometry.as_ref() else {
        panic!("row expected")
    };
    let Geometry::Group { children: after } = after[0].geometry.as_ref() else {
        panic!("row expected")
    };
    assert!((after[0].position.values[0] - before[0].position.values[0] - 100.).abs() < 1e-6);
}
