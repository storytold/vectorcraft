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
mod tests {
    use super::*;

    fn short_timeouts() -> ConnectionTimeouts {
        ConnectionTimeouts { idle: Duration::from_millis(150), line: Duration::from_millis(120), write: Duration::from_millis(150) }
    }

    fn socket_fixture(
        timeouts: ConnectionTimeouts,
        handle: impl FnMut(String, Value) -> Option<Value> + Send + 'static,
    ) -> (TcpStream, Receiver<()>, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let (finished, done) = channel();
        let worker = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            socket2::SockRef::from(&stream).set_send_buffer_size(4096).unwrap();
            serve_socket(stream, timeouts, handle);
            finished.send(()).unwrap();
        });
        let client = TcpStream::connect(addr).unwrap();
        socket2::SockRef::from(&client).set_recv_buffer_size(4096).unwrap();
        client.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        (client, done, worker)
    }

    fn expect_socket_deadline(client: TcpStream, done: Receiver<()>, worker: std::thread::JoinHandle<()>) {
        let stopped = done.recv_timeout(Duration::from_secs(3)).is_ok();
        // Always unblock the old implementation too, so a failed regression leaves no worker behind.
        let _ = client.shutdown(std::net::Shutdown::Both);
        drop(client);
        worker.join().unwrap();
        assert!(stopped, "the server retained a stalled connection beyond its transport deadline");
    }

    #[test]
    fn idle_control_socket_releases_its_worker() {
        let (client, done, worker) = socket_fixture(short_timeouts(), |_, _| panic!("no request expected"));
        expect_socket_deadline(client, done, worker);
    }

    /// An idle connection is closed without a line: a client that sends its next request late
    /// must not find an error waiting where its reply should be.
    #[test]
    fn idle_expiry_closes_without_writing_a_line() {
        let (mut client, done, worker) = socket_fixture(short_timeouts(), |_, _| panic!("no request expected"));
        let mut got = Vec::new();
        client.read_to_end(&mut got).unwrap();
        assert_eq!(String::from_utf8_lossy(&got), "", "the app wrote to an idle connection");
        expect_socket_deadline(client, done, worker);
    }

    /// A line that stalls is refused in words a client's author can act on, not the system's.
    #[test]
    fn stalled_control_line_is_refused_with_the_reason() {
        let (mut client, done, worker) = socket_fixture(short_timeouts(), |_, _| panic!("incomplete requests must not execute"));
        client.write_all(b"{\"method\":").unwrap();
        let mut line = String::new();
        BufReader::new(client.try_clone().unwrap()).read_line(&mut line).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&line).unwrap(),
            json!({"ok": false, "error": "request line not completed in time; closing the connection"})
        );
        expect_socket_deadline(client, done, worker);
    }

    #[test]
    fn partial_control_line_releases_its_worker() {
        let (mut client, done, worker) = socket_fixture(short_timeouts(), |_, _| panic!("incomplete requests must not execute"));
        client.write_all(b"{\"method\":").unwrap();
        expect_socket_deadline(client, done, worker);
    }

    #[test]
    fn trickled_control_line_has_an_absolute_deadline() {
        let (mut client, done, worker) = socket_fixture(short_timeouts(), |_, _| panic!("incomplete requests must not execute"));
        client.write_all(b"{").unwrap();
        let mut writer = client.try_clone().unwrap();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let writing = Arc::clone(&stop);
        let trickle = std::thread::spawn(move || {
            while !writing.load(Ordering::SeqCst) {
                if writer.write_all(b" ").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        let stopped = done.recv_timeout(Duration::from_secs(3)).is_ok();
        stop.store(true, Ordering::SeqCst);
        // Both socket copies belong to this fixture; shutdown cleans up the baseline failure as well.
        let _ = client.shutdown(std::net::Shutdown::Both);
        trickle.join().unwrap();
        worker.join().unwrap();
        assert!(stopped, "trickled bytes restarted the line's deadline");
    }

    #[test]
    fn a_nonreading_control_peer_cannot_retain_the_writer() {
        let payload = "x".repeat(2 * 1024 * 1024);
        let (mut client, done, worker) = socket_fixture(short_timeouts(), move |_, _| Some(json!({"ok": true, "result": payload})));
        client.write_all(b"{\"method\":\"large\"}\n").unwrap();
        expect_socket_deadline(client, done, worker);
    }

    #[test]
    fn persistent_control_requests_get_fresh_transport_deadlines() {
        let timeouts = ConnectionTimeouts { idle: Duration::from_secs(2), ..short_timeouts() };
        let (mut client, done, worker) = socket_fixture(timeouts, |_, _| {
            // Application processing is not transport time; begin the reply budget afterwards.
            std::thread::sleep(Duration::from_millis(200));
            Some(json!({"ok": true}))
        });
        let mut reader = BufReader::new(client.try_clone().unwrap());
        for id in [1, 2] {
            // The client sends each line in one write. `writeln!` on the socket sends the JSON in many
            // small writes, and when the machine is busy, a pause of more than the 120 ms line deadline
            // between two of them closes the connection.
            client.write_all(format!("{}\n", json!({"id": id, "method": "inspect"})).as_bytes()).unwrap();
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert_eq!(serde_json::from_str::<Value>(&line).unwrap(), json!({"id": id, "ok": true}));
        }
        client.shutdown(std::net::Shutdown::Write).unwrap();
        done.recv_timeout(Duration::from_secs(3)).unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn buffered_partial_control_line_also_expires() {
        let (mut client, done, worker) = socket_fixture(short_timeouts(), |_, _| Some(json!({"ok": true})));
        client.write_all(b"{\"id\":1,\"method\":\"inspect\"}\n{").unwrap();
        let mut line = String::new();
        BufReader::new(client.try_clone().unwrap()).read_line(&mut line).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["id"], 1);
        expect_socket_deadline(client, done, worker);
    }

    #[test]
    fn timed_out_worker_returns_its_connection_slot() {
        let open = Arc::new(AtomicUsize::new(0));
        let held: Vec<_> = (0..MAX_CONNECTIONS - 1).map(|_| acquire_slot(&open).unwrap()).collect();
        let slot = acquire_slot(&open).unwrap();
        assert!(acquire_slot(&open).is_none());
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        let (tx, _rx) = channel();
        let worker = spawn_connection(stream, tx, egui::Context::default(), slot, short_timeouts()).unwrap();
        let start = std::time::Instant::now();
        while open.load(Ordering::SeqCst) == MAX_CONNECTIONS && start.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(10));
        }
        let released = open.load(Ordering::SeqCst) < MAX_CONNECTIONS;
        // Clean up an old implementation that never returned its slot as well.
        let _ = client.shutdown(std::net::Shutdown::Both);
        worker.join().unwrap();
        assert!(released, "the deadline did not free capacity for the next client");
        let replacement = acquire_slot(&open).expect("a new client can reserve the freed slot");
        drop(replacement);
        drop(held);
        assert_eq!(open.load(Ordering::SeqCst), 0);
    }

    fn run(input: &[u8]) -> (Vec<Value>, Vec<String>) {
        let mut out = Vec::new();
        let mut calls = Vec::new();
        serve_lines(input, &mut out, |m, _p| {
            calls.push(m);
            Some(json!({"ok": true, "result": null}))
        });
        let replies = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str::<Value>(l).unwrap()).collect();
        (replies, calls)
    }

    /// Counts the `write` calls made on it and keeps the bytes they write.
    #[derive(Default)]
    struct Writes {
        calls: usize,
        bytes: Vec<u8>,
    }

    impl Write for Writes {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.calls += 1;
            self.bytes.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// A reply shaped like `document.inspect`'s for a layer of `n` squares in rows of 200.
    fn inspect_reply(n: u64) -> Value {
        let paths: Vec<Value> = (0..n)
            .map(|k| {
                let (x, y) = ((k % 200) as f64 * 20.0, (k / 200) as f64 * 20.0);
                json!({"anchors": 4, "bounds": {"height": 10.0, "width": 10.0, "x": x, "y": y}, "closed": true, "fill": "#f4a261",
                    "id": k + 2, "kind": "Path", "locked": false, "name": "<Path>", "stroke": "None", "strokeWidth": 0.0, "visible": true})
            })
            .collect();
        json!({"ok": true, "result": {"layers": [{"children": paths, "id": 1, "kind": "Layer", "name": "Layer 1"}]}})
    }

    /// #1725: a request its caller gave up before the app took it never runs; one the app took
    /// first can't be withdrawn, and the timeout says so.
    #[test]
    fn a_timed_out_request_is_withdrawn_unless_it_started() {
        let (waiting, _rx) = ControlRequest::new("engine.execute", json!({}));
        assert!(timed_out(&waiting.ticket())["error"].as_str().unwrap().contains("will not run"));
        assert!(!waiting.claim(), "the app skips a withdrawn request");
        let (running, _rx) = ControlRequest::new("engine.execute", json!({}));
        assert!(running.claim());
        assert!(timed_out(&running.ticket())["error"].as_str().unwrap().contains("still running"));
        assert!(!running.claim(), "a request is taken once");
    }

    #[test]
    fn requests_are_answered_in_order_with_ids() {
        let (r, calls) = run(b"{\"id\":1,\"method\":\"a\"}\n\n{\"id\":2,\"method\":\"b\",\"params\":{}}\r\n{\"method\":\"c\"}");
        assert_eq!(calls, ["a", "b", "c"]);
        assert_eq!(r.len(), 3);
        assert_eq!(r[0]["id"], 1);
        assert_eq!(r[1]["id"], 2);
        assert_eq!(r[2]["id"], Value::Null);
    }

    #[test]
    fn replies_are_written_in_few_large_writes() {
        for reply in [json!({"ok": true, "result": null}), inspect_reply(2_000)] {
            let mut out = Writes::default();
            let mut once = Some(reply.clone());
            serve_lines(&b"{\"id\":1,\"method\":\"document.inspect\"}\n"[..], &mut out, |_, _| once.take());
            let mut want = reply;
            want["id"] = json!(1);
            let text = String::from_utf8(out.bytes).unwrap();
            // The writer receives the bytes that `writeln!(out, "{want}")` writes, in at most one write per 4 KiB.
            assert_eq!(text, format!("{want}\n"));
            assert!(out.calls <= text.len().div_ceil(4096), "{} writes for {} bytes", out.calls, text.len());
        }
        // A refusal is one write as well.
        let mut out = Writes::default();
        serve_lines(&b"hello\n"[..], &mut out, |_, _| None);
        assert_eq!(out.calls, 1);
    }

    #[test]
    fn http_request_closes_before_the_body_runs() {
        // A browser's cross-origin `fetch` POST: the request line is rejected and the
        // connection closed, so the JSON body is never executed.
        let http = b"POST / HTTP/1.1\r\nHost: 127.0.0.1:7979\r\nOrigin: https://attacker.example\r\nContent-Type: text/plain\r\n\r\n\
            {\"id\":7,\"method\":\"engine.execute\",\"params\":{\"command\":\"file.new\",\"params\":{}}}\n";
        let (r, calls) = run(http);
        assert!(calls.is_empty(), "nothing may run: {calls:?}");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0]["ok"], false);
        assert!(r[0]["error"].as_str().unwrap().contains("closing the connection"));
    }

    #[test]
    fn non_request_json_closes_the_connection() {
        for junk in [
            &b"[1,2]\n{\"method\":\"a\"}\n"[..],
            b"{\"id\":1}\n{\"method\":\"a\"}\n",
            b"{\"method\":5}\n{\"method\":\"a\"}\n",
            b"\xff\xfe\n{\"method\":\"a\"}\n",
        ] {
            let (r, calls) = run(junk);
            assert!(calls.is_empty(), "{calls:?}");
            assert_eq!(r.len(), 1);
            assert_eq!(r[0]["ok"], false);
        }
    }

    #[test]
    fn a_request_before_junk_still_runs() {
        let (r, calls) = run(b"{\"method\":\"a\"}\nGET / HTTP/1.1\n{\"method\":\"b\"}\n");
        assert_eq!(calls, ["a"]);
        assert_eq!(r.len(), 2);
        assert_eq!(r[1]["ok"], false);
    }

    #[test]
    fn overlong_line_is_refused() {
        // One byte over the limit, then a valid request: neither runs.
        let mut input = vec![b'x'; MAX_LINE + 1];
        input.extend_from_slice(b"\n{\"method\":\"a\"}\n");
        let (r, calls) = run(&input);
        assert!(calls.is_empty(), "{calls:?}");
        assert_eq!(r.len(), 1);
        assert!(r[0]["error"].as_str().unwrap().contains("longer than"));
    }

    #[test]
    fn line_at_the_limit_is_accepted() {
        let mut buf = Vec::new();
        let line = vec![b'a'; 10];
        let mut input = line.clone();
        input.push(b'\n');
        assert_eq!(read_bounded_line(&mut &input[..], &mut buf, 10), Ok(true));
        assert_eq!(buf, line);
        assert!(read_bounded_line(&mut &[b'a'; 11][..], &mut buf, 10).is_err());
    }

    #[test]
    fn tcp_connection_is_closed_after_junk() {
        // End to end over a real loopback socket (the transport the app uses).
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            let read = s.try_clone().unwrap();
            serve_lines(read, s, |_, _| Some(json!({"ok": true})));
        });
        let mut c = TcpStream::connect(addr).unwrap();
        c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        c.write_all(b"{\"id\":7,\"method\":\"x\"}\nhello\n{\"method\":\"y\"}\n").unwrap();
        let mut text = String::new();
        c.read_to_string(&mut text).unwrap(); // returns at EOF: the server closed the connection
        server.join().unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "{text}");
        assert!(lines[0].contains("\"id\":7"));
        assert!(lines[1].contains("closing the connection"));
    }

    #[test]
    fn ten_inspect_replies_for_10000_objects_arrive_within_20_s() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let reply = inspect_reply(10_000);
        let mut want = reply.clone();
        want["id"] = json!(1);
        let want = format!("{want}\n");
        std::thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            let read = s.try_clone().unwrap();
            serve_lines(read, s, |_, _| Some(reply.clone()));
        });
        // The client sends each request line and reads its reply line through a `BufReader`, as `vectorcraft_mcp::Remote` does.
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut c = TcpStream::connect(addr).unwrap();
            let mut read = BufReader::new(c.try_clone().unwrap());
            let mut lines = vec![];
            for _ in 0..10 {
                c.write_all(b"{\"id\":1,\"method\":\"document.inspect\"}\n").unwrap();
                let mut line = String::new();
                read.read_line(&mut line).unwrap();
                lines.push(line);
            }
            tx.send(lines).ok();
        });
        let lines = rx.recv_timeout(Duration::from_secs(20)).unwrap_or_else(|_| panic!("ten replies did not arrive within 20 s"));
        assert!(lines.iter().all(|l| *l == want), "a reply differs from the bytes `writeln!` writes");
    }

    #[test]
    fn extra_connections_are_refused() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, _rx) = channel::<ControlRequest>();
        std::thread::spawn(move || accept_loop(listener, tx, egui::Context::default()));
        // Idle connections are accepted in order and held open.
        let held: Vec<TcpStream> = (0..16).map(|_| TcpStream::connect(addr).unwrap()).collect();
        let mut extra = TcpStream::connect(addr).unwrap();
        extra.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut line = String::new();
        BufReader::new(&mut extra).read_line(&mut line).unwrap();
        assert!(line.contains("too many control connections"), "{line}");
        drop(held);
    }

    // A test process is not an app that App Nap applies to, so this checks the activity the
    // channel asks for; the throttling itself shows only in a running app with its window hidden.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_control_channel_keeps_the_app_at_full_priority_on_macos() {
        use objc2_foundation::NSActivityOptions;
        let options = activity_options();
        assert!(options.contains(NSActivityOptions::LatencyCritical), "without it App Nap still lowers the priority to 20");
        assert!(options.contains(NSActivityOptions::UserInitiatedAllowingIdleSystemSleep));
        assert!(!options.contains(NSActivityOptions::IdleSystemSleepDisabled), "the channel must not keep the Mac awake");
        keep_answering();
    }
}
