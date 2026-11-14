use sanim_core::{
    Fraction,
    code::{values::range, view::Range},
};
use sanim_lang::{Engine, MemorySources, Preparation, Prepared};

const VIEW: &str = r#"let code=<CodeView lang="python">
class WorkQueue:
    def push(self, value):
        if value:
            return value
        return 0

    def pop(self):
        return 1

class Other:
    def push(self, value):
        return value
</CodeView>
add(code)
"#;
fn prepare(body: &str) -> Result<Prepared, sanim_core::Diagnostic> {
    let mut files = MemorySources::default();
    files.insert("main.sanim", format!("{VIEW}\n{body}"));
    Engine::default().prepare("main.sanim", &files, &Default::default())
}
fn selected(prepared: &Prepared, name: &str) -> Range {
    range(&prepared.bindings[name]).unwrap()
}

#[test]
fn named_range_methods_are_typed_and_fold_exactly_one_node_with_nested_choices_retained() {
    let prepared = prepare(
        r#"fn close(selected: CodeRange): Unit {
    selected.fold(duration=100ms)
}
let method: CodeRange=code.range(name="WorkQueue.push")
let nested=code.range(line=3,column=9)
let outer=code.range(name="WorkQueue")
close(method)
nested.fold(duration=100ms)
outer.fold(duration=100ms)
outer.unfold(duration=100ms)
wait(1s)"#,
    )
    .unwrap();
    assert_eq!(prepared.animation.duration, Fraction::new(7u64, 5u64));
    let state = &prepared.animation.code_views["code"];
    let method = selected(&prepared, "method");
    let nested = selected(&prepared, "nested");
    assert_eq!(
        state.folded.len(),
        2,
        "closing a parent must retain independently closed descendants"
    );
    assert!(
        state
            .folded
            .contains(method.origin.unwrap().scope_id.as_ref().unwrap())
    );
    // Text ranges acquire an exact AST target when folded; their origin remains a text query.
    assert!(nested.origin.unwrap().scope_id.is_none());
    assert!(state.folded.iter().any(|id| id.contains("if_statement")));
    assert!(
        state
            .folded
            .iter()
            .all(|id| !id.ends_with("class_definition:WorkQueue#1"))
    );
}

#[test]
fn plural_ast_ranges_are_explicit_and_name_ambiguity_lists_qualified_declarations() {
    let error = prepare("let selected=code.range(name=\"push\")").unwrap_err();
    assert!(
        error.message.contains("ambiguous")
            && error.message.contains("WorkQueue.push")
            && error.message.contains("Other.push"),
        "{error}"
    );
    let prepared = prepare(
        r#"let all=code.ranges()
let top=code.ranges(level=1)
let outer=code.range(name="WorkQueue")
let scoped=code.range(name="push",within=outer)
let second=code.range(name="push",occurrence=2)
let nested=code.ranges(level=3,within=outer)
wait(1s)"#,
    )
    .unwrap();
    assert_eq!(prepared.bindings["top"].list().unwrap().len(), 2);
    assert_eq!(prepared.bindings["nested"].list().unwrap().len(), 1);
    assert!(prepared.bindings["all"].list().unwrap().len() > 2);
    assert!(
        selected(&prepared, "scoped")
            .origin
            .unwrap()
            .scope_id
            .as_ref()
            .unwrap()
            .contains("WorkQueue")
    );
    assert!(
        selected(&prepared, "second")
            .origin
            .unwrap()
            .scope_id
            .as_ref()
            .unwrap()
            .contains("Other")
    );
}

#[test]
fn line_character_and_regex_queries_share_unicode_coordinates_and_within_semantics() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        r#"let code=CodeView(code=r"αβ\nred red\ntail",lang="plain")
add(code)
let letter=code.range(character=2)
let point=code.range(line=2,column=5)
let part=code.range(line=2,column=5,length=3)
let whole=code.range(line=2)
let matches=code.ranges(regex="red")
let second=code.range(regex="red",occurrence=2)
let limited=code.range(regex="red",within=part)
let zeroes=code.ranges(regex=r"",within=code.range(line=1))
wait(1s)"#,
    );
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    let letter = selected(&prepared, "letter");
    assert_eq!(
        (
            letter.start_line,
            letter.start_column,
            letter.end_line,
            letter.end_column
        ),
        (1, 2, 1, 3)
    );
    let point = selected(&prepared, "point");
    assert_eq!(
        (
            point.start_line,
            point.start_column,
            point.end_line,
            point.end_column
        ),
        (2, 5, 2, 5)
    );
    let whole = selected(&prepared, "whole");
    assert_eq!(
        (
            whole.start_line,
            whole.start_column,
            whole.end_line,
            whole.end_column
        ),
        (2, 1, 3, 1)
    );
    assert_eq!(
        selected(&prepared, "second"),
        selected(&prepared, "limited")
    );
    assert_eq!(prepared.bindings["matches"].list().unwrap().len(), 2);
    assert_eq!(prepared.bindings["zeroes"].list().unwrap().len(), 4);
}

