mod common;
use common::*;
use serde_json::json;

#[test]
fn spawned_functions_expose_main_and_side_durations_and_task_local_definitions() {
    let dir = tempfile::tempdir().unwrap();
    let uri = format!("file://{}", dir.path().join("tasks.sanim").display());
    let source = "fn<mut> launch():Task{return spawn{wait(2s)}}\nlet job=launch()\nwait(1s)\njob.join()\nlet second=spawn{let delay=500ms;wait(delay)}\nsecond.join()\n";
    let mut messages = vec![
        initialize(dir.path()),
        notification(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":uri,"languageId":"sanim","version":1,"text":source}}),
        ),
        request(
            2,
            "textDocument/hover",
            json!({"textDocument":{"uri":uri},"position":{"line":0,"character":source.lines().next().unwrap().find("launch").unwrap()+1}}),
        ),
        request(
            3,
            "textDocument/hover",
            json!({"textDocument":{"uri":uri},"position":{"line":3,"character":5}}),
        ),
        request(
            4,
            "textDocument/definition",
            json!({"textDocument":{"uri":uri},"position":{"line":4,"character":source.lines().nth(4).unwrap().rfind("delay").unwrap()+1}}),
        ),
    ];
    finish(&mut messages);
    let (status, replies) = session(&messages);
    assert_eq!(status, 0);
    let launch = reply(&replies, 2)["result"]["contents"]["value"]
        .as_str()
        .unwrap();
    assert!(launch.contains("Main duration: **0s**"), "{launch}");
    assert!(launch.contains("Side duration: **2s**"), "{launch}");
    let join = reply(&replies, 3)["result"]["contents"]["value"]
        .as_str()
        .unwrap();
    assert!(join.contains("Main duration: **1s**"), "{join}");
    assert!(join.contains("Side duration: **0s**"), "{join}");
    assert_eq!(reply(&replies, 4)["result"]["range"]["start"]["line"], 4);
}

#[test]
fn capabilities_are_visible_and_impure_callbacks_fail_in_real_editor_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let uri = format!("file://{}", dir.path().join("capabilities.sanim").display());
    let source = "fn apply(callback:fn(value:Number):Number,value:Number):Number{return callback(value)}\nfn<mut> delayed(value:Number):Number{wait(1s);return value}\nlet result=apply(delayed,1)\n";
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
