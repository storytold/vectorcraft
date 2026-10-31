//! Reading while a call runs: `ping`, cancellation and busy answers reach the client while a long
//! command holds the engine (#2165, #2171). Driven through [`Server::serve`] on a pipe, the way a
//! client on stdio drives it, with a backend that works until it is told to stop.

use std::io::{BufReader, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use vectorcraft_engine::cancel;

use crate::{Backend, Server};

/// A backend whose `engine.execute` runs until cancelled (or for `limit`), like a long export.
struct Slow {
    limit: Duration,
    /// Commands that ran to the end, and the ones that were cancelled.
    log: Arc<Mutex<Vec<String>>>,
    /// Told when a command starts.
    started: Sender<String>,
}

impl Backend for Slow {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        if method != "engine.execute" {
            return Ok(json!({"method": method}));
        }
        let id = params.get("command").and_then(Value::as_str).unwrap_or("?").to_string();
        self.started.send(id.clone()).ok();
        if id == "progress.demo" {
            for page in 1..=3 {
                // Enough apart to pass the sender's throttle.
                std::thread::sleep(Duration::from_millis(230));
                cancel::progress(page, 3, &format!("page {page}"));
            }
            return Ok(json!({"ran": id}));
        }
        let limit = if id.starts_with("quick") { Duration::ZERO } else { self.limit };
        let start = Instant::now();
        while start.elapsed() < limit {
            if let Err(e) = cancel::check() {
                self.log.lock().unwrap().push(format!("cancelled {id}"));
                return Err(e.to_string());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        self.log.lock().unwrap().push(format!("done {id}"));
        Ok(json!({"ran": id}))
    }

    fn has_ui(&self) -> bool {
        false
    }

    fn describe(&self) -> String {
        "slow".into()
    }
}

/// Collects the lines the server writes.
struct Lines(Sender<String>, Vec<u8>);

impl Write for Lines {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        for &b in buf {
            if b == b'\n' {
                self.0.send(String::from_utf8_lossy(&std::mem::take(&mut self.1)).into_owned()).ok();
            } else {
                self.1.push(b);
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A running server with a pipe in and a channel out.
struct Rig {
    to_server: std::io::PipeWriter,
    replies: Receiver<String>,
    started: Receiver<String>,
    log: Arc<Mutex<Vec<String>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Rig {
    fn new(limit: Duration, busy_after: Duration) -> Self {
        let (read, to_server) = std::io::pipe().unwrap();
        let (tx, replies) = mpsc::channel();
        let (started_tx, started) = mpsc::channel();
        let log = Arc::new(Mutex::new(vec![]));
        let backend = Slow { limit, log: log.clone(), started: started_tx };
        let thread = std::thread::spawn(move || {
            let mut server = Server::new(Box::new(backend)).with_busy_after(busy_after);
            server.serve(BufReader::new(read), Lines(tx, vec![])).unwrap();
        });
        Self { to_server, replies, started, log, thread: Some(thread) }
    }

    fn send(&mut self, msg: Value) {
        writeln!(self.to_server, "{msg}").unwrap();
    }

    fn request(&mut self, id: u64, method: &str, params: Value) {
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
    }

    fn run(&mut self, id: u64, command: &str) {
        self.request(id, "tools/call", json!({"name": "command_run", "arguments": {"id": command}}));
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    fn next(&self, within: Duration) -> Option<Value> {
        match self.replies.recv_timeout(within) {
            Ok(l) => serde_json::from_str(&l).ok(),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => None,
        }
    }

    /// The next reply that answers request `id`, skipping notifications.
    fn reply(&self, id: u64, within: Duration) -> Option<Value> {
        let end = Instant::now() + within;
        while let Some(left) = end.checked_duration_since(Instant::now()) {
            let v = self.next(left)?;
            if v.get("id") == Some(&json!(id)) {
                return Some(v);
            }
        }
        None
    }

    fn finish(mut self) {
        let thread = self.thread.take().unwrap();
        drop(self.to_server);
        thread.join().unwrap();
    }
}

/// How long a test waits for the server to start a command, to answer or to log. The wait only ends
/// a test whose server never does. A machine busy with other builds can hold a thread for seconds.
const WAIT: Duration = Duration::from_secs(20);
/// How long [`Slow`]'s long command runs unless it is cancelled: longer than [`WAIT`], so a request
/// that waits behind it is not answered within the wait.
const LONG: Duration = Duration::from_secs(60);

#[test]
fn ping_is_answered_while_a_command_runs() {
    let mut rig = Rig::new(LONG, Duration::from_millis(50));
    rig.run(1, "document.export");
    assert_eq!(rig.started.recv_timeout(WAIT).unwrap(), "document.export");
    // Past `busy_after` the call counts as long-running.
    std::thread::sleep(Duration::from_millis(120));
    rig.request(2, "ping", json!({}));
    let pong = rig.reply(2, WAIT).expect("ping must not wait for the running command");
    assert_eq!(pong["result"], json!({}));
    rig.notify("notifications/cancelled", json!({"requestId": 1}));
    rig.finish();
}

#[test]
fn cancelling_stops_the_running_command_and_sends_no_reply() {
    let mut rig = Rig::new(LONG, Duration::from_secs(60));
    rig.run(1, "document.export");
    rig.started.recv_timeout(WAIT).unwrap();
    rig.notify("notifications/cancelled", json!({"requestId": 1, "reason": "user"}));
    // The engine is free again: the next command runs and answers.
    rig.run(2, "pdf.preset.list");
    let end = Instant::now() + WAIT;
    loop {
        if rig.log.lock().unwrap().iter().any(|l| l == "cancelled document.export") {
            break;
        }
        assert!(Instant::now() < end, "the export kept running: {:?}", rig.log.lock().unwrap());
        std::thread::sleep(Duration::from_millis(10));
    }
    rig.notify("notifications/cancelled", json!({"requestId": 2}));
    // Request 1 was cancelled: nothing answers it (MCP: the receiver should not reply).
    assert!(rig.reply(1, Duration::from_millis(300)).is_none());
    rig.finish();
}

#[test]
fn a_request_that_would_wait_behind_a_long_call_is_refused_as_busy() {
    let mut rig = Rig::new(LONG, Duration::from_millis(50));
    rig.run(1, "document.export");
    rig.started.recv_timeout(WAIT).unwrap();
    std::thread::sleep(Duration::from_millis(120));
    rig.run(2, "pdf.preset.list");
    let busy = rig.reply(2, WAIT).expect("a busy answer, not a wait");
    assert_eq!(busy["error"]["code"], -32003, "{busy}");
    assert!(busy["error"]["message"].as_str().unwrap().contains("document.export"), "{busy}");
    assert_eq!(busy["error"]["data"]["busy"]["requestId"], "1", "{busy}");
    rig.notify("notifications/cancelled", json!({"requestId": 1}));
    // The refused request never ran, and the engine is free once the export stops.
    rig.run(3, "pdf.preset.list");
    rig.notify("notifications/cancelled", json!({"requestId": 3}));
    rig.finish();
}

#[test]
fn quick_calls_pipelined_together_queue_and_answer_in_order() {
    let mut rig = Rig::new(Duration::from_millis(30), Duration::from_secs(60));
    for id in 1..=4 {
        rig.run(id, &format!("cmd{id}"));
    }
    for id in 1..=4 {
        let r = rig.reply(id, WAIT).unwrap_or_else(|| panic!("no reply to {id}"));
        assert!(r.get("error").is_none(), "{r}");
    }
    assert_eq!(*rig.log.lock().unwrap(), ["done cmd1", "done cmd2", "done cmd3", "done cmd4"]);
    rig.finish();
}

#[test]
fn a_queued_request_that_is_cancelled_never_runs() {
    let mut rig = Rig::new(LONG, Duration::from_secs(60));
    rig.run(1, "document.export");
    rig.started.recv_timeout(WAIT).unwrap();
    rig.run(2, "never.runs");
    rig.run(3, "quick.last");
    // Request 2 may not have been read yet when the cancel is: it is the order of lines that counts.
    rig.notify("notifications/cancelled", json!({"requestId": 2}));
    rig.notify("notifications/cancelled", json!({"requestId": 1}));
    let r = rig.reply(3, WAIT).expect("the engine goes on to the next request");
    assert!(r.get("error").is_none(), "{r}");
    rig.notify("notifications/cancelled", json!({"requestId": 3}));
    let log = rig.log.lock().unwrap().clone();
    assert!(log.iter().all(|l| !l.contains("never.runs")), "{log:?}");
    assert!(log.iter().any(|l| l == "cancelled document.export"), "{log:?}");
    drop(log);
    rig.finish();
}

#[test]
fn cancelling_an_unknown_or_finished_request_changes_nothing() {
    let mut rig = Rig::new(Duration::from_millis(10), Duration::from_secs(60));
    rig.run(1, "quick");
    assert!(rig.reply(1, WAIT).unwrap().get("result").is_some());
    rig.notify("notifications/cancelled", json!({"requestId": 1}));
    rig.notify("notifications/cancelled", json!({"requestId": 99}));
    rig.notify("notifications/cancelled", json!({"requestId": [1], "reason": "junk"}));
    rig.notify("notifications/cancelled", json!("junk"));
    rig.run(2, "quick");
    assert!(rig.reply(2, WAIT).unwrap().get("result").is_some());
    rig.finish();
}

#[test]
fn initialize_cannot_be_cancelled() {
    let mut rig = Rig::new(LONG, Duration::from_secs(60));
    rig.request(1, "initialize", json!({}));
    rig.notify("notifications/cancelled", json!({"requestId": 1}));
    assert!(rig.reply(1, WAIT).unwrap().get("result").is_some());
    rig.finish();
}

#[test]
fn progress_reaches_a_client_that_gave_a_token_before_the_reply() {
    let mut rig = Rig::new(LONG, Duration::from_secs(60));
    rig.request(1, "tools/call", json!({"name": "command_run", "arguments": {"id": "progress.demo"}, "_meta": {"progressToken": "tok"}}));
    let mut seen = vec![];
    let reply = loop {
        let v = rig.next(WAIT).expect("a reply");
        if v["method"] == "notifications/progress" {
            seen.push(v["params"].clone());
        } else {
            break v;
        }
    };
    assert!(reply.get("result").is_some(), "{reply}");
    assert_eq!(seen.len(), 3, "{seen:?}");
    let values: Vec<u64> = seen.iter().map(|p| p["progress"].as_u64().unwrap()).collect();
    assert_eq!(values, [1, 2, 3]);
    assert!(seen.iter().all(|p| p["progressToken"] == "tok" && p["total"] == 3), "{seen:?}");
    assert_eq!(seen[1]["message"], "page 2");
    // Without a token nothing is sent.
    rig.run(2, "progress.demo");
    let next = rig.next(WAIT).unwrap();
    assert_eq!(next["id"], 2, "{next}");
    rig.finish();
}

/// The real engine behind the server: waits (like a slow render would) until the call is
/// cancelled, then lets the export run into the cancelled token.
struct Gated {
    inner: crate::Headless,
    started: Sender<()>,
}

impl Backend for Gated {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        if method == "engine.execute" && params["command"] == "document.export" {
            self.started.send(()).ok();
            let end = Instant::now() + LONG;
            while !cancel::is_cancelled() && Instant::now() < end {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        self.inner.call(method, params)
    }

    fn has_ui(&self) -> bool {
        false
    }

    fn describe(&self) -> String {
        "gated".into()
    }
}

#[test]
fn a_cancelled_export_through_the_real_engine_writes_nothing_and_leaves_the_session_usable() {
    let path = vectorcraft_testkit::temp_dir("mcp-cancel").join("out.pdf");
    std::fs::remove_file(&path).ok();
    let (read, mut to_server) = std::io::pipe().unwrap();
    let (tx, replies) = mpsc::channel();
    let (started_tx, started) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        let backend = Gated { inner: crate::Headless::with_document(), started: started_tx };
        // Request 2 waits for the cancelled export to return, also when the export ran past the
        // default busy limit (2 s) before it saw the cancel.
        Server::new(Box::new(backend)).with_busy_after(Duration::from_secs(60)).serve(BufReader::new(read), Lines(tx, vec![])).unwrap();
    });
    let send = |w: &mut std::io::PipeWriter, v: Value| writeln!(w, "{v}").unwrap();
    let params = json!({"name": "command_run", "arguments": {"id": "document.export", "params": {"path": path.to_str().unwrap()}}});
    send(&mut to_server, json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": params}));
    started.recv_timeout(WAIT).unwrap();
    send(&mut to_server, json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 1}}));
    // The next request is answered: the cancelled export neither blocks nor breaks the engine.
    send(&mut to_server, json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "doc_inspect", "arguments": {}}}));
    let reply: Value = serde_json::from_str(&replies.recv_timeout(WAIT).unwrap()).unwrap();
    assert_eq!(reply["id"], 2, "request 1 is not answered: {reply}");
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    assert!(!path.exists(), "a cancelled export writes no file");
    drop(to_server);
    thread.join().unwrap();
}

#[test]
fn a_remote_call_stops_waiting_for_the_app_when_cancelled() {
    use std::io::BufRead;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    // An app that takes the request and never answers.
    let (done, held) = mpsc::channel::<()>();
    let app = std::thread::spawn(move || {
        let (conn, _) = listener.accept().unwrap();
        let mut line = String::new();
        BufReader::new(conn.try_clone().unwrap()).read_line(&mut line).unwrap();
        // Hold the connection open until the test is done with it (or for `WAIT`).
        let _ = held.recv_timeout(WAIT);
        drop(conn);
    });
    let mut remote = crate::Remote::connect(&addr).unwrap();
    let token = cancel::CancelToken::new();
    let raise = token.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        raise.cancel();
    });
    let r = cancel::scope(&token, None, || remote.call("engine.execute", json!({"command": "document.export"})));
    let why = r.unwrap_err();
    assert!(why.contains("cancelled"), "{why}");
    // The app still holds the connection open.
    assert!(!app.is_finished(), "returned when cancelled, not when the app answered");
    done.send(()).ok();
    app.join().unwrap();
}
