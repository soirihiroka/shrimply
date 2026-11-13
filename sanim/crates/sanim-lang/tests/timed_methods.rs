use sanim_core::Fraction;
use sanim_lang::{Engine, MemorySources};
use std::collections::BTreeMap;

#[test]
fn timed_relationship_methods_match_native_duration_and_preserve_receiver_effects() {
    for source in [
        "let a=Rect()\nlet b=Circle()\nlet relation=a.left_of(b,duration=1s)",
        "let a=Rect()\nlet b=Circle()\nlet relation=a.left_of(b,0pt,1s)",
        "let b=Circle()\nlet relation=Rect(duration=250ms).left_of(b,duration=750ms)",
        "let a=Rect()\nlet b=Circle()\nlet duration=read<Duration>(\"time.json\")\nlet relation=a.left_of(b,duration=duration)",
        "fn relate():Unit{let a=Rect();let b=Circle();let relation=a.left_of(b,duration=1s)}\nrelate()",
    ] {
        let mut files = MemorySources::default();
        files.insert("main.sanim", source);
        files.insert("time.json", "\"1s\"");
        let full = Engine::default()
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap();
        let planned = Engine::default()
            .duration("main.sanim", &files, &BTreeMap::new())
            .unwrap();
        assert_eq!(full.animation.duration, Fraction::from(1), "{source}");
        assert_eq!(planned.duration, full.animation.duration, "{source}");
    }
}

#[test]
fn timed_relationship_methods_cannot_hide_inside_pure_or_imported_code() {
    for (source, code) in [
        (
            "let a=Rect()\nlet b=Circle()\nsig relation=a.left_of(b,duration=1s)",
            "S131",
        ),
        (
            "let a=Rect()\nlet b=Circle()\nfn hidden():Number{let relation=a.left_of(b,duration=1s);return 1}\nlet value=reflect(hidden())",
            "S113",
        ),
    ] {
        let mut files = MemorySources::default();
        files.insert("main.sanim", source);
        let error = Engine::default()
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap_err();
        assert_eq!(error.code, code, "{error}");
    }
    let mut files = MemorySources::default();
    files.insert("main.sanim", "import \"theme.sanim\" as theme\nwait(1s)");
    files.insert(
        "theme.sanim",
        "let a=Rect()\nlet b=Circle()\nlet relation=a.left_of(b,duration=1s)",
    );
    for duration_only in [false, true] {
        let error = if duration_only {
            Engine::default()
                .duration("main.sanim", &files, &BTreeMap::new())
                .unwrap_err()
        } else {
            Engine::default()
                .prepare("main.sanim", &files, &BTreeMap::new())
                .unwrap_err()
        };
        assert_eq!(error.code, "S115", "{error}");
    }
}

#[test]
fn zero_time_envelopes_do_not_become_retained_math_or_vector_inputs() {
    let mut files = MemorySources::default();
    files.insert("main.sanim","mut x=1\nsig y=abs(x,duration=0s)\nsig position=Vec2(x*1pt,0pt,duration=0s)\nfn double(value:Number):Number{return value*2}\nsig clipped=clamp(max(min(y,100),0),0,10,duration=0s)\nsig radius=double(clipped,duration=0s)*1pt\nlet marker=Circle(radius=radius,position=position,fill=Color(duration=0s),duration=0s)\nadd(marker)\nbackground{animate(x,to=3,duration=1s)}\nwait(1s)");
    let full = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    let at_end = full.animation.sample(Fraction::from(1)).unwrap();
    let sanim_core::Geometry::Circle { radius } = at_end[0].geometry.as_ref() else {
        panic!("circle expected")
    };
    assert_eq!(*radius, Fraction::from(6));
    assert_eq!(at_end[0].position.values, [3.0, 0.0]);
}

#[test]
fn shared_exact_math_preserves_fraction_types_units_and_validation() {
    let mut files = MemorySources::default();
    files.insert("main.sanim","let exact:Fraction=clamp(abs(Fraction(-1,3)),Fraction(1,4),Fraction(1,2))\nassert(exact==Fraction(1,3))\nassert(min(2pt,3pt)==2pt)\nassert(max(2s,3s)==3s)");
    Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    files.insert("main.sanim", "let invalid=clamp(2,3,1)");
    let error = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap_err();
    assert!(
        error.message.contains("clamp low must not exceed high"),
        "{error}"
    );
}
