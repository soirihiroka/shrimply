use sanim_lang::{Engine, MemorySources};

const BASE: &str = "def one():\n    return 1\n\ndef two():\n    return 2\n";
fn files(script: &str) -> MemorySources {
    let mut files = MemorySources::default();
    files.insert("main.sanim", script);
    files.insert("test.py", BASE);
    files.insert("inside.patch", "--- a/test.py\n+++ b/test.py\n@@ -1,5 +1,5 @@\n def one():\n-    return 1\n+    return 3\n \n def two():\n     return 2\n");
    files.insert("outside.patch", "--- a/test.py\n+++ b/test.py\n@@ -1,5 +1,5 @@\n def one():\n     return 1\n \n def two():\n-    return 2\n+    return 3\n");
    files
}

#[test]
fn a_range_edit_checks_actual_diff_changes_and_preserves_other_scopes() {
    let files = files(
        r#"let view=<CodeView src="test.py" />
add(view)
let selected=view.range(name="one")
selected.style(style=CodeStyle(background=Color("blue5")),duration=100ms)
selected.edit(src="inside.patch",animation="morph",duration=400ms)
wait(1s)"#,
    );
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    let state = &prepared.animation.code_views["view"];
    assert_eq!(
        state.document.text.as_ref(),
        BASE.replace("return 1", "return 3")
    );
    assert!(
        state.options.spans.is_empty(),
        "bound styles must not retain stale range provenance after an edit"
    );
    assert_eq!(state.events.len(), 2);
    let last = state.events.iter().next_back().unwrap().1;
    assert!(last.from.document.syntax.is_present() && last.to.document.syntax.is_present());
    assert_eq!(
        prepared.animation.duration,
        sanim_core::Fraction::new(3u64, 2u64)
    );
}

#[test]
fn a_range_patch_cannot_change_a_sibling_scope() {
    let files = files(
        r#"let view=<CodeView src="test.py" />
add(view)
view.range(name="one").edit(src="outside.patch",duration=400ms)"#,
    );
    let error = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap_err();
    assert!(error.message.contains("outside the selected"), "{error}");
}

#[test]
fn regex_and_character_edits_use_the_same_unicode_range_representation() {
    let files = files(
        r#"let view=<CodeView lang="plain">éλz</CodeView>
add(view)
let character=view.range(character=2)
assert(character.start_column==2)
assert(character.end_column==3)
character.edit(code="Q",duration=200ms)
view.range(regex="Q").edit(code="R",duration=200ms)
wait(1s)"#,
    );
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    assert_eq!(
        prepared.animation.code_views["view"].document.text.as_ref(),
        "éRz"
    );
}

#[test]
fn an_old_selection_fails_cleanly_after_editing_its_source() {
    let files = files(
        r#"let view=<CodeView src="test.py" />
add(view)
let selected=view.range(name="one")
selected.edit(src="inside.patch",duration=100ms)
selected.fold(duration=100ms)"#,
    );
    let error = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap_err();
    assert!(
        error.message.contains("revision") || error.message.contains("stale"),
        "{error}"
    );
}

#[test]
fn named_one_line_ranges_edit_one_sibling_without_touching_the_same_line_neighbor() {
    let files = files(
        r#"let view=<CodeView lang="javascript">function one() { return 1; } function two() { return 2; }</CodeView>
add(view)
view.range(name="one").edit(code=r"function one() { return 3; }",duration=100ms)
wait(1s)"#,
    );
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    assert_eq!(
        prepared.animation.code_views["view"].document.text.as_ref(),
        "function one() { return 3; } function two() { return 2; }"
    );
}
