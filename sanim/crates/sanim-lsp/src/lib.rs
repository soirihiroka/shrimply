//! Sanim language server, with optional read-only runtime queries supplied by its host.
mod features;
mod runtime;
mod stdlib;
mod text;
mod transport;
mod type_display;
mod workspace;
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use workspace::Workspace;

pub type RuntimeQuery<'a> = &'a mut dyn FnMut(&std::path::Path) -> Result<Value, String>;

#[derive(Default)]
struct Server<'a> {
    workspace: Workspace,
    runtime: Option<RuntimeQuery<'a>>,
    initialized: bool,
    shutdown: bool,
}

/// Run the standalone server without a project runtime connection.
pub fn run(input: impl BufRead, output: impl Write) -> Result<u8, String> {
    run_with_runtime(input, output, None)
}

/// Run an LSP session; the host owns discovery and read-only runtime access.
pub fn run_with_runtime(
    mut input: impl BufRead,
    mut output: impl Write,
    runtime: Option<RuntimeQuery<'_>>,
) -> Result<u8, String> {
    let mut server = Server {
        runtime,
        ..Default::default()
    };
    while let Some(message) = transport::read(&mut input)? {
        if message.get("method").and_then(Value::as_str) == Some("exit") {
            return Ok(u8::from(!server.shutdown));
        }
        for response in server.handle(message) {
            transport::write(&mut output, &response)?;
        }
    }
    Ok(u8::from(!server.shutdown))
}

fn response(id: Value, result: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":result})
}
fn error(id: Value, code: i32, message: impl Into<String>) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message.into()}})
}
fn log(message: impl Into<String>) -> Value {
    json!({"jsonrpc":"2.0","method":"window/logMessage","params":{"type":1,"message":message.into()}})
}

