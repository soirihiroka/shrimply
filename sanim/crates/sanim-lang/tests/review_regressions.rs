use sanim_core::{Fraction, Property, Value};
use sanim_lang::{Engine, MemorySources, Reload, Session};
use std::collections::BTreeMap;
fn run(text: &str) -> Result<sanim_lang::Prepared, sanim_core::Diagnostic> {
    let mut files = MemorySources::default();
    files.insert("main.sanim", format!("{}\nAnimation()", text));
    Engine::default().prepare("main.sanim", &files, &BTreeMap::new())
}
#[test]
fn repeated_shift_uses_current_timeline_state() {
    let p=run("let marker = Box(id=\"marker\", width=2*(32pt), height=2*(32pt), radius=\"full\", fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)\nrepeat 4 { marker.shift([10pt, 0pt], duration=1s, ease=\"linear\") }\n").unwrap();
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
    run("enum Pair { Both(String, Number) }\nlet pair = Pair.Both(value1= 2, value0= \"hello\")\nmatch pair { case Pair.Both(text,count) { assert(text == \"hello\"); assert(count == 2) } }\n").unwrap();
}
#[test]
fn default_elements_have_per_call_identity() {
    let p=run("fn make(element: Element = Box(width=2*(32pt), height=2*(32pt), radius=\"full\", fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)): Element { return element }\nlet a=make()\nlet b=make()\nAnimation(children=[a])\nAnimation(children=[b])\n").unwrap();
    assert_eq!(p.animation.element_count(), 2);
}
#[test]
fn empty_list_concatenation_preserves_concrete_types() {
    let e = run("let empty: List<Number> = []\nlet mixed = concat(concat(empty,[1]),[\"x\"])\n")
        .unwrap_err();
    assert!(
        e.message.contains("compatible") || e.message.contains("concaten"),
        "{e}"
    );
}
#[test]
fn hot_reload_birth_time_marks_layer_and_children_changed() {
    let mut files = MemorySources::default();
    files.insert("main.sanim","let delay: Duration=reflect(1s)\nwait(delay)\nAnimation(children=[Box(id= \"parent\", children= [Box(id=\"child\", width=2*(32pt), height=2*(32pt), radius=\"full\", fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)])])\n");
    let mut session = Session::new("main.sanim");
    session.poll(&files);
    session.set(
        "main.sanim::delay",
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
        "let a = Box(id=\"a\", width=2*(32pt), height=2*(32pt), radius=\"full\", fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)\nlet b = Box(id=\"b\", width=64pt, height=64pt, fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)\nAnimation(children=[a])\nAnimation(children=[b])\n",
    );
    let mut session = Session::new("main.sanim");
    session.poll(&files);
    files.insert(
        "main.sanim",
        "let a = Box(id=\"a\", width=2*(32pt), height=2*(32pt), radius=\"full\", fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)\nlet b = Box(id=\"b\", width=64pt, height=64pt, fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)\nAnimation(children=[b])\nAnimation(children=[a])\n",
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
        script.push_str(&format!("let a{i}=concat(a{},a{})\n", i - 1, i - 1));
    }
    files.insert("main.sanim", format!("{}\nAnimation()", script));
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
        "<Animation absolute flow=\"sequence\">{row.fade_in(duration=1s) }</Animation>; child.move_to([10pt, 0pt], duration=1s)",
        "<Animation absolute flow=\"sequence\">{child.move_to([10pt, 0pt], duration=1s) }</Animation>; row.fade_in(duration=1s)",
    ] {
        let e = run(&format!(
            "let child=Box(id=\"child\", width=2*(32pt), height=2*(32pt), radius=\"full\", fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)\nlet row=Box(flex_direction=\"row\",children= [child])\nAnimation(children=[row])\n{motions}\n"
        ))
        .unwrap_err();
        assert!(e.message.contains("flow layout controls"), "{e}");
    }
}

#[test]
fn changed_flow_child_tracks_invalidate_sibling_layout() {
    let mut files = MemorySources::default();
    files.insert("main.sanim","let factor=reflect(2)\nlet a=Box(id=\"a\", width=64pt, height=64pt, fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)\nlet b=Box(id=\"b\", width=64pt, height=64pt, fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)\nAnimation(children=[Box(flex_direction=\"row\",id= \"row\", children= [a,b])])\na.scale_to(factor, duration=1s, ease=\"linear\")\n");
    let mut session = Session::new("main.sanim");
    session.poll(&files);
    session.set(
        "main.sanim::factor",
        Value::Number(sanim_core::Number::scalar(3)),
    );
    let Reload::Updated {
        changed_elements, ..
    } = session.poll(&files)
    else {
        panic!()
    };
    assert_eq!(changed_elements, vec!["a", "b", "row"]);
}
