//! JSON-RPC 2.0 framing and the MCP lifecycle / tools / resources / prompts methods.

use std::cell::Cell;
use std::io::{BufRead, Write};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use vectorcraft_engine::cancel;

use crate::backend::Backend;
use crate::intake::{Job, Message, Shared, read_loop};
use crate::prompts;
use crate::resources;
use crate::tools::{call_tool, tool_definitions};

/// The MCP revision we implement.
pub const PROTOCOL_VERSION: &str = "2025-06-18";
const MODERN_VERSION: &str = "2026-07-28";
const SUPPORTED_VERSIONS: &[&str] = &[MODERN_VERSION, PROTOCOL_VERSION, "2025-03-26", "2024-11-05"];

pub(crate) const PARSE_ERROR: i64 = -32700;
pub(crate) const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;
const RESOURCE_NOT_FOUND: i64 = -32002;
/// The engine is running a long call and this request was not run (implementation-defined range).
pub(crate) const SERVER_BUSY: i64 = -32003;

/// How long a call runs before a request that would have to wait behind it is refused as busy
/// instead ([`Server::with_busy_after`]).
const DEFAULT_BUSY_AFTER: Duration = Duration::from_secs(2);

/// Longest accepted request line (bytes, without the newline). Requests are small JSON objects;
/// anything longer is answered with an error and skipped, so one huge line can't make the server
/// allocate without bound (#1727).
pub(crate) const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;
/// Most messages one batch may hold. Batches were removed from the protocol; this only bounds what
/// older clients can make the server parse and answer at once (#1727).
pub(crate) const MAX_BATCH_MESSAGES: usize = 64;

/// Lines waiting to be written. A full queue holds up the thread that wants to write (the
/// engine thread, the reader) rather than growing; progress is dropped instead (it is advisory).
const OUTBOX: usize = 256;
/// Fewest time between two `notifications/progress` of one request, the last one excepted.
const PROGRESS_EVERY: Duration = Duration::from_millis(200);

/// The sink that turns the engine's progress into `notifications/progress` for the request
/// `msg`, when the client asked for it with `_meta.progressToken`.
/// <https://modelcontextprotocol.io/specification/2025-06-18/basic/utilities/progress>
fn progress_sink(out: SyncSender<String>, msg: &Value) -> Option<cancel::ProgressSink> {
    let token = msg.get("params")?.get("_meta")?.get("progressToken").filter(|t| t.is_string() || t.is_number())?.clone();
    let last = Cell::new(None::<u64>);
    let at = Cell::new(None::<Instant>);
    Some(Rc::new(move |done, total, what| {
        // `progress` must increase with each notification.
        if last.get().is_some_and(|l| done <= l) {
            return;
        }
        let finished = total > 0 && done >= total;
        if !finished && at.get().is_some_and(|t| t.elapsed() < PROGRESS_EVERY) {
            return;
        }
        last.set(Some(done));
        at.set(Some(Instant::now()));
        let mut params = json!({"progressToken": token, "progress": done, "message": what});
        if total > 0 {
            params["total"] = json!(total);
        }
        let line = json!({"jsonrpc": "2.0", "method": "notifications/progress", "params": params}).to_string();
        // Advisory: a client that doesn't read fast enough misses an update, not the reply.
        let _ = out.try_send(line);
    }))
}

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
    /// Whether the client speaks the `2026-07-28` revision. Shared with the reader thread, which
    /// answers some requests itself while the engine is busy.
    modern: Arc<AtomicBool>,
    /// Whether this server's client asked for log records (`logging/setLevel`). The queue is
    /// process-wide, so a server whose client didn't ask leaves it to the one that did.
    logging: bool,
    busy_after: Duration,
}

pub(crate) fn response(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

pub(crate) fn error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

pub(crate) fn error_with(id: Value, code: i64, message: impl Into<String>, data: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into(), "data": data}})
}

/// The requests that need neither the engine nor the server's state beyond the protocol revision,
/// so the reader can answer them while the engine is busy.
pub(crate) fn stateless_request(method: &str, params: &Value) -> Option<Result<Value, (i64, String)>> {
    Some(match method {
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tool_definitions()})),
        "resources/list" => Ok(resources::list()),
        "resources/templates/list" => Ok(resources::templates()),
        "prompts/list" => Ok(prompts::list()),
        "prompts/get" => {
            let name = params.get("name").and_then(Value::as_str);
            let args = params.get("arguments").cloned().unwrap_or(Value::Null);
            match name {
                Some(name) => prompts::get(name, &args).map_err(|e| (INVALID_PARAMS, e)),
                None => Err((INVALID_PARAMS, "missing prompt `name`".to_string())),
            }
        }
        _ => return None,
    })
}

