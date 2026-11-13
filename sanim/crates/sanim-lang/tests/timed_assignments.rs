use sanim_core::{Fraction, Geometry};
use sanim_lang::{Engine, MemorySources};
use std::collections::BTreeMap;

#[test]
fn list_transition_starts_after_timed_arguments_finish() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "mut items:List<Element>=[]\nlet row=Row(children=items,width=100pt,height=100pt)\nadd(row)\nitems=append(items,Rect(width=10pt,height=10pt,duration=1s),duration=500ms)\nwait(1s)");
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    let (width, height) = prepared.animation.logical_size();
    let at = |time| {
        let elements = prepared.animation.layout(time, width, height).unwrap();
        let Geometry::Group { children } = elements[0].geometry.as_ref() else {
            panic!("row")
        };
        children.clone()
    };
    assert!(at(Fraction::new(3u64, 4u64)).is_empty());
    assert_eq!(
        at(Fraction::new(5u64, 4u64))[0].opacity,
        Fraction::new(1u64, 2u64)
    );
    assert_eq!(at(Fraction::new(3u64, 2u64))[0].opacity, Fraction::from(1));
    assert_eq!(prepared.animation.duration, Fraction::new(5u64, 2u64));
}

#[test]
fn imported_call_provenance_and_timed_assignment_use_the_caller_span() {
    let mut files = MemorySources::default();
    files.insert(
        "helpers.sanim",
        "fn target(value:Length):Length{return value}",
    );
    files.insert("main.sanim", "import \"helpers\" as helpers\nmut radius=10pt\nlet marker=Circle(radius=radius)\nadd(marker)\nradius=helpers.target(abs(30pt,duration=1s),duration=500ms)\nwait(1s)");
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    let marker = prepared.animation.find("marker").unwrap();
    let radius = |time| {
        prepared
            .animation
            .sample_property(marker, sanim_core::Property::Radius, time)
            .unwrap()
            .number()
            .unwrap()
            .value
    };
    assert_eq!(radius(Fraction::new(3u64, 4u64)), Fraction::from(10));
    assert_eq!(radius(Fraction::new(5u64, 4u64)), Fraction::from(20));
    assert_eq!(radius(Fraction::new(3u64, 2u64)), Fraction::from(30));
    assert!(
        prepared
            .time_map
            .entries
            .iter()
            .any(|entry| entry.source == "main.sanim"
                && entry.span.line == 5
                && entry.start == Fraction::from(1)
                && entry.end == Fraction::new(3u64, 2u64))
    );
}

#[test]
fn duration_planning_skips_animated_list_geometry_but_preserves_needed_mutations() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "let font=read<Font>(\"missing.ttf\")\nfn tile():Element{return Text(\"test\",font=font)}\nmut items:List<Element>=[]\nlet row=Row(children=items,width=100pt,height=100pt)\nadd(row)\nitems=append(items,tile(),duration=500ms)\nwait(1s)");
    let planned = Engine::default()
        .duration("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(planned.duration, Fraction::new(3u64, 2u64));
    assert!(!planned.dependencies.contains_key("missing.ttf"));
    for (source, expected) in [
        (
            "mut sizes:List<Duration>=[0.5s]\nsizes=append(sizes,1s)\nfor delay in sizes{wait(delay)}",
            Fraction::new(3u64, 2u64),
        ),
        (
            "mut count=1\nfn next():Number{count=count+1;return 99}\nmut ignored=0\nignored=next()\nwait(count*1s)",
            Fraction::from(2),
        ),
        (
            "mut counter=0\nfn answer():Number{counter=counter+1;return 2}\nwait(answer()*1s)",
            Fraction::from(2),
        ),
        (
            "mut counter=0\nfn answer():Number{counter=1;return 2}\nwait(answer()*1s)",
            Fraction::from(2),
        ),
    ] {
        files.insert("main.sanim", source);
        let planned = Engine::default()
            .duration("main.sanim", &files, &BTreeMap::new())
            .unwrap();
        let full = Engine::default()
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap();
        assert_eq!(planned.duration, expected);
        assert_eq!(planned.duration, full.animation.duration);
    }
}

#[test]
fn zero_time_operators_do_not_borrow_their_operands_timeline_interval() {
    for expression in ["abs(10pt,duration=1s)+1pt", "+abs(11pt,duration=1s)"] {
        let mut files = MemorySources::default();
        files.insert("main.sanim",format!("mut size=1pt\nlet marker=Circle(radius=size)\nadd(marker)\nsize={expression}\nwait(1s)"));
        let prepared = Engine::default()
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap();
        let marker = prepared.animation.find("marker").unwrap();
        let radius = |time| {
            prepared
                .animation
                .sample_property(marker, sanim_core::Property::Radius, time)
                .unwrap()
                .number()
                .unwrap()
                .value
        };
        assert_eq!(radius(Fraction::new(1u64, 2u64)), Fraction::from(1));
        assert_eq!(radius(Fraction::from(1)), Fraction::from(11));
    }
    for expression in ["[abs(1,duration=1s)]", "append([],abs(1,duration=1s))"] {
        let mut files = MemorySources::default();
        files.insert("main.sanim",format!("mut rows:List<Number>=[]\nbackground{{rows={expression}}}\nlet before=len(rows)\nwait(1s)\nlet after=len(rows)"));
        let prepared = Engine::default()
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap();
        assert_eq!(
            prepared.bindings["before"].number().unwrap().value,
            Fraction::from(0)
        );
        assert_eq!(
            prepared.bindings["after"].number().unwrap().value,
            Fraction::from(1)
        );
    }
    let mut files = MemorySources::default();
    files.insert("main.sanim", "mut state={value=0}\nbackground{state={value=abs(1,duration=1s)}}\nlet before=state.value\nwait(1s)\nlet after=state.value");
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(
        prepared.bindings["before"].number().unwrap().value,
        Fraction::from(0)
    );
    assert_eq!(
        prepared.bindings["after"].number().unwrap().value,
        Fraction::from(1)
    );
}

#[test]
fn planned_duration_observes_intermediate_background_value_transitions() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        "mut phase=0\nbackground{phase=abs(10,duration=2s)}\nwait(1s)\nwait(phase*1s)",
    );
    let full = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    let planned = Engine::default()
        .duration("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(full.animation.duration, Fraction::from(6));
    assert_eq!(planned.duration, full.animation.duration);
}
