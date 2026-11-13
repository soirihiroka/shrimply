use sanim_core::{Fraction, Property, Unit, Vec2, Viewport};
use sanim_lang::{Engine, MemorySources};

fn prepare(source: &str) -> Result<sanim_lang::Prepared, sanim_core::Diagnostic> {
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    Engine::new(Viewport {
        width: 200,
        height: 120,
        dpi: 72,
        ..Default::default()
    })
    .prepare("main.sanim", &files, &Default::default())
}
const CONTENT: &str = r#"let first=Rect(width=20pt,height=20pt,x_align=Align.Start,y_align=Align.Start)
let last=Rect(width=20pt,height=20pt,position=Vec2(0pt,80pt),x_align=Align.Start,y_align=Align.Start)
let clip=ScrollView(width=100pt,height=60pt,children=[first,last])
add(clip)
"#;

#[test]
fn bounded_scroll_methods_use_normal_tracks_and_resolve_percentages_after_resize() {
    let prepared = prepare(&format!(
        "{CONTENT}clip.scroll_to(to=Vec2(0pt,500pt),duration=1s,ease=\"linear\")\nwait(1s)\n"
    ))
    .unwrap();
    let clip = prepared.animation.find("clip").unwrap();
    assert_eq!(
        prepared
            .animation
            .sample_property(clip, Property::Scroll, Fraction::new(1u64, 2u64))
            .unwrap()
            .vec2()
            .unwrap(),
        Vec2 {
            values: [0., 20.],
            unit: Unit::Length
        }
    );
    let sampled = prepared
        .animation
        .layout(1.into(), 200.into(), 120.into())
        .unwrap();
    assert_eq!(sampled[0].layout.scroll.unwrap().values, [0., 40.]);
    assert_eq!(prepared.animation.duration, 2.into());
    let relative = CONTENT.replace("height=60pt", "height=50%");
    let percent = prepare(&format!(
        "{relative}clip.scroll_to(to=Vec2(0%,100%),duration=1s)\n"
    ))
    .unwrap();
    assert_eq!(
        percent
            .animation
            .layout(1.into(), 200.into(), 120.into())
            .unwrap()[0]
            .layout
            .scroll
            .unwrap()
            .values,
        [0., 40.]
    );
    assert_eq!(
        percent
            .animation
            .layout(1.into(), 200.into(), 160.into())
            .unwrap()[0]
            .layout
            .scroll
            .unwrap()
            .values,
        [0., 20.]
    );
}

#[test]
fn scroll_signal_and_code_line_targets_are_supported() {
    let source = r#"mut offset=Vec2(0pt,0pt)
let shape=Rect(width=20pt,height=20pt,position=Vec2(0pt,80pt),x_align=Align.Start,y_align=Align.Start)
let clip=ScrollView(width=100pt,height=60pt,children=[shape],scroll=offset)
add(clip)
animate(offset,to=Vec2(0pt,40pt),duration=1s,ease="linear")
"#;
    let prepared = prepare(source).unwrap();
    assert_eq!(
        prepared
            .animation
            .layout(Fraction::new(1u64, 2u64), 200.into(), 120.into())
            .unwrap()[0]
            .layout
            .scroll
            .unwrap()
            .values,
        [0., 20.]
    );
    let code = prepare(
        r#"let view=CodeView(code=r"""a=1
b=2
c=3
d=4
e=5
f=6
g=7
h=8
i=9
j=10
""",lang="python",width=100pt,height=40pt,font_size=10pt,line_height=12pt,line_numbers=false)
add(view)
view.scroll_to(line=6,align="start",duration=1s,ease="linear")
"#,
    )
    .unwrap();
    let track = code.animation.find("view").unwrap();
    assert_eq!(
        code.animation
            .sample_property(track, Property::Scroll, 1.into())
            .unwrap()
            .vec2()
            .unwrap()
            .values,
        [0., 60.]
    );
}

#[test]
fn invalid_viewports_and_ambiguous_scroll_targets_fail_with_helpful_errors() {
    for (source, expected) in [
        ("let x=ScrollView(children=[])", "requires width and height"),
        (
            "let c=Circle()\nc.scroll_to(to=Vec2(0pt,1pt))",
            "requires ScrollView",
        ),
        (
            "let v=ScrollView(width=10pt,height=10pt,children=[])\nv.scroll_to(to=Vec2(0pt,0pt),y=2pt)",
            "one target",
        ),
        (
            "let v=ScrollView(width=10pt,height=10pt,children=[])\nv.scroll_to(line=2)",
            "require CodeView",
        ),
    ] {
        let error = prepare(source).unwrap_err();
        assert!(error.message.contains(expected), "{source}: {error}");
    }
}
