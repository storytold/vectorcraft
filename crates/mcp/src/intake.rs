//! What lets the server hear a client while a call runs: a reader thread that takes lines off the
//! transport, a queue the engine thread works through, and a writer thread that owns the output.
//!
//! The engine runs one command at a time, on the thread that called [`crate::Server::serve`]. The
//! reader never touches the engine. While a call runs it can still
//!
//! - act on [`notifications/cancelled`](https://modelcontextprotocol.io/specification/2025-06-18/basic/utilities/cancellation):
//!   a queued request is dropped, a running one has its [`CancelToken`] raised, and neither gets
//!   a reply;
//! - answer the requests that need no engine (`ping`, the `…/list` methods, `prompts/get`), and
//! - refuse, with a "busy" error that names the running call, a request that would have to wait
//!   behind a call that has already run for [`Server::with_busy_after`](crate::Server::with_busy_after).
//!
//! Requests that arrive sooner than that queue up and run in order, so a client that pipelines a
//! few quick calls sees them answered in order, as before. The queue is bounded: past
//! [`MAX_QUEUED`] lines the reader waits for the engine rather than buffering without bound.

use std::collections::VecDeque;
use std::io::BufRead;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use vectorcraft_engine::cancel::CancelToken;

use crate::server::{INVALID_REQUEST, MAX_LINE_BYTES, PARSE_ERROR, SERVER_BUSY, error, error_with, read_line_capped};

/// Most lines waiting for the engine. Past this the reader waits (it still reads nothing else).
pub(crate) const MAX_QUEUED: usize = 64;

/// A line the engine thread has still to run.
pub(crate) enum Job {
    /// A reply that is already known (a parse error, an overlong line), kept in order with the rest.
    Canned(String),
    /// A parsed message: a request, a notification or a batch.
    Message(Message),
}

pub(crate) struct Message {
    pub(crate) value: Value,
    /// The request id as JSON text, for a request; what `notifications/cancelled` names.
    pub(crate) key: Option<String>,
    pub(crate) method: String,
    /// Raised by `notifications/cancelled`, or when the transport closes under the call.
    pub(crate) token: CancelToken,
    /// What the call is, for the "busy" answer: `tools/call command_run document.export`.
    pub(crate) label: String,
}

/// The call the engine thread is running.
struct Running {
    key: Option<String>,
    method: String,
    token: CancelToken,
    label: String,
    since: Instant,
}

#[derive(Default)]
struct State {
    queue: VecDeque<Job>,
    running: Option<Running>,
    /// The transport has no more lines.
    eof: bool,
    /// The output failed: nobody reads replies any more.
    closed: bool,
}

/// What the reader and the engine thread share.
pub(crate) struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    /// Whether the client speaks the `2026-07-28` revision, for results answered by the reader.
    pub(crate) modern: Arc<AtomicBool>,
    busy_after: Duration,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A thread that panicked while holding the lock left a queue that is still well formed.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    pub(crate) fn new(modern: Arc<AtomicBool>, busy_after: Duration) -> Self {
        Self { state: Mutex::new(State::default()), wake: Condvar::new(), modern, busy_after }
    }

    /// The next job, or `None` once the transport is done and the queue empty (or the output
    /// failed). The job counts as running until [`Shared::finished`].
    pub(crate) fn next(&self) -> Option<Job> {
        let mut st = lock(&self.state);
        loop {
            if st.closed {
                return None;
            }
            if let Some(job) = st.queue.pop_front() {
                if let Job::Message(m) = &job {
                    st.running = Some(Running {
                        key: m.key.clone(),
                        method: m.method.clone(),
                        token: m.token.clone(),
                        label: m.label.clone(),
                        since: Instant::now(),
                    });
                }
                // The reader may be waiting for room.
                self.wake.notify_all();
                return Some(job);
            }
            if st.eof {
                return None;
            }
            st = self.wake.wait(st).unwrap_or_else(PoisonError::into_inner);
        }
    }

    pub(crate) fn finished(&self) {
        lock(&self.state).running = None;
        self.wake.notify_all();
    }

    /// The output failed: stop the running call and everything queued behind it.
    pub(crate) fn close(&self) {
        let mut st = lock(&self.state);
        st.closed = true;
        if let Some(r) = &st.running {
            r.token.cancel();
        }
        st.queue.clear();
        self.wake.notify_all();
    }

    fn end_of_input(&self) {
        lock(&self.state).eof = true;
        self.wake.notify_all();
    }

    fn is_closed(&self) -> bool {
        lock(&self.state).closed
    }

    /// The running call, once it has run for `busy_after` (so a quick call never makes a
    /// pipelined neighbour fail).
    fn long_running(&self, st: &State) -> Option<Value> {
        let r = st.running.as_ref()?;
        let secs = r.since.elapsed();
        (secs >= self.busy_after).then(|| json!({"running": r.label, "requestId": r.key, "seconds": secs.as_secs()}))
    }
}

