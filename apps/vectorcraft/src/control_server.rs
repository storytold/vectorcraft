//! Loopback JSON-lines control server: one request per line, one reply per line.
//! This is the transport the MCP server (`vectorcraft mcp --connect`) wraps.
//!
//! The server only ever reads requests: a line that is not a JSON object with a string `method`
//! (an HTTP request line from a browser's cross-origin `fetch`, a stray `nc`, binary junk) gets
//! one error reply and the connection is closed, so nothing after it is executed. Lines are
//! bounded ([`MAX_LINE`]) and so is the number of open connections ([`MAX_CONNECTIONS`]); junk
//! never reaches the UI thread. See `docs/control-protocol.md`.

use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use serde_json::{Value, json};
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

pub fn start(port: u16, ctx: egui::Context) -> Receiver<ControlRequest> {
    let (tx, rx) = channel::<ControlRequest>();
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("vectorcraft: control server failed to bind 127.0.0.1:{port}: {e}");
            return rx;
        }
    };
    eprintln!("vectorcraft: control server listening on 127.0.0.1:{port}");
    #[cfg(target_os = "macos")]
    keep_answering();
    std::thread::spawn(move || accept_loop(listener, tx, ctx));
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

fn accept_loop(listener: TcpListener, tx: Sender<ControlRequest>, ctx: egui::Context) {
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
        if let Err(e) = spawn_connection(stream, tx.clone(), ctx.clone(), slot, ConnectionTimeouts::default()) {
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
    slot: ConnectionSlot,
    timeouts: ConnectionTimeouts,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new().name("vectorcraft-control".into()).spawn(move || {
        let _slot = slot;
        serve_socket(stream, timeouts, |method, params| {
            let (req, rrx) = ControlRequest::new(method, params);
            let ticket = req.ticket();
            tx.send(req).ok()?;
            ctx.request_repaint();
            Some(rrx.recv_timeout(REPLY_WAIT).unwrap_or_else(|_| timed_out(&ticket)))
        });
    })
}

fn serve_socket(stream: TcpStream, timeouts: ConnectionTimeouts, handle: impl FnMut(String, Value) -> Option<Value>) {
    let Ok(read) = stream.try_clone() else { return };
    let read = DeadlineReader::new(read, timeouts.idle, timeouts.line);
    // Replies reach the writer in [`REPLY_CHUNK`] pieces ([`write_line`]), so checking the remaining
    // budget costs one socket option call per piece, not one per JSON token.
    let out = DeadlineWriter::new(stream, timeouts.write);
    serve_lines_with(
        read,
        out,
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
    serve_lines_with(read, out, handle, |_| Ok(()), |_| Ok(()));
}

fn serve_lines_with<R: Read, W: Write>(
    read: R,
    mut out: W,
    mut handle: impl FnMut(String, Value) -> Option<Value>,
    mut begin_line: impl FnMut(&mut BufReader<R>) -> std::io::Result<()>,
    mut begin_reply: impl FnMut(&mut W) -> std::io::Result<()>,
) {
    let mut reader = BufReader::new(read);
    let mut buf = Vec::new();
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
                Line::Request { id, method, params } => {
                    let Some(mut r) = handle(method, params) else { return };
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
