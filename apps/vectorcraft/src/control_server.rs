//! Loopback JSON-lines control server: one request per line, one reply per line.
//! This is the transport the MCP server (`vectorcraft mcp --connect`) wraps.
//!
//! Every connection starts with `{"method": "auth", "params": {"token": …}}` carrying the
//! per-launch token ([`vectorcraft_engine::control_auth`], #3603): the port is reachable by every
//! process and user on the machine, so without the token, which only the user's own files hold,
//! nothing is run. A missing or wrong token gets one error reply and the connection is closed.
//!
//! The server only ever reads requests: a line that is not a JSON object with a string `method`
//! (an HTTP request line from a browser's cross-origin `fetch`, a stray `nc`, binary junk) gets
//! one error reply and the connection is closed, so nothing after it is executed. Lines are
//! bounded ([`MAX_LINE`]) and so is the number of open connections ([`MAX_CONNECTIONS`]); junk
//! never reaches the UI thread. See `docs/control-protocol.md`.

use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use serde_json::{Value, json};
use vectorcraft_engine::control_auth;
use vectorcraft_ui_egui::ControlRequest;

#[path = "control_deadlines.rs"]
mod deadlines;
use deadlines::{DeadlineReader, DeadlineWriter};

/// Longest accepted request line (bytes, without the newline). Requests are small JSON objects;
/// anything longer is refused and the connection closed.
pub const MAX_LINE: usize = 4 * 1024 * 1024;
/// Connections served at once; further ones get an error line and are closed.
pub const MAX_CONNECTIONS: usize = 16;
/// Size in bytes of the buffer that [`write_line`] writes a reply line through.
const REPLY_CHUNK: usize = 64 * 1024;

/// How long a request waits for the app's reply.
const REPLY_WAIT: Duration = Duration::from_secs(60);

/// The reply to a request the app didn't answer in [`REPLY_WAIT`]. One it had not started is
/// withdrawn, so it never runs after its caller was told it failed; one it is running can't be
/// stopped, and the reply says its outcome isn't known.
fn timed_out(ticket: &vectorcraft_ui_egui::ControlTicket) -> Value {
    let error = if ticket.give_up() {
        "timeout: the app did not start the request in time; it was withdrawn and will not run"
    } else {
        "timeout: the app is still running the request; it may yet complete"
    };
    json!({"ok": false, "error": error})
}

#[derive(Clone, Copy)]
struct ConnectionTimeouts {
    idle: Duration,
    line: Duration,
    write: Duration,
}

impl Default for ConnectionTimeouts {
    fn default() -> Self {
        Self { idle: Duration::from_secs(300), line: Duration::from_secs(30), write: Duration::from_secs(60) }
    }
}

/// The token connections must authenticate with, and where to write it once the port is ours.
pub struct ControlToken {
    pub token: String,
    /// The file to write the token to (a generated one); `None` when the user supplied it.
    pub file: Option<PathBuf>,
}

/// The token for a control server on `port`: the one in `VECTORCRAFT_CONTROL_TOKEN` when set,
/// else a fresh one from the system's random source, to be written next to the settings (or to
/// `VECTORCRAFT_CONTROL_TOKEN_FILE`) for clients to read.
pub fn token_for(port: u16, env_token: Option<std::ffi::OsString>, token_file: Option<PathBuf>) -> Result<ControlToken, String> {
    if let Some(token) = env_token.filter(|v| !v.is_empty()) {
        let env = control_auth::TOKEN_ENV;
        let token = token.into_string().map_err(|_| format!("{env} is not text"))?;
        let token = control_auth::validate_token(&token).map_err(|e| format!("{env}: {e}"))?;
        return Ok(ControlToken { token, file: None });
    }
    let mut bytes = [0u8; control_auth::TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|e| format!("cannot make a control token: {e}"))?;
    let file = token_file.ok_or_else(|| {
        format!("no folder to keep the control token for port {port} in: set {} or {}", control_auth::TOKEN_FILE_ENV, control_auth::TOKEN_ENV)
    })?;
    Ok(ControlToken { token: control_auth::token_from_bytes(&bytes), file: Some(file) })
}

