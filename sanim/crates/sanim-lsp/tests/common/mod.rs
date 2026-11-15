use serde_json::{Value, json};
use std::io::{BufRead, Read};

pub fn session(messages: &[Value]) -> (u8, Vec<Value>) {
    session_with_runtime(messages, None)
}
pub fn session_with_runtime(
    messages: &[Value],
    runtime: Option<sanim_lsp::RuntimeQuery<'_>>,
) -> (u8, Vec<Value>) {
    let mut input = Vec::new();
    for message in messages {
        let bytes = serde_json::to_vec(message).unwrap();
        input.extend_from_slice(format!("Content-Length: {}\r\n\r\n", bytes.len()).as_bytes());
        input.extend(bytes);
    }
    let mut output = Vec::new();
    let status =
        sanim_lsp::run_with_runtime(std::io::Cursor::new(input), &mut output, runtime).unwrap();
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
pub fn request(id: u64, method: &str, params: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
}
pub fn notification(method: &str, params: Value) -> Value {
    json!({"jsonrpc":"2.0","method":method,"params":params})
}
pub fn initialize(root: &std::path::Path) -> Value {
    request(
        1,
        "initialize",
        json!({"rootUri":format!("file://{}",root.display())}),
    )
}
pub fn finish(messages: &mut Vec<Value>) {
    messages.push(request(999, "shutdown", Value::Null));
    messages.push(notification("exit", Value::Null));
}
pub fn reply(replies: &[Value], id: u64) -> &Value {
    replies.iter().find(|reply| reply["id"] == id).unwrap()
}
