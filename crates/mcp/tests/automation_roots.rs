//! `vectorcraft-cli mcp --automation-read-root <dir> --automation-write-root <dir>` (#832): through
//! the MCP protocol, files open only from inside the read root and are saved, exported and
//! captured only inside the write root; paths that leave a root (`..`, another folder, a sibling
//! with the same prefix) are refused, and nothing is written outside.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use vectorcraft_engine::file_access::AutomationRoots;
use vectorcraft_mcp::{Headless, Server};

const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="30"><rect width="20" height="10" fill="red"/></svg>"#;

/// A folder holding `in` (the read root, with `a.svg`), `out` (the write root), `outside` (with
/// `b.svg`) and `in2`, and a server confined to `in` and `out` (either may be left out).
fn confined(tag: &str, read: bool, write: bool) -> (Server, PathBuf) {
    let base = vectorcraft_testkit::temp_dir(&format!("roots-{tag}"));
    for d in ["in", "out", "outside", "in2"] {
        std::fs::create_dir_all(base.join(d)).unwrap();
    }
    std::fs::write(base.join("in/a.svg"), SVG).unwrap();
    std::fs::write(base.join("outside/b.svg"), SVG).unwrap();
    std::fs::write(base.join("in2/c.svg"), SVG).unwrap();
    let in_dir = base.join("in");
    let out_dir = base.join("out");
    let roots = AutomationRoots::new(read.then_some(in_dir.as_path()), write.then_some(out_dir.as_path())).unwrap();
    (Server::new(Box::new(Headless::with_document().with_automation_roots(roots))), base)
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// `tools/call` → (isError, the first text block).
fn call(server: &mut Server, name: &str, args: Value) -> (bool, String) {
    let line = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}}).to_string();
    let reply: Value = serde_json::from_str(&server.handle_line(&line).unwrap()).unwrap();
    let result = &reply["result"];
    let text = result["content"].as_array().unwrap().iter().find_map(|c| c["text"].as_str()).unwrap_or_default().to_string();
    (result["isError"] == true, text)
}

fn ok(server: &mut Server, name: &str, args: Value) -> String {
    let (error, text) = call(server, name, args.clone());
    assert!(!error, "{name} {args}: {text}");
    text
}

fn refused(server: &mut Server, name: &str, args: Value, why: &str) {
    let (error, text) = call(server, name, args.clone());
    assert!(error && text.contains(why), "{name} {args} should be refused ({why}): {text}");
}

#[test]
fn files_open_only_from_inside_the_read_root() {
    let (mut server, base) = confined("open", true, true);
    let sep = std::path::MAIN_SEPARATOR;
    ok(&mut server, "open_file", json!({"path": s(&base.join("in/a.svg"))}));
    ok(&mut server, "run_command", json!({"command": "file.place", "params": {"path": s(&base.join("in/a.svg"))}}));
    refused(&mut server, "open_file", json!({"path": s(&base.join("outside/b.svg"))}), "outside the read root");
    refused(&mut server, "open_file", json!({"path": format!("{}{sep}..{sep}outside{sep}b.svg", s(&base.join("in")))}), "outside the read root");
    refused(&mut server, "open_file", json!({"path": s(&base.join("in2/c.svg"))}), "outside the read root");
    refused(
        &mut server,
        "run_command",
        json!({"command": "file.place", "params": {"path": s(&base.join("outside/b.svg"))}}),
        "outside the read root",
    );
    // The write root isn't readable: the two kinds of access are separate.
    std::fs::write(base.join("out/d.svg"), SVG).unwrap();
    refused(&mut server, "open_file", json!({"path": s(&base.join("out/d.svg"))}), "outside the read root");
    // A relative path is the working directory's (the crate's folder here): outside.
    refused(&mut server, "open_file", json!({"path": "Cargo.toml"}), "outside the read root");
}