pub fn start(port: u16, token: ControlToken, ctx: egui::Context) -> Receiver<ControlRequest> {
    let (tx, rx) = channel::<ControlRequest>();
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("vectorcraft: control server failed to bind 127.0.0.1:{port}: {e}");
            return rx;
        }
    };
    // Written once the port is ours, so a second app that can't bind it leaves the first one's token.
    let token_note = match &token.file {
        Some(path) => match control_auth::write_token_file(path, &token.token) {
            Ok(()) => format!("token in {}", path.display()),
            Err(e) => {
                // Without the file no client can authenticate: don't leave a port open nobody can use.
                log::error!("control token not written: {e}");
                eprintln!("vectorcraft: control server not started: the control token could not be written ({e})");
                return rx;
            }
        },
        None => format!("token from {}", control_auth::TOKEN_ENV),
    };
    eprintln!("vectorcraft: control server listening on 127.0.0.1:{port} ({token_note})");
    #[cfg(target_os = "macos")]
    keep_answering();
    let token: Arc<str> = token.token.into();
    std::thread::spawn(move || accept_loop(listener, tx, ctx, token));
    rx
}

/// The activity the control channel holds on macOS: work the user asked for, which may let the
/// system sleep when idle, with full timer precision. Without it, App Nap lowers every thread to
/// the lowest priority within about a minute of the window no longer being visible.
#[cfg(target_os = "macos")]
fn activity_options() -> objc2_foundation::NSActivityOptions {
    use objc2_foundation::NSActivityOptions;
    NSActivityOptions::UserInitiatedAllowingIdleSystemSleep | NSActivityOptions::LatencyCritical
}

/// Begins the control channel's activity. It lasts until the process ends.
#[cfg(target_os = "macos")]
fn keep_answering() {
    use objc2_foundation::{NSProcessInfo, NSString};
    let reason = NSString::from_str("Answering control requests");
    let activity = NSProcessInfo::processInfo().beginActivityWithOptions_reason(activity_options(), &reason);
    // The token is never released or passed to `endActivity`, so the activity lasts for the process's lifetime.
    std::mem::forget(activity);
}

fn accept_loop(listener: TcpListener, tx: Sender<ControlRequest>, ctx: egui::Context, token: Arc<str>) {
    let open = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming().flatten() {
        let Some(slot) = acquire_slot(&open) else {
            let mut out = DeadlineWriter::new(stream, ConnectionTimeouts::default().write);
            // Best effort: the connection is dropped either way, and a peer that can't read the reason loses nothing.
            if out.begin_reply().is_ok() {
                let _ = write_line(&mut out, &json!({"ok": false, "error": "too many control connections"}));
            }
            continue;
        };
        if let Err(e) = spawn_connection(stream, tx.clone(), ctx.clone(), Arc::clone(&token), slot, ConnectionTimeouts::default()) {
            eprintln!("vectorcraft: control worker failed to start: {e}");
        }
    }
}

struct ConnectionSlot(Arc<AtomicUsize>);

fn acquire_slot(open: &Arc<AtomicUsize>) -> Option<ConnectionSlot> {
    let mut n = open.load(Ordering::SeqCst);
    loop {
        if n >= MAX_CONNECTIONS {
            return None;
        }
        match open.compare_exchange_weak(n, n + 1, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return Some(ConnectionSlot(Arc::clone(open))),
            Err(current) => n = current,
        }
    }
}

impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        // Each successful acquisition creates exactly one non-cloneable guard.
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn spawn_connection(
    stream: TcpStream,
    tx: Sender<ControlRequest>,
    ctx: egui::Context,
    token: Arc<str>,
    slot: ConnectionSlot,
    timeouts: ConnectionTimeouts,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new().name("vectorcraft-control".into()).spawn(move || {
        let _slot = slot;
        serve_socket(stream, timeouts, &token, |method, params| {
            let (req, rrx) = ControlRequest::new(method, params);
            let ticket = req.ticket();
            tx.send(req).ok()?;
            ctx.request_repaint();
            Some(rrx.recv_timeout(REPLY_WAIT).unwrap_or_else(|_| timed_out(&ticket)))
        });
    })
}

