use sanim_core::{Fraction, Geometry, Viewport};
use sanim_lang::{Engine, MemorySources};
use sanim_render::{Options, Renderer};
use std::sync::Arc;

fn prepare(source: &str) -> (Renderer, sanim_render::Scene, Options) {
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
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
    (renderer, scene, options)
}
fn text_height(element: &sanim_core::Element) -> f64 {
    let Geometry::Text { bounds, .. } = element.geometry.as_ref() else {
        panic!("Text")
    };
    bounds.max.values[1] - bounds.min.values[1]
}
#[test]
fn labels_inherit_html_whitespace_and_reflow_at_the_actual_parent_width() {
    let (mut renderer, scene, options) = prepare(
        r#"
mut width=100pt
mut policy: "normal" | "nowrap" = "normal"
let paragraph=<Label font_size={14pt}>  A   paragraph with enough words to wrap over several lines. </Label>
let panel=<Box width={width} white_space={policy}>{paragraph}</Box>
add(panel)
wait(1s)
width=300pt
wait(1s)
policy="nowrap"
wait(1s)
"#,
    );
    let narrow = renderer.frame_state(&scene, 0.into(), &options).unwrap();
    let wide = renderer.frame_state(&scene, 1.into(), &options).unwrap();
    let nowrap = renderer.frame_state(&scene, 2.into(), &options).unwrap();
    let child =
        |state: &sanim_render::FrameState| state.elements()[0].find("paragraph").unwrap().clone();
    let Geometry::Text { text: authored, .. } = scene
        .animation()
        .find("paragraph")
        .unwrap()
        .geometry
        .as_ref()
    else {
        panic!("authored Text")
    };
    for state in [&narrow, &wide, &nowrap] {
        let element = child(state);
        let Geometry::Text { text, .. } = element.geometry.as_ref() else {
            panic!("resolved Text")
        };
        assert!(
            Arc::ptr_eq(authored, text),
            "width/whitespace frame states must share the exact authored text allocation"
        );
    }
    let cached = renderer.frame_state(&scene, 0.into(), &options).unwrap();
    assert!(
        Arc::ptr_eq(&narrow, &cached),
        "the cached frame and its shared source text remain reusable"
    );
    assert!(text_height(&child(&narrow)) > text_height(&child(&wide)));
    assert!(text_height(&child(&wide)) > text_height(&child(&nowrap)));
    let Geometry::Text { text, .. } = child(&narrow).geometry.as_ref().clone() else {
        unreachable!()
    };
    assert!(
        text.contains("A   paragraph"),
        "normalization must never rewrite the authored source"
    );
    assert_eq!(
        child(&narrow).layout.text.resolved.unwrap().width,
        Some(100.into())
    );
}
#[test]
fn overflow_wrapping_keeps_long_words_whole_unless_explicitly_enabled() {
    let (mut renderer, scene, options) = prepare(
        r#"
let normal=<Label w-60 font_size={14pt}>antidisestablishmentarianism</Label>
let break_word=<Label w-60 wrap-break-word font_size={14pt}>antidisestablishmentarianism</Label>
let anywhere=<Label w-60 wrap-anywhere font_size={14pt}>antidisestablishmentarianism</Label>
add(normal)
add(break_word)
add(anywhere)
wait(1s)
"#,
    );
    let state = renderer.frame_state(&scene, 0.into(), &options).unwrap();
    let elements = state.elements();
    assert!(text_height(&elements[1]) > text_height(&elements[0]));
    assert_eq!(text_height(&elements[1]), text_height(&elements[2]));
}
const CODE: &str = r#"
mut width=150pt
mut numbers=true
let code=<CodeView lang="python" whitespace-pre-wrap wrap-anywhere width={100%} line_numbers={numbers} font_size={12pt} line_height={18pt}>
    def example():
        return "A long string that wraps onto multiple visual rows without changing source lines"