#[test]
fn range_style_preserves_other_ranges_and_merges_updates_on_the_same_interval() {
    let prepared = prepare(
        r#"let first=code.range(name="WorkQueue.push")
let second=code.range(name="WorkQueue.pop")
first.style(style=CodeStyle(fill=Color("blue3")),duration=100ms)
second.style(style=CodeStyle(italic=true),duration=100ms)
first.style(style=CodeStyle(underline=true),duration=100ms)
first.select(duration=100ms)
wait(1s)"#,
    )
    .unwrap();
    let state = &prepared.animation.code_views["code"];
    assert_eq!(state.options.spans.len(), 2);
    let first = selected(&prepared, "first");
    let span = state
        .options
        .spans
        .iter()
        .find(|span| span.range == first)
        .unwrap();
    assert!(span.style.fill.is_some());
    assert_eq!(span.style.underline, Some(true));
    assert_eq!(state.selection.as_ref(), [first]);
}

#[test]
fn bound_ranges_reject_cross_view_stale_revision_and_unadded_owner_in_full_and_demand_modes() {
    for preparation in [Preparation::Full, Preparation::Demand] {
        for (body, expected) in [
            (
                r#"let selected=code.range(name="WorkQueue.push")
let other=CodeView(code=r"def other():\n    return 1",lang="python")
add(other)
other.fold(range=selected,duration=0s)"#,
                "belongs to",
            ),
            (
                r#"let selected=code.range(name="WorkQueue.push")
selected.edit(code=r"def push(self, value):\n        return value + 1",duration=0s)
selected.fold(duration=0s)"#,
                "stale",
            ),
        ] {
            let mut files = MemorySources::default();
            files.insert("main.sanim", format!("{VIEW}\n{body}"));
            let mut engine = Engine::default();
            engine.preparation = preparation;
            let error = engine
                .prepare("main.sanim", &files, &Default::default())
                .unwrap_err();
            assert!(error.message.contains(expected), "{preparation:?}: {error}");
        }
    }
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        r#"let code=<CodeView lang="python">def f():
    return 1</CodeView>
let selected=code.range(name="f")
selected.fold()"#,
    );
    let error = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap_err();
    assert!(error.message.contains("add(view)"), "{error}");
}

#[test]
fn old_bulk_scope_and_find_api_are_removed_and_queries_fail_with_actionable_errors() {
    for (body, expected) in [
        ("code.fold(level=1)", "level"),
        ("code.unfold()", "range"),
        ("let obsolete=code.scopes()", "scopes"),
        ("let obsolete=code.scope(\"push\")", "scope"),
        ("let obsolete=code.find(\"push\")", "find"),
        ("let missing=code.range()", "requires"),
        ("let missing=code.range(name=\"absent\")", "matched nothing"),
        ("let bad=code.range(regex=\"[\")", "invalid range regex"),
        ("let bad=code.range(line=0)", "1-based"),
        ("let bad=code.range(character=0)", "1-based"),
        ("let bad=code.range(line=1,regex=\"x\")", "choose one"),
        ("let bad=code.range(regex=\"push\",occurrence=0)", "1-based"),
    ] {
        let error = prepare(body).unwrap_err();
        assert!(error.message.contains(expected), "{body}: {error}");
    }
}

#[test]
fn folding_a_shared_boundary_without_any_complete_body_line_fails_clearly() {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        r#"let code=<CodeView lang="javascript">function one() {
return 1;} function two() {
return 2;}</CodeView>
add(code)
code.range(name="one").fold(duration=0s)"#,
    );
    let error = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap_err();
    assert!(
        error.message.contains("no complete body lines") && error.message.contains("separate line"),
        "{error}"
    );
}