fn serve_socket(stream: TcpStream, timeouts: ConnectionTimeouts, token: &str, handle: impl FnMut(String, Value) -> Option<Value>) {
    let Ok(read) = stream.try_clone() else { return };
    let read = DeadlineReader::new(read, timeouts.idle, timeouts.line);
    // Replies reach the writer in [`REPLY_CHUNK`] pieces ([`write_line`]), so checking the remaining
    // budget costs one socket option call per piece, not one per JSON token.
    let out = DeadlineWriter::new(stream, timeouts.write);
    serve_lines_with(
        read,
        out,
        Some(token),
        handle,
        |reader| {
            let buffered = !reader.buffer().is_empty();
            reader.get_mut().begin_line(buffered)
        },
        DeadlineWriter::begin_reply,
    );
}

/// One parsed line.
enum Line {
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Blank,
    /// Not a request: reply with this error and close the connection.
    Reject(String),
}

fn parse_line(bytes: &[u8]) -> Line {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Line::Reject("not a control request (invalid UTF-8); closing the connection".into());
    };
    let text = text.trim();
    if text.is_empty() {
        return Line::Blank;
    }
    if looks_like_http(text) {
        return Line::Reject("HTTP requests are not accepted: this port reads JSON control requests, one per line; closing the connection".into());
    }
    let msg = match serde_json::from_str::<Value>(text) {
        Ok(v) => v,
        Err(e) => return Line::Reject(format!("bad JSON ({e}); expected one JSON request object per line; closing the connection")),
    };
    let Some(obj) = msg.as_object() else {
        return Line::Reject("not a control request (expected a JSON object); closing the connection".into());
    };
    let Some(method) = obj.get("method").and_then(Value::as_str) else {
        return Line::Reject("not a control request (missing string `method`); closing the connection".into());
    };
    Line::Request {
        id: obj.get("id").cloned().unwrap_or(Value::Null),
        method: method.to_string(),
        params: obj.get("params").cloned().unwrap_or(json!({})),
    }
}

/// Whether `line` is an HTTP request line (`POST / HTTP/1.1`), as a browser's `fetch` or a page
/// rebound to 127.0.0.1 sends: a method of capital letters, a target, and an HTTP version.
fn looks_like_http(line: &str) -> bool {
    let mut words = line.split_ascii_whitespace();
    let (Some(method), Some(_target), Some(version)) = (words.next(), words.next(), words.next()) else { return false };
    !method.is_empty() && method.bytes().all(|b| b.is_ascii_uppercase()) && version.starts_with("HTTP/")
}

/// The refusal of a first request that isn't `auth` with the app's token.
const AUTH_REQUIRED: &str = "authentication required: send {\"method\": \"auth\", \"params\": {\"token\": …}} first, with the token the app wrote to control-<port>.token next to its settings (or the one in VECTORCRAFT_CONTROL_TOKEN); closing the connection";

/// Read one `\n`-terminated line of at most `max` bytes into `buf` (newline stripped).
/// `Ok(false)` at end of stream; `Err` when the line is too long or the read fails.
fn read_bounded_line(reader: &mut impl BufRead, buf: &mut Vec<u8>, max: usize) -> Result<bool, String> {
    buf.clear();
    // `take` bounds the bytes buffered for one line: a peer can't make us allocate more than `max`.
    let limit = u64::try_from(max).unwrap_or(u64::MAX).saturating_add(1);
    let n = reader.take(limit).read_until(b'\n', buf).map_err(|e| match e.kind() {
        // The line's deadline ran out (Windows reports it as TimedOut, Unix as WouldBlock).
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => "request line not completed in time; closing the connection".to_string(),
        _ => e.to_string(),
    })?;
    if n == 0 {
        return Ok(false);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
        if buf.last() == Some(&b'\r') {
            buf.pop();
        }
        return Ok(true);
    }
    if buf.len() > max {
        return Err(format!("request line longer than {max} bytes; closing the connection"));
    }
    // Last line without a newline, then end of stream.
    Ok(true)
}

