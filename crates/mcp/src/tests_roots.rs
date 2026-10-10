//! Confinement tests for `--automation-read-root` / `--automation-write-root` (`#832`):
//! the gates in `dispatch` hold before anything reaches a backend.

use serde_json::{Value, json};

use crate::{FileRoots, Headless, call_tool_confined};

fn tree(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let base = std::env::temp_dir().join(format!("vectorcraft-mcp-roots-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let (inside, outside) = (base.join("inside"), base.join("outside"));
    std::fs::create_dir_all(&inside).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    (inside, outside)
}

fn roots(inside: &std::path::Path) -> FileRoots {
    let dir = inside.to_string_lossy().to_string();
    FileRoots::new(Some(&dir), Some(&dir)).unwrap()
}

fn call(roots: &FileRoots, name: &str, args: Value) -> crate::ToolResult {
    let mut h = Headless::with_document();
    call_tool_confined(&mut h, roots, name, &args)
}

fn err_text(r: &crate::ToolResult) -> String {
    assert!(r.is_error, "{r:?}");
    r.content.iter().filter_map(|c| c.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n")
}

#[test]
fn confined_open_file_reads_inside_and_refuses_outside() {
    let (inside, outside) = tree("open-file");
    let roots = roots(&inside);
    std::fs::write(inside.join("art.svg"), r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"></svg>"#).unwrap();
    std::fs::write(outside.join("secret.svg"), r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"></svg>"#).unwrap();
    let ok = call(&roots, "open_file", json!({"path": inside.join("art.svg").to_string_lossy()}));
    assert!(!ok.is_error, "{ok:?}");
    let err = err_text(&call(&roots, "open_file", json!({"path": outside.join("secret.svg").to_string_lossy()})));
    assert!(err.contains("outside --automation-read-root"), "{err}");
}

#[test]
fn confined_export_writes_inside_and_refuses_outside() {
    let (inside, outside) = tree("export");
    let roots = roots(&inside);
    let target = inside.join("out.svg");
    let ok = call(&roots, "export", json!({"format": "svg", "path": target.to_string_lossy()}));
    assert!(!ok.is_error, "{ok:?}");
    assert!(target.exists());
    let err = err_text(&call(&roots, "export", json!({"format": "svg", "path": outside.join("out.svg").to_string_lossy()})));
    assert!(err.contains("outside --automation-write-root"), "{err}");
    assert!(!outside.join("out.svg").exists());
    // Without a path the bytes come back inline: no filesystem, allowed.
    let inline = call(&roots, "export", json!({"format": "svg"}));
    assert!(!inline.is_error, "{inline:?}");
}

#[test]
fn confined_save_without_a_path_is_rejected() {
    let (inside, _) = tree("save-pathless");
    let roots = roots(&inside);
    let err = err_text(&call(&roots, "save_file", json!({})));
    assert!(err.contains("explicit `path`"), "{err}");
}

#[test]
fn confined_command_run_gates_file_commands_but_not_paint() {
    let (inside, outside) = tree("command-run");
    let roots = roots(&inside);
    std::fs::write(inside.join("art.svg"), r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"></svg>"#).unwrap();
    let err =
        err_text(&call(&roots, "run_command", json!({"command": "document.open", "params": {"path": outside.join("x.svg").to_string_lossy()}})));
    assert!(err.contains("outside --automation-read-root"), "{err}");
    let ok = call(&roots, "run_command", json!({"command": "document.open", "params": {"path": inside.join("art.svg").to_string_lossy()}}));
    assert!(!ok.is_error, "{ok:?}");
    // Non-file commands pass through to the engine untouched (this one succeeds).
    let ok = call(&roots, "run_command", json!({"command": "shape.rectangle", "params": {"x": 10, "y": 10, "width": 80, "height": 40}}));
    assert!(!ok.is_error, "{ok:?}");
}

#[test]
fn confined_batch_stops_at_a_smuggled_open() {
    let (inside, outside) = tree("batch");
    let roots = roots(&inside);
    let r = call(
        &roots,
        "command_batch",
        json!({"steps": [
            {"id": "shape.rectangle", "params": {"x": 1, "y": 1, "width": 5, "height": 5}},
            {"id": "document.open", "params": {"path": outside.join("x.svg").to_string_lossy()}},
        ]}),
    );
    assert!(r.is_error, "{r:?}");
}

#[test]
fn unconfined_server_keeps_legacy_behaviour() {
    let (inside, _) = tree("legacy");
    let roots = FileRoots::unconstrained();
    std::fs::write(inside.join("art.svg"), r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"></svg>"#).unwrap();
    let ok = call(&roots, "open_file", json!({"path": inside.join("art.svg").to_string_lossy()}));
    assert!(!ok.is_error, "{ok:?}");
}
