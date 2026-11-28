//! Integration tests: one test binary for the crate, one module per area.
// Unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Read;
use std::process::{Child, Output};
use std::sync::mpsc;
use std::time::{Duration, Instant};

mod automation_args;
mod help;

/// Nextest relocates archived binaries; Cargo's compile-time path still names the build runner.
fn bin() -> std::path::PathBuf {
    std::env::var_os("NEXTEST_BIN_EXE_vectorcraft")
        .or_else(|| std::env::var_os("CARGO_BIN_EXE_vectorcraft"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_vectorcraft").into())
}

/// Waits for `child`, spawned with piped stdout and stderr, to exit and returns its output. Both
/// pipes end when the binary exits, however long a loaded machine takes to start it. The deadline
/// only stops a binary that is still running: the test then fails with `started`.
fn exit_output(mut child: Child, started: &str) -> Output {
    let deadline = Instant::now() + Duration::from_secs(120);
    let stdout = read_to_end(child.stdout.take().unwrap());
    let stderr = read_to_end(child.stderr.take().unwrap());
    let recv = |rx: &mpsc::Receiver<Vec<u8>>| rx.recv_timeout(deadline.saturating_duration_since(Instant::now()));
    let (Ok(stdout), Ok(stderr)) = (recv(&stdout), recv(&stderr)) else {
        child.kill().unwrap();
        panic!("{started}");
    };
    Output { status: child.wait().unwrap(), stdout, stderr }
}

/// Reads `pipe` to its end on another thread.
fn read_to_end(mut pipe: impl Read + Send + 'static) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = pipe.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    rx
}