</CodeView>
let panel=<Box width={width}>{code}</Box>
add(panel)
wait(1s)
width=350pt
wait(1s)
numbers=false
wait(1s)
"#;
#[test]
fn code_soft_rows_keep_logical_gutter_numbers_and_share_the_source_tree() {
    let (mut renderer, scene, options) = prepare(CODE);
    let narrow = renderer.frame_state(&scene, 0.into(), &options).unwrap();
    let wide = renderer.frame_state(&scene, 1.into(), &options).unwrap();
    let hidden = renderer.frame_state(&scene, 2.into(), &options).unwrap();
    let code = |state: &sanim_render::FrameState| state.elements()[0].find("code").unwrap().clone();
    let (a, b, c) = (code(&narrow), code(&wide), code(&hidden));
    let Geometry::Code {
        document, children, ..
    } = a.geometry.as_ref()
    else {
        unreachable!()
    };
    let Geometry::Code {
        document: other, ..
    } = b.geometry.as_ref()
    else {
        unreachable!()
    };
    assert!(document.syntax.shares_tree(&other.syntax));
    let numbers: Vec<_> = children
        .iter()
        .filter(|child| child.id.contains("line-number:"))
        .collect();
    assert_eq!(
        numbers.len(),
        2,
        "visual continuation rows must not invent source-line numbers"
    );
    let rows: std::collections::BTreeSet<_> = children
        .iter()
        .filter(|child| child.id.contains("token:"))
        .map(|child| child.position.values[1].round() as i64)
        .collect();
    assert!(rows.len() > 2);
    assert!(a.layout.height.unwrap().value > b.layout.height.unwrap().value);
    assert!(
        c.geometry
            .children()
            .unwrap()
            .iter()
            .all(|child| !child.id.contains("line-number:"))
    );
    assert!(
        children
            .iter()
            .filter(|child| child.id.contains("token:"))
            .all(|child| child.id.contains("/@live/1:") || child.id.contains("/@live/2:"))
    );
    assert_eq!(document.source().unwrap(), other.source().unwrap());
}
#[test]
fn collapsed_code_newlines_map_to_the_original_logical_lines() {
    let (mut renderer, scene, options) = prepare(
        r#"
let code=<CodeView lang="plain" whitespace-normal w-500 font_size={12pt} line_numbers={true}>first
second</CodeView>
add(code)
wait(1s)
"#,
    );
    let state = renderer.frame_state(&scene, 0.into(), &options).unwrap();
    let children = state.elements()[0].geometry.children().unwrap();
    assert_eq!(
        children
            .iter()
            .filter(|child| child.id.contains("line-number:"))
            .count(),
        1
    );
    assert!(
        children
            .iter()
            .any(|child| child.id.contains("/@live/2:") && child.id.contains("token:"))
    );
}
#[test]
fn native_soft_wrap_never_splits_combining_character_clusters() {
    let source = format!(
        r#"let code=<CodeView lang="plain" whitespace-pre-wrap wrap-anywhere w-60 line_numbers={{false}} font_size={{12pt}}>{}</CodeView>
add(code)
wait(1s)"#,
        "e\u{301}".repeat(30)
    );
    let (mut renderer, scene, options) = prepare(&source);
    let state = renderer.frame_state(&scene, 0.into(), &options).unwrap();
    let children = state.elements()[0].geometry.children().unwrap();
    assert!(children.len() > 1);
    for child in children {
        let Geometry::Text { text, .. } = child.geometry.as_ref() else {
            continue;
        };
        assert!(!text.starts_with('\u{301}'));
        assert!(text.ends_with('\u{301}'));
    }
}
#[test]
fn replaying_wrapped_states_does_not_recompute_layout_or_prepare_geometry() {
    let (mut renderer, scene, options) = prepare(CODE);
    let mut saved = Vec::new();
    for time in [0, 1, 2] {
        saved.push((
            renderer.frame_state(&scene, time.into(), &options).unwrap(),
            renderer.record(&scene, time.into(), &options).unwrap(),
        ));
    }
    let before = (
        renderer.states_computed,
        renderer.geometries_prepared,
        renderer.cache_hits,
    );
    for _ in 0..4 {
        for (time, (state, recording)) in saved.iter().enumerate().rev() {
            assert!(Arc::ptr_eq(
                state,
                &renderer
                    .frame_state(&scene, Fraction::from(time as u64), &options)
                    .unwrap()
            ));
            assert!(Arc::ptr_eq(
                recording,
                &renderer
                    .record(&scene, Fraction::from(time as u64), &options)
                    .unwrap()
            ));
        }
    }
    assert_eq!(before.0, renderer.states_computed);
    assert_eq!(before.1, renderer.geometries_prepared);
    assert_eq!(renderer.cache_hits - before.2, 12);
}

