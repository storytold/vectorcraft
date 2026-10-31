use super::*;

/// The token the socket fixtures' server expects.
const TOKEN: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

/// The `auth` request line with [`TOKEN`].
fn auth_line() -> String {
    format!("{}\n", control_auth::auth_request(TOKEN))
}

fn short_timeouts() -> ConnectionTimeouts {
    ConnectionTimeouts { idle: Duration::from_millis(150), line: Duration::from_millis(120), write: Duration::from_millis(150) }
}

/// How long a test waits for the server to reply or to close a connection. The wait only ends a
/// test whose server never does either; the budgets under test are 120 and 150 ms, and 1 s in the
/// persistent test. On a busy machine the server's thread can pause for seconds before or between
/// its reads and writes, and a wait of a few seconds then fails a server that closes the
/// connection on time.
const WAIT: Duration = Duration::from_secs(20);

/// The short line deadline, with idle and reply budgets longer than [`WAIT`]. On a busy machine
/// either thread can pause for more than 150 ms between connecting and sending, or between
/// starting a reply and writing it, and a short budget then closes the connection with no reply.
fn line_timeouts() -> ConnectionTimeouts {
    ConnectionTimeouts { idle: Duration::from_secs(30), write: Duration::from_secs(30), ..short_timeouts() }
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
        serve_socket(stream, timeouts, TOKEN, handle);
        finished.send(()).unwrap();
    });
    let client = TcpStream::connect(addr).unwrap();
    socket2::SockRef::from(&client).set_recv_buffer_size(4096).unwrap();
    client.set_read_timeout(Some(WAIT)).unwrap();
    (client, done, worker)
}

fn expect_socket_deadline(client: TcpStream, done: Receiver<()>, worker: std::thread::JoinHandle<()>) {
    let stopped = done.recv_timeout(WAIT).is_ok();
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
    let (mut client, done, worker) = socket_fixture(line_timeouts(), |_, _| panic!("incomplete requests must not execute"));
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
    let stopped = done.recv_timeout(WAIT).is_ok();
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
    client.write_all(format!("{}{{\"method\":\"large\"}}\n", auth_line()).as_bytes()).unwrap();
    expect_socket_deadline(client, done, worker);
}

/// Each request gets its own line and reply budgets. The handler takes longer than both budgets,
/// so a deadline kept from the previous request runs out. The line and reply budgets are 1 s, and
/// the idle budget is 30 s, longer than the client's [`WAIT`]: on a busy machine either thread can
/// pause for more than the short budgets of [`short_timeouts`] between starting a budget and the
/// read or write it covers.
#[test]
fn persistent_control_requests_get_fresh_transport_deadlines() {
    let timeouts = ConnectionTimeouts { idle: Duration::from_secs(30), line: Duration::from_secs(1), write: Duration::from_secs(1) };
    let (mut client, done, worker) = socket_fixture(timeouts, |_, _| {
        // Application processing is not transport time; begin the reply budget afterwards.
        std::thread::sleep(Duration::from_millis(1250));
        Some(json!({"ok": true}))
    });
    let mut reader = BufReader::new(client.try_clone().unwrap());
    client.write_all(auth_line().as_bytes()).unwrap();
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["ok"], true, "{line}");
    for id in [1, 2] {
        // The client sends each line in one write. `writeln!` on the socket sends the JSON in many
        // small writes, and when the machine is busy, a pause of more than the line deadline between
        // two of them closes the connection.
        client.write_all(format!("{}\n", json!({"id": id, "method": "inspect"})).as_bytes()).unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap(), json!({"id": id, "ok": true}));
    }
    client.shutdown(std::net::Shutdown::Write).unwrap();
    done.recv_timeout(WAIT).unwrap();
    worker.join().unwrap();
}

