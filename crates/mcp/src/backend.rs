//! Where MCP tool calls end up: a control-channel method call.

use std::io::{BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use serde_json::{Value, json};

/// Something that answers control-channel methods (`engine.execute`, `document.inspect`,
/// `ui.pointer`, `ui.render`, `app.export`, …). See `vectorcraft_ui_egui::control` for the list.
pub trait Backend {
    /// Call one method. `Ok` carries the `result`, `Err` the error message.
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String>;
    /// True when a real UI is attached (`ui.*` methods like `ui.key` / `ui.inspect` work).
    fn has_ui(&self) -> bool;
    /// Short human description ("headless", "remote 127.0.0.1:7979").
    fn describe(&self) -> String;
}

/// A running VectorCraft app, reached through its loopback control port.
pub struct Remote {
    addr: String,
    conn: Option<(BufReader<TcpStream>, TcpStream)>,
    next_id: u64,
}

/// Longest accepted control-channel reply line (bytes, without the newline). The app answers
/// small JSON objects except for images, which reach tens of megabytes; anything longer is refused
/// and the connection dropped, so a faulty endpoint can't make the server buffer without bound
/// (#1727).
const MAX_REPLY_BYTES: usize = 256 * 1024 * 1024;

impl Remote {
    /// Connect to `addr` (e.g. `127.0.0.1:7979`), failing fast when nothing is listening.
    pub fn connect(addr: &str) -> std::io::Result<Self> {
        let mut r = Self { addr: addr.to_string(), conn: None, next_id: 1 };
        r.reconnect()?;
        Ok(r)
    }

    pub fn addr(&self) -> &str {
        &self.addr
    }

    #[cfg(test)]
    pub(crate) fn connection_probe(&self) -> TcpStream {
        self.conn.as_ref().unwrap().1.try_clone().unwrap()
    }

    fn reconnect(&mut self) -> std::io::Result<()> {
        self.conn = None;
        let mut last = std::io::Error::new(std::io::ErrorKind::NotFound, format!("cannot resolve {}", self.addr));
        for sa in self.addr.to_socket_addrs()? {
            match TcpStream::connect_timeout(&sa, Duration::from_millis(800)) {
                Ok(s) => {
                    s.set_nodelay(true).ok();
                    // The app answers within 60 s (its own timeout); leave headroom.
                    s.set_read_timeout(Some(Duration::from_secs(90))).ok();
                    let read = s.try_clone()?;
                    self.conn = Some((BufReader::new(read), s));
                    return Ok(());
                }
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    fn prepare_connection(&mut self) -> std::io::Result<()> {
        if let Some((_, stream)) = self.conn.as_ref() {
            stream.set_nonblocking(true)?;
            let probe = stream.peek(&mut [0]);
            stream.set_nonblocking(false)?;
            match probe {
                // Closed by the app, or holding bytes nobody asked for (an app that explains why it
                // closes): either way no reply to the next request can be read from it.
                Ok(_) => self.conn = None,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => self.conn = None,
            }
        }
        if self.conn.is_none() {
            self.reconnect()?;
        }
        Ok(())
    }

    /// Wait until the app starts to answer, giving up when the call is cancelled
    /// ([`vectorcraft_engine::cancel`]): the app has no way to stop what it runs, but this server
    /// stops waiting for it (the connection is dropped, so a late answer can't be mistaken for
    /// the next call's). Without a cancellable call this is just the read the caller does next.
    fn wait_for_reply(stream: &TcpStream) -> std::io::Result<()> {
        const POLL: Duration = Duration::from_millis(100);
        const LIMIT: Duration = Duration::from_secs(90);
        if !vectorcraft_engine::cancel::is_cancellable() {
            return Ok(());
        }
        let start = std::time::Instant::now();
        let kept = stream.read_timeout()?;
        stream.set_read_timeout(Some(POLL))?;
        let outcome = loop {
            match stream.peek(&mut [0]) {
                // Bytes (or the end of the stream) to read now.
                Ok(_) => break Ok(()),
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                    if vectorcraft_engine::cancel::is_cancelled() {
                        break Err(std::io::Error::new(
                            std::io::ErrorKind::Interrupted,
                            "cancelled; the app may still finish what it was asked, which can't be stopped from here",
                        ));
                    }
                    if start.elapsed() >= LIMIT {
                        break Err(e);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => break Err(e),
            }
        };
        stream.set_read_timeout(kept)?;
        outcome
    }

    fn roundtrip(&mut self, line: &str) -> std::io::Result<String> {
        let Some((reader, writer)) = self.conn.as_mut() else {
            return Err(std::io::Error::new(std::io::ErrorKind::NotConnected, "not connected to the app"));
        };
        writer.write_all(line.as_bytes())?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        Self::wait_for_reply(writer)?;
        let mut buf = Vec::new();
        if !crate::server::read_line_capped(reader, &mut buf, MAX_REPLY_BYTES)? {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "control channel closed"));
        }
        if buf.len() > MAX_REPLY_BYTES {
            // A faulty endpoint's endless line is refused rather than buffered: the connection is
            // dropped and the call reports uncertainty, as for any reply that never arrived whole.
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("control reply longer than {MAX_REPLY_BYTES} bytes")));
        }
        String::from_utf8(buf).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}

impl Backend for Remote {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.prepare_connection().map_err(|e| {
            self.conn = None;
            format!("VectorCraft app at {} is not reachable: {e}; `{method}`: nothing was sent", self.addr)
        })?;
        let id = self.next_id;
        self.next_id += 1;
        let line = json!({"id": id, "method": method, "params": params}).to_string();
        let v: Value = self
            .roundtrip(&line)
            .and_then(|reply| {
                serde_json::from_str(reply.trim())
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("bad reply from app: {e}")))
            })
            .map_err(|e| {
                self.conn = None;
                format!("VectorCraft app at {}: `{method}` failed: {e}; operation may have completed; inspect state before retrying", self.addr)
            })?;
        if v.get("id").is_none() && v.get("ok").and_then(Value::as_bool) == Some(false) {
            // The app's refusals carry no id (too many connections, a line it would not read):
            // it ran nothing and closes the connection, so the next call starts a fresh one.
            self.conn = None;
            let why = v.get("error").and_then(Value::as_str).unwrap_or("unknown error");
            return Err(format!("VectorCraft app at {} refused `{method}`: {why}; nothing was run", self.addr));
        }
        if v.get("id") != Some(&json!(id)) {
            // Not the answer to this call: the stream is desynchronised (or the endpoint faulty),
            // so the connection is dropped and the next call starts a fresh one. The operation may
            // still have completed, as with any reply that never arrived whole (#1728).
            self.conn = None;
            return Err(format!(
                "VectorCraft app at {}: reply id mismatch (expected {id}); connection dropped; operation may have completed; inspect state before retrying",
                self.addr
            ));
        }
        if v.get("ok").and_then(Value::as_bool) == Some(true) {
            Ok(v.get("result").cloned().unwrap_or(Value::Null))
        } else {
            Err(v.get("error").and_then(Value::as_str).unwrap_or("unknown error").to_string())
        }
    }

    fn has_ui(&self) -> bool {
        true
    }

    fn describe(&self) -> String {
        format!("remote {}", self.addr)
    }
}
