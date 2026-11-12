use sanim_core::{Fraction, Geometry};
use sanim_lang::{Engine, MemorySources};
use std::collections::BTreeMap;

#[test]
fn vector_formulas_and_component_getters_drive_live_position_and_radius() {
    for constructor in ["Vec2(sin(phase)*20pt,0pt)", "Vec2(x=sin(phase)*20pt)"] {
        let mut files = MemorySources::default();
        files.insert("main.sanim",format!("mut phase=0deg\nsig point={constructor}\nsig extent=point.x+30pt\nadd(Circle(id=\"circle\",position=point,radius=extent))\nbackground {{animate(phase,to=360deg,duration=2s)}}\nwait(2s)"));
        let p = Engine::default()
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap();
        for (time, x, radius) in [
            (Fraction::from(0), 0., 30),
            (Fraction::new(1u64, 2u64), 20., 50),
            (Fraction::new(3u64, 2u64), -20., 10),
        ] {
            let frame = p.animation.sample(time).unwrap();
            assert_eq!(frame[0].position.values, [x, 0.]);
            assert!(
                matches!(frame[0].geometry.as_ref(),Geometry::Circle{radius:actual} if *actual==Fraction::from(radius))
            );
        }
    }
}

#[test]
fn vector_cell_components_retain_units_and_instantaneous_writes() {
    for (constructor, updated, component) in [
        ("Vec3(1pt,2pt,3pt)", "Vec3(4pt,5pt,6pt)", "y"),
        ("Vec4(1pt,2pt,3pt,4pt)", "Vec4(4pt,5pt,6pt,7pt)", "y"),
    ] {
        let mut files = MemorySources::default();
        files.insert("main.sanim",format!("mut vector={constructor}\nsig extent=vector.{component}\nadd(Circle(id=\"circle\",radius=extent))\nwait(1s)\nvector={updated}\nwait(1s)"));
        let p = Engine::default()
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap();
        for (time, radius) in [(0, 2), (1, 5), (2, 5)] {
            assert!(
                matches!(p.animation.sample(Fraction::from(time)).unwrap()[0].geometry.as_ref(),Geometry::Circle{radius:actual} if *actual==Fraction::from(radius))
            );
        }
    }
}
