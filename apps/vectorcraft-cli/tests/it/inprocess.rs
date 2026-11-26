//! `vectorcraft_cli::run` called directly: the exit code and what went to each stream.

use serde_json::Value;

/// What `run` returned and wrote.
struct Invoke {
    code: i32,
    stdout: String,
    stderr: String,
}

fn invoke(args: &[&str]) -> Invoke {
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let code = vectorcraft_cli::run(args, &mut stdout, &mut stderr);
    Invoke { code, stdout: String::from_utf8(stdout).unwrap(), stderr: String::from_utf8(stderr).unwrap() }
}

#[test]
fn help_and_version_go_to_stdout_with_exit_0() {
    for args in [&["--help"][..], &["help", "convert"], &["info", "-h"], &["--version"]] {
        let r = invoke(args);
        assert!(r.code == 0 && r.stderr.is_empty() && !r.stdout.is_empty(), "{args:?}: {}", r.stderr);
    }
}

#[test]
fn no_arguments_is_a_usage_error_with_the_help_on_stderr() {
    let r = invoke(&[]);
    assert!(r.code == 2 && r.stdout.is_empty() && r.stderr.contains("Usage: vectorcraft-cli"), "{}", r.stderr);
}

#[test]
fn a_mistake_is_a_usage_error_on_stderr() {
    let r = invoke(&["convert", "a.svg", "b.png", "--artbord"]);
    assert!(r.code == 2 && r.stdout.is_empty() && r.stderr.starts_with("error: unexpected argument '--artbord'"), "{}", r.stderr);
}

#[test]
fn a_failure_is_exit_1_with_the_error_on_stderr() {
    let r = invoke(&["info", "/nonexistent/vectorcraft-cli-test.svg"]);
    assert!(r.code == 1 && r.stdout.is_empty() && r.stderr.starts_with("error: open /nonexistent/vectorcraft-cli-test.svg:"), "{}", r.stderr);
    let r = invoke(&["run", "--cmd", "nope.nothing"]);
    assert!(r.code == 1 && r.stderr.starts_with("error: step 0 (nope.nothing):"), "{}", r.stderr);
}

#[test]
fn run_prints_one_json_line_per_step() {
    let r = invoke(&["run", "--cmd", "file.new", "--params", r#"{"width":10,"height":10}"#, "--cmd", "select.all"]);
    assert!(r.code == 0 && r.stderr.is_empty(), "{}", r.stderr);
    let lines: Vec<Value> = r.stdout.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let ran: Vec<&str> = lines.iter().map(|l| l["command"].as_str().unwrap()).collect();
    assert_eq!(ran, ["file.new", "select.all"]);
}

#[test]
fn commands_lists_the_catalogue() {
    let r = invoke(&["commands"]);
    let v: Value = serde_json::from_str(&r.stdout).unwrap();
    assert!(r.code == 0 && v.as_array().is_some_and(|a| a.iter().any(|c| c["id"] == "shape.rectangle")));
}

/// A path that is not Unicode is a usage error, not a panic.
#[cfg(unix)]
#[test]
fn a_path_that_is_not_unicode_is_a_usage_error() {
    use std::os::unix::ffi::OsStringExt;
    let bad = std::ffi::OsString::from_vec(b"\xff.svg".to_vec());
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = vectorcraft_cli::run([std::ffi::OsString::from("info"), bad], &mut out, &mut err);
    assert_eq!(code, 2, "{}", String::from_utf8_lossy(&err));
}
