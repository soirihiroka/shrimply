use sanim_core::{Element, Fraction, Geometry, Viewport};
use sanim_lang::{Engine, MemorySources};
use sanim_render::{Options, Renderer, Scene};
use std::sync::Arc;

fn scene(source: &str) -> (Renderer, Scene, Options) {
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    let animation = Engine::new(Viewport {
        width: 480,
        height: 300,
        dpi: 72,
        ..Default::default()
    })
    .prepare("main.sanim", &files, &Default::default())
    .unwrap()
    .animation;
    let mut renderer = Renderer::default();
    let scene = renderer.prepare(animation).unwrap();
    let options = Options::for_animation(scene.animation());
    (renderer, scene, options)
}
fn children(state: &sanim_render::FrameState) -> impl Iterator<Item = &Element> {
    let Geometry::Code { children, .. } = state.elements()[0].geometry.as_ref() else {
        panic!("CodeView")
    };
    children.iter()
}
fn controls(state: &sanim_render::FrameState) -> Vec<&Element> {
    children(state)
        .filter(|element| element.id.contains("/@scope/"))
        .collect()
}
fn source_x(state: &sanim_render::FrameState) -> f64 {
    children(state).find(|element| matches!(element.geometry.as_ref(), Geometry::Text {text,..} if text.as_ref()=="class")).unwrap().position.values[0]
}
const SCRIPT: &str = r#"let code=<CodeView lang="python" width={300pt} font_size={12pt}>
class Queue:
    def push(self, item):
        self.items.append(item)
</CodeView>
add(code)
wait(1s)
code.style(line_numbers=true,scope_indicators=true,duration=1s,ease="linear")
wait(1s)
code.range(name="Queue").fold(duration=1s,ease="linear")
wait(1s)
code.range(name="Queue").unfold(duration=1s,ease="linear")
wait(1s)
code.style(line_numbers=false,duration=1s,ease="linear")
wait(1s)
code.style(scope_indicators=false,duration=1s,ease="linear")
wait(1s)
"#;

#[test]
fn gutter_reflow_keeps_unchanged_source_opaque_while_wrap_boundaries_change() {
    let (mut renderer, scene, options) = scene(
        r#"let code=<CodeView lang="plain" width={110pt} font_size={12pt} whitespace-pre-wrap wrap-anywhere>
abcdefghijklmno abcdefghijklmno
</CodeView>
add(code)
wait(1s)
code.style(line_numbers=true,duration=1s,ease="linear")
wait(1s)
code.style(line_numbers=false,duration=1s,ease="linear")
wait(1s)"#,
    );
    let original = renderer.frame_state(&scene, 0.into(), &options).unwrap();
    for time in [
        Fraction::new(5u64, 4u64),
        Fraction::new(3u64, 2u64),
        Fraction::new(7u64, 4u64),
        Fraction::new(13u64, 4u64),
        Fraction::new(7u64, 2u64),
        Fraction::new(15u64, 4u64),
    ] {
        let state = renderer.frame_state(&scene, time, &options).unwrap();
        let fragments: Vec<_> = children(&state)
            .filter(|element| !element.id.contains("line-number:"))
            .collect();
        assert!(!fragments.is_empty());
        assert!(
            fragments.iter().all(|element| element.opacity == 1.into()),
            "source fragments must remain opaque during gutter reflow at{time}"
        );
        let characters = fragments
            .iter()
            .map(|element| match element.geometry.as_ref() {
                Geometry::Text { text, .. } => text.chars().count(),
                _ => 0,
            })
            .sum::<usize>();
        assert_eq!(
            characters, 30,
            "both repeated words keep their own source-coordinate identity"
        );
    }
    let restored = renderer.frame_state(&scene, 4.into(), &options).unwrap();
    for previous in children(&original) {
        let current = children(&restored)
            .find(|element| element.id == previous.id)
            .expect("restored source token");
        assert!(
            Arc::ptr_eq(&previous.geometry, &current.geometry),
            "completed hide must reuse original endpoint glyph geometry"
        );
    }
}

