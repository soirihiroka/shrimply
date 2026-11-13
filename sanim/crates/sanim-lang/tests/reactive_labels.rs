use sanim_core::{Fraction, Geometry, Property, Value};
use sanim_lang::{Engine, MemorySources};

#[test]
fn label_content_font_and_font_size_follow_the_same_retained_timeline() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        r#"mut count=0
sig words="value={count:.0f}"
mut typeface=Font("sans-serif")
mut size=16pt
let title=Label(words,font=typeface,font_size=size)
add(title)
background { animate(count,to=10,duration=1s,ease="linear") }
background { animate(size,to=32pt,duration=1s,ease="linear") }
wait(500ms)
typeface=Font("monospace")
wait(500ms)
"#,
    );
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    let title = prepared.animation.find("title").unwrap();
    assert_eq!(
        prepared
            .animation
            .sample_property(title, Property::Text, 0.into())
            .unwrap()
            .string()
            .unwrap(),
        "value=0"
    );
    assert_eq!(
        prepared
            .animation
            .sample_property(title, Property::Text, 1.into())
            .unwrap()
            .string()
            .unwrap(),
        "value=10"
    );
    let half = Fraction::new(1u64, 2u64);
    assert_eq!(
        prepared
            .animation
            .sample_property(title, Property::FontSize, half)
            .unwrap()
            .number()
            .unwrap()
            .value,
        24.into()
    );
    let Value::Font(font) = prepared
        .animation
        .sample_property(title, Property::Font, half)
        .unwrap()
    else {
        panic!("font")
    };
    assert_eq!(
        font,
        sanim_core::Font::family("monospace", "sans-serif").unwrap()
    );
    let sampled = prepared.animation.sample(1.into()).unwrap();
    assert!(matches!(
        sampled[0].geometry.as_ref(),
        Geometry::Text { pending: true, .. }
    ));
    assert!(
        sanim_core::verify::bounds(&sampled[0])
            .unwrap_err()
            .contains("unresolved")
    );
}

#[test]
fn formatted_strings_can_feed_markup_labels_without_explicit_sig_bindings() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "mut count=1\nlet title=<Label>count={count}</Label>\nadd(title)\nwait(1s)\ncount=2\nwait(1s)\n");
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    let title = prepared.animation.find("title").unwrap();
    assert_eq!(
        prepared
            .animation
            .sample_property(title, Property::Text, 0.into())
            .unwrap()
            .string()
            .unwrap(),
        "count=1"
    );
    assert_eq!(
        prepared
            .animation
            .sample_property(title, Property::Text, 1.into())
            .unwrap()
            .string()
            .unwrap(),
        "count=2"
    );
}

#[test]
fn label_factories_preserve_signal_arguments_and_dynamic_font_constructors() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        r#"mut family="sans-serif"
mut count=1
sig typeface=Font(family)
fn make_label(sig words:String,sig font:Font):Element {
    return Label(words,font=font)
}

let title=make_label("count={count}",typeface)
add(title)
wait(1s)
family="monospace"
count=2
wait(1s)
"#,
    );
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    let Value::Element(title) = &prepared.bindings["title"] else {
        panic!("label")
    };
    assert_eq!(
        prepared
            .animation
            .sample_property(title, Property::Text, 1.into())
            .unwrap()
            .string()
            .unwrap(),
        "count=2"
    );
    let Value::Font(font) = prepared
        .animation
        .sample_property(title, Property::Font, 1.into())
        .unwrap()
    else {
        panic!("font")
    };
    assert_eq!(
        font,
        sanim_core::Font::family("monospace", "sans-serif").unwrap()
    );
}
#[test]
fn native_alignment_arguments_accept_typed_signal_values() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "mut alignment=Align.Start\nlet title=Label(\"aligned\",x_align=alignment,y_align=alignment)\nadd(title)\nwait(1s)\nalignment=Align.End\nwait(1s)\n");
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    let title = prepared.animation.find("title").unwrap();
    for property in [Property::XAlign, Property::YAlign] {
        let Value::Variant { tag, .. } = prepared
            .animation
            .sample_property(title, property, 1.into())
            .unwrap()
        else {
            panic!("typed alignment")
        };
        assert_eq!(tag, "End");
    }
}

#[test]
fn text_constructor_getter_and_setter_share_language_string_allocations() {
    use std::sync::Arc;
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        "let words=\"Shared authored text\"\nlet label=Text(words)\nadd(label)",
    );
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    let Value::String(words) = &prepared.bindings["words"] else {
        panic!("String")
    };
    let source = prepared.animation.find("label").unwrap();
    let Geometry::Text { text, .. } = source.geometry.as_ref() else {
        panic!("Text")
    };
    assert!(
        Arc::ptr_eq(words, text),
        "construction must retain the language string allocation"
    );
    let Value::String(getter) = source.property(Property::Text).unwrap() else {
        panic!("String")
    };
    assert!(Arc::ptr_eq(words, &getter));
    let mut changed = source.clone();
    let replacement: Arc<str> = "Changed text".into();
    changed
        .set(Property::Text, Value::String(replacement.clone()))
        .unwrap();
    let Geometry::Text { text, pending, .. } = changed.geometry.as_ref() else {
        panic!("Text")
    };
    assert!(Arc::ptr_eq(text, &replacement));
    assert!(*pending);
    assert_eq!(
        words.as_ref(),
        "Shared authored text",
        "COW mutation must leave the authored source intact"
    );
}