#[test]
fn regex_empty_matches_advance_and_repeated_suffix_searches_are_work_bounded() {
    let mut files = MemorySources::default();
    files.insert("main.sanim",format!("let code=CodeView(code=r\"{}\",lang=\"plain\")\nlet found=code.ranges(regex=r\"a{{1000}}z|a\")\nwait(1s)","a".repeat(200)));
    let mut engine = Engine::default();
    engine.limits.max_operations = 5_000;
    let error = engine
        .prepare("main.sanim", &files, &Default::default())
        .unwrap_err();
    assert!(
        error.message.contains("work units") || error.message.contains("operations"),
        "{error}"
    );
    let prepared = prepare("let matches=code.ranges(regex=r\"$\")\nwait(1s)").unwrap();
    assert_eq!(prepared.bindings["matches"].list().unwrap().len(), 1);
    files.insert(
        "main.sanim",
        r#"let code=CodeView(code=r"a\r\nβ",lang="plain")
let matches=code.ranges(regex=r"")
wait(1s)"#,
    );
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    assert_eq!(
        prepared.bindings["matches"].list().unwrap().len(),
        4,
        "empty regex ranges must not split a CRLF newline"
    );
}

#[test]
fn named_single_line_declarations_are_exact_ranges_and_remain_stylable_but_not_foldable() {
    for (language, source, name, exact) in [
        (
            "javascript",
            "function first(){return 1;} function second(){return 2;}",
            "second",
            "function second(){return 2;}",
        ),
        (
            "rust",
            "fn first() {} fn second() {}",
            "second",
            "fn second() {}",
        ),
        (
            "sanim",
            "fn first(): Number { return 1 } fn second(): Number { return 2 }",
            "second",
            "fn second(): Number { return 2 }",
        ),
    ] {
        let mut files = MemorySources::default();
        files.insert("main.sanim", format!("let code=CodeView(code=r\"{source}\",lang=\"{language}\")\nadd(code)\nlet selected=code.range(name=\"{name}\")\nselected.style(style=CodeStyle(underline=true),duration=0s)\nwait(1s)"));
        let prepared = Engine::default()
            .prepare("main.sanim", &files, &Default::default())
            .unwrap();
        let selected = selected(&prepared, "selected");
        assert_eq!(
            &source[selected.byte_range(source).unwrap()],
            exact,
            "{language}"
        );
        assert_eq!(prepared.animation.code_views["code"].options.spans.len(), 1);
        files.insert("main.sanim", format!("let code=CodeView(code=r\"{source}\",lang=\"{language}\")\nadd(code)\ncode.range(name=\"{name}\").fold(duration=0s)"));
        let error = Engine::default()
            .prepare("main.sanim", &files, &Default::default())
            .unwrap_err();
        assert!(error.message.contains("one line"), "{language}: {error}");
    }
}

#[test]
#[ignore = "run with make code-range-bench"]
fn range_query_benchmark() {
    let mut files = MemorySources::default();
    files.insert(
        "large.py",
        (0..1000)
            .map(|index| format!("def item_{index}():\n    return {index}\n"))
            .collect::<String>(),
    );
    files.insert("main.sanim", "let code=CodeView(src=\"large.py\")\nlet all=code.ranges()\nlet one=code.range(name=\"item_999\")\nwait(1s)");
    let mut timings = Vec::new();
    for iteration in 0..9 {
        let start = std::time::Instant::now();
        let prepared = Engine::default()
            .prepare("main.sanim", &files, &Default::default())
            .unwrap();
        let elapsed = start.elapsed();
        assert_eq!(prepared.bindings["all"].list().unwrap().len(), 1000);
        std::hint::black_box(prepared);
        if iteration > 0 {
            timings.push(elapsed);
        }
    }
    timings.sort();
    eprintln!(
        "1000 bound syntax ranges plus exact name query, cached parse median {:?}",
        timings[4]
    );
}

#[test]
fn line_character_and_regex_queries_share_unicode_and_crlf_coordinates() {
    let source = "α\rβ\r\n😊\n";
    let mut files = MemorySources::default();
    files.insert("sample.txt", source);
    files.insert(
        "main.sanim",
        r#"
let code=<CodeView src="sample.txt" lang="plain"/>
add(code)
let beta_line=code.range(line=1,column=3,length=1)
let beta_character=code.range(character=3)
let beta_regex=code.range(regex="β")
let emoji_line=code.range(line=2,column=1,length=1)
beta_line.select()
"#,
    );
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    for name in ["beta_line", "beta_character", "beta_regex"] {
        let selection = selected(&prepared, name);
        assert_eq!((selection.start_line, selection.start_column), (1, 3));
        assert_eq!(&source[selection.byte_range(source).unwrap()], "β");
    }
    let emoji = selected(&prepared, "emoji_line");
    assert_eq!((emoji.start_line, emoji.start_column), (2, 1));
    assert_eq!(&source[emoji.byte_range(source).unwrap()], "😊");
}