#[test]
fn flex_remeasurement_wraps_at_the_allocated_child_width() {
    let (mut renderer, scene, options) = prepare(
        r#"
let first=<Label w-full flex-1 font_size={14pt}>A paragraph with enough words to require another row after flex shrink.</Label>
let second=<Label w-full flex-1 font_size={14pt}>Another paragraph shares the same parent width with its sibling.</Label>
let panel=<Box w-240 flex flex-row>{first}{second}</Box>
add(panel)
wait(1s)
"#,
    );
    let state = renderer.frame_state(&scene, 0.into(), &options).unwrap();
    for id in ["first", "second"] {
        let child = state.elements()[0].find(id).unwrap();
        assert_eq!(child.layout.text.resolved.unwrap().width, Some(120.into()));
        assert!(text_height(child) > 28.);
    }
}

#[test]
fn rtl_code_words_use_their_visual_left_edge_and_do_not_overlap() {
    let (mut renderer, scene, options) = prepare(
        r#"
let code=<CodeView lang="plain" whitespace-pre-wrap w-200 line_numbers={false} font_size={20pt}>مرحبا بالعالم</CodeView>
add(code)
wait(1s)
"#,
    );
    let state = renderer.frame_state(&scene, 0.into(), &options).unwrap();
    let children = state.elements()[0].geometry.children().unwrap();
    let mut words=children.iter().filter(|element|matches!(element.geometry.as_ref(),Geometry::Text{text,..} if !text.trim().is_empty())).map(|element|sanim_core::verify::bounds(element).unwrap()).collect::<Vec<_>>();
    words.sort_by(|a, b| a.min.values[0].total_cmp(&b.min.values[0]));
    assert_eq!(words.len(), 2);
    assert!(
        words[0].max.values[0] <= words[1].min.values[0],
        "RTL word intervals must be disjoint: {words:?}, children={:?}",
        children
            .iter()
            .map(|child| (&child.position, &child.geometry))
            .collect::<Vec<_>>()
    );
    assert!(words[0].min.values[0] >= -1.);
    assert!(words[1].max.values[0] <= 201.);
}

#[test]
fn trailing_blank_source_lines_keep_their_gutter_numbers_after_wrapping() {
    let (mut renderer, scene, options) = prepare(
        "let code = r\"\"\"one\ntwo\n\n\"\"\"\nlet plain=CodeView(code=code,lang=\"plain\",width=200pt,line_numbers=true)\nlet wrapped=CodeView(code=code,lang=\"plain\",width=200pt,white_space=\"pre-wrap\",line_numbers=true)\nadd(plain)\nadd(wrapped)\nwait(1s)",
    );
    let state = renderer.frame_state(&scene, 0.into(), &options).unwrap();
    let labels = |element: &sanim_core::Element| {
        element
            .geometry
            .children()
            .unwrap()
            .iter()
            .filter(|child| child.id.contains("line-number:"))
            .map(|child| match child.geometry.as_ref() {
                Geometry::Text { text, .. } => text.to_string(),
                _ => panic!("line number text"),
            })
            .collect::<Vec<_>>()
    };
    let expected = labels(&state.elements()[0]);
    assert_eq!(expected, ["1", "2", "3"]);
    assert_eq!(labels(&state.elements()[1]), expected);
}

