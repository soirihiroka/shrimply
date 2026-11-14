use sanim_core::{Element, Fraction, Geometry, Property, Value};
use sanim_lang::{Engine, MemorySources, Prepared};
use std::collections::{BTreeMap, BTreeSet};
fn prepare(source: &str) -> Result<Prepared, sanim_core::Diagnostic> {
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    Engine::default().prepare("main.sanim", &files, &Default::default())
}
fn sample(prepared: &Prepared, time: Fraction) -> Vec<Element> {
    prepared
        .animation
        .sample_with(time, sanim_data::code::view::resolve_pending)
        .unwrap()
}
fn texts(element: &Element, output: &mut BTreeMap<String, String>) {
    if let Geometry::Text { text, .. } = element.geometry.as_ref() {
        output.insert(text.to_string(), element.id.clone());
    }
    if let Some(children) = element.geometry.children() {
        for child in children {
            texts(child, output);
        }
    }
}
#[test]
fn watched_source_patch_and_disjoint_changes_preserve_unchanged_source() {
    let mut files = MemorySources::default();
    files.insert("sample.py", "first = 1\nkeep_middle = 2\nlast = 3\n");
    files.insert("sample.py.patch","--- a/sample.py\n+++ b/sample.py\n@@ -1 +1 @@\n-first = 1\n+first = 4\n@@ -3 +3 @@\n-last = 3\n+last = 5\n");
    files.insert(
        "main.sanim",
        r#"let view=<CodeView src="sample.py" line_numbers={false}/>
add(view)
wait(1s)
view.patch(src="sample.py.patch", duration=1s)
wait(1s)"#,
    );
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    let mut before = BTreeMap::new();
    let mut after = BTreeMap::new();
    texts(&sample(&prepared, 0.into())[0], &mut before);
    texts(&sample(&prepared, 2.into())[0], &mut after);
    assert_eq!(before["keep_middle"], after["keep_middle"]);
    assert!(before.contains_key("1") && after.contains_key("4"));
    assert_eq!(prepared.animation.duration, 3.into());

    assert!(
        prepared
            .dependencies
            .iter()
            .any(|(path, _)| path.ends_with("sample.py.patch"))
    );
}
#[test]
fn raw_markup_foreign_syntax_is_literal_dedented_and_ast_foldable() {
    let prepared = prepare(
        r#"let view=<CodeView lang="python" line_numbers={false}>
    def main():
        if 1 < 2:
            return f"value {1}"
        return 2

    print(main())
</CodeView>
add(view)
wait(1s)
view.fold(range=view.range(line=3),duration=1s)
wait(1s)
view.unfold(range=view.range(line=3),duration=1s)
wait(1s)"#,
    )
    .unwrap();
    let Geometry::Code { document, .. } =
        prepared.animation.find("view").unwrap().geometry.as_ref()
    else {
        panic!()
    };
    assert!(document.text.starts_with("def main():\n"));
    assert!(document.text.contains("f\"value {1}\""));
    assert!(
        document.diagnostics.is_empty(),
        "{:?}",
        document.diagnostics
    );
    assert!(document.syntax.is_present());
    let mut folded = BTreeMap::new();
    texts(&sample(&prepared, 2.into())[0], &mut folded);
    assert!(folded.keys().any(|text| text.contains('…')));
    let mut after = BTreeMap::new();
    texts(&sample(&prepared, 4.into())[0], &mut after);
    assert!(after.contains_key("print"));
}
#[test]
fn sanim_uses_native_ast_highlights_and_shares_actual_tree_across_views() {
    let prepared = prepare(
        r####"let code=r'''# source comment
let count: Number = 2
let title = """value={count:04d}"""
let raw = r"{unchanged}"
let label=<Label fill="blue3">value {count}</Label>
if true {
    repeat 2 {
        wait(0.5s)
    }
} else {
    repeat 3 {
        wait(1s)
    }
}
'''
let view=CodeView(code=code,lang="sanim")
let other=CodeView(code=code,lang="sanim")
add(view)
wait(1s)"####,
    )
    .unwrap();
    let doc = |id| match prepared
        .animation
        .find(id)
        .or_else(|| match &prepared.bindings[id] {
            Value::Element(e) => Some(e.as_ref()),
            _ => None,
        })
        .unwrap()
        .geometry
        .as_ref()
    {
        Geometry::Code { document, .. } => document,
        _ => panic!(),
    };
    let document = doc("view");
    assert!(
        document.diagnostics.is_empty(),
        "{:?}",
        document.diagnostics
    );
    assert!(document.syntax.shares_tree(&doc("other").syntax));
    let kinds: BTreeSet<_> = document
        .highlights
        .iter()
        .map(|span| span.kind.as_ref())
        .collect();
    for kind in [
        "comment",
        "keyword",
        "string",
        "number",
        "type",
        "tag",
        "property",
        "punctuation",
    ] {
        assert!(kinds.contains(kind), "{kind}");
    }
    let ids: BTreeSet<_> = document
        .scopes
        .iter()
        .map(|scope| scope.id.as_ref())
        .collect();
    assert_eq!(ids.len(), document.scopes.len());
    let closing = document.text.find("</Label>").unwrap() + 2;
    assert!(
        document
            .highlights
            .iter()
            .any(|span| span.start == closing && span.kind.as_ref() == "tag")
    );
}
#[test]
fn style_selection_and_readable_errors_are_native_methods() {
    let prepared=prepare(r#"let view=CodeView(code=r"value = 1",lang="python",spans=[CodeSpan(CodeRange(1,1,1,6),CodeStyle(fill=Color("orange3"),bold=true,underline=true))])
add(view)
view.select(ranges=[view.range(regex="value")],duration=1s)
view.style(theme=CodeTheme(rules=[CodeRule("number",CodeStyle(fill=Color("purple3")))]),duration=1s)
wait(1s)"#).unwrap();
    assert_eq!(prepared.animation.duration, 3.into());
    let error = prepare(
        r#"let v=CodeView(code="x=1",lang="python",spans=[CodeSpan(CodeRange(9,1,9,3),CodeStyle())])
add(v)
wait(1s)"#,
    )
    .unwrap_err();
    assert!(error.message.contains("line 9"), "{error}");
    for (source, expected) in [
        (
            r#"let v=CodeView(code="x=1",lang="python")
v.fold(range=v.range(name="missing"))"#,
            "matched nothing",
        ),
        (
            r#"let v=CodeView(code="x=1",lang="python")
v.select(ranges=[CodeRange(1,0,1,1)])"#,
            "1-based",
        ),
        (
            r#"let v=CodeView(code="x=1",lang="python")
let invalid=v.range(level=0)"#,
            "1-based",
        ),
        (
            r#"let v=CodeView(code="anything")
v.fold(range=CodeRange(1,1,1,1))"#,
            "no native syntax tree",
        ),
        (r#"let v=CodeView()"#, "exactly one"),
        (
            r#"let v=<CodeView src="missing.py">body</CodeView>"#,
            "exactly one",
        ),
    ] {
        let error = prepare(source).unwrap_err();
        assert!(error.message.contains(expected), "{source}: {error}");
    }
}
#[test]
fn invalid_source_is_renderable_data_with_diagnostics() {
    for (language, text) in [
        ("python", "def incomplete(:\n return 1\n"),
        ("sanim", "let value =\n"),
    ] {
        let source = format!(
            "let view=CodeView(code=r\"\"\"{text}\"\"\",lang=\"{language}\")\nadd(view)\nwait(1s)"
        );
        let prepared = prepare(&source).unwrap();
        let Geometry::Code { document, .. } =
            prepared.animation.find("view").unwrap().geometry.as_ref()
        else {
            panic!()
        };
        assert!(!document.diagnostics.is_empty(), "{language}");
        sample(&prepared, 0.into());
    }
}
#[test]
fn signals_drive_code_and_all_presentation_arguments() {
    let prepared=prepare(r#"mut count=1
mut tint=Color("blue3")
mut size=12pt
mut numbers=true
mut family="monospace"
sig contents="value = {count:02d}"
let view=CodeView(code=contents,lang="python",font=Font(family,"monospace"),font_size=size,line_numbers=numbers,theme=CodeTheme(rules=[CodeRule("number",CodeStyle(fill=tint))]),spans=[CodeSpan(CodeLines(1),CodeStyle(underline=true))])
add(view)
wait(1s)
count=2
tint=Color("orange3")
size=24pt
family="sans-serif"
numbers=false
wait(1s)"#).unwrap();
    let view = prepared.animation.find("view").unwrap();
    for (time, words, size, numbers) in [
        (Fraction::from(0), "value = 01", 12, true),
        (Fraction::from(1), "value = 02", 24, false),
    ] {
        assert_eq!(
            prepared
                .animation
                .sample_property(view, Property::Code, time)
                .unwrap()
                .string()
                .unwrap(),
            words
        );
        assert_eq!(
            prepared
                .animation
                .sample_property(view, Property::FontSize, time)
                .unwrap()
                .number()
                .unwrap()
                .value,
            Fraction::from(size as u32)
        );
        assert_eq!(
            prepared
                .animation
                .sample_property(view, Property::CodeLineNumbers, time)
                .unwrap()
                .boolean()
                .unwrap(),
            numbers
        );
        sample(&prepared, time);
    }
}
#[test]
fn old_document_api_is_removed_and_writer_conflicts_are_explicit() {
    for source in [
        "let x=Code<Python>(r\"x=1\")",
        "let x=read<Code<Python>>(\"a.py\")",
        "edit_code()",
    ] {
        assert!(prepare(source).is_err(), "{source}");
    }
    let error=prepare(r#"mut tint=Color("blue3")
let view=CodeView(code="x=1",lang="python",theme=CodeTheme(rules=[CodeRule("number",CodeStyle(fill=tint))]))
view.style(theme=CodeTheme())"#).unwrap_err();
    assert!(error.message.contains("signal-controlled theme"), "{error}");
}

#[test]
fn gutters_default_hidden_and_timed_style_retains_both_visibility_endpoints() {
    let source = r#"let view=<CodeView lang="python">print("hello")</CodeView>
add(view)
wait(1s)
view.style(line_numbers=true,scope_indicators=true,duration=400ms,ease="linear")
wait(600ms)
view.style(line_numbers=false,duration=200ms,ease="linear")
wait(800ms)"#;
    let prepared = prepare(source).unwrap();
    let Geometry::Code { options, .. } = prepared.animation.find("view").unwrap().geometry.as_ref()
    else {
        panic!("CodeView constructor")
    };
    assert!(!options.line_numbers && !options.scope_indicators);
    let state = &prepared.animation.code_views["view"];
    let showing = state
        .presentation(Fraction::new(6u64, 5u64))
        .unwrap()
        .unwrap();
    assert!(!showing.from.options.line_numbers && !showing.from.options.scope_indicators);
    assert!(showing.to.options.line_numbers && showing.to.options.scope_indicators);
    assert_eq!(showing.raw_progress, Fraction::new(1u64, 2u64));
    let hiding = state
        .presentation(Fraction::new(21u64, 10u64))
        .unwrap()
        .unwrap();
    assert!(hiding.from.options.line_numbers && hiding.from.options.scope_indicators);
    assert!(!hiding.to.options.line_numbers && hiding.to.options.scope_indicators);
    assert_eq!(hiding.raw_progress, Fraction::new(1u64, 2u64));
    assert_eq!(prepared.animation.duration, 3.into());
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    let duration = Engine::default()
        .duration("main.sanim", &files, &Default::default())
        .unwrap();
    assert_eq!(duration.duration, prepared.animation.duration);

    let mut serialized = serde_json::to_value(options).unwrap();
    let fields = serialized.as_object_mut().unwrap();
    fields.remove("line_numbers");
    fields.remove("scope_indicators");
    let decoded: sanim_core::code::view::Options = serde_json::from_value(serialized).unwrap();
    assert!(!decoded.line_numbers && !decoded.scope_indicators);
}

#[test]
fn gutter_style_calls_use_the_shared_default_duration_and_reject_invalid_flags() {
    let prepared = prepare("let view=CodeView(code=r\"x=1\",lang=\"python\")\nview.style(line_numbers=true,scope_indicators=true)").unwrap();
    assert_eq!(prepared.animation.duration, Fraction::new(3u64, 10u64));
    for flag in ["line_numbers", "scope_indicators"] {
        let error = prepare(&format!(
            "let view=CodeView(code=r\"x=1\",lang=\"python\",{flag}=1)"
        ))
        .unwrap_err();
        assert!(error.message.contains("Bool"), "{error}");
    }
}

#[test]
fn scope_indicator_signals_retain_types_and_timed_style_has_a_single_writer() {
    let prepared = prepare(
        r#"mut scopes=false
let view=CodeView(code=r"x=1",lang="python",scope_indicators=scopes)
add(view)
wait(1s)
scopes=true
wait(1s)"#,
    )
    .unwrap();
    let view = prepared.animation.find("view").unwrap();
    for (time, expected) in [(0, false), (1, true)] {
        assert_eq!(
            prepared
                .animation
                .sample_property(view, Property::CodeScopeIndicators, time.into())
                .unwrap()
                .boolean()
                .unwrap(),
            expected
        );
    }
    for flag in ["line_numbers", "scope_indicators"] {
        let error = prepare(&format!("mut showing=false\nlet view=CodeView(code=r\"x=1\",lang=\"python\",{flag}=showing)\nview.style({flag}=true,duration=500ms)")).unwrap_err();
        assert!(
            error.message.contains(&format!("signal-controlled {flag}")),
            "{error}"
        );
    }
}

#[test]
fn displayed_sanim_has_a_fixed_budget_independent_of_executable_module_limits() {
    let mut files = MemorySources::default();
    let displayed = (0..50)
        .map(|index| format!("fn displayed_{index}(): Number {{\n    return {index}\n}}\n"))
        .collect::<String>();
    files.insert("display.sanim", &displayed);
    files.insert(
        "main.sanim",
        "let view=<CodeView src=\"display.sanim\"/>\nadd(view)\nwait(1s)",
    );
    let mut engine = Engine::default();
    engine.limits.max_parse_steps = 256;
    let prepared = engine
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    let Geometry::Code { document, .. } =
        prepared.animation.find("view").unwrap().geometry.as_ref()
    else {
        panic!()
    };
    assert!(document.syntax.is_present());
    assert!(
        document.diagnostics.is_empty(),
        "{:?}",
        document.diagnostics
    );
    assert_eq!(document.scopes.len(), 50);
    let error = engine
        .prepare("display.sanim", &files, &Default::default())
        .unwrap_err();
    assert!(
        error.message.contains("parse"),
        "executable module still obeys its own budget: {error}"
    );
}

#[test]
fn large_code_stays_unshaped_until_needed_and_can_be_folded_before_first_layout() {
    let mut files = MemorySources::default();
    let source = (0..1200)
        .map(|index| format!("def large_{index}():\n    first = 1\n    second = 2\n"))
        .collect::<String>();
    files.insert("large.py", &source);
    files.insert(
        "unused.sanim",
        "let view=<CodeView src=\"large.py\"/>\nwait(1s)",
    );
    let unused = Engine::default()
        .prepare("unused.sanim", &files, &Default::default())
        .unwrap();
    let Value::Element(view) = &unused.bindings["view"] else {
        panic!()
    };
    let Geometry::Code {
        children, pending, ..
    } = view.geometry.as_ref()
    else {
        panic!()
    };
    assert!(
        *pending && children.is_empty(),
        "unused source must not build glyphs"
    );
    files.insert("main.sanim","let view=<CodeView src=\"large.py\" line_numbers={false}/>\nadd(view)\nfor selected in view.ranges() { selected.fold(duration=0s) }\nwait(1s)");
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    assert_eq!(
        prepared.element_sources.len(),
        1,
        "glyph provenance is resolved through the owning CodeView, not eagerly expanded"
    );
    let sampled = sample(&prepared, 0.into());
    let Geometry::Code {
        children, pending, ..
    } = sampled[0].geometry.as_ref()
    else {
        panic!()
    };
    assert!(!*pending);
    assert!(
        children.len() < 10_000,
        "fold first, then shape only visible scope headers"
    );
    let mut visible = BTreeMap::new();
    texts(&sampled[0], &mut visible);
    assert!(visible.keys().any(|word| word.contains('…')));
    assert!(!visible.contains_key("second"));
}

#[test]
fn no_op_presentations_keep_their_duration_without_events_or_duplicate_retained_payloads() {
    let prepared = prepare(
        r#"let view=<CodeView lang="python">x = 1</CodeView>
add(view)
repeat 200 {
    view.select(duration=1ms)
    view.style(duration=1ms)
}
wait(1s)"#,
    )
    .unwrap();
    assert!(prepared.animation.code_views.is_empty());
    assert_eq!(prepared.animation.duration, Fraction::new(7u64, 5u64));
}
#[test]
fn changed_presentation_metadata_and_checkpoint_roots_obey_byte_limits() {
    let mut files = MemorySources::default();
    let changed = r#"let view=<CodeView lang="python">x = 1</CodeView>
add(view)
let selection=[CodeRange(1,1,1,2)]
repeat 100 {
    view.select(ranges=selection,duration=1ms)
    view.select(duration=1ms)
}
wait(1s)"#;
    files.insert("main.sanim", changed);
    let mut engine = Engine::default();
    engine.limits.data.max_bytes = 64 * 1024;
    let error = engine
        .prepare("main.sanim", &files, &Default::default())
        .unwrap_err();
    // Event storage spends the same retained-data budget as subsequent call
    // arguments, so either check may be the first to reject the next edit.
    assert!(
        error.message.contains("CodeView timeline metadata")
            || error.message.contains("generated value byte budget"),
        "{error}"
    );
    files.insert("main.sanim", changed.replace("100", "4"));
    let mut engine = Engine::default();
    engine.limits.max_cache_bytes = 64 * 1024;
    let prepared = engine
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    assert_eq!(prepared.animation.code_views["view"].events.len(), 8);
    assert!(prepared.statistics.checkpoint_bytes <= engine.limits.max_cache_bytes);
    assert!(prepared.statistics.checkpoint_count > 0);
}

#[test]
#[ignore = "run with make code-view-bench"]
fn code_view_benchmark() {
    use std::{hint::black_box, time::Instant};
    let mut files = MemorySources::default();
    let source = (0..1000)
        .map(|index| format!("def lazy_{index}():\n    first = 1\n    second = 2\n"))
        .collect::<String>();
    files.insert("lazy.py", &source);
    let mut preparation = Vec::new();
    let mut demanded = Vec::new();
    for index in 0..9 {
        files.insert("main.sanim",format!("let view=CodeView(src=\"lazy.py\",id=\"bench_{index}\",line_numbers=false)\nwait(1s)"));
        let start = Instant::now();
        let prepared = Engine::default()
            .prepare("main.sanim", &files, &Default::default())
            .unwrap();
        let elapsed = start.elapsed();
        let Value::Element(view) = &prepared.bindings["view"] else {
            panic!()
        };
        let Geometry::Code {
            children, pending, ..
        } = view.geometry.as_ref()
        else {
            panic!()
        };
        assert!(*pending && children.is_empty());
        let start = Instant::now();
        black_box(
            prepared
                .animation
                .sample_element_with(view, 0.into(), sanim_data::code::view::resolve_pending)
                .unwrap(),
        );
        if index > 0 {
            preparation.push(elapsed);
            demanded.push(start.elapsed());
        }
    }
    preparation.sort();
    demanded.sort();
    eprintln!(
        "CodeView unused 1000-function source median prepare {:?}; native work deferred until demand {:?}; eager-equivalent {:?}",
        preparation[4],
        demanded[4],
        preparation[4] + demanded[4]
    );
}
