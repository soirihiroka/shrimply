use sanim_core::Fraction;
use sanim_lang::{Engine, MemorySources};
use std::collections::BTreeMap;

#[test]
fn duration_observes_sampled_scene_state_through_all_value_paths() {
    for (source, expected) in [
        (
            "let marker=Circle(position=Vec2(0pt,0pt))\nMoveTo(marker,Vec2(10pt,0pt),duration=1s,ease=\"linear\")\nwait(decode<Vec2<Length>>(Data(marker.position)).x/1pt*1s)",
            11,
        ),
        (
            "let marker=Circle(position=Vec2(0pt,0pt))\nbackground{MoveTo(marker,Vec2(10pt,0pt),duration=2s,ease=\"linear\")}\nwait(1s)\nwait(decode<Vec2<Length>>(Data(marker.position)).x/1pt*1s)",
            6,
        ),
        (
            "fn delay():Duration{let marker=Circle(position=Vec2(0pt,0pt));MoveTo(marker,Vec2(10pt,0pt),duration=1s,ease=\"linear\");return decode<Vec2<Length>>(Data(marker.position)).x/1pt*1s}\nwait(delay())",
            11,
        ),
        (
            "let state={marker=Circle(position=Vec2(0pt,0pt))}\nMoveTo(state.marker,Vec2(10pt,0pt),duration=1s,ease=\"linear\")\nwait(decode<Vec2<Length>>(Data(state.marker.position)).x/1pt*1s)",
            11,
        ),
        (
            "let fallback=Circle(position=Vec2(0pt,0pt))\nlet markers=[fallback]\nMoveTo(unwrap(markers[0],fallback),Vec2(10pt,0pt),duration=1s,ease=\"linear\")\nwait(decode<Vec2<Length>>(Data(unwrap(markers[0],fallback).position)).x/1pt*1s)",
            11,
        ),
        ("import \"helper\" as helper\nwait(helper.delay())", 11),
        (
            "let marker=Circle(position=Vec2(0pt,0pt))\nMoveTo(marker,Vec2(10pt,0pt),duration=1s,ease=\"linear\")\nfn pause(length:Duration=decode<Vec2<Length>>(Data(marker.position)).x/1pt*1s):Unit{wait(length)}\npause()",
            11,
        ),
    ] {
        let mut files = MemorySources::default();
        files.insert("main.sanim", source);
        files.insert("helper.sanim", "fn delay():Duration{let marker=Circle(position=Vec2(0pt,0pt));MoveTo(marker,Vec2(10pt,0pt),duration=1s,ease=\"linear\");return decode<Vec2<Length>>(Data(marker.position)).x/1pt*1s}");
        let full = Engine::default()
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap_or_else(|error| panic!("full: {error}\n{source}"));
        assert_eq!(
            full.animation.duration,
            Fraction::from(expected),
            "{source}"
        );
        let mut engine = Engine::default();
        for _ in 0..2 {
            let planned = engine
                .duration("main.sanim", &files, &BTreeMap::new())
                .unwrap_or_else(|error| panic!("duration: {error}\n{source}"));
            assert_eq!(planned.duration, full.animation.duration, "{source}");
            assert!(
                planned
                    .performance
                    .iter()
                    .any(|warning| warning.code == "P004"),
                "{source}"
            );
        }
        let json = engine
            .duration_json("main.sanim", &files, &BTreeMap::new())
            .unwrap();
        assert_eq!(json.duration, full.animation.duration, "{source}");
    }
}

#[test]
fn data_only_duration_skips_scene_and_font_work() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "let font=read<Font>(\"absent.ttf\")\nlet delay=read<Duration>(\"delay.json\")\nfn unused():Element{return Text(\"unused\",font=font)}\nrepeat 1000{FadeIn(Circle(),duration=delay)}");
    files.insert("delay.json", "\"1ms\"");
    let planned = Engine::default()
        .duration("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(planned.duration, Fraction::from(1));
    assert!(!planned.dependencies.contains_key("absent.ttf"));
    assert!(
        !planned
            .performance
            .iter()
            .any(|warning| warning.code == "P004")
    );
}