#[test]
fn gutter_visibility_and_scope_rotation_are_timed_and_cached() {
    let (mut renderer, scene, options) = scene(SCRIPT);
    let before = renderer.frame_state(&scene, 0.into(), &options).unwrap();
    assert!(controls(&before).is_empty());
    assert!(!children(&before).any(|element| element.id.contains("line-number:")));
    let shown = renderer.frame_state(&scene, 2.into(), &options).unwrap();
    assert_eq!(controls(&shown).len(), 2);
    let outer_id = controls(&shown)[0].id.clone();
    assert!(
        controls(&shown)
            .iter()
            .all(|element| element.rotation == 90.into())
    );
    let showing = renderer
        .frame_state(&scene, Fraction::new(3u64, 2u64), &options)
        .unwrap();
    assert!(
        controls(&showing)
            .iter()
            .all(|element| element.opacity == Fraction::new(1u64, 2u64))
    );
    assert!(
        source_x(&before) < source_x(&showing) && source_x(&showing) < source_x(&shown),
        "source slides while gutter width changes"
    );
    assert!(
        children(&showing)
            .filter(
                |element| !element.id.contains("line-number:") && !element.id.contains("/@scope/")
            )
            .all(|element| element.opacity == 1.into()),
        "surviving source must stay opaque"
    );

    for (time, count, angle) in [
        (Fraction::new(7u64, 2u64), 2, 45),
        (4.into(), 1, 0),
        (Fraction::new(11u64, 2u64), 2, 45),
    ] {
        let state = renderer.frame_state(&scene, time, &options).unwrap();
        let markers = controls(&state);
        assert_eq!(markers.len(), count);
        let outer = markers
            .iter()
            .find(|element| element.id == outer_id)
            .expect("scope keeps its identity while folding");
        assert_eq!(
            outer.rotation,
            angle.into(),
            "scope arrow rotates continuously at {time}"
        );
    }
    let hiding = renderer
        .frame_state(&scene, Fraction::new(15u64, 2u64), &options)
        .unwrap();
    assert!(
        children(&hiding)
            .filter(|element| element.id.contains("line-number:"))
            .all(|element| element.opacity == Fraction::new(1u64, 2u64))
    );
    assert!(
        controls(&hiding)
            .iter()
            .all(|element| element.opacity == 1.into())
    );
    let hidden = renderer.frame_state(&scene, 10.into(), &options).unwrap();
    assert!(controls(&hidden).is_empty());
    assert_eq!(source_x(&hidden), source_x(&before));
    let cached = renderer
        .record(&scene, Fraction::new(7u64, 2u64), &options)
        .unwrap();
    let counts = (renderer.states_computed, renderer.geometries_prepared);
    for _ in 0..3 {
        assert!(Arc::ptr_eq(
            &cached,
            &renderer
                .record(&scene, Fraction::new(7u64, 2u64), &options)
                .unwrap()
        ));
    }
    assert_eq!(
        counts,
        (renderer.states_computed, renderer.geometries_prepared)
    );
}

#[test]
fn wrapped_scope_controls_only_mark_foldable_original_header_lines() {
    let source = r#"let code=<CodeView lang="python" width={90pt} font_size={12pt} scope_indicators={true} whitespace-pre-wrap wrap-anywhere>
def long_function_name(argument):
    return "this long string wraps over several visual rows"

def other():
    return 2
</CodeView>
add(code)
wait(1s)"#;
    let (mut renderer, scene, options) = scene(source);
    let state = renderer.frame_state(&scene, 0.into(), &options).unwrap();
    assert_eq!(
        controls(&state).len(),
        2,
        "continuation rows must not invent extra scope controls"
    );
    assert!(!children(&state).any(|element| element.id.contains("line-number:")));

    let source = r#"let code=<CodeView lang="javascript" scope_indicators={true}>
function one() {
    return 1; } function two() {
    return 2;
}
</CodeView>
add(code)
wait(1s)"#;
    let (mut renderer, scene, options) = self::scene(source);
    let state = renderer.frame_state(&scene, 0.into(), &options).unwrap();
    let markers = controls(&state);
    assert_eq!(
        markers.len(),
        1,
        "a scope with no complete body line cannot fold"
    );
    assert!(markers[0].id.contains(":two#"));
}

