mod common;
use common::*;
use serde_json::{Value, json};

fn complete(marked: &str, snippets: bool) -> (String, Vec<Value>) {
    let cursor = marked.find('§').unwrap();
    let source = marked.replacen('§', "", 1);
    let before = &source[..cursor];
    let position = json!({"line":before.bytes().filter(|byte| *byte==b'\n').count(),"character":before.rsplit('\n').next().unwrap().encode_utf16().count()});
    let dir = tempfile::tempdir().unwrap();
    let uri = format!("file://{}", dir.path().join("main.sanim").display());
    let mut init = initialize(dir.path());
    init["params"]["capabilities"] =
        json!({"textDocument":{"completion":{"completionItem":{"snippetSupport":snippets}}}});
    let mut messages = vec![
        init,
        notification(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":uri,"languageId":"sanim","version":1,"text":source}}),
        ),
        request(
            2,
            "textDocument/completion",
            json!({"textDocument":{"uri":uri},"position":position}),
        ),
    ];
    finish(&mut messages);
    let (status, replies) = session(&messages);
    assert_eq!(status, 0);
    (
        source,
        reply(&replies, 2)["result"]["items"]
            .as_array()
            .unwrap()
            .clone(),
    )
}
fn item<'a>(items: &'a [Value], label: &str) -> &'a Value {
    items
        .iter()
        .find(|item| item["label"] == label)
        .unwrap_or_else(|| panic!("missing {label}: {items:?}"))
}
fn apply_ascii_edit(source: &str, item: &Value) -> String {
    assert!(source.is_ascii());
    let edit = &item["textEdit"];
    assert_eq!(edit["range"]["start"]["line"], 0);
    assert_eq!(edit["range"]["end"]["line"], 0);
    let start = edit["range"]["start"]["character"].as_u64().unwrap() as usize;
    let end = edit["range"]["end"]["character"].as_u64().unwrap() as usize;
    format!(
        "{}{}{}",
        &source[..start],
        edit["newText"].as_str().unwrap(),
        &source[end..]
    )
}

#[test]
fn applying_value_completions_preserves_quotes_braces_and_tag_delimiters() {
    for (marked, label) in [
        ("let x=<CodeView line_numbers=§/>", "true"),
        ("let x=<CodeView line_numbers={§}/>", "true"),
        ("let x=<CodeView line_numbers={tr§}/>", "true"),
        ("let x=<Box x_align={§}/>", "Align.Center"),
        ("let x=<Box fill=\"§\"/>", "\"blue3\""),
    ] {
        let (source, items) = complete(marked, true);
        let edited = apply_ascii_edit(&source, item(&items, label));
        assert!(edited.ends_with("/>"), "{marked}: {edited}");
        sanim_lang::parser::parse("main.sanim", &edited, 100_000)
            .unwrap_or_else(|error| panic!("{marked}: {edited}: {error}"));
    }
}
#[test]
fn opening_tags_use_native_registry_and_only_advertised_snippets() {
    let (_, items) = complete("let element=<§", true);
    assert_eq!(
        item(&items, "Box")["textEdit"]["newText"],
        "Box $1>$0</Box>"
    );
    assert_eq!(item(&items, "Box")["insertTextFormat"], 2);
    assert!(!items.iter().any(|item| matches!(
        item["label"].as_str(),
        Some("Group" | "Rect" | "Circle" | "div")
    )));
    let (_, plain) = complete("let element=<Bo§", false);
    assert_eq!(item(&plain, "Box")["textEdit"]["newText"], "Box");
    assert!(item(&plain, "Box")["insertTextFormat"].is_null());
    let (_, existing) = complete("let element=<Bo§x></Box>", true);
    assert_eq!(item(&existing, "Box")["textEdit"]["newText"], "Box");
}