/// Write `v` and a newline to `out` through a buffer of [`REPLY_CHUNK`] bytes, then flush `out`.
fn write_line(out: &mut impl Write, v: &Value) -> std::io::Result<()> {
    let mut out = BufWriter::with_capacity(REPLY_CHUNK, out);
    writeln!(out, "{v}")?;
    out.flush()
}

/// Serve requests from `read`, answering on `out`, until the peer closes, sends something that is
/// not a request, or `handle` returns `None` (app gone). `handle` gets `(method, params)` and
/// returns the reply object (the request's `id` is added here).
#[cfg(test)]
fn serve_lines<R: Read, W: Write>(read: R, out: W, handle: impl FnMut(String, Value) -> Option<Value>) {
    serve_lines_with(read, out, None, handle, |_| Ok(()), |_| Ok(()));
}

/// As [`serve_lines`], for a connection that must authenticate with `token` first.
#[cfg(test)]
fn serve_lines_authenticated<R: Read, W: Write>(read: R, out: W, token: &str, handle: impl FnMut(String, Value) -> Option<Value>) {
    serve_lines_with(read, out, Some(token), handle, |_| Ok(()), |_| Ok(()));
}

/// `token`: the first request must be `auth` with it, or nothing runs and the connection closes.
/// `None` (tests of what follows authentication) serves requests from the first line.
fn serve_lines_with<R: Read, W: Write>(
    read: R,
    mut out: W,
    token: Option<&str>,
    mut handle: impl FnMut(String, Value) -> Option<Value>,
    mut begin_line: impl FnMut(&mut BufReader<R>) -> std::io::Result<()>,
    mut begin_reply: impl FnMut(&mut W) -> std::io::Result<()>,
) {
    let mut reader = BufReader::new(read);
    let mut buf = Vec::new();
    let mut authenticated = token.is_none();
    loop {
        if begin_line(&mut reader).is_err() {
            return;
        }
        let reject = match read_bounded_line(&mut reader, &mut buf, MAX_LINE) {
            Ok(false) => return,
            // Nothing of a line arrived (the idle wait ran out, or the peer is gone): close without
            // a word. A line nobody asked for would be read as the reply to the client's next request.
            Err(_) if buf.is_empty() => return,
            Err(e) => e,
            Ok(true) => match parse_line(&buf) {
                Line::Blank => continue,
                Line::Reject(e) => e,
                // Nothing runs before the connection proved it holds the token.
                Line::Request { method, params, .. }
                    if !authenticated && !token.is_some_and(|t| control_auth::is_valid_auth(&method, &params, t)) =>
                {
                    AUTH_REQUIRED.to_string()
                }
                Line::Request { id, method, params } => {
                    let reply = if !authenticated {
                        authenticated = true;
                        Some(json!({"ok": true, "result": {"authenticated": true}}))
                    } else if token.is_some() && method == control_auth::AUTH_METHOD {
                        Some(json!({"ok": false, "error": "already authenticated"}))
                    } else {
                        handle(method, params)
                    };
                    let Some(mut r) = reply else { return };
                    if let Some(o) = r.as_object_mut() {
                        o.insert("id".into(), id);
                    }
                    if begin_reply(&mut out).and_then(|()| write_line(&mut out, &r)).is_err() {
                        return;
                    }
                    continue;
                }
            },
        };
        // Best effort: the connection closes either way, and a peer that can't read the reason loses nothing.
        let _ = begin_reply(&mut out).and_then(|()| write_line(&mut out, &json!({"ok": false, "error": reject})));
        return;
    }
}

#[cfg(test)]
mod tests;
