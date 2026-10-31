//! Caps on what one message, batch or tool call can make the server do (#1727): overlong
//! framing is answered and skipped rather than buffered, batches and `command_batch` have message,
//! step and kept-result limits.

use std::io::BufRead;

use serde_json::{Value, json};

use crate::server::{MAX_BATCH_MESSAGES, MAX_LINE_BYTES, read_line_capped};
use crate::tools::MAX_COMMAND_BATCH_STEPS;
use crate::{Headless, Server};

fn rpc(s: &mut Server, method: &str, params: Value) -> Value {
    s.handle(json!({"jsonrpc":"2.0","id":1,"method":method,"params":params})).unwrap()
}

fn call(s: &mut Server, name: &str, arguments: Value) -> Value {
    let r = rpc(s, "tools/call", json!({"name":name,"arguments":arguments}));
    assert!(r.get("error").is_none(), "{r}");
    r["result"].clone()
}

fn payload(r: &Value) -> Value {
    serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap()
}

fn error_text(r: &Value) -> String {
    r["content"][0]["text"].as_str().unwrap_or_default().to_string()
}

/// One request line, exactly as it arrives on stdin.
fn line(method: &str, params: Value) -> String {
    json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string() + "\n"
}

/// Serve every line of `input`; the replies in order.
fn serve(s: &mut Server, input: &[u8]) -> Vec<Value> {
    let mut out = Vec::new();
    s.serve(input, &mut out).unwrap();
    String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str::<Value>(l).unwrap()).collect()
}

// ---------- line bounds ----------

#[test]
fn read_line_capped_bounds_a_line_without_a_newline() {
    let mut buf = Vec::new();
    // Three times the cap, no newline: only `max + 1` bytes are buffered, and the caller sees an
    // overlong line.
    let input = vec![b'x'; 3 * MAX_LINE_BYTES];
    assert!(read_line_capped(&mut &input[..], &mut buf, MAX_LINE_BYTES).unwrap());
    assert_eq!(buf.len(), MAX_LINE_BYTES + 1);
    // Exactly the cap, no newline: the whole last line still parses.
    let input = vec![b'x'; MAX_LINE_BYTES];
    assert!(read_line_capped(&mut &input[..], &mut buf, MAX_LINE_BYTES).unwrap());
    assert_eq!(buf.len(), MAX_LINE_BYTES);
    // Newlines and CRLF still split, and the stream ends cleanly.
    let input = b"{\"a\":1}\n{\"b\":2}\r\n";
    let mut lines = Vec::new();
    let mut reader = &input[..];
    while read_line_capped(&mut reader, &mut buf, MAX_LINE_BYTES).unwrap() {
        lines.push(String::from_utf8(buf.clone()).unwrap());
    }
    assert_eq!(lines, ["{\"a\":1}", "{\"b\":2}"]);
    assert!(reader.fill_buf().unwrap().is_empty());
}

#[test]
fn overlong_request_line_is_answered_and_framing_continues() {
    let mut s = Server::new(Box::new(Headless::with_document()));
    let mut input = vec![b'x'; MAX_LINE_BYTES + 100];
    input.extend_from_slice(b"\n");
    input.extend_from_slice(line("ping", json!({})).as_bytes());
    input.extend_from_slice(line("ping", json!({})).as_bytes());
    let replies = serve(&mut s, &input);
    assert_eq!(replies.len(), 3, "{replies:?}");
    assert_eq!(replies[0]["error"]["code"], -32600);
    assert!(replies[0]["error"]["message"].as_str().unwrap_or_default().contains("longer than"), "{replies:?}");
    assert_eq!(replies[1]["result"], json!({}), "framing continues: {replies:?}");
    assert_eq!(replies[2]["result"], json!({}));
}

#[test]
fn line_at_the_cap_is_still_a_message() {
    let mut s = Server::new(Box::new(Headless::with_document()));
    // Exactly the cap, newline-terminated: it parses (and fails as JSON, not as framing).
    let mut input = vec![b'x'; MAX_LINE_BYTES];
    input.extend_from_slice(b"\n");
    let replies = serve(&mut s, &input);
    assert_eq!(replies.len(), 1, "{replies:?}");
    assert_eq!(replies[0]["error"]["code"], -32700, "{replies:?}");
    assert!(replies[0]["error"]["message"].as_str().unwrap_or_default().contains("parse error"), "{replies:?}");
}

// ---------- batch bounds ----------

fn batch(n: usize) -> String {
    let messages: Vec<Value> = (0..n).map(|i| json!({"jsonrpc": "2.0", "id": i, "method": "ping", "params": {}})).collect();
    Value::Array(messages).to_string()
}

