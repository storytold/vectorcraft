//! JSON-RPC 2.0 framing and the MCP lifecycle / tools / resources / prompts methods.

use std::io::{BufRead, Write};

use serde_json::{Value, json};

use crate::backend::Backend;
use crate::prompts;
use crate::resources;
use crate::tools::{call_tool, tool_definitions};

/// The MCP revision we implement.
pub const PROTOCOL_VERSION: &str = "2025-06-18";
const MODERN_VERSION: &str = "2026-07-28";
const SUPPORTED_VERSIONS: &[&str] = &[MODERN_VERSION, PROTOCOL_VERSION, "2025-03-26", "2024-11-05"];

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;
const RESOURCE_NOT_FOUND: i64 = -32002;

/// Longest accepted request line (bytes, without the newline). Requests are small JSON objects;
/// anything longer is answered with an error and skipped, so one huge line can't make the server
/// allocate without bound (#1727).
pub(crate) const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;
/// Most messages one batch may hold. Batches were removed from the protocol; this only bounds what
/// older clients can make the server parse and answer at once (#1727).
pub(crate) const MAX_BATCH_MESSAGES: usize = 64;

const INSTRUCTIONS: &str = "VectorCraft is a professional vector illustration app. Coordinates are \
points in document space (y down, origin at the first artboard's top-left; a new document is 612×792). \
Draw with draw_shape / draw_path, change colours with set_paint, look with render_preview and doc_inspect. \
Every menu action is a command: find it with command_list and run it with command_run (or command_batch for several). New objects become \
the selection, and most commands act on the selection (or on explicit `ids`). prompts/list has ready-made \
workflows, completion/complete finishes a command id, a format, an effect or a swatch name, and \
vectorcraft://object/{id}, vectorcraft://command/{id}, vectorcraft://effect/{id} and \
vectorcraft://swatch/{name} read one thing at a time instead of the whole document.";

/// An MCP server bound to one backend.
pub struct Server {
    backend: Box<dyn Backend>,
    initialized: bool,
    modern: bool,
    /// Whether this server's client asked for log records (`logging/setLevel`). The queue is
    /// process-wide, so a server whose client didn't ask leaves it to the one that did.
    logging: bool,
}