/// The reply to `msg` when it is a request [`stateless_request`] answers.
pub(crate) fn stateless_reply(msg: &Value, modern: bool) -> Option<Value> {
    let o = msg.as_object()?;
    let method = o.get("method")?.as_str()?;
    let id = o.get("id").filter(|id| id.is_string() || id.is_number())?.clone();
    if o.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return None;
    }
    let params = o.get("params").cloned().unwrap_or(Value::Null);
    // A panic here is a bug: leave the request to the engine thread, which reports it.
    let r = vectorcraft_engine::guard::catch_panic(|| stateless_request(method, &params)).ok()??;
    Some(match r {
        Ok(r) => response(id, complete_result(method, &params, modern, r)),
        Err((code, m)) => error(id, code, m),
    })
}

/// The cache hints the `2026-07-28` revision adds to list and read results. `modern` is whether the
/// session negotiated that revision; a request can also name it in its `_meta`.
pub(crate) fn complete_result(method: &str, params: &Value, modern: bool, mut r: Value) -> Value {
    let modern = params
        .get("_meta")
        .and_then(|m| m.get("io.modelcontextprotocol/protocolVersion"))
        .and_then(Value::as_str)
        .map_or(modern, |v| v == MODERN_VERSION);
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
    r
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
            // Exactly `max` content bytes plus the `\r` of a CRLF ending fill the bound; the line
            // is still within the limit when the `\n` follows.
            if buf.last() == Some(&b'\r') && reader.fill_buf()?.first() == Some(&b'\n') {
                reader.consume(1);
                buf.pop();
                return Ok(true);
            }
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
        Self { backend, initialized: false, modern: Arc::new(AtomicBool::new(false)), logging: false, busy_after: DEFAULT_BUSY_AFTER }
    }

    /// How long a call runs before [`Server::serve`] turns away a request that would have to wait
    /// behind it, with an error naming the call (default two seconds). Shorter waits queue, so a
    /// client pipelining quick calls is answered in order.
    pub fn with_busy_after(mut self, after: Duration) -> Self {
        self.busy_after = after;
        self
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
    ///
    /// A reader thread takes lines off `input` while the calling thread runs them, so the server
    /// hears a client during a long call: `notifications/cancelled` stops a queued or running
    /// request (which then gets no reply), and `ping` and the other engine-free requests are
    /// answered at once. A request that would have to wait behind a call that has run for
    /// [`Server::with_busy_after`] is refused with a "busy" error. Calls still run one at a time,
    /// in arrival order, and replies of quick calls keep that order. See the module docs of
    /// `intake` and `docs/mcp.md`.
    ///
    /// Returns when `input` ends and everything read has been answered, or when `output` fails
    /// (the running call is then cancelled and the queue dropped; the reader thread ends with its
    /// next line, which a client that closed our output closes as well).
    pub fn serve<R: BufRead + Send, W: Write + Send>(&mut self, input: R, mut output: W) -> std::io::Result<()> {
        let shared = Shared::new(self.modern.clone(), self.busy_after);
        let shared = &shared;
        let (tx, rx) = sync_channel::<String>(OUTBOX);
        let reader_tx = tx.clone();
        std::thread::scope(|scope| {
            // The only thread that writes: replies, progress and log records stay whole lines.
            let writer = scope.spawn(move || {
                let mut failed = None;
                for line in rx {
                    if failed.is_none()
                        && let Err(e) = write_reply(&mut output, &line)
                    {
                        failed = Some(e);
                        shared.close();
                    }
                }
                failed
            });
            let reader = scope.spawn(move || read_loop(shared, &reader_tx, input));
            self.work(shared, &tx);
            drop(tx);
            let read = reader.join().unwrap_or_else(|_| Err(std::io::Error::other("the MCP reader thread panicked")));
            let written = writer.join().unwrap_or_else(|_| Some(std::io::Error::other("the MCP writer thread panicked")));
            match written {
                Some(e) => Err(e),
                None => read,
            }
        })
    }

    /// Run the queued jobs, one at a time, until the reader is done and the queue empty.
    fn work(&mut self, shared: &Shared, out: &SyncSender<String>) {
        while let Some(job) = shared.next() {
            match job {
                Job::Canned(line) => {
                    let _ = out.send(line);
                }
                Job::Message(m) => self.run(m, out),
            }
            for note in self.take_notifications() {
                let _ = out.send(note);
            }
            shared.finished();
        }
    }

    fn run(&mut self, m: Message, out: &SyncSender<String>) {
        let Message { value, key, token, .. } = m;
        let sink = key.is_some().then(|| progress_sink(out.clone(), &value)).flatten();
        let reply = cancel::scope(&token, sink, || self.handle_parsed(value));
        // A cancelled request gets no reply (MCP: the receiver should not answer it). A call that
        // finished before it noticed is dropped the same way: the client has stopped waiting.
        if key.is_some() && token.is_cancelled() {
            return;
        }
        if let Some(reply) = reply {
            let _ = out.send(reply.to_string());
        }
    }

    /// Handle one line; returns the reply line (None for notifications and blank lines).
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        let reply = match serde_json::from_str::<Value>(line) {
            Ok(value) => self.handle_parsed(value),
            Err(e) => Some(error(Value::Null, PARSE_ERROR, format!("parse error: {e}"))),
        };
        reply.map(|r| r.to_string())
    }

    /// Handle one parsed line: a message, or a batch of them.
    fn handle_parsed(&mut self, value: Value) -> Option<Value> {
        match value {
            Value::Array(batch) => {
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
            msg => self.handle(msg),
        }
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
            Ok(r) => response(id, complete_result(method, &params, self.modern.load(Ordering::Relaxed), r)),
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
        if let Some(r) = stateless_request(method, params) {
            return r;
        }
        match method {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL_VERSION);
                let version = if SUPPORTED_VERSIONS.contains(&asked) { asked } else { PROTOCOL_VERSION };
                self.modern.store(version == MODERN_VERSION, Ordering::Relaxed);
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
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).ok_or((INVALID_PARAMS, "missing tool `name`".to_string()))?;
                let args = params.get("arguments").cloned().unwrap_or(Value::Null);
                if let Some(arguments) = args.as_object() {
                    crate::tools::check_tool_arguments(name, arguments).map_err(|e| (INVALID_PARAMS, e))?;
                }
                Ok(call_tool(self.backend.as_mut(), name, &args).to_value())
            }
            "resources/read" => {
                let uri = params.get("uri").and_then(Value::as_str).ok_or((INVALID_PARAMS, "missing `uri`".to_string()))?;
                let v = resources::read(self.backend.as_mut(), uri).map_err(|e| match e {
                    resources::ReadError::NotFound(uri) => (RESOURCE_NOT_FOUND, format!("resource not found: {uri}")),
                    resources::ReadError::Invalid(why) => (INVALID_PARAMS, why),
                    resources::ReadError::Backend(why) => (INTERNAL_ERROR, why),
                })?;
                Ok(json!({"contents": [{"uri": uri, "mimeType": "application/json", "text": serde_json::to_string_pretty(&v).unwrap_or_default()}]}))
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

#[cfg(test)]
mod line_reader_tests {
    use super::read_line_capped;
    use std::io::Cursor;

    #[test]
    fn crlf_line_of_exactly_max_bytes_is_within_the_limit() {
        for ending in ["\n", "\r\n"] {
            let mut input = Cursor::new(format!("{}{ending}next\n", "a".repeat(10)).into_bytes());
            let mut buf = Vec::new();
            assert!(read_line_capped(&mut input, &mut buf, 10).unwrap());
            assert_eq!(buf, b"aaaaaaaaaa", "ending {ending:?}");
            assert!(read_line_capped(&mut input, &mut buf, 10).unwrap());
            assert_eq!(buf, b"next");
        }
    }

    #[test]
    fn crlf_line_over_max_is_still_overlong() {
        let mut input = Cursor::new(format!("{}\r\n", "a".repeat(11)).into_bytes());
        let mut buf = Vec::new();
        assert!(read_line_capped(&mut input, &mut buf, 10).unwrap());
        assert!(buf.len() > 10);
    }
}
