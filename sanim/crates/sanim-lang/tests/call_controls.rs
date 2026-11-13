use sanim_core::Fraction;
use sanim_lang::{Engine, MemorySources};
use std::collections::BTreeMap;

#[test]
fn duplicate_controls_and_named_before_positional_calls_fail_before_filtering() {
    for (source, expected) in [
        (
            "fn helper():Number{return 1}\nlet result=helper(duration=1s,duration=2s)",
            "duration",
        ),
        (
            "fn helper():Number{return 1}\nlet result=helper(ease=\"linear\",ease=\"spring\")",
            "ease",
        ),
        (
            "type Entry={n:Number}\nlet result=Entry(n=1,duration=1s,duration=2s)",
            "duration",
        ),
        (
            "type Entry={n:Number}\nlet result=Entry(n=1,ease=\"linear\",ease=\"spring\")",
            "ease",
        ),
        (
            "fn helper(n:Number):Number{return n}\nlet result=helper(duration=1s,2)",
            "positional",
        ),
        (
            "type Entry={n:Number}\nlet result=Entry(ease=\"linear\",2)",
            "positional",
        ),
        ("let result=Color(value=\"red\",value=\"blue\")", "value"),
        (
            "let first=Rect()\nlet second=Circle()\nlet result=first.left_of(second,duration=1s,duration=2s)",
            "duration",
        ),
        (
            "fn helper(n:Number):Number{return n}\nlet result=helper(1,n=2)",
            "n",
        ),
    ] {
        let mut files = MemorySources::default();
        files.insert("main.sanim", source);
        let error = Engine::default()
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap_err();
        assert_eq!(error.code, "S124", "{source}: {error}");
        assert!(error.message.contains(expected), "{source}: {error}");
    }
}

#[test]
fn declared_duration_and_ease_parameters_remain_values() {
    let mut files = MemorySources::default();
    files.insert("main.sanim","type Settings={duration:Number,ease:Number}\nlet config=Settings(duration=2,ease=3)\nfn helper(duration:Number,ease:Number):Number{return duration+ease}\nlet result=helper(2,3)\nfn untimed():Number{return 7}\nlet enveloped=untimed(duration=0s,ease=\"linear\")\nwait(1s)");
    let full = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(
        full.bindings["result"].number().unwrap().value,
        Fraction::from(5)
    );
    assert_eq!(
        full.bindings["enveloped"].number().unwrap().value,
        Fraction::from(7)
    );
    assert_eq!(full.animation.duration, Fraction::from(1));
    let planned = Engine::default()
        .duration("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(planned.duration, full.animation.duration);
}
