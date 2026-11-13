use sanim_core::{Fraction, Number, Value};
use sanim_lang::{Engine, MemorySources, Reload, Session};
use std::collections::BTreeMap;

#[test]
fn unchanged_sources_do_not_prepare_again() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "wait(1s)\n");
    let mut session = Session::new("main.sanim");
    assert!(matches!(
        session.poll(&files),
        Reload::Updated { revision: 1, .. }
    ));
    assert!(matches!(session.poll(&files), Reload::Unchanged));
}
#[test]
fn invalid_edit_preserves_last_good_and_recovers() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "wait(1s)\n");
    let mut session = Session::new("main.sanim");
    session.poll(&files);
    files.insert("main.sanim", "wait(\n");
    assert!(matches!(
        session.poll(&files),
        Reload::Failed {
            last_good_revision: Some(1),
            ..
        }
    ));
    assert_eq!(
        session.current().unwrap().animation.duration,
        Fraction::from(1)
    );
    assert!(matches!(session.poll(&files), Reload::Unchanged));
    files.insert("main.sanim", "wait(2s)\n");
    assert!(matches!(
        session.poll(&files),
        Reload::Updated { revision: 2, .. }
    ));
}
#[test]
fn missing_resource_creation_recovers() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        "let data = read<List<Number>>(\"data.json\")\nwait(1s)\n",
    );
    let mut session = Session::new("main.sanim");
    assert!(matches!(session.poll(&files), Reload::Failed { .. }));
    files.insert("data.json", "[1,2]");
    assert!(matches!(session.poll(&files), Reload::Updated { .. }));
}
#[test]
fn changing_data_invalidates_dependent_values() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        "let data = read<List<Number>>(\"data.json\")\nwait(len(data) * 1s)\n",
    );
    files.insert("data.json", "[1]");
    let mut session = Session::new("main.sanim");
    session.poll(&files);
    files.insert("data.json", "[1,2,3]");
    assert!(matches!(
        session.poll(&files),
        Reload::Updated { revision: 2, .. }
    ));
    assert_eq!(
        session.current().unwrap().animation.duration,
        Fraction::from(3)
    );
}
#[test]
fn imported_module_changes_are_watched() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "import \"lib\" as lib\nwait(lib.duration)\n");
    files.insert("lib.sanim", "let duration = 1s\n");
    let mut session = Session::new("main.sanim");
    session.poll(&files);
    files.insert("lib.sanim", "let duration = 2s\n");
    assert!(matches!(
        session.poll(&files),
        Reload::Updated { revision: 2, .. }
    ));
    assert_eq!(session.current().unwrap().dependencies.len(), 2);
}
#[test]
fn source_comments_publish_new_provenance_without_visual_changes() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "wait(1s)\n");
    let mut session = Session::new("main.sanim");
    session.poll(&files);
    let old = session.current().unwrap().clone();
    files.insert("main.sanim", "wait(1s)\n# comment\n");
    let Reload::Updated {
        prepared,
        changed_elements,
        ..
    } = session.poll(&files)
    else {
        panic!("changed source provenance must be published")
    };
    assert_ne!(old.fingerprint, prepared.fingerprint);
    assert!(changed_elements.is_empty());
    assert_eq!(old.animation, prepared.animation);
    assert_eq!(old.bindings, prepared.bindings);
}
#[test]
fn reflected_patch_reports_changed_elements() {
    let mut files = MemorySources::default();
    files.insert("main.sanim","let tint: Color=reflect(Color(\"red\"))\nlet marker = Circle(id= \"marker\", fill= tint)\nadd(marker)\n");
    let mut session = Session::new("main.sanim");
    session.poll(&files);
    session.set(
        "main.sanim::tint",
        Value::Color(sanim_core::Color::parse("blue").unwrap()),
    );
    let Reload::Updated {
        changed_elements,
        prepared,
        ..
    } = session.poll(&files)
    else {
        panic!()
    };
    assert_eq!(changed_elements, vec!["marker"]);
    assert!(prepared.statistics.reused_expressions > 0);
}
#[test]
fn warm_prepare_reuses_assets_and_expressions() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        "let data = read<List<Number>>(\"data.json\")\nlet result = len(data) * 2\n",
    );
    files.insert("data.json", "[1,2]");
    let mut engine = Engine::default();
    let first = engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    let second = engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(first.fingerprint, second.fingerprint);
    assert_eq!(second.statistics.decoded_assets, 0);
    assert_eq!(second.statistics.restored_statements, 2);
    assert_eq!(second.statistics.reused_assets, 0);
    assert_eq!(second.statistics.evaluated_expressions, 0);
    assert_eq!(second.statistics.operations, 0);
}
#[test]
fn invalid_override_is_transactional() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        "let count=reflect(1)\nrepeat count { wait(1s) }\n",
    );
    let mut session = Session::new("main.sanim");
    session.poll(&files);
    session.set("main.sanim::count", Value::String("bad".into()));
    assert!(matches!(session.poll(&files), Reload::Failed { .. }));
    assert_eq!(
        session.current().unwrap().animation.duration,
        Fraction::from(1)
    );
    session.set("main.sanim::count", Value::Number(Number::scalar(2)));
    assert!(matches!(session.poll(&files), Reload::Updated { .. }));
}
#[test]
fn work_warnings_have_source_locations() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "repeat 10 { wait(1ms) }\n");
    let mut engine = Engine::default();
    engine.limits.warn_operations = 10;
    let prepared = engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert!(
        prepared
            .performance
            .iter()
            .any(|v| v.code == "P001" && v.span.line == 1 && v.operations >= 10)
    );
}
#[test]
fn cyclic_import_fails_with_chain() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "import \"other\" as other\n");
    files.insert("other.sanim", "import \"main\" as main\n");
    let e = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap_err();
    assert_eq!(e.code, "S202");
    assert!(
        e.message
            .contains("main.sanim -> other.sanim -> main.sanim")
    );
}
#[test]
fn imported_helpers_can_schedule_when_called() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "import \"lib\" as lib\nlib.pause()\n");
    files.insert("lib.sanim", "fn pause(): Unit { wait(1s) }\n");
    let p = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(p.animation.duration, Fraction::from(1));
}

