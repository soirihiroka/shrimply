use sanim_core::{Font, Geometry, Type, Value};
use sanim_lang::{Engine, MemorySources};
use std::collections::BTreeMap;

#[test]
fn native_font_constructor_infers_font_and_shapes_without_file_dependencies() {
    let mut files = MemorySources::default();
    files.insert("main.sanim","let typeface=Font(\"Sanim Missing Preferred\",\"sans-serif\")\nlet label=Text(\"Stack & Queue\",font=typeface)\nadd(label)\nwait(1s)");
    let mut engine = Engine::default();
    let prepared = engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(prepared.bindings["typeface"].ty(), Type::Font);
    let Value::Font(Font::Family(families)) = &prepared.bindings["typeface"] else {
        panic!("family font")
    };
    assert_eq!(families.as_ref(), ["Sanim Missing Preferred", "sans-serif"]);
    assert_eq!(prepared.dependencies.len(), 1);
    let Geometry::Text { outline, .. } = prepared.animation.layers[0].element.geometry.as_ref()
    else {
        panic!("text")
    };
    assert!(!outline.is_empty());
    let warm = engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(prepared.animation, warm.animation);
}

#[test]
fn font_defaults_fallback_types_and_diagnostics_are_explicit() {
    for expression in [
        "Font()",
        "Font(\"sans-serif\")",
        "Font(\"sans-serif\",\"sans-serif\")",
    ] {
        let mut files = MemorySources::default();
        files.insert("main.sanim", format!("let typeface={expression}"));
        let prepared = Engine::default()
            .prepare("main.sanim", &files, &BTreeMap::new())
            .unwrap();
        assert_eq!(
            prepared.bindings["typeface"],
            Value::Font(Font::family("sans-serif", "sans-serif").unwrap())
        );
    }
    for expression in [
        "Font(12)",
        "Font(\"sans-serif\",false)",
        "Font(\"\")",
        "Font(\"Arial,\")",
    ] {
        let mut files = MemorySources::default();
        files.insert("main.sanim", format!("let typeface={expression}"));
        assert!(
            Engine::default()
                .prepare("main.sanim", &files, &BTreeMap::new())
                .is_err(),
            "{expression}"
        );
    }
}

#[test]
fn css_family_fallback_lists_are_native_font_values() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        "let typeface=Font(\"'Google Sans', Arial, sans-serif\")",
    );
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(
        prepared.bindings["typeface"],
        Value::Font(Font::Family(
            vec!["Google Sans".into(), "Arial".into(), "sans-serif".into()].into()
        ))
    );
}

#[test]
fn text_without_font_uses_the_sans_default_without_asset_dependencies() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        "let label=Text(\"Hello\")\nadd(label)\nwait(0.5s)",
    );
    let mut engine = Engine::default();
    let default = engine
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(default.dependencies.len(), 1);
    let planned = Engine::default()
        .duration("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(planned.duration, default.animation.duration);
    files.insert(
        "main.sanim",
        "let label=Text(\"Hello\",font=Font())\nadd(label)\nwait(0.5s)",
    );
    let explicit = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(default.animation, explicit.animation);
}

#[test]
fn axis_labels_share_the_same_optional_font_default() {
    let mut files = MemorySources::default();
    files.insert("main.sanim","let scale=LinearScale(domain=Vec2(0,10),range=Vec2(0pt,300pt))\nlet axis=Axis(scale=scale)\nadd(axis)\nwait(1s)");
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &BTreeMap::new())
        .unwrap();
    assert_eq!(prepared.dependencies.len(), 1);
    let Geometry::Box { children, .. } = prepared.animation.layers[0].element.geometry.as_ref()
    else {
        panic!("axis")
    };
    assert!(
        children
            .iter()
            .any(|child| matches!(child.geometry.as_ref(), Geometry::Text { .. }))
    );
}
