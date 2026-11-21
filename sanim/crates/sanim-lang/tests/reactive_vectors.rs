use sanim_core::{Fraction, Property};
use sanim_lang::{Engine, MemorySources};
use std::collections::BTreeMap;

#[test]
fn numeric_list_formulas_and_indexing_drive_live_position_and_radius() {
    for constructor in ["[sin(phase)*20pt, 0pt]", "[20pt*sin(phase), 0pt]"] {
        let mut files = MemorySources::default();
        files.insert("main.sanim",format!("mut phase=0deg\nsig point={constructor}\nsig extent=point[0]+30pt\nAnimation(children=[Box(id=\"circle\", position=point, width=2*(extent), height=2*(extent), radius=\"full\", fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)])\n<Animation absolute>{{animate(phase,to=360deg,duration=2s,ease=\"linear\")}}</Animation>\nwait(2s)"));
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
            assert_eq!(
                frame[0]
                    .property(Property::Radius)
                    .unwrap()
                    .number()
                    .unwrap()
                    .value,
                Fraction::from(radius)
            );
        }
    }
}

#[test]
fn fixed_list_cells_retain_units_and_instantaneous_writes() {
    for (constructor, updated, component) in [
        ("[1pt, 2pt, 3pt]", "[4pt, 5pt, 6pt]", "1"),
        ("[1pt, 2pt, 3pt, 4pt]", "[4pt, 5pt, 6pt, 7pt]", "1"),
    ] {
        let mut files = MemorySources::default();
        let length = constructor.matches("pt").count();
        files.insert("main.sanim",format!("mut vector:List<Length,{length}>={constructor}\nsig extent=vector[{component}]\nAnimation(children=[Box(id=\"circle\", width=2*(extent), height=2*(extent), radius=\"full\", fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)])\nwait(1s)\nanimate(vector,to={updated},duration=0s)\nwait(1s)"));
        let p = Engine::default()
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap();
        for (time, radius) in [(0, 2), (1, 5), (2, 5)] {
            assert_eq!(
                p.animation.sample(Fraction::from(time)).unwrap()[0]
                    .property(Property::Radius)
                    .unwrap()
                    .number()
                    .unwrap()
                    .value,
                Fraction::from(radius)
            );
        }
    }
}
