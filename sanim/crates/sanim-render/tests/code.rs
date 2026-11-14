use sanim_core::{Fraction, Viewport};
use sanim_lang::{Engine, MemorySources};
use sanim_render::{Options, Renderer};
fn animation() -> sanim_core::Animation {
    let mut sources = MemorySources::default();
    sources.insert(
        "main.sanim",
        r#"let source=r"""def main():
    value = 1
    return value

print(main())
"""
let view=CodeView(code=source,lang="python",font_size=18pt,line_numbers=false)
add(view)
wait(1s)
view.range(name="main").fold(duration=1s)
wait(1s)
view.range(name="main").unfold(duration=1s)
wait(1s)"#,
    );
    Engine::new(Viewport {
        width: 640,
        height: 360,
        dpi: 72,
        ..Default::default()
    })
    .prepare("main.sanim", &sources, &Default::default())
    .unwrap()
    .animation
}
#[test]
fn code_is_native_vector_text_and_cached_frames_survive_reverse_scrubbing() {
    let mut renderer = Renderer::default();
    let scene = renderer.prepare(animation()).unwrap();
    let options = Options::for_animation(scene.animation());
    let first = renderer.render(&scene, 0.into(), &options).unwrap();
    let mid = renderer
        .render(&scene, Fraction::new(3u64, 2u64), &options)
        .unwrap();
    let folded = renderer.render(&scene, 2.into(), &options).unwrap();
    let final_frame = renderer.render(&scene, 4.into(), &options).unwrap();
    assert_ne!(first.rgba, mid.rgba);
    assert_ne!(first.rgba, folded.rgba);
    assert_eq!(first.rgba, final_frame.rgba);
    let warm = renderer.geometries_prepared;
    let replay = renderer.render(&scene, 0.into(), &options).unwrap();
    assert_eq!(replay.rgba, first.rgba);
    assert_eq!(renderer.geometries_prepared, warm);
}
#[test]
#[ignore = "run with make code-view-bench"]
fn code_view_benchmark() {
    use std::{hint::black_box, time::Instant};
    let animation = animation();
    let mut renderer = Renderer::default();
    let scene = renderer.prepare(animation).unwrap();
    let options = Options::for_animation(scene.animation());
    let start = Instant::now();
    for index in 0u64..150 {
        black_box(
            renderer
                .record(&scene, Fraction::new(index, 30u64), &options)
                .unwrap(),
        );
    }
    let cold = start.elapsed();
    let warm = renderer.geometries_prepared;
    let start = Instant::now();
    for index in (0u64..150).rev() {
        black_box(
            renderer
                .record(&scene, Fraction::new(index, 30u64), &options)
                .unwrap(),
        );
    }
    let replay = start.elapsed();
    assert_eq!(warm, renderer.geometries_prepared);
    eprintln!(
        "CodeView 150 timeline samples: cold {cold:?}, cached reverse {replay:?}, {} native glyph paths",
        warm
    );
}

#[test]
fn signal_driven_source_font_theme_and_fold_are_resolved_before_layout() {
    let mut files = MemorySources::default();
    files.insert("main.sanim",r#"mut number=1
mut size=14pt
mut tint=Color("blue3")
sig source="def main():\n    return {number:d}\n\nprint(main())\n"
let view=CodeView(code=source,lang="python",font_size=size,line_numbers=false,theme=CodeTheme(rules=[CodeRule("number",CodeStyle(fill=tint))]))
add(view)
wait(1s)
number=2
size=18pt
tint=Color("orange3")
view.range(name="main").fold(duration=1s)
wait(1s)
view.range(name="main").unfold(duration=1s)
wait(1s)"#);
    let prepared = Engine::new(Viewport {
        width: 640,
        height: 360,
        dpi: 72,
        ..Default::default()
    })
    .prepare("main.sanim", &files, &Default::default())
    .unwrap();
    let mut renderer = Renderer::default();
    let scene = renderer.prepare(prepared.animation).unwrap();
    let options = Options::for_animation(scene.animation());
    let first = renderer.render(&scene, 0.into(), &options).unwrap();
    let folded = renderer.render(&scene, 2.into(), &options).unwrap();
    let unfolded = renderer.render(&scene, 4.into(), &options).unwrap();
    assert_ne!(first.rgba, folded.rgba);
    assert_ne!(folded.rgba, unfolded.rgba);
    let warm = renderer.geometries_prepared;
    for time in [Fraction::from(4), Fraction::from(0), Fraction::from(2)] {
        renderer.render(&scene, time, &options).unwrap();
    }
    assert_eq!(warm, renderer.geometries_prepared);
}

#[test]
fn custom_highlight_queries_participate_in_visual_and_native_endpoint_keys() {
    use sanim_core::{
        Geometry,
        code::{Document, Language},
    };
    use std::sync::Arc;
    let mut files = MemorySources::default();
    files.insert("main.sanim", "let view=CodeView(code=\"answer = 1\",lang=\"python\",line_numbers=false)\nadd(view)\nwait(1s)\n");
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    let base = prepared.animation;
    let language = sanim_data::code::grammar(Language::Python)
        .unwrap()
        .unwrap()
        .language
        .clone();
    let make = |capture| {
        let grammar = sanim_data::code::Grammar::new(
            language.clone(),
            &format!("(identifier) @{capture}"),
            "",
        )
        .unwrap();
        let document = sanim_data::code::parse_with(
            Document::read("queries.py", "answer = 1", Language::Python).unwrap(),
            &grammar,
            true,
        )
        .unwrap();
        let mut animation = base.clone();
        let layer = &mut animation.layers[0];
        let Geometry::Code {
            document: target,
            children,
            pending,
            ..
        } = Arc::make_mut(&mut layer.element.geometry)
        else {
            panic!("code")
        };
        *target = Arc::new(document);
        children.clear();
        *pending = true;
        animation
    };
    let string_style = make("string");
    let keyword_style = make("keyword");
    assert_ne!(
        string_style.visual_key().unwrap(),
        keyword_style.visual_key().unwrap(),
        "custom grammar queries change native syntax colors even when source text is unchanged"
    );
    let mut renderer = Renderer::default();
    let first = renderer.prepare(string_style).unwrap();
    let options = Options::for_animation(first.animation());
    let first_pixels = renderer.render(&first, 0.into(), &options).unwrap();
    let second = renderer.prepare(keyword_style).unwrap();
    let second_pixels = renderer.render(&second, 0.into(), &options).unwrap();
    assert_ne!(first_pixels.rgba, second_pixels.rgba);
}

#[test]
fn core_verification_never_proves_using_unresolved_code_children() {
    use sanim_core::verify::{Evidence, Predicate, Relation};
    let mut files = MemorySources::default();
    files.insert("after.txt", "a".repeat(200));
    files.insert("main.sanim", "let view=CodeView(code=\"x\",lang=\"plain\",font_size=20pt,line_numbers=false)\nadd(view)\nwait(1s)\nview.patch(src=\"after.txt\",duration=1s)\nwait(1s)\n");
    let prepared = Engine::new(Viewport {
        width: 640,
        height: 360,
        dpi: 72,
        ..Default::default()
    })
    .prepare("main.sanim", &files, &Default::default())
    .unwrap();
    let predicate = Predicate {
        left: prepared.animation.find("view").unwrap().clone(),
        right: None,
        relation: Relation::InsideCanvas,
        gap: 0.into(),
        continuous: false,
    };
    let without_native = sanim_core::verify::evaluate(&prepared.animation, &predicate, 2.into());
    assert_eq!(
        without_native.evidence,
        Evidence::Unknown,
        "pending CodeView geometry has no native bounds evidence; initial children must not prove a later revision: {}",
        without_native.message
    );
    let resolved = sanim_core::verify::evaluate_with(
        &prepared.animation,
        &predicate,
        2.into(),
        sanim_data::resolve_pending,
    );
    assert_eq!(resolved.evidence, Evidence::Refuted);
}