/// The id of a request message: `Some` when `msg` is a JSON-RPC 2.0 request with a usable id.
fn request_id(msg: &Value) -> Option<&Value> {
    let o = msg.as_object()?;
    if o.get("jsonrpc").and_then(Value::as_str) != Some("2.0") || !o.get("method")?.is_string() {
        return None;
    }
    o.get("id").filter(|id| id.is_string() || id.is_number())
}

/// A short name for the call, as the "busy" answer shows it.
fn label_of(method: &str, params: &Value) -> String {
    if method != "tools/call" {
        return method.to_string();
    }
    let name = params.get("name").and_then(Value::as_str).unwrap_or("?");
    let args = params.get("arguments");
    let which = ["id", "command", "path"].iter().find_map(|k| args?.get(k)?.as_str());
    match which {
        Some(w) => format!("{method} {name} {w}"),
        None => format!("{method} {name}"),
    }
}

/// Read lines from `reader` until the end, acting on each as described in the module docs.
pub(crate) fn read_loop(shared: &Shared, out: &SyncSender<String>, mut reader: impl BufRead) -> std::io::Result<()> {
    let mut buf = Vec::new();
    let result = loop {
        match read_line_capped(&mut reader, &mut buf, MAX_LINE_BYTES) {
            Ok(true) => {}
            Ok(false) => break Ok(()),
            Err(e) => break Err(e),
        }
        if shared.is_closed() {
            break Ok(());
        }
        if buf.len() > MAX_LINE_BYTES {
            // Longer than anything a client can mean: answer the framing violation, skip the
            // rest of the line in bounded chunks, and keep serving what follows.
            let reply = error(Value::Null, INVALID_REQUEST, format!("request line longer than {MAX_LINE_BYTES} bytes")).to_string();
            submit_canned(shared, out, reply);
            let ended = loop {
                match read_line_capped(&mut reader, &mut buf, MAX_LINE_BYTES) {
                    Ok(true) if buf.len() > MAX_LINE_BYTES => {}
                    Ok(true) => break Ok(false),
                    Ok(false) => break Ok(true),
                    Err(e) => break Err(e),
                }
            };
            match ended {
                Ok(false) => continue,
                Ok(true) => break Ok(()),
                Err(e) => break Err(e),
            }
        }
        let line = match std::str::from_utf8(&buf) {
            Ok(l) => l,
            Err(e) => break Err(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
        };
        intake(shared, out, line);
    };
    shared.end_of_input();
    result
}

/// A reply that is already known: in order behind queued work, or at once when the engine is
/// tied up for long (then nobody can expect order). Like a message, it waits while
/// [`MAX_QUEUED`] lines are queued.
fn submit_canned(shared: &Shared, out: &SyncSender<String>, reply: String) {
    let mut st = lock(&shared.state);
    loop {
        if st.closed {
            return;
        }
        if shared.long_running(&st).is_some() {
            drop(st);
            let _ = out.send(reply);
            return;
        }
        if st.queue.len() < MAX_QUEUED {
            break;
        }
        // Wake now and then: the call ahead may turn long-running while the queue is full.
        st = shared.wake.wait_timeout(st, Duration::from_millis(50)).unwrap_or_else(PoisonError::into_inner).0;
    }
    st.queue.push_back(Job::Canned(reply));
    shared.wake.notify_all();
}

fn intake(shared: &Shared, out: &SyncSender<String>, line: &str) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }
    let value = match serde_json::from_str::<Value>(line) {
        Ok(v) => v,
        Err(e) => return submit_canned(shared, out, error(Value::Null, PARSE_ERROR, format!("parse error: {e}")).to_string()),
    };
    let method = value.get("method").and_then(Value::as_str).unwrap_or("").to_string();
    let enveloped = value.get("jsonrpc").and_then(Value::as_str) == Some("2.0");
    if enveloped && method == "notifications/cancelled" && value.get("id").is_none() {
        return cancel(shared, value.get("params").unwrap_or(&Value::Null));
    }
    let params = value.get("params").cloned().unwrap_or(Value::Null);
    let id = request_id(&value).cloned();
    let is_batch = value.is_array();
    let token = CancelToken::new();
    let msg = Message { key: id.as_ref().map(Value::to_string), label: label_of(&method, &params), method, token, value };
    let mut st = lock(&shared.state);
    loop {
        if st.closed {
            return;
        }
        let busy = shared.long_running(&st);
        if let Some(busy) = busy {
            if let Some(id) = &id {
                // Engine-free requests are answered here; others are told to retry.
                let reply = match crate::server::stateless_reply(&msg.value, shared.modern.load(Ordering::Relaxed)) {
                    Some(r) => r,
                    None => error_with(id.clone(), SERVER_BUSY, busy_message(&busy), json!({"busy": busy})),
                };
                drop(st);
                let _ = out.send(reply.to_string());
                return;
            }
            if is_batch {
                drop(st);
                let reply = error_with(Value::Null, SERVER_BUSY, busy_message(&busy), json!({"busy": busy}));
                let _ = out.send(reply.to_string());
                return;
            }
        }
        if st.queue.len() < MAX_QUEUED {
            break;
        }
        // Wake now and then: the call that fills the queue may be the one that turns it away.
        st = shared.wake.wait_timeout(st, Duration::from_millis(50)).unwrap_or_else(PoisonError::into_inner).0;
    }
    st.queue.push_back(Job::Message(msg));
    shared.wake.notify_all();
}

