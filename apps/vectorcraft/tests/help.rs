// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! `vectorcraft --help` prints the usage and exits before the app starts (#2110): run from a
//! terminal, SSH or an agent's shell it used to open the window (or hang without a display).

use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_vectorcraft");

/// Runs the desktop binary without a display, so a regression fails instead of opening a window.
fn run(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut child = Command::new(BIN)
        .args(args)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .env_remove("VECTORCRAFT_CONTROL_PORT")
        .env("VECTORCRAFT_NO_PREFS", "1")
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            panic!("{args:?}: the app started instead of printing its usage");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    child.wait_with_output().unwrap()
}

fn prints_usage(args: &[&str], env: &[(&str, &str)]) {
    let out = run(args, env);
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
    assert!(out.stderr.is_empty(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout.starts_with("Usage: vectorcraft "), "{args:?}: {stdout}");
    for flag in ["--control", "--automation-read-root", "--automation-write-root", "--in-window-menus", "--version", "--help"] {
        assert!(stdout.contains(flag), "{args:?}: usage doesn't mention {flag}: {stdout}");
    }
    // Headless work is the CLI's: the usage says where to go.
    assert!(stdout.contains("vectorcraft-cli"), "{args:?}: {stdout}");
}

#[test]
fn help_prints_usage_and_exits_before_startup() {
    prints_usage(&["--help"], &[]);
    prints_usage(&["-h"], &[]);
    // After a file or a flag, too: nothing is opened.
    prints_usage(&["drawing.svg", "--help"], &[]);
    prints_usage(&["--in-window-menus", "-h"], &[]);
}

#[test]
fn help_wins_over_a_bad_control_port_variable() {
    prints_usage(&["--help"], &[("VECTORCRAFT_CONTROL_PORT", "abc")]);
}