#[test]
fn viewport_changes_report_all_affected_elements() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "add(Circle(id= \"a\"))\n");
    let mut session = Session::new("main.sanim");
    session.engine.viewport.width = 100;
    session.engine.viewport.height = 100;
    session.poll(&files);
    session.engine.viewport.width = 200;
    let Reload::Updated {
        changed_elements, ..
    } = session.poll(&files)
    else {
        panic!()
    };
    assert_eq!(changed_elements, vec!["a"]);
}
#[test]
fn parent_changes_report_descendant_dependencies() {
    let mut files = MemorySources::default();
    files.insert("main.sanim","let offset: Length=reflect(0pt)\nlet child = Circle(id= \"child\")\nadd(Box(id= \"parent\", children= [child], position= Vec2(offset, 0pt)))\n");
    let mut session = Session::new("main.sanim");
    session.poll(&files);
    session.set(
        "main.sanim::offset",
        Value::Number(Number {
            value: Fraction::from(10),
            unit: sanim_core::Unit::Length,
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
fn reactive_cell_track_edits_report_bound_elements_but_not_unbound_cells() {
    for bound in [true, false] {
        let program = |turns| {
            format!(
                "mut phase=0deg\nsig size=32pt+8pt*sin(phase)\nlet shape=Rect(width={},height=32pt)\nadd(shape)\nanimate(phase,to={turns}deg,duration=1s)",
                if bound { "size" } else { "32pt" }
            )
        };
        let mut files = MemorySources::default();
        files.insert("main.sanim", program(360));
        let mut session = Session::new("main.sanim");
        assert!(matches!(session.poll(&files), Reload::Updated { .. }));
        let old = session.current().unwrap().clone();
        files.insert("main.sanim", program(720));
        let Reload::Updated {
            prepared,
            changed_elements,
            ..
        } = session.poll(&files)
        else {
            panic!("reactive edit must prepare successfully")
        };
        assert_eq!(
            old.animation.layers, prepared.animation.layers,
            "initial elements must remain unchanged"
        );
        assert_eq!(
            old.animation.tracks, prepared.animation.tracks,
            "only retained cell tracks change"
        );
        assert_eq!(changed_elements, if bound { vec!["shape"] } else { vec![] });
    }
}

#[test]
fn reactive_edits_report_future_mounted_list_items_and_exclude_unmounted_catalogs() {
    let program = |turns| {
        format!(
            "mut phase=0deg\nsig size=32pt+8pt*sin(phase)\nlet future=Rect(width=size,height=32pt)\nlet hidden_item=Rect(width=size,height=32pt)\nmut items:List<Element>=[]\nmut hidden_items:List<Element>=[]\nlet view=Box(children=items,width=100pt,height=100pt)\nlet hidden_view=Box(children=hidden_items,width=100pt,height=100pt)\nadd(view)\nwait(1s)\nitems=append(items,future)\nhidden_items=append(hidden_items,hidden_item)\nanimate(phase,to={turns}deg,duration=1s)"
        )
    };
    let mut files = MemorySources::default();
    files.insert("main.sanim", program(360));
    let mut session = Session::new("main.sanim");
    assert!(matches!(session.poll(&files), Reload::Updated { .. }));
    let old = session.current().unwrap().clone();
    assert!(old.animation.find("future").is_some());
    assert!(
        old.animation.layers[0].element.find("future").is_none(),
        "item must be absent from the static initial tree"
    );
    files.insert("main.sanim", program(720));
    let Reload::Updated {
        changed_elements, ..
    } = session.poll(&files)
    else {
        panic!("reactive list edit must prepare successfully")
    };
    assert_eq!(
        changed_elements,
        vec!["future", "view"],
        "only reachable item and its layout parent are affected"
    );
}