#[test]
fn generated_code_glyphs_are_already_resolved_and_keep_their_native_styles() {
    let (_renderer, scene, _options) = prepare(
        r#"
let code=<CodeView lang="python" whitespace-pre-wrap wrap-anywhere w-150 font_size={12pt}>
    def example():
        return "A long message with several wrapped words"
</CodeView>
add(code)
wait(1s)
"#,
    );
    let mut code_resolutions = 0;
    let mut glyph_resolutions = 0;
    let elements = scene
        .animation()
        .layout_with(0.into(), 640.into(), 360.into(), |element, time| {
            if sanim_data::requires_resolution(element) {
                match element.geometry.as_ref() {
                    Geometry::Code { .. } => code_resolutions += 1,
                    Geometry::Text { .. } => glyph_resolutions += 1,
                    _ => {}
                }
            }
            sanim_data::resolve_pending(element, time)
        })
        .unwrap();
    assert_eq!(code_resolutions, 1);
    assert_eq!(
        glyph_resolutions, 0,
        "generated glyphs must not be normalized or reshaped as authored labels"
    );
    let Geometry::Code {
        options, children, ..
    } = elements[0].geometry.as_ref()
    else {
        unreachable!()
    };
    let keyword=children.iter().find(|element|matches!(element.geometry.as_ref(),Geometry::Text{text,..} if text.as_ref()=="def")).unwrap();
    let Geometry::Text { outline, .. } = keyword.geometry.as_ref() else {
        unreachable!()
    };
    let native =
        sanim_data::font::cached_shape(&options.font, "def", options.font_size, true, false)
            .unwrap();
    assert!(
        Arc::ptr_eq(outline, &native.outline),
        "keyword must retain its bold native outline"
    );
}

#[test]
#[ignore = "separates native layout, state accounting and recording costs"]
fn codeview_frame_pipeline_benchmark() {
    let code=(0..100).map(|index|format!("def function_{index}():\n    return \"A long line of source text with several word boundaries\"\n")).collect::<String>();
    let source = format!(
        "mut width=180pt\nlet code=CodeView(code=r\"\"\"{code}\"\"\",lang=\"python\",white_space=\"pre-wrap\",overflow_wrap=\"anywhere\",width=width)\nadd(code)\nanimate(width,to=300pt,duration=3s,ease=\"linear\")\nwait(1s)"
    );
    let (mut renderer, scene, options) = prepare(&source);
    for time in [0, 1, 2, 3] {
        let mut native = std::time::Duration::ZERO;
        let mut glyph_resolutions = 0;
        let started = std::time::Instant::now();
        let layout = scene
            .animation()
            .layout_with(time.into(), 640.into(), 360.into(), |element, time| {
                if matches!(element.geometry.as_ref(), Geometry::Text { .. })
                    && sanim_data::requires_resolution(element)
                {
                    glyph_resolutions += 1;
                }
                let started = std::time::Instant::now();
                let result = sanim_data::resolve_pending(element, time);
                native += started.elapsed();
                result
            })
            .unwrap();
        eprintln!(
            "time={time} native={native:?} full_layout={:?} glyph_resolutions={glyph_resolutions} glyphs={}",
            started.elapsed(),
            layout[0].geometry.children().unwrap().len()
        );
        let state = renderer.frame_state(&scene, time.into(), &options).unwrap();
        let started = std::time::Instant::now();
        renderer.record(&scene, time.into(), &options).unwrap();
        eprintln!(
            "time={time} cached_native_state_us={} recording={:?}",
            state.compute_micros,
            started.elapsed()
        );
    }
}