#[test]
fn batch_over_the_message_cap_is_refused() {
    let mut s = Server::new(Box::new(Headless::with_document()));
    let reply: Value = serde_json::from_str(&s.handle_line(&batch(MAX_BATCH_MESSAGES + 1)).unwrap()).unwrap();
    assert_eq!(reply["error"]["code"], -32600, "{reply}");
    assert!(reply["error"]["message"].as_str().unwrap_or_default().contains("exceeds"), "{reply}");
}

#[test]
fn batch_at_the_message_cap_is_answered() {
    let mut s = Server::new(Box::new(Headless::with_document()));
    let reply: Value = serde_json::from_str(&s.handle_line(&batch(MAX_BATCH_MESSAGES)).unwrap()).unwrap();
    assert_eq!(reply.as_array().map(Vec::len), Some(MAX_BATCH_MESSAGES), "{reply}");
}

// ---------- command_batch bounds ----------

#[test]
fn command_batch_over_the_step_cap_is_refused_before_anything_runs() {
    let mut s = Server::new(Box::new(Headless::with_document()));
    let steps: Vec<Value> =
        (0..MAX_COMMAND_BATCH_STEPS + 1).map(|i| json!({"id": "shape.rectangle", "params": {"x": i, "y": 1, "width": 5, "height": 5}})).collect();
    let before = payload(&call(&mut s, "doc_inspect", json!({"depth": 0})))["objects"].clone();
    let r = call(&mut s, "command_batch", json!({"steps": steps}));
    assert_eq!(r["isError"], true, "{r}");
    assert!(error_text(&r).contains(&format!("at most {MAX_COMMAND_BATCH_STEPS}")), "{r}");
    assert_eq!(payload(&call(&mut s, "doc_inspect", json!({"depth": 0})))["objects"], before, "nothing ran: {r}");
}

/// A document whose inspect is large, then a batch of inspects: the kept results hit the byte cap
/// and the batch stops with the reason, instead of piling up an unbounded reply.
fn big_document(s: &mut Server) {
    for _ in 0..6 {
        let steps: Vec<Value> = (0..250).map(|i| json!({"id": "shape.rectangle", "params": {"x": i, "y": 1, "width": 5, "height": 5}})).collect();
        let r = call(s, "command_batch", json!({"steps": steps}));
        assert_eq!(r["isError"], false, "{r}");
    }
}

#[test]
fn command_batch_stops_when_kept_results_grow_too_big() {
    let mut s = Server::new(Box::new(Headless::with_document()));
    big_document(&mut s);
    let steps: Vec<Value> = (0..20).map(|_| json!({"id": "document.inspect", "params": {}})).collect();
    let r = call(&mut s, "command_batch", json!({"steps": steps}));
    assert_eq!(r["isError"], true, "{r}");
    let p = payload(&r);
    let results = p["results"].as_array().unwrap();
    assert!(results.len() < steps.len(), "the batch stopped early: {}", results.len());
    assert!(!results.is_empty(), "earlier results are kept: {p}");
    assert_eq!(results[0]["ok"], json!(true), "{p}");
    assert_eq!(p["failed"], 1, "{p}");
    let completed = p["completed"].as_u64().unwrap();
    assert_eq!(completed + 1, results.len() as u64, "{p}");
    assert!(results.last().unwrap()["error"].as_str().unwrap_or_default().contains("exceed"), "{p}");
}

#[test]
fn atomic_batch_still_closes_its_undo_group_when_results_grow_too_big() {
    let mut s = Server::new(Box::new(Headless::with_document()));
    big_document(&mut s);
    let objects = |s: &mut Server| payload(&call(s, "doc_inspect", json!({"depth":0})))["objects"].as_u64().unwrap();
    let before = objects(&mut s);
    // Edits first (grouped), then the inspects that breach the cap: the stopped batch must still
    // close its group into one undo step, not leave it open.
    let mut steps = vec![json!({"id": "shape.rectangle", "params": {"x": 1, "y": 1, "width": 5, "height": 5}})];
    steps.extend((0..20).map(|_| json!({"id": "document.inspect", "params": {}})));
    let r = call(&mut s, "command_batch", json!({"steps": steps, "atomic": true, "label": "Capped batch"}));
    assert_eq!(r["isError"], true, "{r}");
    assert_eq!(objects(&mut s), before + 1, "the edit ran: {r}");
    assert_eq!(payload(&call(&mut s, "run_command", json!({"command": "edit.undo"})))["undone"], "Capped batch");
    assert_eq!(objects(&mut s), before, "one undo removed the grouped edit");
}
