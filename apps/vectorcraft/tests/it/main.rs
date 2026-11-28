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
fn exit_output(child: Child, started: &str) -> Output {
    exit_output_by(child, started, Instant::now() + Duration::from_secs(120))
}

/// [`exit_output`] with the deadline given. A binary that has exited by the deadline passes even
/// when its output arrives after it; the output then has 10 seconds more to arrive.
fn exit_output_by(mut child: Child, started: &str, deadline: Instant) -> Output {
    let stdout = read_to_end(child.stdout.take().unwrap());
    let stderr = read_to_end(child.stderr.take().unwrap());
    let (mut out, mut err) = (None, None);
    if !receive([(&stdout, &mut out), (&stderr, &mut err)], deadline) && child.try_wait().unwrap().is_none() {
        child.kill().unwrap();
        child.wait().unwrap();
        panic!("{started}");
    }
    receive([(&stdout, &mut out), (&stderr, &mut err)], Instant::now() + Duration::from_secs(10));
    let (Some(stdout), Some(stderr)) = (out, err) else {
        panic!("{started}: the binary exited, but its output pipes stayed open");
    };
    Output { status: child.wait().unwrap(), stdout, stderr }
}

/// A pipe's reader and the slot that holds its output once it arrives.
type Pipe<'a> = (&'a mpsc::Receiver<Vec<u8>>, &'a mut Option<Vec<u8>>);

/// Fills each empty slot from its reader, waiting until `until` at the latest. Returns whether
/// every slot is filled.
fn receive(pipes: [Pipe<'_>; 2], until: Instant) -> bool {
    let mut filled = true;
    for (rx, slot) in pipes {
        if slot.is_none() {
            *slot = rx.recv_timeout(until.saturating_duration_since(Instant::now())).ok();
        }
        filled &= slot.is_some();
    }
    filled
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

/// A binary that exits before the deadline passes even when a process it started keeps its
/// output pipes open past the deadline.
#[cfg(unix)]
#[test]
fn exit_output_accepts_an_exit_whose_output_arrives_after_the_deadline() {
    use std::process::{Command, Stdio};
    let mut child =
        Command::new("sh").args(["-c", "sleep 1 & exit 0"]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    // `sh` has exited; the `sleep` it started still holds both pipes for about a second.
    child.wait().unwrap();
    let out = exit_output_by(child, "an exited binary was reported as running", Instant::now());
    assert!(out.status.success());
    assert!(out.stdout.is_empty() && out.stderr.is_empty());
}