impl Server<'_> {
    fn handle(&mut self, message: Value) -> Vec<Value> {
        if message["jsonrpc"] != "2.0" {
            return vec![error(
                message.get("id").cloned().unwrap_or(Value::Null),
                -32600,
                "expected a JSON-RPC 2.0 message",
            )];
        }
        let Some(method) = message["method"].as_str() else {
            if message.get("result").is_some() || message.get("error").is_some() {
                return Vec::new();
            }
            return vec![error(
                message.get("id").cloned().unwrap_or(Value::Null),
                -32600,
                "request requires method",
            )];
        };
        let id = message.get("id").cloned();
        let params = &message["params"];
        if method == "initialize" {
            let Some(id) = id else {
                return vec![log("initialize requires a request ID")];
            };
            if self.initialized {
                return vec![error(id, -32600, "server was already initialized")];
            }
            let roots = params["workspaceFolders"]
                .as_array()
                .map(|folders| {
                    folders
                        .iter()
                        .filter_map(|folder| folder["uri"].as_str())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_else(|| params["rootUri"].as_str().into_iter().collect());
            for uri in roots {
                match text::path(uri)
                    .and_then(|path| path.canonicalize().map_err(|error| error.to_string()))
                {
                    Ok(root) => self.workspace.roots.push(root),
                    Err(message) => {
                        return vec![error(id, -32602, format!("workspace root: {message}"))];
                    }
                }
            }
            self.initialized = true;
            return vec![response(
                id,
                json!({"serverInfo":{"name":"sanim-lsp","version":env!("CARGO_PKG_VERSION")},"capabilities":{
                    "positionEncoding":"utf-16","textDocumentSync":{"openClose":true,"change":2,"save":{"includeText":true}},
                    "experimental":{"sanimRuntime":self.runtime.is_some()},
                    "completionProvider":{"triggerCharacters":[".","-"]},"hoverProvider":true,"definitionProvider":true,
                "signatureHelpProvider":{"triggerCharacters":["(",","]},"documentSymbolProvider":true,
                "inlayHintProvider":true,
                "documentFormattingProvider":true,
                "semanticTokensProvider":{"legend":{"tokenTypes":features::TOKEN_TYPES,"tokenModifiers":["declaration"]},"full":true}
                }}),
            )];
        }
        if !self.initialized {
            return id.map_or_else(Vec::new, |id| {
                vec![error(id, -32002, "server is not initialized")]
            });
        }
        if self.shutdown {
            return id.map_or_else(Vec::new, |id| {
                vec![error(id, -32600, "server has shut down; send exit")]
            });
        }
        match method {
            "sanim/stdlib" => {
                id.map_or_else(Vec::new, |id| vec![response(id, json!(stdlib::content()))])
            }
            "sanim/runtime" => {
                let Some(id) = id else { return Vec::new() };
                let Some(query) = self.runtime.as_mut() else {
                    return vec![error(
                        id,
                        -32601,
                        "Runtime queries require shrimply sanim lsp",
                    )];
                };
                let result = params["textDocument"]["uri"]
                    .as_str()
                    .ok_or("request requires textDocument.uri".into())
                    .and_then(|uri| {
                        self.workspace
                            .documents
                            .get(uri)
                            .ok_or("document is not open".into())
                    })
                    .and_then(|document| query(&document.path));
                vec![match result {
                    Ok(value) => response(id, value),
                    Err(message) => error(id, -32603, message),
                }]
            }
            "initialized" | "$/cancelRequest" | "$/setTrace" => Vec::new(),
            "shutdown" => {
                self.shutdown = true;
                id.map_or_else(Vec::new, |id| vec![response(id, Value::Null)])
            }
            "textDocument/didOpen" => {
                let result = (|| {
                    let uri = params["textDocument"]["uri"]
                        .as_str()
                        .ok_or("didOpen requires URI")?;
                    let text = params["textDocument"]["text"]
                        .as_str()
                        .ok_or("didOpen requires text")?;
                    let version = params["textDocument"]["version"]
                        .as_i64()
                        .ok_or("didOpen requires integer version")?;
                    if text.len() > sanim_lang::Limits::default().max_source_bytes {
                        return Err("document exceeds Sanim's source-byte limit".into());
                    }
                    self.workspace.open(uri, text.into(), version)?;
                    Ok(self.workspace.refresh(Some(uri)))
                })();
                result.unwrap_or_else(|message: String| vec![log(message)])
            }
            "textDocument/didChange" => {
                let result = (|| {
                    let uri = params["textDocument"]["uri"]
                        .as_str()
                        .ok_or("didChange requires URI")?;
                    let version = params["textDocument"]["version"]
                        .as_i64()
                        .ok_or("didChange requires integer version")?;
                    let changes = params["contentChanges"]
                        .as_array()
                        .ok_or("didChange requires contentChanges")?;
                    let document = self
                        .workspace
                        .documents
                        .get_mut(uri)
                        .ok_or("changed document is not open")?;
                    if version <= document.document.version {
                        return Ok(Vec::new());
                    }
                    let changed = document.document.changed(version, changes)?;
                    if changed.text.len() > sanim_lang::Limits::default().max_source_bytes {
                        return Err("document exceeds Sanim's source-byte limit".into());
                    }
                    document.document = changed;
                    Ok(self.workspace.refresh(Some(uri)))
                })();
                result.unwrap_or_else(|message: String| vec![log(message)])
            }
            "textDocument/didSave" => {
                let Some(uri) = params["textDocument"]["uri"].as_str() else {
                    return vec![log("didSave requires URI")];
                };
                if let Some(text) = params["text"].as_str()
                    && let Some(document) = self.workspace.documents.get_mut(uri)
                {
                    document.document = text::Document::new(text.into(), document.document.version);
                }
                self.workspace.refresh(Some(uri))
            }
            "textDocument/didClose" => {
                let Some(uri) = params["textDocument"]["uri"].as_str() else {
                    return vec![log("didClose requires URI")];
                };
                self.workspace.close(uri);
                self.workspace.refresh(Some(uri))
            }
            "workspace/didChangeWatchedFiles" => {
                // The extension forwards native editor file events. Imported
                // modules are rechecked; runtime data assets are never evaluated.
                let changes = params["changes"].as_array().cloned().unwrap_or_default();
                let mut notifications = Vec::new();
                for change in changes {
                    if let Some(uri) = change["uri"].as_str() {
                        notifications.extend(self.workspace.refresh(Some(uri)));
                    }
                }
                notifications
            }
            "textDocument/completion"
            | "textDocument/hover"
            | "textDocument/definition"
            | "textDocument/signatureHelp"
            | "textDocument/documentSymbol"
            | "textDocument/inlayHint"
            | "textDocument/formatting"
            | "textDocument/semanticTokens/full" => {
                let Some(id) = id else { return Vec::new() };
                let result = (|| {
                    let uri = params["textDocument"]["uri"]
                        .as_str()
                        .ok_or("request requires textDocument.uri")?;
                    match method {
                        "textDocument/completion" => {
                            features::completion(&self.workspace, uri, &params["position"])
                        }
                        "textDocument/hover" => {
                            let mut hover =
                                features::hover(&self.workspace, uri, &params["position"])?;
                            if let Some(query) = self.runtime.as_mut() {
                                runtime::hover(
                                    &self.workspace,
                                    uri,
                                    &params["position"],
                                    &mut hover,
                                    &mut **query,
                                );
                            }
                            Ok(hover)
                        }
                        "textDocument/definition" => {
                            features::definition(&self.workspace, uri, &params["position"])
                        }
                        "textDocument/signatureHelp" => {
                            features::signature(&self.workspace, uri, &params["position"])
                        }
                        "textDocument/inlayHint" => {
                            features::inlay_hints(&self.workspace, uri, &params["range"])
                        }
                        "textDocument/formatting" => {
                            features::formatting(&self.workspace, uri, &params["options"])
                        }
                        "textDocument/semanticTokens/full" => {
                            features::semantic_tokens(&self.workspace, uri)
                        }
                        _ => features::document_symbols(&self.workspace, uri),
                    }
                })();
                vec![match result {
                    Ok(value) => response(id, value),
                    Err(message) => error(id, -32602, message),
                }]
            }
            _ => id.map_or_else(Vec::new, |id| {
                vec![error(
                    id,
                    -32601,
                    format!("method '{method}' is not implemented"),
                )]
            }),
        }
    }
}
