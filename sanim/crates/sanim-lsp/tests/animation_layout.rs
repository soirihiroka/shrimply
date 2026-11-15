mod common;
use common::*;
use sanim_lang::{
    ast::StatementKind,
    core::{
        Fraction,
        plan::{self, Node, Origin, TimeDimension},
    },
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

fn fixture(text: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "sanim-layout-hover-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("main.sanim"), text).unwrap();
    root
}
fn receipts(text: &str, parent: u64) -> Value {
    let module = sanim_lang::parser::parse("main.sanim", text, 100_000).unwrap();
    let StatementKind::Bind { value, .. } = &module.statements[0].kind else {
        panic!("binding")
    };
    let mut animation = plan::Animation::new(Node::Container(Default::default()));
    animation.style.duration = TimeDimension::MatchParent;
    animation.origin = Some(Origin {
        location: sanim_lang::core::SourceLocation {
            source: "main.sanim".into(),
            span: value.span,
        },
        binding: Some("clip".into()),
        checkpoint: 0,
    });
    serde_json::to_value(
        animation
            .layout(Some(Fraction::from(parent)), 100)
            .unwrap()
            .receipts,
    )
    .unwrap()
}
fn messages(root: &Path, text: &str, character: usize) -> Vec<Value> {
    let uri = format!("file://{}", root.join("main.sanim").display());
    let mut messages = vec![
        initialize(root),
        notification(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":uri,"languageId":"sanim","version":1,"text":text}}),
        ),
        request(
            2,
            "textDocument/hover",
            json!({"textDocument":{"uri":uri},"position":{"line":0,"character":character}}),
        ),
    ];
    finish(&mut messages);
    messages
}
#[test]
fn animation_hover_reports_the_same_resolved_size_as_each_editor_clip_layout() {
    let text = "let clip=<Animation duration=\"match_parent\"/>\nclip";
    let root = fixture(text);
    let contexts = [4,6].map(|parent|json!({"project":"project.shrimp","label":format!("clip-{parent}"),"state":{"source":root.join("main.sanim"),"address":{"item_id":format!("item-{parent}")},"parameters":[],"temporal_layout":receipts(text,parent),"error":null}}));
    let mut query = |_: &Path| Ok(json!({"contexts":contexts}));
    for character in [4, text.find("Animation").unwrap()] {
        let (status, replies) =
            session_with_runtime(&messages(&root, text, character), Some(&mut query));
        assert_eq!(status, 0);
        for message in &replies {
            assert!(
                !message["params"]["diagnostics"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|diagnostic| diagnostic["severity"] == 1),
                "{message}"
            );
        }
        let hover = reply(&replies, 2)["result"]["contents"]["value"]
            .as_str()
            .unwrap();
        assert!(hover.contains("Duration: **4s**"), "{hover}");
        assert!(hover.contains("Duration: **6s**"), "{hover}");
        assert!(hover.contains("match_parent"));
        assert!(!hover.contains("Main duration"));
    }
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn unsaved_animation_hover_does_not_query_or_display_an_old_clip_size() {
    let saved = "let clip=<Animation duration={2s}/>\nclip";
    let current = "let clip=<Animation duration={3s}/>\nclip";
    let root = fixture(saved);
    let mut calls = 0;
    let mut query = |_: &Path| {
        calls += 1;
        Ok(json!({"contexts":[]}))
    };
    let (_, replies) = session_with_runtime(&messages(&root, current, 4), Some(&mut query));
    assert_eq!(calls, 0);
    for message in &replies {
        assert!(
            !message["params"]["diagnostics"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|diagnostic| diagnostic["severity"] == 1),
            "{message}"
        );
    }
    assert!(
        reply(&replies, 2)["result"]["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("Save this document")
    );
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn static_animation_tags_complete_and_function_hovers_have_no_timeline_effect() {
    let text = "fn clip(): Animation { return <Animation duration={2s}/> }\nclip()";
    let root = fixture(text);
    let uri = format!("file://{}", root.join("main.sanim").display());
    let mut messages = vec![
        initialize(&root),
        notification(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":uri,"languageId":"sanim","version":1,"text":text}}),
        ),
        request(
            2,
            "textDocument/hover",
            json!({"textDocument":{"uri":uri},"position":{"line":0,"character":4}}),
        ),
    ];
    finish(&mut messages);
    let (_, replies) = session(&messages);
    for message in &replies {
        assert!(
            !message["params"]["diagnostics"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|diagnostic| diagnostic["severity"] == 1),
            "{message}"
        );
    }
    let hover = reply(&replies, 2)["result"]["contents"]["value"]
        .as_str()
        .unwrap();
    assert!(hover.contains("Animation"));
    assert!(!hover.contains("duration:"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn program_duration_is_one_hint_at_the_last_code_line() {
    for (text, expected) in [
        ("let saved=wait(8s)\nwait(1s)\nwait(2s)\n", "3s"),
        (
            "fn build():Animation{return wait(2s)}\nrepeat 3 {build()}",
            "6s",
        ),
        (
            "<Animation flow=\"sequence\">{wait(1s)}{wait(2s)}</Animation>",
            "3s",
        ),
        ("wait(forever)", "forever"),
        ("<Animation duration=\"match_parent\"/>", "match_parent"),
    ] {
        let root = fixture(text);
        let uri = format!("file://{}", root.join("main.sanim").display());
        let lines = text.split('\n').collect::<Vec<_>>();
        let range = json!({"start":{"line":0,"character":0},"end":{"line":lines.len()-1,"character":lines.last().unwrap().encode_utf16().count()}});
        let mut messages = vec![
            initialize(&root),
            notification(
                "textDocument/didOpen",
                json!({"textDocument":{"uri":uri,"languageId":"sanim","version":1,"text":text}}),
            ),
            request(
                2,
                "textDocument/inlayHint",
                json!({"textDocument":{"uri":uri},"range":range}),
            ),
        ];
        finish(&mut messages);
        let (_, replies) = session(&messages);
        let hint = reply(&replies, 2)["result"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|hint| {
                hint["label"]
                    .as_str()
                    .is_some_and(|label| label.starts_with("duration:"))
            })
            .collect::<Vec<_>>();
        assert_eq!(hint.len(), 1, "{text}: {replies:?}");
        assert_eq!(
            hint[0]["label"],
            format!("duration: {expected}"),
            "{text}: {replies:?}"
        );
        let last = text.trim_end().split('\n').collect::<Vec<_>>();
        assert_eq!(
            hint[0]["position"],
            json!({"line":last.len()-1,"character":last.last().unwrap().encode_utf16().count()})
        );
        assert!(hint[0].get("textEdits").is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn standalone_hover_measures_literal_animation_with_the_canonical_solver() {
    let text = "let clip=<Animation duration={2s}/>\nclip";
    let root = fixture(text);
    let (_, replies) = session(&messages(&root, text, text.find("Animation").unwrap()));
    let hover = reply(&replies, 2)["result"]["contents"]["value"]
        .as_str()
        .unwrap();
    assert!(hover.contains("Duration: **2s**"), "{replies:?}");
    std::fs::remove_dir_all(root).unwrap();
}
