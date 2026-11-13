use serde_json::{Value, json};
use std::io::{BufRead, Read};

fn session(messages: &[Value]) -> (u8, Vec<Value>) {
    let mut input = Vec::new();
    for message in messages {
        let bytes = serde_json::to_vec(message).unwrap();
        input.extend_from_slice(format!("Content-Length: {}\r\n\r\n", bytes.len()).as_bytes());
        input.extend(bytes);
    }
    let mut output = Vec::new();
    let status = sanim_lsp::run(std::io::Cursor::new(input), &mut output).unwrap();
    let mut reader = std::io::Cursor::new(output);
    let mut replies = Vec::new();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).unwrap() == 0 {
            break;
        }
        let length = header
            .strip_prefix("Content-Length: ")
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let mut empty = String::new();
        reader.read_line(&mut empty).unwrap();
        assert_eq!(empty, "\r\n");
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        replies.push(serde_json::from_slice(&body).unwrap());
    }
    (status, replies)
}
fn request(id: u64, method: &str, params: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
}
fn notification(method: &str, params: Value) -> Value {
    json!({"jsonrpc":"2.0","method":method,"params":params})
}
fn initialize(root: &std::path::Path) -> Value {
    request(
        1,
        "initialize",
        json!({"rootUri":format!("file://{}",root.display())}),
    )
}
fn finish(messages: &mut Vec<Value>) {
    messages.push(request(999, "shutdown", Value::Null));
    messages.push(notification("exit", Value::Null));
}
fn reply(replies: &[Value], id: u64) -> &Value {
    replies.iter().find(|reply| reply["id"] == id).unwrap()
}