#[test]
#[ignore = "manual editor gutter performance measurement"]
fn gutter_benchmark() {
    use std::{hint::black_box, time::Instant};
    let code = (0..60)
        .map(|n| format!("def task_{n}():\n    return {n}\n\n"))
        .collect::<String>();
    for enabled in [false, true] {
        let script = format!(
            "let code=CodeView(code=r\"\"\"{code}\"\"\",lang=\"python\",width=300pt,font_size=12pt,line_numbers={enabled},scope_indicators={enabled})\nadd(code)\ncode.range(name=\"task_0\").fold(duration=1s,ease=\"linear\")\nwait(1s)"
        );
        let (mut renderer, scene, options) = scene(&script);
        let started = Instant::now();
        for frame in 0..30u64 {
            black_box(
                renderer
                    .record(&scene, Fraction::new(frame, 30u64), &options)
                    .unwrap(),
            );
        }
        let cold = started.elapsed();
        let counts = (renderer.states_computed, renderer.geometries_prepared);
        let started = Instant::now();
        for frame in (0..30u64).rev() {
            black_box(
                renderer
                    .record(&scene, Fraction::new(frame, 30u64), &options)
                    .unwrap(),
            );
        }
        assert_eq!(
            counts,
            (renderer.states_computed, renderer.geometries_prepared)
        );
        eprintln!(
            "gutter enabled={enabled}: 60 functions,30 fold frames cold={cold:?}, cached reverse={:?}, paths={}",
            started.elapsed(),
            renderer.geometries_prepared
        );
    }
}

#[test]
fn sibling_folds_are_independent_and_survive_a_parent_fold() {
    let source = r#"let code=<CodeView lang="python" width={300pt} scope_indicators={true}>
class Worker:
    def push(self):
        return one_unique
    def pop(self):
        return two_unique
</CodeView>
add(code)
let push=code.range(name="Worker.push")
let pop_range=code.range(name="Worker.pop")
let parent=code.range(name="Worker")
push.fold(duration=1s)
wait(1s)
pop_range.fold(duration=1s)
wait(1s)
push.unfold(duration=1s)
wait(1s)
parent.fold(duration=1s)
wait(1s)
parent.unfold(duration=1s)
wait(1s)"#;
    let (mut renderer, scene, options) = scene(source);
    for (time, one, two) in [
        (1, true, false),
        (3, true, true),
        (5, false, true),
        (9, false, true),
    ] {
        let state = renderer.frame_state(&scene, time.into(), &options).unwrap();
        let has = |word: &str| {
            children(&state).any(|element|matches!(element.geometry.as_ref(),Geometry::Text{text,..} if text.as_ref()==word))
        };
        assert_eq!(has("one_unique"), !one, "push at {time}");
        assert_eq!(has("two_unique"), !two, "pop at {time}");
    }
}

#[test]
fn folding_keeps_a_shared_boundary_sibling_visible_and_scrolls_to_its_own_header() {
    for wrap in ["", "whitespace-pre-wrap wrap-anywhere"] {
        let source = format!(
            r#"let code=<CodeView lang="javascript" width={{300pt}} height={{40pt}} font_size={{12pt}} line_height={{18pt}} {wrap}>
function one() {{
    return first_unique;
}} function two() {{
    return second_unique;
}}
</CodeView>
add(code)
let first=code.range(name="one")
let second=code.range(name="two")
first.fold(duration=0s)
wait(1s)
second.scroll_to(align="start",duration=1s,ease="linear")
wait(1s)
second.fold(duration=0s)
wait(1s)
first.unfold(duration=0s)
wait(1s)
second.unfold(duration=0s)
wait(1s)"#
        );
        let (mut renderer, scene, options) = scene(&source);
        for (time, first, second) in [
            (1, false, true),
            (3, false, false),
            (4, true, false),
            (5, true, true),
        ] {
            let state = renderer.frame_state(&scene, time.into(), &options).unwrap();
            let has = |word: &str| {
                children(&state).any(|element|matches!(element.geometry.as_ref(),Geometry::Text{text,..} if text.as_ref()==word))
            };
            assert!(
                has("two"),
                "the sibling declaration header must never disappear: wrap={wrap},t={time}"
            );
            assert_eq!(has("first_unique"), first);
            assert_eq!(has("second_unique"), second);
            renderer.render(&scene, time.into(), &options).unwrap();
        }
        let state = renderer.frame_state(&scene, 2.into(), &options).unwrap();
        assert_eq!(
            state.elements()[0].layout.scroll.unwrap().values[1],
            18.,
            "a range outside the first syntax node must scroll to its own preserved boundary line"
        );
    }
}
