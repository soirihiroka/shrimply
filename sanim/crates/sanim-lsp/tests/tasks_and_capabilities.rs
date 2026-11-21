mod common;
use common::*;
use serde_json::json;

#[test]
fn pure_animation_factories_expose_value_types_and_local_definitions() {
    let dir = tempfile::tempdir().unwrap();
    let uri = format!("file://{}", dir.path().join("plans.sanim").display());
    let source = "fn launch():Animation{return wait(2s)}\nlet first=launch()\nlet delay=500ms\n<Animation flow=\"sequence\">{first}{wait(delay)}</Animation>\n";
    let mut messages = vec![
        initialize(dir.path()),
        notification(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":uri,"languageId":"sanim","version":1,"text":source}}),
        ),
        request(
            2,
            "textDocument/hover",
            json!({"textDocument":{"uri":uri},"position":{"line":0,"character":4}}),
        ),
        request(
            3,
            "textDocument/definition",
            json!({"textDocument":{"uri":uri},"position":{"line":3,"character":source.lines().nth(3).unwrap().find("delay").unwrap()+1}}),
        ),
    ];
    finish(&mut messages);
    let (status, replies) = session(&messages);
    assert_eq!(status, 0);
    let hover = reply(&replies, 2)["result"]["contents"]["value"]
        .as_str()
        .unwrap();
    assert!(hover.contains("Animation"), "{hover}");
    assert!(!hover.contains("Side duration"), "{hover}");
    assert_eq!(reply(&replies, 3)["result"]["range"]["start"]["line"], 2);
}

#[test]
fn capabilities_are_visible_and_impure_callbacks_fail_in_real_editor_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let uri = format!("file://{}", dir.path().join("capabilities.sanim").display());
    let source = "fn apply(callback:fn(value:Number):Number,value:Number):Number{return callback(value)}\nfn<mut> delayed(value:Number):Number{check(true);return value}\nlet result=apply(delayed,1)\n";
    let mut messages = vec![
        initialize(dir.path()),
        notification(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":uri,"languageId":"sanim","version":1,"text":source}}),
        ),
        request(
            2,
            "textDocument/hover",
            json!({"textDocument":{"uri":uri},"position":{"line":1,"character":source.lines().nth(1).unwrap().find("delayed").unwrap()+1}}),
        ),
    ];
    finish(&mut messages);
    let (status, replies) = session(&messages);
    assert_eq!(status, 0);
    let diagnostics: Vec<_> = replies
        .iter()
        .filter(|message| message["method"] == "textDocument/publishDiagnostics")
        .flat_map(|message| {
            message["params"]["diagnostics"]
                .as_array()
                .into_iter()
                .flatten()
        })
        .collect();
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic["message"]
                .as_str()
                .is_some_and(|message| message.contains("fn<mut>") && message.contains("fn("))
        }),
        "{diagnostics:?}"
    );
    let hover = reply(&replies, 2)["result"]["contents"]["value"]
        .as_str()
        .unwrap();
    assert!(hover.contains("fn<mut>"), "{hover}");
}