#[test]
fn real_stdio_session_reports_types_completion_hover_definition_and_symbols() {
    let dir = tempfile::tempdir().unwrap();
    let uri = format!("file://{}", dir.path().join("main.sanim").display());
    let source = "let size=10pt\nlet marker=Circle(radius=size)\nadd(marker)\nwait(1s)";
    let mut messages = vec![
        initialize(dir.path()),
        notification("initialized", json!({})),
        notification(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":uri,"languageId":"sanim","version":1,"text":source}}),
        ),
    ];
    for (id, method, character) in [
        (2, "textDocument/completion", 15),
        (3, "textDocument/hover", 15),
        (4, "textDocument/definition", 26),
        (6, "textDocument/signatureHelp", 26),
    ] {
        messages.push(request(
            id,
            method,
            json!({"textDocument":{"uri":uri},"position":{"line":1,"character":character}}),
        ));
    }
    messages.push(request(
        5,
        "textDocument/documentSymbol",
        json!({"textDocument":{"uri":uri}}),
    ));
    messages.push(notification("textDocument/didChange",json!({"textDocument":{"uri":uri,"version":2},"contentChanges":[{"range":{"start":{"line":0,"character":9},"end":{"line":0,"character":13}},"text":"false"}]})));
    messages.push(notification(
        "textDocument/didChange",
        json!({"textDocument":{"uri":uri,"version":1},"contentChanges":[{"text":"let invalid=("}]}),
    ));
    messages.push(notification(
        "textDocument/didChange",
        json!({"textDocument":{"uri":uri,"version":3},"contentChanges":[{"text":source}]}),
    ));
    finish(&mut messages);
    let (status, replies) = session(&messages);
    assert_eq!(status, 0);
    assert_eq!(
        reply(&replies, 1)["result"]["capabilities"]["textDocumentSync"]["change"],
        2
    );
    assert!(
        reply(&replies, 2)["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["label"] == "Circle")
    );
    assert!(
        reply(&replies, 3)["result"]["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("radius")
    );
    assert_eq!(reply(&replies, 4)["result"]["uri"], uri);
    assert_eq!(reply(&replies, 4)["result"]["range"]["start"]["line"], 0);
    assert!(
        reply(&replies, 5)["result"]
            .as_array()
            .unwrap()
            .iter()
            .any(|symbol| symbol["name"] == "marker")
    );
    assert!(
        reply(&replies, 6)["result"]["signatures"][0]["label"]
            .as_str()
            .unwrap()
            .contains("Circle")
    );
    assert!(replies.iter().any(|reply| {
        reply["method"] == "textDocument/publishDiagnostics"
            && reply["params"]["version"] == 2
            && reply["params"]["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|diagnostic| diagnostic["severity"] == 1)
    }));
    assert!(
        replies
            .iter()
            .any(|reply| reply["method"] == "textDocument/publishDiagnostics"
                && reply["params"]["version"] == 3
                && reply["params"]["diagnostics"] == json!([]))
    );
}

#[test]
fn unsaved_import_edits_recheck_dependents_without_evaluating_data() {
    let dir = tempfile::tempdir().unwrap();
    let main = format!("file://{}", dir.path().join("main.sanim").display());
    let helper = format!("file://{}", dir.path().join("helper.sanim").display());
    std::fs::write(dir.path().join("helper.sanim"), "let size=10pt").unwrap();
    let source = "import \"helper.sanim\" as helper\nlet marker=Circle(radius=helper.size)\nadd(marker)\nwait(1s)";
    let mut messages = vec![
        initialize(dir.path()),
        notification(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":main,"version":1,"text":source}}),
        ),
        notification(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":helper,"version":1,"text":"let size=false"}}),
        ),
    ];
    messages.push(notification("textDocument/didChange",json!({"textDocument":{"uri":helper,"version":2},"contentChanges":[{"text":"let size=10pt\nlet dormant=read<Number>(\"absent.json\")"}]})));
    messages.push(request(
        2,
        "textDocument/definition",
        json!({"textDocument":{"uri":main},"position":{"line":1,"character":33}}),
    ));
    finish(&mut messages);
    let (_, replies) = session(&messages);
    let diagnostics = replies
        .iter()
        .filter(|reply| {
            reply["method"] == "textDocument/publishDiagnostics" && reply["params"]["uri"] == main
        })
        .map(|reply| &reply["params"]["diagnostics"])
        .collect::<Vec<_>>();
    assert!(diagnostics.iter().any(|diagnostics| {
        diagnostics
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["severity"] == 1)
    }));
    assert_eq!(diagnostics.last().unwrap().as_array().unwrap().len(), 0);
    assert_eq!(reply(&replies, 2)["result"]["uri"], helper);
    assert!(
        !replies
            .iter()
            .any(|reply| reply.to_string().contains("cannot read 'absent.json'"))
    );
}

#[test]
fn lifecycle_and_unknown_methods_return_protocol_errors_without_fake_capabilities() {
    let dir = tempfile::tempdir().unwrap();
    let mut messages = vec![
        request(2, "textDocument/hover", json!({})),
        initialize(dir.path()),
        request(3, "textDocument/rename", json!({})),
    ];
    finish(&mut messages);
    let (status, replies) = session(&messages);
    assert_eq!(status, 0);
    assert_eq!(reply(&replies, 2)["error"]["code"], -32002);
    assert_eq!(reply(&replies, 3)["error"]["code"], -32601);
    assert!(
        reply(&replies, 1)["result"]["capabilities"]
            .get("renameProvider")
            .is_none()
    );
}

#[test]
fn transport_rejects_ambiguous_or_truncated_frames() {
    for bytes in [
        b"Content-Length: 2\r\nContent-Length: 2\r\n\r\n{}".as_slice(),
        b"Content-Length: 5\r\n\r\n{}".as_slice(),
        b"Content-Length: 999999999\r\n\r\n".as_slice(),
    ] {
        assert!(sanim_lsp::run(std::io::Cursor::new(bytes), Vec::new()).is_err());
    }
}

#[test]
fn semantic_tokens_and_inlays_use_utf16_types_and_exact_timeline_effects() {
    let dir = tempfile::tempdir().unwrap();
    let uri = format!("file://{}", dir.path().join("main.sanim").display());
    let source = "let size=10pt\nlet title=\"😊 {size}\"\nlet marker=Circle(radius=size)\nadd(marker)\nwait(0.5s)\n";
    let mut messages = vec![
        initialize(dir.path()),
        notification(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":uri,"version":1,"text":source}}),
        ),
    ];
    messages.push(request(
        2,
        "textDocument/semanticTokens/full",
        json!({"textDocument":{"uri":uri}}),
    ));
    messages.push(request(3,"textDocument/inlayHint",json!({"textDocument":{"uri":uri},"range":{"start":{"line":0,"character":0},"end":{"line":5,"character":0}}})));
    messages.push(request(
        4,
        "textDocument/hover",
        json!({"textDocument":{"uri":uri},"position":{"line":4,"character":2}}),
    ));
    finish(&mut messages);
    let (_, replies) = session(&messages);
    assert_eq!(
        reply(&replies, 1)["result"]["capabilities"]["inlayHintProvider"],
        true
    );
    assert_eq!(
        reply(&replies, 1)["result"]["capabilities"]["semanticTokensProvider"]["full"],
        true
    );
    let data = reply(&replies, 2)["result"]["data"].as_array().unwrap();
    assert!(!data.is_empty());
    assert_eq!(data.len() % 5, 0);
    let (mut line, mut character) = (0, 0);
    for token in data.chunks(5) {
        let delta = token[0].as_u64().unwrap();
        line += delta;
        character = if delta == 0 {
            character + token[1].as_u64().unwrap()
        } else {
            token[1].as_u64().unwrap()
        };
        let length = token[2].as_u64().unwrap();
        assert!(
            length > 0,
            "zero-width synthetic nodes must never become tokens"
        );
        let text = source.lines().nth(line as usize).unwrap();
        assert!(
            character + length <= text.encode_utf16().count() as u64,
            "{line}:{character}+{length} outside {text}"
        );
    }
    let hints = reply(&replies, 3)["result"].as_array().unwrap();
    assert!(
        hints
            .iter()
            .any(|hint| hint["position"] == json!({"line":0,"character":8})
                && hint["label"].as_str().unwrap().contains("Length"))
    );
    assert!(
        hints
            .iter()
            .any(|hint| hint["label"].as_str().unwrap().contains("1/2s"))
    );
    assert!(hints.windows(2).all(|hints| {
        let key = |value: &Value| {
            (
                value["position"]["line"].as_u64(),
                value["position"]["character"].as_u64(),
            )
        };
        key(&hints[0]) <= key(&hints[1])
    }));
    assert!(
        reply(&replies, 4)["result"]["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("1/2s")
    );
}

#[test]
fn inline_types_are_named_and_hover_shows_the_full_structure() {
    let dir = tempfile::tempdir().unwrap();
    let uri = format!("file://{}", dir.path().join("main.sanim").display());
    let source = "let xs=LinearScale(domain=Vec2(0,1),range=Vec2(0pt,100pt))\nlet custom={value=1,enabled=true}\nfn current_scale():sig LinearScale {return xs}\nlet live_scale=current_scale()\n";
    let mut messages = vec![
        initialize(dir.path()),
        notification(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":uri,"version":1,"text":source}}),
        ),
        request(
            2,
            "textDocument/inlayHint",
            json!({"textDocument":{"uri":uri},"range":{"start":{"line":0,"character":0},"end":{"line":4,"character":0}}}),
        ),
    ];
    finish(&mut messages);
    let (_, replies) = session(&messages);
    assert!(
        !replies
            .iter()
            .any(|reply| reply["method"] == "textDocument/publishDiagnostics"
                && reply["params"]["diagnostics"]
                    .as_array()
                    .is_some_and(|items| items.iter().any(|item| item["severity"] == 1))),
        "{replies:?}"
    );
    let hints = reply(&replies, 2)["result"].as_array().unwrap();
    let scale = hints
        .iter()
        .find(|hint| hint["position"]["line"] == 0)
        .unwrap();
    assert_eq!(scale["label"], ": LinearScale");
    let tooltip = scale["tooltip"]["value"].as_str().unwrap();
    for field in ["clamp: Bool", "domain: Vec2", "range: Vec2"] {
        assert!(tooltip.contains(field), "{tooltip}");
    }
    assert!(!tooltip.contains("No animation"));
    assert!(!tooltip.contains("static type checker"));
    let record = hints
        .iter()
        .find(|hint| hint["position"]["line"] == 1)
        .unwrap();
    assert_eq!(record["label"], ": {…}");
    assert!(
        record["tooltip"]["value"]
            .as_str()
            .unwrap()
            .contains("enabled: Bool")
    );
    let live = hints
        .iter()
        .find(|hint| hint["position"]["line"] == 3)
        .unwrap();
    assert_eq!(live["label"], ": sig LinearScale");
    assert!(
        live["tooltip"]["value"]
            .as_str()
            .unwrap()
            .contains("domain: Vec2")
    );
}

#[test]
#[ignore = "measured by make -C sanim lsp-bench"]
fn incremental_analysis_benchmark() {
    let dir = tempfile::tempdir().unwrap();
    let uri = format!("file://{}", dir.path().join("main.sanim").display());
    let mut messages = vec![
        initialize(dir.path()),
        notification(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":uri,"version":1,"text":"let size=1pt\nlet marker=Circle(radius=size)\nadd(marker)\nwait(1s)"}}),
        ),
    ];
    for version in 2..=101 {
        messages.push(notification("textDocument/didChange",json!({"textDocument":{"uri":uri,"version":version},"contentChanges":[{"text":format!("let size={version}pt\nlet marker=Circle(radius=size)\nadd(marker)\nwait(1s)")}]})));
    }
    finish(&mut messages);
    let started = std::time::Instant::now();
    let (status, replies) = session(&messages);
    assert_eq!(status, 0);
    assert!(
        replies
            .iter()
            .filter(|reply| reply["method"] == "textDocument/publishDiagnostics")
            .count()
            >= 101
    );
    eprintln!(
        "LSP 100 versioned edits: {} us total, {} us/edit (static analysis + framing, debug/release dependent)",
        started.elapsed().as_micros(),
        started.elapsed().as_micros() / 100
    );
}

#[test]
fn declaration_hover_and_parameter_definition_use_canonical_name_ranges() {
    let dir = tempfile::tempdir().unwrap();
    let uri = format!("file://{}", dir.path().join("main.sanim").display());
    let source = "fn only(argument: Number): Number {return argument}";
    let parameter_use = source.rfind("argument").unwrap() + 1;
    let mut messages = vec![
        initialize(dir.path()),
        notification(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":uri,"version":1,"text":source}}),
        ),
    ];
    messages.push(request(
        2,
        "textDocument/hover",
        json!({"textDocument":{"uri":uri},"position":{"line":0,"character":4}}),
    ));
    messages.push(request(
        3,
        "textDocument/definition",
        json!({"textDocument":{"uri":uri},"position":{"line":0,"character":parameter_use}}),
    ));
    messages.push(request(
        4,
        "textDocument/documentSymbol",
        json!({"textDocument":{"uri":uri}}),
    ));
    finish(&mut messages);
    let (_, replies) = session(&messages);
    assert!(
        reply(&replies, 2)["result"]["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("only")
    );
    assert_eq!(
        reply(&replies, 3)["result"]["range"],
        json!({"start":{"line":0,"character":8},"end":{"line":0,"character":16}})
    );
    let symbols = reply(&replies, 4)["result"].as_array().unwrap();
    assert_eq!(
        symbols.len(),
        1,
        "a function spanning the whole file does not make parameters root symbols"
    );
    assert_eq!(symbols[0]["name"], "only");
}

#[cfg(unix)]
#[test]
fn unsaved_symlink_uri_overlays_keep_client_uris_and_recheck_imports() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("real");
    std::fs::create_dir(&root).unwrap();
    let alias = dir.path().join("alias route");
    std::os::unix::fs::symlink(&root, &alias).unwrap();
    std::fs::write(root.join("helper.sanim"), "let size=10pt").unwrap();
    let main = format!("file://{}", alias.join("main.sanim").display()).replace(' ', "%20");
    let helper = format!("file://{}", alias.join("helper.sanim").display()).replace(' ', "%20");
    let source = "import \"helper.sanim\" as helper\nlet marker=Circle(radius=helper.size)\nadd(marker)\nwait(1s)";
    let mut messages = vec![
        initialize(&root),
        notification(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":main,"version":1,"text":source}}),
        ),
        notification(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":helper,"version":1,"text":"\n\nlet size=false"}}),
        ),
    ];
    messages.push(notification("textDocument/didChange",json!({"textDocument":{"uri":helper,"version":2},"contentChanges":[{"text":"\n\nlet size=10pt"}]})));
    messages.push(request(
        2,
        "textDocument/definition",
        json!({"textDocument":{"uri":main},"position":{"line":1,"character":33}}),
    ));
    messages.push(notification("textDocument/didChange",json!({"textDocument":{"uri":helper,"version":3},"contentChanges":[{"text":"\n\nlet size=("}]})));
    finish(&mut messages);
    let (_, replies) = session(&messages);
    assert_eq!(reply(&replies, 2)["result"]["uri"], helper);
    let diagnostics = replies
        .iter()
        .filter(|reply| reply["method"] == "textDocument/publishDiagnostics")
        .map(|reply| &reply["params"])
        .collect::<Vec<_>>();
    assert!(diagnostics.iter().any(|params| {
        params["uri"] == main
            && params["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|diagnostic| diagnostic["severity"] == 1)
    }));
    assert!(
        diagnostics
            .iter()
            .any(|params| params["uri"] == main && params["diagnostics"] == json!([]))
    );
    assert!(diagnostics.iter().any(|params| {
        params["uri"] == helper
            && params["version"] == 3
            && params["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|diagnostic| diagnostic["range"]["start"]["line"] == 2)
    }));
    assert!(
        diagnostics
            .iter()
            .all(|params| params["uri"] == main || params["uri"] == helper)
    );
}

#[test]
fn unfinished_calls_keep_typed_prefix_completion_and_native_user_signatures() {
    let dir = tempfile::tempdir().unwrap();
    let uri = format!("file://{}", dir.path().join("main.sanim").display());
    let prefix = "let size=10pt\nfn delayed(period:Duration):Unit{wait(period)}\n";
    let native = format!("{prefix}let marker=Circle(radius=");
    let user = format!("{prefix}delayed(");
    let mut messages = vec![
        initialize(dir.path()),
        notification(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":uri,"version":1,"text":native}}),
        ),
    ];
    let position = json!({"line":2,"character":"let marker=Circle(radius=".len()});
    messages.push(request(
        2,
        "textDocument/signatureHelp",
        json!({"textDocument":{"uri":uri},"position":position}),
    ));
    messages.push(request(
        3,
        "textDocument/completion",
        json!({"textDocument":{"uri":uri},"position":position}),
    ));
    messages.push(notification(
        "textDocument/didChange",
        json!({"textDocument":{"uri":uri,"version":2},"contentChanges":[{"text":user}]}),
    ));
    messages.push(request(
        4,
        "textDocument/signatureHelp",
        json!({"textDocument":{"uri":uri},"position":{"line":2,"character":"delayed(".len()}}),
    ));
    finish(&mut messages);
    let (_, replies) = session(&messages);
    let native = &reply(&replies, 2)["result"];
    assert!(
        native["signatures"][0]["label"]
            .as_str()
            .unwrap()
            .contains("Circle")
    );
    let active = native["activeParameter"].as_u64().unwrap() as usize;
    assert!(
        native["signatures"][0]["parameters"][active]["label"]
            .as_str()
            .unwrap()
            .starts_with("radius")
    );
    assert!(
        reply(&replies, 3)["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["label"] == "size"
                && item["detail"].as_str().unwrap().contains("Length"))
    );
    assert!(
        reply(&replies, 4)["result"]["signatures"][0]["label"]
            .as_str()
            .unwrap()
            .contains("period")
    );
    assert!(replies.iter().any(|reply| {
        reply["method"] == "textDocument/publishDiagnostics"
            && reply["params"]["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|diagnostic| diagnostic["severity"] == 1)
    }));
}

#[test]
fn source_formatting_uses_unsaved_text_preserves_literals_and_utf16_ranges() {
    let directory = tempfile::tempdir().unwrap();
    let uri = format!("file://{}", directory.path().join("format.sanim").display());
    let source = "#abcdef\nlet title=\"😀 {round(2.5):.1f}\"\nfn value():Number{return 1}\n";
    let canonical =
        sanim_lang::formatter::format_source("main.sanim", source, Default::default(), 1_000_000)
            .unwrap();
    let mut messages = vec![
        initialize(directory.path()),
        notification(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":uri,"version":1,"text":source}}),
        ),
        request(
            2,
            "textDocument/formatting",
            json!({"textDocument":{"uri":uri},"options":{"tabSize":4,"insertSpaces":true}}),
        ),
        request(
            5,
            "textDocument/semanticTokens/full",
            json!({"textDocument":{"uri":uri}}),
        ),
        notification(
            "textDocument/didChange",
            json!({"textDocument":{"uri":uri,"version":2},"contentChanges":[{"text":canonical}]}),
        ),
        request(
            3,
            "textDocument/formatting",
            json!({"textDocument":{"uri":uri},"options":{"tabSize":4,"insertSpaces":true}}),
        ),
        notification(
            "textDocument/didChange",
            json!({"textDocument":{"uri":uri,"version":3},"contentChanges":[{"text":"let incomplete=("}]}),
        ),
        request(
            4,
            "textDocument/formatting",
            json!({"textDocument":{"uri":uri},"options":{"tabSize":4,"insertSpaces":true}}),
        ),
    ];
    finish(&mut messages);
    let (_, responses) = session(&messages);
    assert_eq!(
        reply(&responses, 1)["result"]["capabilities"]["documentFormattingProvider"],
        true
    );
    let edit = &reply(&responses, 2)["result"][0];
    assert_eq!(edit["newText"], canonical);
    assert!(
        reply(&responses, 5)["result"]["data"]
            .as_array()
            .unwrap()
            .chunks_exact(5)
            .any(|token| token[3] == 9 && token[2] == 7)
    );
    assert_eq!(
        edit["range"],
        json!({"start":{"line":0,"character":0},"end":{"line":3,"character":0}})
    );
    assert_eq!(reply(&responses, 3)["result"], json!([]));
    assert!(reply(&responses, 4).get("error").is_some());
}

#[test]
fn dataframe_and_series_methods_use_the_typed_native_registry() {
    let dir = tempfile::tempdir().unwrap();
    let uri = format!("file://{}", dir.path().join("main.sanim").display());
    let source = "let df=DataFrame([{kind=\"a\",x=1}])\nlet values=df[\"x\"]\nlet chosen=df.head(1)\nlet total=values.sum()\nlet grouped=df.groupby(\"kind\")\nlet stats=grouped.sum()\n";
    let position = |line: usize, needle: &str| json!({"line":line,"character":source.lines().nth(line).unwrap().find(needle).unwrap()});
    let mut messages = vec![
        initialize(dir.path()),
        notification(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":uri,"version":1,"text":source}}),
        ),
        request(
            2,
            "textDocument/completion",
            json!({"textDocument":{"uri":uri},"position":position(2,"head")}),
        ),
        request(
            3,
            "textDocument/completion",
            json!({"textDocument":{"uri":uri},"position":position(3,"sum")}),
        ),
        request(
            4,
            "textDocument/completion",
            json!({"textDocument":{"uri":uri},"position":position(5,"sum")}),
        ),
        request(
            5,
            "textDocument/hover",
            json!({"textDocument":{"uri":uri},"position":position(2,"head")}),
        ),
        request(
            6,
            "textDocument/signatureHelp",
            json!({"textDocument":{"uri":uri},"position":position(2,"1)")}),
        ),
    ];
    let pending = format!("{source}let pending=values.fillna(");
    messages.push(notification(
        "textDocument/didChange",
        json!({"textDocument":{"uri":uri,"version":2},"contentChanges":[{"text":pending}]}),
    ));
    messages.push(request(7,"textDocument/signatureHelp",json!({"textDocument":{"uri":uri},"position":{"line":6,"character":"let pending=values.fillna(".len()}})));
    finish(&mut messages);
    let (_, replies) = session(&messages);
    let labels = |id| {
        reply(&replies, id)["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["label"].as_str().unwrap())
            .collect::<Vec<_>>()
    };
    assert!(labels(2).contains(&"head"), "{replies:#?}");
    assert!(labels(2).contains(&"columns"));
    assert!(!labels(2).contains(&"fillna"));
    assert!(labels(3).contains(&"fillna"));
    assert!(labels(3).contains(&"sum"));
    assert!(!labels(3).contains(&"head"));
    assert!(labels(4).contains(&"agg"));
    assert!(!labels(4).contains(&"head"));
    let hover = reply(&replies, 5)["result"]["contents"]["value"]
        .as_str()
        .unwrap();
    assert!(hover.contains("head(n?: Number"), "{hover}");
    assert!(!hover.contains("frame:"));
    assert!(!hover.contains("Unknown"));
    assert_eq!(
        reply(&replies, 6)["result"]["signatures"][0]["parameters"][0]["label"],
        "n?: Number"
    );
    assert_eq!(
        reply(&replies, 7)["result"]["signatures"][0]["parameters"][0]["label"],
        "value"
    );
}

#[test]
fn a_type_error_keeps_earlier_typed_declarations_available_for_navigation() {
    let dir = tempfile::tempdir().unwrap();
    let uri = format!("file://{}", dir.path().join("main.sanim").display());
    let source = "let size=12pt\nlet invalid=Circle(radius=false)\nlet value=size\n";
    let mut messages = vec![
        initialize(dir.path()),
        notification(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":uri,"version":1,"text":source}}),
        ),
        request(
            2,
            "textDocument/hover",
            json!({"textDocument":{"uri":uri},"position":{"line":2,"character":11}}),
        ),
        request(
            3,
            "textDocument/definition",
            json!({"textDocument":{"uri":uri},"position":{"line":2,"character":11}}),
        ),
    ];
    finish(&mut messages);
    let (_, replies) = session(&messages);
    assert!(replies.iter().any(|reply| {
        reply["method"] == "textDocument/publishDiagnostics"
            && reply["params"]["diagnostics"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
    }));
    let hover = reply(&replies, 2)["result"]["contents"]["value"]
        .as_str()
        .unwrap();
    assert!(hover.contains("size: Length"), "{hover}");
    assert_eq!(
        reply(&replies, 3)["result"]["range"]["start"],
        json!({"line":0,"character":4})
    );
}

#[test]
fn multiline_string_tokens_use_the_document_line_index_for_every_line_ending() {
    for ending in ["\r", "\r\n", "\n"] {
        let dir = tempfile::tempdir().unwrap();
        let uri = format!("file://{}", dir.path().join("main.sanim").display());
        let lines = [
            "let title=\"\"\"one",
            "two 😊",
            "three\"\"\"",
            "wait(1s)",
            "",
        ];
        let source = lines.join(ending);
        let mut messages = vec![
            initialize(dir.path()),
            notification(
                "textDocument/didOpen",
                json!({"textDocument":{"uri":uri,"version":1,"text":source}}),
            ),
            request(
                2,
                "textDocument/semanticTokens/full",
                json!({"textDocument":{"uri":uri}}),
            ),
        ];
        finish(&mut messages);
        let (_, replies) = session(&messages);
        let data = reply(&replies, 2)["result"]["data"].as_array().unwrap();
        let (mut line, mut character, mut string_parts) = (0, 0, 0);
        for token in data.chunks_exact(5) {
            let delta = token[0].as_u64().unwrap() as usize;
            line += delta;
            character = if delta == 0 {
                character + token[1].as_u64().unwrap() as usize
            } else {
                token[1].as_u64().unwrap() as usize
            };
            assert!(
                character + token[2].as_u64().unwrap() as usize
                    <= lines[line].encode_utf16().count(),
                "{ending:?}: token crosses line {line}: {token:?}"
            );
            if token[3] == 6 {
                string_parts += 1;
            }
        }
        assert_eq!(string_parts, 3, "{ending:?}");
    }
}

#[test]
fn template_literals_keep_string_highlighting_and_typed_interpolation_names() {
    let dir = tempfile::tempdir().unwrap();
    let uri = format!("file://{}", dir.path().join("main.sanim").display());
    let lines = [
        "let count=7",
        "let caption=\"\"\"literal 😊 {count}",
        "# still string text\"\"\"",
        "let label=<Label>native {count}</Label>",
        "",
    ];
    let source = lines.join("\n");
    let mut messages = vec![
        initialize(dir.path()),
        notification(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":uri,"version":1,"text":source}}),
        ),
        request(
            2,
            "textDocument/semanticTokens/full",
            json!({"textDocument":{"uri":uri}}),
        ),
    ];
    finish(&mut messages);
    let (_, replies) = session(&messages);
    let data = reply(&replies, 2)["result"]["data"].as_array().unwrap();
    let (mut line, mut character) = (0usize, 0usize);
    let mut tokens = Vec::new();
    for token in data.chunks_exact(5) {
        let delta = token[0].as_u64().unwrap() as usize;
        line += delta;
        character = if delta == 0 {
            character + token[1].as_u64().unwrap() as usize
        } else {
            token[1].as_u64().unwrap() as usize
        };
        let length = token[2].as_u64().unwrap() as usize;
        let text = String::from_utf16(
            &lines[line]
                .encode_utf16()
                .skip(character)
                .take(length)
                .collect::<Vec<_>>(),
        )
        .unwrap();
        tokens.push((line, text, token[3].as_u64().unwrap()));
    }
    assert!(
        tokens
            .iter()
            .any(|(line, text, kind)| *line == 1 && text.contains("literal 😊") && *kind == 6),
        "{tokens:?}"
    );
    assert!(
        tokens
            .iter()
            .any(|(line, text, kind)| *line == 2 && text.starts_with("# still") && *kind == 6),
        "{tokens:?}"
    );
    for expected_line in [1, 3] {
        assert!(
            tokens
                .iter()
                .any(|(line, text, kind)| *line == expected_line && text == "count" && *kind == 3),
            "{tokens:?}"
        );
    }
    assert!(
        tokens
            .iter()
            .any(|(line, text, kind)| *line == 3 && text == "Label" && *kind == 2),
        "{tokens:?}"
    );
}

#[test]
fn markup_utility_completion_replaces_full_hyphenated_tokens_and_hover_resolves_properties() {
    let dir = tempfile::tempdir().unwrap();
    let uri = format!("file://{}", dir.path().join("main.sanim").display());
    // UTF-16 edit ranges must remain correct after non-BMP source characters.
    let source = "let title=\"🙂\"; let panel=<Box w-180.5 items-center h-50% />\nwait(1s)";
    let position = |needle: &str, inside: usize| {
        let start = source.find(needle).unwrap();
        json!({"line":0,"character":source[..start + inside].encode_utf16().count()})
    };
    let mut messages = vec![
        initialize(dir.path()),
        notification(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":uri,"languageId":"sanim","version":1,"text":source}}),
        ),
        request(
            2,
            "textDocument/completion",
            json!({"textDocument":{"uri":uri},"position":position("w-180.5",2)}),
        ),
        request(
            3,
            "textDocument/completion",
            json!({"textDocument":{"uri":uri},"position":position("items-center",6)}),
        ),
        request(
            4,
            "textDocument/hover",
            json!({"textDocument":{"uri":uri},"position":position("h-50%",3)}),
        ),
        notification(
            "textDocument/didChange",
            json!({"textDocument":{"uri":uri,"version":2},"contentChanges":[{"text":"let panel=<Box w-"}]}),
        ),
        request(
            5,
            "textDocument/completion",
            json!({"textDocument":{"uri":uri},"position":{"line":0,"character":17}}),
        ),
    ];
    finish(&mut messages);
    let (_, replies) = session(&messages);
    assert!(
        reply(&replies, 1)["result"]["capabilities"]["completionProvider"]["triggerCharacters"]
            .as_array()
            .unwrap()
            .contains(&json!("-"))
    );
    let items = reply(&replies, 2)["result"]["items"].as_array().unwrap();
    let width = items.iter().find(|item| item["label"] == "w-180").unwrap();
    assert_eq!(width["detail"], "width=180pt");
    assert_eq!(width["textEdit"]["range"]["start"], position("w-180.5", 0));
    assert_eq!(width["textEdit"]["range"]["end"], position("w-180.5", 7));
    let align = reply(&replies, 3)["result"]["items"].as_array().unwrap();
    assert!(align.iter().any(|item| item["label"] == "items-end"));
    assert!(!align.iter().any(|item| item["label"] == "Circle"));
    let hover = reply(&replies, 4)["result"]["contents"]["value"]
        .as_str()
        .unwrap();
    assert!(hover.contains("height=50%"), "{hover}");
    assert!(hover.contains("parent layout box"), "{hover}");
    let incomplete = reply(&replies, 5)["result"]["items"].as_array().unwrap();
    assert!(incomplete.iter().any(|item| item["label"] == "w-full"));
}

#[test]
fn markup_utility_diagnostics_are_specific_and_explicit_properties_are_intentional() {
    let dir = tempfile::tempdir().unwrap();
    let uri = format!("file://{}", dir.path().join("main.sanim").display());
    let cases = [
        (
            "let panel=<Box widht-20 />",
            "unknown native utility 'widht-20'",
        ),
        (
            "let panel=<Box w-20px />",
            "nonnegative logical point value",
        ),
        ("let panel=<Line rounded-8 />", "does not support"),
        ("let panel=<Box w-100 w-180 />", "overridden"),
        ("let panel=<Box w-100 w-100 />", "repeated"),
        ("let panel=<Box w-100 w-180 width={220pt} />", ""),
    ];
    for (index, (source, expected)) in cases.iter().enumerate() {
        let mut messages = vec![
            initialize(dir.path()),
            notification(
                "textDocument/didOpen",
                json!({"textDocument":{"uri":uri,"languageId":"sanim","version":index+1,"text":source}}),
            ),
        ];
        finish(&mut messages);
        let (_, replies) = session(&messages);
        let diagnostics = replies
            .iter()
            .filter(|reply| reply["method"] == "textDocument/publishDiagnostics")
            .flat_map(|reply| reply["params"]["diagnostics"].as_array().unwrap());
        let messages = diagnostics
            .map(|diagnostic| diagnostic["message"].as_str().unwrap())
            .collect::<Vec<_>>();
        if expected.is_empty() {
            assert!(
                !messages
                    .iter()
                    .any(|message| message.contains("overridden") || message.contains("repeated")),
                "{messages:?}"
            );
        } else {
            assert!(
                messages.iter().any(|message| message.contains(expected)),
                "{source}: {messages:?}"
            );
        }
    }
}

#[test]
fn utility_completion_never_treats_strings_comments_or_attribute_values_as_attributes() {
    let dir = tempfile::tempdir().unwrap();
    let uri = format!("file://{}", dir.path().join("main.sanim").display());
    for source in [
        "let value=\"<Box w-\"",
        "# <Box w-",
        "let panel=<Box id=\"w-\" />",
        "let view=<CodeView lang=\"python\">\n<Box w-\n</CodeView>",
    ] {
        let offset = source.find("w-").unwrap() + 2;
        let prefix = &source[..offset];
        let line = prefix.bytes().filter(|byte| *byte == b'\n').count();
        let character = prefix.rsplit('\n').next().unwrap().encode_utf16().count();
        let mut messages = vec![
            initialize(dir.path()),
            notification(
                "textDocument/didOpen",
                json!({"textDocument":{"uri":uri,"languageId":"sanim","version":1,"text":source}}),
            ),
            request(
                2,
                "textDocument/completion",
                json!({"textDocument":{"uri":uri},"position":{"line":line,"character":character}}),
            ),
        ];
        finish(&mut messages);
        let (_, replies) = session(&messages);
        assert!(
            !reply(&replies, 2)["result"]["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["label"] == "w-full"),
            "{source}"
        );
    }
}

#[test]
#[ignore = "manual editor latency benchmark"]
fn markup_utility_completion_benchmark() {
    let dir = tempfile::tempdir().unwrap();
    let uri = format!("file://{}", dir.path().join("main.sanim").display());
    let source = "let panel=<Box w-180 items-center h-50% />\nwait(1s)";
    let mut messages = vec![
        initialize(dir.path()),
        notification(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":uri,"languageId":"sanim","version":1,"text":source}}),
        ),
    ];
    const REQUESTS: u64 = 500;
    for index in 0..REQUESTS {
        messages.push(request(
            index + 2,
            "textDocument/completion",
            json!({"textDocument":{"uri":uri},"position":{"line":0,"character":17}}),
        ));
    }
    finish(&mut messages);
    let started = std::time::Instant::now();
    let (_, replies) = session(&messages);
    let elapsed = started.elapsed();
    assert!(
        reply(&replies, 2)["result"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["label"] == "w-full")
    );
    println!(
        "native utility completion: {REQUESTS} requests in {elapsed:?}, {:.3} ms/request including framing, initial analysis and JSON",
        elapsed.as_secs_f64() * 1000.0 / REQUESTS as f64
    );
}

#[test]
fn closed_string_union_arguments_complete_from_checked_signatures_in_complete_and_partial_calls() {
    let dir = tempfile::tempdir().unwrap();
    let uri = format!("file://{}", dir.path().join("main.sanim").display());
    let declaration = "type Edge = \"left\" | \"center\" | \"right\"\nfn choose(edge:Edge):String {return edge}\n";
    for call in ["choose(\"left\")", "choose(edge=\"le", "choose("] {
        let source = format!("{declaration}{call}");
        let character = if call.ends_with(')') {
            call.len() - 2
        } else {
            call.len()
        };
        let mut messages = vec![
            initialize(dir.path()),
            notification(
                "textDocument/didOpen",
                json!({"textDocument":{"uri":uri,"languageId":"sanim","version":1,"text":source}}),
            ),
            request(
                2,
                "textDocument/completion",
                json!({"textDocument":{"uri":uri},"position":{"line":2,"character":character}}),
            ),
        ];
        finish(&mut messages);
        let (_, replies) = session(&messages);
        let items = reply(&replies, 2)["result"]["items"].as_array().unwrap();
        let center = items
            .iter()
            .find(|item| item["label"] == "\"center\"")
            .unwrap_or_else(|| panic!("{call}: {items:?}"));
        assert_eq!(center["textEdit"]["newText"], "\"center\"");
        assert!(!items.iter().any(|item| item["label"] == "Circle"));
    }
}

#[test]
fn literal_completion_edits_round_trip_sanim_braces_and_control_characters() {
    let dir = tempfile::tempdir().unwrap();
    let uri = format!("file://{}", dir.path().join("main.sanim").display());
    let value = "{before}\u{8}\u{c}\0";
    let declaration = format!("type Tag=r\"{value}\"\nfn choose(tag:Tag):Tag {{return tag}}\n");
    let call = format!("let selected=choose(r\"{value}\")");
    let mut source = format!("{declaration}{call}");
    let mut messages = vec![
        initialize(dir.path()),
        notification(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":uri,"languageId":"sanim","version":1,"text":source}}),
        ),
        request(
            2,
            "textDocument/completion",
            json!({"textDocument":{"uri":uri},"position":{"line":2,"character":call.len()-2}}),
        ),
    ];
    finish(&mut messages);
    let (status, replies) = session(&messages);
    assert_eq!(status, 0);
    let items = reply(&replies, 2)["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{items:?}");
    let edit = &items[0]["textEdit"];
    assert_eq!(edit["range"]["start"]["line"], 2);
    assert_eq!(edit["range"]["end"]["line"], 2);
    let start = edit["range"]["start"]["character"].as_u64().unwrap() as usize;
    let end = edit["range"]["end"]["character"].as_u64().unwrap() as usize;
    let text = edit["newText"].as_str().unwrap();
    assert!(text.contains("{{before}}"));
    assert!(!text.contains("\\b") && !text.contains("\\u"));
    source.replace_range(declaration.len() + start..declaration.len() + end, text);
    let mut files = sanim_lang::MemorySources::default();
    files.insert("main.sanim", &source);
    let prepared = sanim_lang::Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap_or_else(|error| panic!("{error}; source={source:?}"));
    assert_eq!(prepared.bindings["selected"].string().unwrap(), value);
}