#[test]
fn buffered_partial_control_line_also_expires() {
    let (mut client, done, worker) = socket_fixture(line_timeouts(), |_, _| Some(json!({"ok": true})));
    client.write_all(format!("{}{{\"id\":1,\"method\":\"inspect\"}}\n{{", auth_line()).as_bytes()).unwrap();
    let mut reader = BufReader::new(client.try_clone().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["id"], "auth");
    line.clear();
    reader.read_line(&mut line).unwrap();
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
    let worker = spawn_connection(stream, tx, egui::Context::default(), TOKEN.into(), slot, short_timeouts()).unwrap();
    let start = std::time::Instant::now();
    while open.load(Ordering::SeqCst) == MAX_CONNECTIONS && start.elapsed() < WAIT {
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
    c.set_read_timeout(Some(WAIT)).unwrap();
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
    std::thread::spawn(move || accept_loop(listener, tx, egui::Context::default(), TOKEN.into()));
    // Idle connections are accepted in order and held open.
    let held: Vec<TcpStream> = (0..16).map(|_| TcpStream::connect(addr).unwrap()).collect();
    let mut extra = TcpStream::connect(addr).unwrap();
    extra.set_read_timeout(Some(WAIT)).unwrap();
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

/// Serve `input` on a connection that must authenticate with [`TOKEN`]; the replies and the
/// methods that ran.
fn run_authenticated(input: &[u8]) -> (Vec<Value>, Vec<String>) {
    let mut out = Vec::new();
    let mut calls = Vec::new();
    serve_lines_authenticated(input, &mut out, TOKEN, |m, _p| {
        calls.push(m);
        Some(json!({"ok": true, "result": null}))
    });
    let replies = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str::<Value>(l).unwrap()).collect();
    (replies, calls)
}

/// #3603: without the token, nothing runs: a request sent first, an `auth` without a token or
/// with a wrong one, each gets one refusal and the connection is closed.
#[test]
fn a_missing_or_wrong_token_is_refused_and_closes_the_connection() {
    let wrong = control_auth::auth_request(&"f".repeat(64));
    let short = control_auth::auth_request("1234");
    for first in [
        json!({"id": 1, "method": "engine.execute", "params": {"command": "file.new"}}),
        json!({"id": 1, "method": "auth"}),
        json!({"id": 1, "method": "auth", "params": {"token": null}}),
        wrong,
        short,
        // The right token on another method is no authentication either.
        json!({"id": 1, "method": "ui.inspect", "params": {"token": TOKEN}}),
    ] {
        let input = format!("{first}\n{}{{\"method\":\"engine.execute\"}}\n", auth_line());
        let (r, calls) = run_authenticated(input.as_bytes());
        assert!(calls.is_empty(), "{first}: nothing may run: {calls:?}");
        assert_eq!(r.len(), 1, "{first}: one refusal, then the connection closes: {r:?}");
        assert_eq!(r[0]["ok"], false);
        assert!(r[0]["error"].as_str().unwrap().starts_with("authentication required"), "{r:?}");
    }
}

#[test]
fn the_right_token_authenticates_the_connection() {
    let input = format!("\n{}{{\"id\":2,\"method\":\"a\"}}\n{}{{\"id\":3,\"method\":\"b\"}}\n", auth_line(), auth_line());
    let (r, calls) = run_authenticated(input.as_bytes());
    assert_eq!(calls, ["a", "b"], "a repeated `auth` is not passed on as a method");
    assert_eq!(r[0], json!({"id": "auth", "ok": true, "result": {"authenticated": true}}));
    assert_eq!(r[1]["id"], 2);
    assert_eq!(r[2], json!({"id": "auth", "ok": false, "error": "already authenticated"}));
    assert_eq!(r[3]["id"], 3);
    // Hex letters match in either case.
    let upper = format!("{}\n{{\"method\":\"a\"}}\n", control_auth::auth_request(&TOKEN.to_uppercase()));
    assert_eq!(run_authenticated(upper.as_bytes()).1, ["a"]);
}

/// #3603: a browser (a page's `fetch`, or one rebound to 127.0.0.1) is refused as HTTP before any
/// token is asked for, and its body never runs, even when it carries the right token.
#[test]
fn an_http_request_is_refused_before_authentication() {
    let body = format!("{}{{\"method\":\"engine.execute\"}}\n", auth_line());
    for start in ["POST / HTTP/1.1", "GET /?x=1 HTTP/1.0", "OPTIONS * HTTP/1.1"] {
        let http = format!("{start}\r\nHost: 127.0.0.1:7979\r\nOrigin: https://attacker.example\r\n\r\n{body}");
        let (r, calls) = run_authenticated(http.as_bytes());
        assert!(calls.is_empty(), "{start}: {calls:?}");
        assert_eq!(r.len(), 1);
        assert!(r[0]["error"].as_str().unwrap().starts_with("HTTP requests are not accepted"), "{r:?}");
    }
    assert!(!looks_like_http("{\"method\": \"a HTTP/1.1\"}"));
}

/// End to end over a socket: no token, then the right one on a new connection.
#[test]
fn a_socket_without_the_token_runs_nothing() {
    let (mut client, done, worker) = socket_fixture(line_timeouts(), |_, _| panic!("nothing may run without the token"));
    client.write_all(b"{\"id\":1,\"method\":\"app.save\",\"params\":{\"path\":\"/tmp/x\"}}\n").unwrap();
    let mut text = String::new();
    client.read_to_string(&mut text).unwrap();
    assert!(text.contains("authentication required"), "{text}");
    expect_socket_deadline(client, done, worker);

    let (mut client, done, worker) = socket_fixture(line_timeouts(), |m, _| Some(json!({"ok": true, "result": m})));
    client.write_all(format!("{}{{\"id\":1,\"method\":\"ui.inspect\"}}\n", auth_line()).as_bytes()).unwrap();
    client.shutdown(std::net::Shutdown::Write).unwrap();
    let mut text = String::new();
    client.read_to_string(&mut text).unwrap();
    let lines: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines, [json!({"id": "auth", "ok": true, "result": {"authenticated": true}}), json!({"id": 1, "ok": true, "result": "ui.inspect"})]);
    expect_socket_deadline(client, done, worker);
}

