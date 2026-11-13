use sanim_core::{CornerRadius, Fraction, Geometry, Paint, Property, Value};
use sanim_lang::{Engine, MemorySources};
fn prepare(source: &str) -> Result<sanim_lang::Prepared, sanim_core::Diagnostic> {
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    Engine::default().prepare("main.sanim", &files, &Default::default())
}
#[test]
fn native_markup_lowers_into_the_same_typed_shapes_and_formats_text() {
    let prepared=prepare(r#"let count=7
let label=<Label fill="blue3">hello {{literal}} {count:04d} &amp; more</Label>
let card=<Rect w-96 h-40 pill border-2 border-light1 fill={LinearGradient(from=Color("blue3"),to=Color("purple3"))}><Label>native</Label></Rect>
add(label)
add(card)
wait(1s)"#).unwrap();
    let Geometry::Text { text, .. } = prepared.animation.find("label").unwrap().geometry.as_ref()
    else {
        panic!("Text alias")
    };
    assert_eq!(text.as_ref(), "hello {literal} 0007 & more");
    let card = prepared.animation.find("card").unwrap();
    let Geometry::Rectangle {
        width,
        height,
        radius,
        children,
    } = card.geometry.as_ref()
    else {
        panic!("painted container")
    };
    assert_eq!((*width, *height), (96.into(), 40.into()));
    assert_eq!(radius, &CornerRadius::Pill);
    assert_eq!(children.len(), 1);
    assert!(prepared.animation.find(&children[0].id).is_some());
    assert!(matches!(card.fill, Paint::LinearGradient { .. }));
    assert_eq!(card.stroke.as_ref().unwrap().width, 2.into());
}
#[test]
fn markup_diagnostics_explain_syntax_types_and_design_mistakes() {
    for (source, expected) in [
        ("let x=<Rect><Label>bad</Rect>", "does not match"),
        ("let x=<Rect width=12pt/>", "quoted string or"),
        ("let x=<Rect width={\"bad\"}/>", "Length"),
        ("let x=<Rect fill=\"not-a-color\"/>", "unknown color"),
        ("let x=<Rect nonesuch/>", "unknown native utility"),
        ("let x=<Label card>x</Label>", "does not support"),
        ("let x=<Rect from-blue3/>", "fill={LinearGradient"),
        ("let x=<Rect bg-gradient-to-r/>", "fill={LinearGradient"),
        ("let x=<Rect via-blue3/>", "fill={LinearGradient"),
        ("let x=<Rect to-purple3/>", "fill={LinearGradient"),
        ("let x=<Rect fill=\"blue3\" fill=\"red3\"/>", "duplicate"),
    ] {
        let error = prepare(source).unwrap_err().to_string();
        assert!(error.contains(expected), "{source}: {error}");
    }
}
#[test]
fn markup_signal_arguments_retain_existing_property_animation() {
    let prepared = prepare(
        r#"mut size=30pt
mut tint=Color("blue3")
let shape=<Rect width={size} height={20pt} fill={tint} pill><Label text-sm>signal</Label></Rect>
add(shape)
background {animate(size,to=90pt,duration=1s,ease="linear")}
animate(tint,to=Color("orange3"),duration=1s,ease="linear")
"#,
    )
    .unwrap();
    let frame = prepared
        .animation
        .sample(Fraction::new(1u64, 2u64))
        .unwrap();
    let shape = &frame[0];
    assert_eq!(
        shape.property(Property::Width).unwrap(),
        Value::Number(sanim_core::Number {
            value: 60.into(),
            unit: sanim_core::Unit::Length
        })
    );
    assert_ne!(
        shape.fill,
        Paint::Solid(sanim_core::Color::parse("blue3").unwrap())
    );
    assert!(matches!(
        shape.geometry.as_ref(),
        Geometry::Rectangle {
            radius: CornerRadius::Pill,
            ..
        }
    ));
}
#[test]
fn gradient_values_validate_stops_and_do_not_silently_accept_invalid_data() {
    for (source, expected) in [
        (
            r#"let x=LinearGradient(stops=[{offset=0,color=Color("red")},{offset=0.5,color=Color("blue")}])"#,
            "end at offset 1",
        ),
        (
            r#"let x=LinearGradient(stops=[{offset=0,color=Color("red")},{offset=0.8,color=Color("blue")},{offset=0.7,color=Color("white")},{offset=1,color=Color("black")}])"#,
            "nondecreasing",
        ),
        (
            r#"let x=LinearGradient(from=Color("blue3"),stops=[])"#,
            "not both",
        ),
    ] {
        let error = prepare(source).unwrap_err().to_string();
        assert!(error.contains(expected), "{error}");
    }
}
#[test]
fn markup_interpolation_spans_keep_distinct_cached_values_and_signal_snapshots() {
    let prepared = prepare(
        r#"mut count=1
sig description="count={count}"
count=5
let snapshot=description
let label=<Label>{snapshot} + {6:02d} = {11} / done</Label>
add(label)
wait(1s)"#,
    )
    .unwrap();
    let Geometry::Text { text, .. } = prepared.animation.find("label").unwrap().geometry.as_ref()
    else {
        panic!("label")
    };
    assert_eq!(text.as_ref(), "count=5 + 06 = 11 / done");
    let prepared =
        prepare("let card=<Rect>first<Label>middle</Label>last</Rect>\nadd(card)\nwait(1s)")
            .unwrap();
    let children = prepared
        .animation
        .find("card")
        .unwrap()
        .geometry
        .children()
        .unwrap();
    let text = children
        .iter()
        .map(|child| {
            if let Geometry::Text { text, .. } = child.geometry.as_ref() {
                text.as_ref()
            } else {
                panic!("text")
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(text, ["first", "middle", "last"]);
}
#[test]
fn unsupported_gradient_interpolation_fails_during_preparation() {
    let error = prepare(
        "let shape=Rect(fill=LinearGradient())\nFillTo(shape,to=Color(\"blue3\"),duration=1s)",
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("gradient") || error.contains("LinearGradient"),
        "{error}"
    );
    assert!(error.contains("interpolat"), "{error}");
}
#[test]
fn later_corner_utilities_and_explicit_radius_have_consistent_precedence() {
    for (source, expected) in [
        (
            "let shape=<Rect pill rounded-4/>\nadd(shape)",
            CornerRadius::Fixed(4.into()),
        ),
        (
            "let shape=<Rect rounded-4 pill/>\nadd(shape)",
            CornerRadius::Pill,
        ),
        (
            "let shape=<Rect pill radius={3pt}/>\nadd(shape)",
            CornerRadius::Fixed(3.into()),
        ),
    ] {
        let prepared = prepare(source).unwrap();
        let Geometry::Rectangle { radius, .. } =
            prepared.animation.find("shape").unwrap().geometry.as_ref()
        else {
            panic!("Rect")
        };
        assert_eq!(radius, &expected);
    }
}
#[test]
fn relative_pill_radius_does_not_claim_percentages_are_point_lengths() {
    let error = prepare("let shape=<Rect pill w-50% h-50%/>\nlet invalid=shape.radius\nwait(1s)")
        .unwrap_err()
        .to_string();
    assert!(error.contains("radius depends on its parent"), "{error}");
}
#[test]
fn label_alias_uses_the_native_registry_and_is_protected() {
    let prepared = prepare("let label=Label(\"registered\")\nadd(label)\nwait(1s)").unwrap();
    assert!(
        matches!(prepared.animation.find("label").unwrap().geometry.as_ref(),Geometry::Text{text,..}if text.as_ref()=="registered")
    );
    let error = prepare("let Label=1").unwrap_err().to_string();
    assert!(
        error.contains("protected") || error.contains("reserved"),
        "{error}"
    );
}

#[test]
fn one_utility_can_lower_to_several_distinct_cached_arguments() {
    let prepared = prepare("let child=<Box flex-1/>\nlet parent=<Box w-180 h-60 flex>{child}<Box flex-1/></Box>\nadd(parent)\nwait(1s)").unwrap();
    let child = prepared.animation.find("child").unwrap();
    assert_eq!(child.layout.flex_grow, Fraction::from(1));
    assert_eq!(child.layout.flex_shrink, Fraction::from(1));
    let basis = child.layout.flex_basis.unwrap();
    assert_eq!(basis.value, Fraction::from(0));
    assert_eq!(basis.unit, sanim_core::Unit::Length);
    let laid = prepared
        .animation
        .layout(0.into(), 180.into(), 60.into())
        .unwrap();
    let children = laid[0].geometry.children().unwrap();
    assert_eq!(children[0].layout.width.unwrap().value, Fraction::from(90));
    assert_eq!(children[1].layout.width.unwrap().value, Fraction::from(90));
}