fn response(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

/// Read one `\n`-terminated line into `buf` (newline stripped). `Ok(false)` at end of stream.
/// `buf` holding more than `max` bytes means the line is overlong: no newline arrived within the
/// bound, so the bytes so far are only its start. Only `max + 1` bytes are ever buffered for one
/// line, in the reader's own bounded chunks, so a peer can't make us allocate without bound
/// (#1727).
pub(crate) fn read_line_capped(reader: &mut impl BufRead, buf: &mut Vec<u8>, max: usize) -> std::io::Result<bool> {
    buf.clear();
    loop {
        let take = {
            let chunk = reader.fill_buf()?;
            if chunk.is_empty() {
                return Ok(!buf.is_empty());
            }
            let upto = chunk.iter().position(|&b| b == b'\n').map_or(chunk.len(), |i| i + 1);
            upto.min(max.saturating_add(1).saturating_sub(buf.len()))
        };
        if take == 0 {
            // Already past the bound: the caller sees `buf.len() > max`.
            return Ok(true);
        }
        // The chunk is still buffered (nothing was consumed), so the same bytes are there; the
        // `min` keeps even a misbehaving reader from panicking us.
        let chunk = reader.fill_buf()?;
        let take = take.min(chunk.len());
        buf.extend_from_slice(&chunk[..take]);
        reader.consume(take);
        if buf.last() == Some(&b'\n') {
            buf.pop();
            if buf.last() == Some(&b'\r') {
                buf.pop();
            }
            return Ok(true);
        }
    }
}

/// One reply line on the protocol stream.
fn write_reply(output: &mut impl Write, line: &str) -> std::io::Result<()> {
    output.write_all(line.as_bytes())?;
    output.write_all(b"\n")?;
    output.flush()
}

impl Server {
    pub fn new(backend: Box<dyn Backend>) -> Self {
        Self { backend, initialized: false, modern: false, logging: false }
    }

    pub fn backend(&mut self) -> &mut dyn Backend {
        self.backend.as_mut()
    }

    /// Whether the client has sent `notifications/initialized`.
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// The `notifications/message` records queued since the last call.
    ///
    /// Log records belong to no particular request, so these go out after the reply to whatever
    /// line produced them rather than before it. [`serve`] drains this itself; a caller driving
    /// [`handle_line`] by hand does the same.
    pub fn take_notifications(&mut self) -> Vec<String> {
        if !self.logging {
            return vec![];
        }
        crate::logging::drain()
            .into_iter()
            .map(|record| json!({"jsonrpc": "2.0", "method": "notifications/message", "params": record}).to_string())
            .collect()
    }

    /// Serve newline-delimited JSON-RPC until `input` closes. Logs go to stderr only (stdout is
    /// the protocol stream).
    pub fn serve(&mut self, input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
        let mut reader = input;
        let mut buf = Vec::new();
        loop {
            if !read_line_capped(&mut reader, &mut buf, MAX_LINE_BYTES)? {
                return Ok(());
            }
            if buf.len() > MAX_LINE_BYTES {
                // Longer than anything a client can mean: answer the framing violation, skip the
                // rest of the line in bounded chunks, and keep serving what follows.
                write_reply(
                    &mut output,
                    &error(Value::Null, INVALID_REQUEST, format!("request line longer than {MAX_LINE_BYTES} bytes")).to_string(),
                )?;
                loop {
                    if !read_line_capped(&mut reader, &mut buf, MAX_LINE_BYTES)? {
                        return Ok(());
                    }
                    if buf.len() <= MAX_LINE_BYTES {
                        break;
                    }
                }
                continue;
            }
            let line = std::str::from_utf8(&buf).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            let reply = self.handle_line(line);
            let notes = self.take_notifications();
            for line in reply.iter().chain(notes.iter()) {
                write_reply(&mut output, line)?;
            }
        }
    }

    /// Handle one line; returns the reply line (None for notifications and blank lines).
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        let reply = match serde_json::from_str::<Value>(line) {
            Ok(Value::Array(batch)) => {
                // Batches were removed in 2025-06-18; still answer older clients sensibly.
                if batch.is_empty() {
                    Some(error(Value::Null, INVALID_REQUEST, "empty batch"))
                } else if batch.len() > MAX_BATCH_MESSAGES {
                    Some(error(Value::Null, INVALID_REQUEST, format!("batch of {} messages exceeds {MAX_BATCH_MESSAGES}", batch.len())))
                } else {
                    let replies: Vec<Value> = batch.into_iter().filter_map(|m| self.handle(m)).collect();
                    (!replies.is_empty()).then_some(Value::Array(replies))
                }
            }
            Ok(msg) => self.handle(msg),
            Err(e) => Some(error(Value::Null, PARSE_ERROR, format!("parse error: {e}"))),
        };
        reply.map(|r| r.to_string())
    }

    /// Handle one JSON-RPC message; `None` for notifications and responses.
    pub fn handle(&mut self, msg: Value) -> Option<Value> {
        let Value::Object(o) = &msg else { return Some(error(Value::Null, INVALID_REQUEST, "message must be an object")) };
        let id = o.get("id").cloned();
        let Some(method) = o.get("method").and_then(Value::as_str) else {
            // A response to a server→client request (we send none) — ignore; anything else is invalid.
            if o.contains_key("result") || o.contains_key("error") {
                return None;
            }
            return Some(error(id.unwrap_or(Value::Null), INVALID_REQUEST, "missing `method`"));
        };
        // JSON-RPC 2.0 is the only envelope this server speaks: a request or notification in any
        // other envelope is never dispatched and never changes state (#1728).
        if o.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            // A notification has no id to address a reply to; ignoring it is the only answer.
            return id.map(|id| error(id, INVALID_REQUEST, "expected `\"jsonrpc\": \"2.0\"`"));
        };
        let params = o.get("params").cloned().unwrap_or(Value::Null);
        let Some(id) = id else {
            self.notification(method, &params);
            return None;
        };
        if !(id.is_string() || id.is_number()) {
            return Some(error(Value::Null, INVALID_REQUEST, "`id` must be a string or number"));
        }
        // A bug that panics fails this request only; the server keeps serving.
        let r = vectorcraft_engine::guard::catch_panic(|| self.request(method, &params))
            .unwrap_or_else(|msg| Err((INTERNAL_ERROR, format!("internal error in `{method}`: {msg} (please report this bug)"))));
        Some(match r {
            Ok(mut r) => {
                let modern = params
                    .get("_meta")
                    .and_then(|m| m.get("io.modelcontextprotocol/protocolVersion"))
                    .and_then(Value::as_str)
                    .map_or(self.modern, |v| v == MODERN_VERSION);
                if modern && let Some(result) = r.as_object_mut() {
                    let ttl = match method {
                        "tools/list" | "resources/list" | "resources/templates/list" | "prompts/list" => Some(600_000),
                        "resources/read" | "prompts/get" => Some(0),
                        _ => None,
                    };
                    if let Some(ttl) = ttl {
                        result.insert("resultType".into(), json!("complete"));
                        result.insert("ttlMs".into(), json!(ttl));
                        result.insert("cacheScope".into(), json!("private"));
                    }
                }
                response(id, r)
            }
            Err((code, m)) => error(id, code, m),
        })
    }

    fn notification(&mut self, method: &str, _params: &Value) {
        match method {
            "notifications/initialized" => self.initialized = true,
            "notifications/cancelled" | "notifications/progress" | "notifications/roots/list_changed" => {}
            other => log::debug!("ignoring notification {other}"),
        }
    }

    fn request(&mut self, method: &str, params: &Value) -> Result<Value, (i64, String)> {
        match method {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL_VERSION);
                let version = if SUPPORTED_VERSIONS.contains(&asked) { asked } else { PROTOCOL_VERSION };
                self.modern = version == MODERN_VERSION;
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": {
                        "tools": {},
                        "resources": {},
                        "prompts": {"listChanged": false},
                        "completions": {},
                        "logging": {},
                    },
                    "serverInfo": {"name": "vectorcraft", "title": "VectorCraft", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": format!("{INSTRUCTIONS} Backend: {}.", self.backend.describe()),
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tool_definitions()})),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).ok_or((INVALID_PARAMS, "missing tool `name`".to_string()))?;
                let args = params.get("arguments").cloned().unwrap_or(Value::Null);
                if let Some(arguments) = args.as_object() {
                    crate::tools::check_tool_arguments(name, arguments).map_err(|e| (INVALID_PARAMS, e))?;
                }
                Ok(call_tool(self.backend.as_mut(), name, &args).to_value())
            }
            "resources/list" => Ok(resources::list()),
            "resources/templates/list" => Ok(resources::templates()),
            "resources/read" => {
                let uri = params.get("uri").and_then(Value::as_str).ok_or((INVALID_PARAMS, "missing `uri`".to_string()))?;
                let v = resources::read(self.backend.as_mut(), uri).map_err(|e| match e {
                    resources::ReadError::NotFound(uri) => (RESOURCE_NOT_FOUND, format!("resource not found: {uri}")),
                    resources::ReadError::Invalid(why) => (INVALID_PARAMS, why),
                    resources::ReadError::Backend(why) => (INTERNAL_ERROR, why),
                })?;
                Ok(json!({"contents": [{"uri": uri, "mimeType": "application/json", "text": serde_json::to_string_pretty(&v).unwrap_or_default()}]}))
            }
            "prompts/list" => Ok(prompts::list()),
            "prompts/get" => {
                let name = params.get("name").and_then(Value::as_str).ok_or((INVALID_PARAMS, "missing prompt `name`".to_string()))?;
                let args = params.get("arguments").cloned().unwrap_or(Value::Null);
                prompts::get(name, &args).map_err(|e| (INVALID_PARAMS, e))
            }
            "completion/complete" => Ok(prompts::complete(self.backend.as_mut(), params)),
            "logging/setLevel" => {
                // `level` is required. A null level is our own way of turning logging back off,
                // which the spec has no method for and a long session needs.
                let Some(asked) = params.get("level") else {
                    return Err((INVALID_PARAMS, "missing `level`".into()));
                };
                let level = if asked.is_null() { None } else { asked.as_str() };
                crate::logging::set_level(level).map_err(|_| match level {
                    Some(given) => (INVALID_PARAMS, format!("unknown level `{given}`")),
                    None => (INVALID_PARAMS, "`level` must be a severity name or null".to_string()),
                })?;
                self.logging = level.is_some();
                log::debug!("client set logging to {}", level.unwrap_or("off"));
                Ok(json!({}))
            }
            other => Err((METHOD_NOT_FOUND, format!("method not found: {other}"))),
        }
    }
}