#[test]
fn a_supplied_token_is_used_and_a_generated_one_needs_a_file() {
    let t = token_for(7979, Some(TOKEN.to_uppercase().into()), None).unwrap();
    assert_eq!((t.token.as_str(), t.file), (TOKEN, None));
    assert!(token_for(7979, Some("nope".into()), None).err().unwrap().contains("VECTORCRAFT_CONTROL_TOKEN"));
    let file = std::path::PathBuf::from("control-7979.token");
    let (a, b) = (token_for(7979, None, Some(file.clone())).unwrap(), token_for(7979, Some("".into()), Some(file.clone())).unwrap());
    assert_eq!(a.file, Some(file));
    assert!(control_auth::validate_token(&a.token).is_ok());
    assert_ne!(a.token, b.token, "a fresh token per launch");
    assert!(token_for(7979, None, None).is_err());
}

/// The server writes the token it generated, after binding, to a file only the user can read.
#[test]
fn the_generated_token_is_written_once_the_port_is_bound() {
    let dir = std::env::temp_dir().join(format!("vectorcraft-control-server-token-{}", std::process::id()));
    let path = dir.join("control.token");
    let port = TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
    let token = token_for(port, None, Some(path.clone())).unwrap();
    let expected = token.token.clone();
    let _rx = start(port, token, egui::Context::default());
    assert_eq!(control_auth::read_token_file(&path).unwrap(), expected);
    let mut c = TcpStream::connect(("127.0.0.1", port)).unwrap();
    c.set_read_timeout(Some(WAIT)).unwrap();
    c.write_all(format!("{}\n", control_auth::auth_request(&expected)).as_bytes()).unwrap();
    let mut line = String::new();
    BufReader::new(c).read_line(&mut line).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["ok"], true, "{line}");
    let _ = std::fs::remove_dir_all(dir);
}
