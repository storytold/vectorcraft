// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_vectorcraft");

/// Invalid roots must exit before desktop initialization on every platform, including a CI
/// machine with no display. A regression must fail rather than hang with a window open.
fn rejects(args: &[&str], message: &str) {
    rejects_with_env(args, &[], message);
}

fn rejects_with_env(args: &[&str], env: &[(&str, &str)], message: &str) {
    let mut child = Command::new(BIN)
        .args(args)
        .env_remove("VECTORCRAFT_CONTROL_PORT")
        .env_remove("VECTORCRAFT_AUTOMATION_READ_ROOT")
        .env_remove("VECTORCRAFT_AUTOMATION_WRITE_ROOT")
        .env("VECTORCRAFT_NO_PREFS", "1")
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Long enough for the binary to start on a loaded machine (the workspace's other tests run
    // meanwhile), short enough to catch a desktop that started instead of exiting.
    let deadline = Instant::now() + Duration::from_secs(30);
    let timed_out = loop {
        if child.try_wait().unwrap().is_some() {
            break false;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            break true;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let out = child.wait_with_output().unwrap();
    assert!(!timed_out, "{args:?}: desktop started or did not exit promptly");
    assert!(!out.status.success(), "{args:?}");
    assert!(out.stdout.is_empty(), "{args:?}: {:?}", out.stdout);
    // Exact stderr excludes unrelated startup warnings or display errors that could otherwise
    // make a malformed invocation fail for an unrelated reason.
    assert_eq!(String::from_utf8(out.stderr).unwrap(), format!("vectorcraft: {message}\n"), "{args:?}");
}

#[test]
fn missing_empty_or_option_operands_fail_before_startup() {
    for flag in ["--automation-read-root", "--automation-write-root"] {
        rejects(&["--control", "7979", flag], &format!("{flag} needs a folder"));
        rejects(&[flag], &format!("{flag} needs a folder"));
        rejects(&["--control", "7979", &format!("{flag}=")], &format!("{flag} needs a non-empty folder"));
        rejects(&[flag, ""], &format!("{flag} needs a non-empty folder"));
        for next in ["--version", "--control", "--automation-write-root", "-folder"] {
            rejects(&[flag, next], &format!("{flag} needs a folder; use {flag}=<dir> for a folder starting with '-'"));
        }
    }
}

#[test]
fn duplicate_roots_fail_for_both_spellings() {
    for flag in ["--automation-read-root", "--automation-write-root"] {
        rejects(&[flag, ".", &format!("{flag}=.")], &format!("{flag} given twice"));
        rejects(&[&format!("{flag}=."), flag, "."], &format!("{flag} given twice"));
    }
}

#[test]
fn explicit_dash_folder_does_not_consume_version() {
    let out = Command::new(BIN).args(["--automation-read-root=-folder", "--automation-write-root", "./-folder", "--version"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8(out.stdout).unwrap(), format!("vectorcraft {}\n", env!("CARGO_PKG_VERSION")));
    assert!(out.stderr.is_empty());
}

/// `--control` without a port no longer starts the app without a control server, and the
/// argument after it isn't reinterpreted (#1373).
#[test]
fn control_without_a_valid_port_fails_before_startup() {
    let need = "--control needs a port number from 1 to 65535";
    rejects(&["--control", "--automation-read-root", ".", "--automation-write-root", "."], &format!("{need}, not `--automation-read-root`"));
    rejects(&["--control"], need);
    rejects(&["--control", "abc"], &format!("{need}, not `abc`"));
    rejects(&["--control=0"], &format!("{need}, not `0`"));
    rejects(&["--control="], &format!("{need}, not ``"));
    rejects(&["--control", "7979", "--control", "7980"], "--control given twice");
    rejects_with_env(&[], &[("VECTORCRAFT_CONTROL_PORT", "abc")], "VECTORCRAFT_CONTROL_PORT needs a port number from 1 to 65535, not `abc`");
}

#[test]
fn version_wins_over_a_bad_control_port_variable() {
    let out = Command::new(BIN).arg("--version").env("VECTORCRAFT_CONTROL_PORT", "abc").output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8(out.stdout).unwrap(), format!("vectorcraft {}\n", env!("CARGO_PKG_VERSION")));
}