#[test]
fn voxel_layout_tags_and_attributes_come_from_the_native_registry() {
    let (_, tags) = complete("let world=<Voxel§", true);
    item(&tags, "VoxelBox");
    item(&tags, "VoxelPart");
    item(&tags, "VoxelScene");
    let (_, attributes) = complete("let world=<VoxelBox §/>", true);
    for name in [
        "flow", "size", "gap", "align", "justify", "padding", "children",
    ] {
        item(&attributes, name);
    }
    let (_, values) = complete("let world=<VoxelBox flow=\"§\"/>", true);
    for axis in ["\"x\"", "\"y\"", "\"z\""] {
        item(&values, axis);
    }
}
#[test]
fn closing_tags_select_the_innermost_scope_and_preserve_existing_brackets() {
    for (source, label, text) in [
        ("let x=<Box><Label>hello</§", "Label", "Label>"),
        ("let x=<Box><Label>hello</Label></B§", "Box", "Box>"),
        ("let x=<Box><Label>hello</La§bel></Box>", "Label", "Label"),
        (
            "let x=<CodeView lang=\"python\">print(a < b)\n</Co§",
            "CodeView",
            "CodeView>",
        ),
    ] {
        let (_, items) = complete(source, true);
        assert_eq!(items.len(), 1, "{source}: {items:?}");
        assert_eq!(item(&items, label)["textEdit"]["newText"], text);
    }
}
#[test]
fn attribute_names_snippets_and_replacement_ranges_do_not_duplicate_syntax() {
    let (_, items) = complete("let x=<Box width={12pt} §/>", true);
    assert!(items.iter().all(|item| {
        item["label"] != "width"
            && !item["label"]
                .as_str()
                .is_some_and(|label| label.starts_with("w-"))
    }));
    assert_eq!(
        item(&items, "height")["textEdit"]["newText"],
        "height={$1}$0"
    );
    let (source, items) = complete("let emoji=\"🙂\";let x=<Box wi§dth={12pt}/>", true);
    let width = item(&items, "width");
    assert_eq!(width["textEdit"]["newText"], "width");
    let start = source.find("width").unwrap();
    assert_eq!(
        width["textEdit"]["range"]["start"]["character"],
        source[..start].encode_utf16().count()
    );
    assert_eq!(
        width["textEdit"]["range"]["end"]["character"],
        source[..start + 5].encode_utf16().count()
    );
}
#[test]
fn typed_attribute_values_cover_colors_literals_booleans_and_enums() {
    let (_, items) = complete("let x=<Box fill=\"bl§ue3\"/>", true);
    assert_eq!(
        item(&items, "\"blue3\"")["textEdit"]["newText"],
        "\"blue3\""
    );
    let (_, items) = complete("let x=<Box overflow=\"a§uto\"/>", true);
    assert_eq!(item(&items, "\"auto\"")["textEdit"]["newText"], "\"auto\"");
    let (_, items) = complete("let x=<CodeView line_numbers=§/>", true);
    assert_eq!(item(&items, "true")["textEdit"]["newText"], "{true}");
    let (_, items) = complete("let x=<CodeView line_numbers={§}/>", true);
    assert_eq!(item(&items, "true")["textEdit"]["newText"], "true");
    let (_, items) = complete("let x=<Box x_align={§}/>", true);
    assert_eq!(
        item(&items, "Align.Center")["textEdit"]["newText"],
        "Align.Center"
    );
}
#[test]
fn gradient_completions_share_native_conflict_rules() {
    let (_, items) = complete("let x=<Box bg-linear-to-r from-blue3 §/>", true);
    assert!(items.iter().any(|item| item["label"] == "to-orange3"));
    assert!(items.iter().any(|item| item["label"] == "from-10%"));
    assert!(
        !items.iter().any(|item| item["label"] == "fill"),
        "explicit fill cannot coexist with native gradient utilities"
    );
    assert!(
        !items
            .iter()
            .any(|item| item["label"] == "bg-linear-to-l" || item["label"] == "from-orange3")
    );
    let (_, items) = complete("let x=<Box fill=\"blue3\" bg-li§/>", true);
    assert!(!items.iter().any(|item| {
        item["label"]
            .as_str()
            .is_some_and(|label| label.starts_with("bg-linear-"))
    }));
}
#[test]
fn invalid_utilities_keep_diagnostics_and_editor_completion_but_never_render() {
    let source = "let x=<Box definitely_invalid height={20pt}><Label>text</Label></§";
    let (clean, items) = complete(source, true);
    assert_eq!(item(&items, "Box")["textEdit"]["newText"], "Box>");
    assert!(sanim_lang::parser::parse("main.sanim", &clean, 100_000).is_err());
    let invalid = "let x=<Box definitely_invalid height={20pt}><Label>text</Label></Box>";
    let error = sanim_lang::parser::parse("main.sanim", invalid, 100_000).unwrap_err();
    assert!(error.message.contains("definitely_invalid"), "{error}");
    let mut sources = sanim_lang::MemorySources::default();
    sources.insert("main.sanim", invalid);
    let error = sanim_lang::Engine::default()
        .prepare("main.sanim", &sources, &Default::default())
        .unwrap_err();
    assert_eq!(error.code, "S002");
    assert!(error.message.contains("definitely_invalid"), "{error}");
    let (recovery, _) = sanim_lang::parser::parse_highlighted("main.sanim", &clean, 100_000);
    assert!(recovery.diagnostic.is_some());
    let (_, items) = complete("let x=<Box definitely_invalid he§/>", true);
    assert!(items.iter().any(|item| item["label"] == "height"));
}
#[test]
fn strings_comments_comparisons_and_generics_are_not_xml() {
    for source in [
        "let text=r\"<§Box>\"",
        "# <§",
        "let x=<CodeView lang=\"python\">print(<§)",
    ] {
        let (_, items) = complete(source, true);
        assert!(items.is_empty(), "{source}: {items:?}");
    }
    for source in ["let a=1;let b=2;let result=a < §", "let data=read<List<§"] {
        let (_, items) = complete(source, true);
        assert!(
            items.iter().all(|item| !item["textEdit"]["newText"]
                .as_str()
                .is_some_and(|text| text.contains("</"))),
            "{source}: {items:?}"
        );
    }
}
#[test]
#[ignore = "performance measurement: make xml-completion-bench"]
fn xml_completion_benchmark() {
    use std::time::Instant;
    const ELEMENTS: usize = 800;
    const REQUESTS: usize = 120;
    let mut source = String::from("let view=<Box w-");
    let cursor = source.len();
    source.push('>');
    for i in 0..ELEMENTS {
        source.push_str(&format!("<Label>row {i}</Label>"));
    }
    source.push_str("</Box>");
    let dir = tempfile::tempdir().unwrap();
    let uri = format!("file://{}", dir.path().join("main.sanim").display());
    let mut messages = vec![
        initialize(dir.path()),
        notification(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":uri,"languageId":"sanim","version":1,"text":source}}),
        ),
    ];
    for id in 2..REQUESTS + 2 {
        messages.push(request(
            id as u64,
            "textDocument/completion",
            json!({"textDocument":{"uri":uri},"position":{"line":0,"character":cursor}}),
        ));
    }
    finish(&mut messages);
    let started = Instant::now();
    let (_, replies) = session(&messages);
    let elapsed = started.elapsed();
    for id in 2..REQUESTS + 2 {
        assert!(
            reply(&replies, id as u64)["result"]["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["label"] == "w-full")
        );
    }
    println!(
        "XML elements={ELEMENTS} requests={REQUESTS} total_ms={} per_request_us={} (includes one cold parse/check)",
        elapsed.as_millis(),
        elapsed.as_micros() / REQUESTS as u128
    );
}
