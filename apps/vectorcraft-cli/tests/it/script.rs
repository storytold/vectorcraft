use std::process::Command;

use serde_json::{Value, json};

/// `run` with `script.run` after `file.new`, with `params`.
fn run_script(params: &Value) -> std::process::Output {
    Command::new(crate::bin()).args(["run", "--cmd", "file.new", "--cmd", "script.run", "--params", &params.to_string()]).output().unwrap()
}

/// `run` runs a script headless: the reply holds the report, with what the script logged and the
/// documents it changed.
#[test]
fn run_gives_the_report_of_a_script() {
    let source = "vc.run('shape.rectangle', {x: 0, y: 0, width: 10, height: 10}); vc.log(vc.args.n * 2);";
    let out = run_script(&json!({"source": source, "name": "double.js", "args": {"n": 21}}));
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let lines: Vec<Value> = String::from_utf8_lossy(&out.stdout).lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(lines[1]["command"], "script.run");
    let report = &lines[1]["result"];
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["name"], "double.js");
    assert_eq!(report["log"], json!([42]), "{report}");
    assert_eq!(report["changed"].as_array().map(Vec::len), Some(1), "{report}");
}

/// A script that throws fails its step with the script's message, and the steps before it ran.
#[test]
fn run_fails_the_step_of_a_script_that_throws() {
    let out = run_script(&json!({"source": "vc.run('shape.rectangle', {x: 0, y: 0, width: 10, height: 10}); throw new Error('no margin given');"}));
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("step 1 (script.run)") && stderr.contains("no margin given"), "{stderr}");
    let printed: Vec<Value> = String::from_utf8_lossy(&out.stdout).lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(printed.len(), 1, "only file.new printed: {printed:?}");
    assert_eq!(printed[0]["command"], "file.new");
}
