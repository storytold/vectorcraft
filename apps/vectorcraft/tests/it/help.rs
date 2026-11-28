//! `vectorcraft --help` prints the usage and exits before the app starts (#2110): run from a
//! terminal, SSH or an agent's shell it used to open the window (or hang without a display).

use std::process::{Command, Output, Stdio};

/// Runs the desktop binary without a display and with a control port it cannot parse, so a
/// regression exits with that error instead of opening a window.
fn run(args: &[&str], env: &[(&str, &str)]) -> Output {
    let child = Command::new(crate::bin())
        .args(args)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .env("VECTORCRAFT_CONTROL_PORT", "not-a-port")
        .env("VECTORCRAFT_NO_PREFS", "1")
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    crate::exit_output(child, &format!("{args:?}: the app started instead of printing its usage"))
}

fn prints_usage(args: &[&str], env: &[(&str, &str)]) {
    let out = run(args, env);
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
    assert!(out.stderr.is_empty(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout.starts_with("Usage: vectorcraft "), "{args:?}: {stdout}");
    for flag in [
        "--control",
        "--automation-read-root",
        "--automation-write-root",
        "--in-window-menus",
        "--safe-mode",
        "--present-mode",
        "--frame-latency",
        "--version",
        "--help",
    ] {
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

/// A bad presentation variable stops the app with a message before it starts; `--help` and the
/// matching flag win over it. A run that gets past it stops at the control port `run` makes fail.
#[test]
fn a_bad_presentation_variable_is_an_error_unless_help_or_its_flag_is_given() {
    for (var, value, need) in [
        ("VECTORCRAFT_FRAME_LATENCY", "0", "needs 1, 2 or 3 frames, not `0`"),
        ("VECTORCRAFT_PRESENT_MODE", "mailbox", "needs fifo, autovsync or novsync, not `mailbox`"),
    ] {
        let out = run(&[], &[(var, value)]);
        assert!(!out.status.success(), "{var}");
        assert_eq!(String::from_utf8(out.stderr).unwrap(), format!("vectorcraft: {var} {need}\n"));
        prints_usage(&["--help"], &[(var, value)]);
    }
    for (args, env) in [
        (&["--frame-latency", "2"][..], ("VECTORCRAFT_FRAME_LATENCY", "0")),
        (&["--present-mode=fifo"], ("VECTORCRAFT_PRESENT_MODE", "mailbox")),
        (&[], ("VECTORCRAFT_FRAME_LATENCY", "3")),
    ] {
        let stderr = String::from_utf8(run(args, &[env]).stderr).unwrap();
        assert!(stderr.contains("VECTORCRAFT_CONTROL_PORT") && !stderr.contains(env.0), "{args:?}: {stderr}");
    }
    let out = run(&["--frame-latency", "4"], &[]);
    assert!(!out.status.success());
    assert_eq!(String::from_utf8(out.stderr).unwrap(), "vectorcraft: --frame-latency needs 1, 2 or 3 frames, not `4`\n");
}

/// An option the app doesn't know (a typo of `--safe-mode`) is an error, not a file to open with
/// plug-ins; a later `--help` still wins.
#[test]
fn an_unknown_option_is_an_error_and_not_a_file() {
    for args in [&["--safe-mod"][..], &["drawing.svg", "--safe-mod"], &["-x"]] {
        let out = run(args, &[]);
        assert!(!out.status.success(), "{args:?}");
        assert!(out.stdout.is_empty(), "{args:?}");
        let stderr = String::from_utf8(out.stderr).unwrap();
        let unknown = args.iter().find(|a| a.starts_with('-')).unwrap();
        assert_eq!(
            stderr,
            format!("vectorcraft: unknown option {unknown}; see vectorcraft --help, or put -- before a file whose name starts with '-'\n"),
            "{args:?}"
        );
    }
    prints_usage(&["--safe-mod", "--help"], &[]);
    // After `--`, a name starting with '-' is a file: the run goes on to the control port, which
    // `run` makes fail.
    let out = run(&["--", "-x.svg"], &[]);
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("VECTORCRAFT_CONTROL_PORT") && !stderr.contains("unknown option"), "{stderr}");
}
