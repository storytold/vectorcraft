//! Absolute transport budgets, independent of the time spent executing a request.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

fn deadline(budget: Duration) -> io::Result<Instant> {
    Instant::now().checked_add(budget).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "control transport deadline out of range"))
}

fn remaining(until: Option<Instant>) -> io::Result<Duration> {
    until
        .and_then(|d| d.checked_duration_since(Instant::now()))
        .filter(|d| !d.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "control transport deadline exceeded"))
}

pub(super) struct DeadlineReader {
    stream: TcpStream,
    idle: Duration,
    line: Duration,
    until: Option<Instant>,
    waiting_for_first_byte: bool,
}

impl DeadlineReader {
    pub(super) fn new(stream: TcpStream, idle: Duration, line: Duration) -> Self {
        Self { stream, idle, line, until: None, waiting_for_first_byte: false }
    }

    pub(super) fn begin_line(&mut self, buffered: bool) -> io::Result<()> {
        // A partial next request already in BufReader must not get an idle budget too.
        self.waiting_for_first_byte = !buffered;
        self.until = Some(deadline(if buffered { self.line } else { self.idle })?);
        Ok(())
    }
}

impl Read for DeadlineReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        self.stream.set_read_timeout(Some(remaining(self.until)?))?;
        let n = self.stream.read(buf)?;
        if n > 0 && self.waiting_for_first_byte {
            self.until = Some(deadline(self.line)?);
            self.waiting_for_first_byte = false;
        }
        Ok(n)
    }
}

pub(super) struct DeadlineWriter {
    stream: TcpStream,
    budget: Duration,
    until: Option<Instant>,
}

impl DeadlineWriter {
    pub(super) fn new(stream: TcpStream, budget: Duration) -> Self {
        Self { stream, budget, until: None }
    }

    pub(super) fn begin_reply(&mut self) -> io::Result<()> {
        self.until = Some(deadline(self.budget)?);
        Ok(())
    }
}

impl Write for DeadlineWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        // Each partial write gets the remaining budget, never a fresh full timeout.
        self.stream.set_write_timeout(Some(remaining(self.until)?))?;
        self.stream.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.set_write_timeout(Some(remaining(self.until)?))?;
        self.stream.flush()
    }
}
