use sanim_core::{Fraction, Property, Value};
use sanim_lang::{Engine, MemorySources, Reload, Session};
use std::collections::BTreeMap;
fn run(text: &str) -> Result<sanim_lang::Prepared, sanim_core::Diagnostic> {
    let mut files = MemorySources::default();
    files.insert("main.sanim", text);
    Engine::default().prepare("main.sanim", &files, &BTreeMap::new())
}
#[test]
fn repeated_shift_uses_current_timeline_state() {
    let p=run("let marker = Circle(id: \"marker\")\nrepeat 4 { play(Shift(marker, Vec2(10pt,0pt)), duration: 1s) }\n").unwrap();
    for (index, track) in p.animation.tracks["marker"][&Property::Position]
        .iter()
        .enumerate()
    {
        assert_eq!(
            track.to.vec2().unwrap().values[0],
            (index + 1) as f64 * 10.0
        );
    }
}
#[test]
fn enum_named_arguments_follow_payload_parameter_order() {
    run("enum Pair { Both(String, Number) }\nlet pair = Pair.Both(value1: 2, value0: \"hello\")\nmatch pair { case Pair.Both(text,count) { assert(text == \"hello\"); assert(count == 2) } }\n").unwrap();
}
#[test]
fn default_elements_have_per_call_identity() {
    let p=run("fn make(element: Element = Circle()): Element { return element }\nlet a=make()\nlet b=make()\nadd(a)\nadd(b)\n").unwrap();
    assert_eq!(p.animation.element_count(), 2);
}
#[test]
fn empty_list_concatenation_preserves_concrete_types() {
    let e = run("let empty: List<Number> = []\nlet mixed = (empty + [1]) + [\"x\"]\n").unwrap_err();
    assert_eq!(e.code, "S106");
}
#[test]
fn hot_reload_birth_time_marks_layer_and_children_changed() {
    let mut files = MemorySources::default();
    files.insert("main.sanim","reflect delay: Duration = 1s\nwait(delay)\nadd(Group(id: \"parent\", children: [Circle(id: \"child\")]))\n");
    let mut session = Session::new("main.sanim");
    session.poll(&files);
    session.set(
        "delay",
        Value::Number(sanim_core::Number {
            value: Fraction::from(2),
            unit: sanim_core::Unit::Duration,
        }),
    );
    let Reload::Updated {
        changed_elements, ..
    } = session.poll(&files)
    else {
        panic!()
    };
    assert_eq!(changed_elements, vec!["child", "parent"]);
}
#[test]
fn hot_reload_paint_order_marks_changed_layers() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        "let a = Circle(id: \"a\")\nlet b = Rect(id: \"b\")\nadd(a)\nadd(b)\n",
    );
    let mut session = Session::new("main.sanim");
    session.poll(&files);
    files.insert(
        "main.sanim",
        "let a = Circle(id: \"a\")\nlet b = Rect(id: \"b\")\nadd(b)\nadd(a)\n",
    );
    let Reload::Updated {
        changed_elements, ..
    } = session.poll(&files)
    else {
        panic!()
    };
    assert_eq!(changed_elements, vec!["a", "b"]);
}
#[test]
fn circular_import_points_to_closing_import_and_recovers() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "import \"other\" as other\nwait(1s)\n");
    files.insert("other.sanim", "let value = 1\n");
    let mut session = Session::new("main.sanim");
    session.poll(&files);
    files.insert("other.sanim", "let value = 1\nimport \"main\" as main\n");
    let Reload::Failed {
        diagnostic,
        last_good_revision,
    } = session.poll(&files)
    else {
        panic!()
    };
    assert_eq!(last_good_revision, Some(1));
    assert_eq!(diagnostic.source, "other.sanim");
    assert_eq!(diagnostic.span.line, 2);
    assert!(
        diagnostic
            .message
            .contains("main.sanim -> other.sanim -> main.sanim")
            && diagnostic.message.contains("third module")
    );
    files.insert("other.sanim", "let value = 2\n");
    assert!(matches!(session.poll(&files), Reload::Updated { .. }));
}

#[test]
fn generated_concatenations_are_bounded_before_allocation() {
    let mut files = MemorySources::default();
    let mut script = String::from("let a0=[1,2,3,4]\n");
    for i in 1..16 {
        script.push_str(&format!("let a{i}=a{}+a{}\n", i - 1, i - 1));
    }
    files.insert("main.sanim", script);
    let mut engine = Engine::default();
    engine.limits.data.max_bytes = 4096;
    let e = engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap_err();
    assert_eq!(e.code, "S303");
    assert!(e.message.contains("budget"));
}

#[test]
fn flow_position_conflicts_in_same_play_fail_before_scheduling() {
    for motions in [
        "FadeIn(row), MoveTo(child, Vec2(10pt,0pt))",
        "MoveTo(child, Vec2(10pt,0pt)), FadeIn(row)",
    ] {
        let e=run(&format!("let child=Circle(id: \"child\")\nlet row=Row(children: [child])\nplay({motions}, duration: 1s)\n")).unwrap_err();
        assert!(e.message.contains("flow layout controls"), "{e}");
    }
}

#[test]
fn changed_flow_child_tracks_invalidate_sibling_layout() {
    let mut files = MemorySources::default();
    files.insert("main.sanim","reflect factor=2\nlet a=Rect(id: \"a\")\nlet b=Rect(id: \"b\")\nadd(Row(id: \"row\", children: [a,b]))\nplay(ScaleTo(a, factor), duration: 1s)\n");
    let mut session = Session::new("main.sanim");
    session.poll(&files);
    session.set("factor", Value::Number(sanim_core::Number::scalar(3)));
    let Reload::Updated {
        changed_elements, ..
    } = session.poll(&files)
    else {
        panic!()
    };
    assert_eq!(changed_elements, vec!["a", "b", "row"]);
}
