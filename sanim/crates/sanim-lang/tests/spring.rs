use sanim_core::{Fraction, Property};
use sanim_lang::{Engine, MemorySources};
use std::collections::BTreeMap;

#[test]
fn default_spring_preserves_valid_signal_dimensions_at_all_sampled_times() {
    let mut files = MemorySources::default();
    files.insert("main.sanim","mut extent=100pt\nlet marker=Box(width=2*(extent), height=2*(extent), radius=\"full\", fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)\nAnimation(children=[marker])\nanimate(extent,to=abs(1pt),duration=1s)");
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    let marker = prepared.animation.find("marker").unwrap();
    for index in 0u64..=1000 {
        let value = prepared
            .animation
            .sample_property(marker, Property::Width, Fraction::new(index, 1000u64))
            .unwrap()
            .number()
            .unwrap()
            .value;
        assert!(value >= Fraction::from(2) && value <= Fraction::from(200));
    }
}

#[test]
fn explicit_bouncy_spring_retains_spatial_overshoot_and_exact_endpoint() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        "let marker=Box(width=2*(32pt), height=2*(32pt), radius=\"full\", fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)\nmarker.move_to([100pt, 0pt], duration=1s, ease=\"spring_bouncy\")\nAnimation()",
    );
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    let marker = prepared.animation.find("marker").unwrap();
    let position = |time| {
        prepared
            .animation
            .sample_property(marker, Property::Position, time)
            .unwrap()
            .vector()
            .unwrap()
            .0[0]
    };
    assert!(position(Fraction::new(2u64, 5u64)) > 100.);
    assert_eq!(position(1.into()), 100.);
}

#[test]
fn continuous_proofs_preserve_bouncy_motion_conservatism_for_a_box() {
    use sanim_core::verify::Evidence;
    for (ease, expected) in [
        ("spring_bouncy", Evidence::Unknown),
        ("linear", Evidence::Proved),
    ] {
        let mut files = MemorySources::default();
        files.insert("main.sanim",format!("let marker=Box(width=2*(5pt), height=2*(5pt), radius=\"full\", fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)\nlet panel=Box(children=[marker],width=10pt,height=10pt)\nAnimation(children=[panel])\npanel.move_to([1pt, 0pt], duration=1s, ease=\"{ease}\")\ncheck(always(inside_canvas(panel)))"));
        let prepared = Engine::default()
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap();
        let proof = &prepared.checks[0];
        assert_eq!(proof.evidence, expected, "{}", proof.message);
        if expected == Evidence::Unknown {
            assert!(
                proof.message.contains("bounded translations"),
                "{}",
                proof.message
            );
        }
    }
}