fn busy_message(busy: &Value) -> String {
    format!(
        "the engine is busy: `{}` has been running for {} s and this request was not run. Retry when it has finished, or cancel it with notifications/cancelled (requestId {})",
        busy["running"].as_str().unwrap_or("a call"),
        busy["seconds"].as_u64().unwrap_or(0),
        busy["requestId"],
    )
}

/// `notifications/cancelled {requestId}`: stop that request if it is still queued or running.
/// An unknown or finished id is ignored, as is a request that can't be cancelled (`initialize`).
fn cancel(shared: &Shared, params: &Value) {
    let Some(id) = params.get("requestId").filter(|id| id.is_string() || id.is_number()) else { return };
    let key = id.to_string();
    let mut st = lock(&shared.state);
    if let Some(r) = st.running.as_ref().filter(|r| r.key.as_deref() == Some(&key) && r.method != "initialize") {
        log::debug!("cancelling running request {key}: {}", r.label);
        r.token.cancel();
        return;
    }
    let at = st.queue.iter().position(|j| matches!(j, Job::Message(m) if m.key.as_deref() == Some(&key) && m.method != "initialize"));
    if let Some(at) = at {
        log::debug!("dropping queued request {key}");
        st.queue.remove(at);
        shared.wake.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_errors_wait_for_room_in_the_queue() {
        // No call has run past the busy limit, so the replies queue rather than go out at once.
        let shared = Shared::new(Arc::new(AtomicBool::new(false)), Duration::from_secs(60));
        let (out, _replies) = std::sync::mpsc::sync_channel(1);
        let lines = 3 * MAX_QUEUED;
        let input = "not json\n".repeat(lines);
        std::thread::scope(|scope| {
            let reader = scope.spawn(|| read_loop(&shared, &out, input.as_bytes()));
            // Nothing takes jobs off the queue yet, so the reader stops at the limit.
            let end = Instant::now() + Duration::from_secs(10);
            while !reader.is_finished() && lock(&shared.state).queue.len() < MAX_QUEUED && Instant::now() < end {
                std::thread::sleep(Duration::from_millis(5));
            }
            std::thread::sleep(Duration::from_millis(100));
            let queued = lock(&shared.state).queue.len();
            assert!(queued <= MAX_QUEUED, "{queued} lines queued, more than {MAX_QUEUED}");
            // Once the engine takes jobs, every line is answered with its parse error.
            let mut answered = 0;
            while let Some(job) = shared.next() {
                assert!(matches!(&job, Job::Canned(r) if r.contains("parse error")));
                answered += 1;
                shared.finished();
            }
            assert_eq!(answered, lines);
            reader.join().unwrap().unwrap();
        });
    }
}