#[test]
fn saves_exports_and_captures_go_only_into_the_write_root() {
    let (mut server, base) = confined("write", true, true);
    let sep = std::path::MAIN_SEPARATOR;
    ok(&mut server, "draw_shape", json!({"shape": "rectangle", "x": 10, "y": 10, "width": 50, "height": 40, "fill": "#1e88e5"}));

    ok(&mut server, "export", json!({"path": s(&base.join("out/art.png"))}));
    assert!(base.join("out/art.png").is_file());
    ok(&mut server, "export", json!({"path": s(&base.join("out/art.svg"))}));
    ok(&mut server, "save_file", json!({"path": s(&base.join("out/doc.vectorcraft"))}));
    ok(&mut server, "screenshot", json!({"path": s(&base.join("out/shot.png"))}));
    assert!(base.join("out/shot.png").is_file(), "the backend wrote the capture");

    refused(&mut server, "export", json!({"path": s(&base.join("in/art.png"))}), "outside the write root");
    refused(&mut server, "export", json!({"path": s(&base.join("outside/art.svg"))}), "outside the write root");
    refused(&mut server, "export", json!({"path": format!("{}{sep}..{sep}outside{sep}up.png", s(&base.join("out")))}), "outside the write root");
    refused(&mut server, "save_file", json!({"path": s(&base.join("outside/doc.vectorcraft"))}), "outside the write root");
    refused(&mut server, "screenshot", json!({"path": s(&base.join("outside/shot.png"))}), "outside the write root");
    refused(
        &mut server,
        "run_command",
        json!({"command": "document.exportForScreens", "params": {"folder": s(&base.join("outside/screens"))}}),
        "outside the write root",
    );
    refused(
        &mut server,
        "run_command",
        json!({"command": "file.package", "params": {"folder": s(&base.join("outside/pkg"))}}),
        "outside the write root",
    );
    // Folders the app writes on its own later: only inside the roots.
    refused(
        &mut server,
        "run_command",
        json!({"command": "prefs.set", "params": {"key": "recoveryFolder", "value": s(&base.join("outside"))}}),
        "outside the write root",
    );
    ok(&mut server, "run_command", json!({"command": "prefs.set", "params": {"key": "recoveryFolder", "value": s(&base.join("out"))}}));
    refused(&mut server, "run_command", json!({"command": "plugin.reload", "params": {"path": s(&base.join("outside"))}}), "outside the read root");

    // The document saved inside the write root saves again in place.
    ok(&mut server, "save_file", json!({}));
    let written: Vec<String> =
        std::fs::read_dir(base.join("outside")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(written, ["b.svg"], "nothing was written outside the roots");
    assert!(!base.join("in/art.png").exists());
}

#[test]
fn a_root_left_out_grants_none_of_its_access() {
    let (mut server, base) = confined("read-only", true, false);
    ok(&mut server, "open_file", json!({"path": s(&base.join("in/a.svg"))}));
    refused(&mut server, "export", json!({"path": s(&base.join("out/art.png"))}), "write authority is absent");
    // Bytes that come back instead of a file are fine.
    ok(&mut server, "export", json!({"format": "svg"}));

    let (mut server, base) = confined("write-only", false, true);
    refused(&mut server, "open_file", json!({"path": s(&base.join("in/a.svg"))}), "read authority is absent");
    ok(&mut server, "export", json!({"path": s(&base.join("out/art.png"))}));
}

#[test]
fn links_out_of_a_root_are_refused() {
    let (mut server, base) = confined("links", true, true);
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(base.join("outside"), base.join("out/away")).is_ok();
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_dir(base.join("outside"), base.join("out/away")).is_ok();
    #[cfg(not(any(unix, windows)))]
    let linked = false;
    // Windows needs Developer Mode or elevation to make links.
    if !linked {
        eprintln!("skipped: symbolic links can't be made here");
        return;
    }
    refused(&mut server, "export", json!({"path": s(&base.join("out/away/art.png"))}), "outside the write root");
    assert!(!base.join("outside/art.png").exists());
}
