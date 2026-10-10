// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

const BIN: &str = env!("CARGO_BIN_EXE_vectorcraft-cli");

fn tmp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vectorcraft-cli-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

#[test]
fn commands_prints_catalogue() {
    let out = Command::new(BIN).arg("commands").output().unwrap();
    assert!(out.status.success());
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let ids: Vec<&str> = v.as_array().unwrap().iter().filter_map(|c| c["id"].as_str()).collect();
    assert!(ids.contains(&"shape.rectangle") && ids.contains(&"object.group") && ids.contains(&"file.export"));
}

#[test]
fn run_batch_exports() {
    let svg = tmp("batch.svg");
    let png = tmp("batch.png");
    let dc = tmp("batch.vectorcraft");
    let out = Command::new(BIN)
        .args(["run", "--cmd", "file.new", "--params", r#"{"width":200,"height":100}"#])
        .args(["--cmd", "shape.ellipse", "--params", r#"{"x":10,"y":10,"width":80,"height":60}"#])
        .args(["--cmd", "paint.setFill", "--params", r##"{"color":"#3366ff"}"##])
        .args(["--export", svg.to_str().unwrap(), "--export", png.to_str().unwrap(), "--export", dc.to_str().unwrap(), "--scale", "2"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let lines: Vec<Value> = String::from_utf8(out.stdout).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.len(), 6);
    assert!(std::fs::read_to_string(&svg).unwrap().contains("3366ff"));
    let png_bytes = std::fs::read(&png).unwrap();
    assert_eq!(&png_bytes[..4], b"\x89PNG");
    // 200×100 pt at scale 2 → 400×200 px (IHDR width/height).
    assert_eq!(u32::from_be_bytes(png_bytes[16..20].try_into().unwrap()), 400);

    // Re-open the native file and inspect.
    let out = Command::new(BIN).args(["run", "--in", dc.to_str().unwrap(), "--cmd", "document.inspect"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let last: Value = serde_json::from_str(String::from_utf8(out.stdout).unwrap().lines().last().unwrap()).unwrap();
    assert_eq!(last["result"]["artboards"][0]["width"], 200.0);
}

#[test]
fn run_reports_errors() {
    let out = Command::new(BIN).args(["run", "--cmd", "nope.nothing"]).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("nope.nothing"));
    let out = Command::new(BIN).args(["run", "--params", "{}"]).output().unwrap();
    assert!(!out.status.success());
}

#[test]
fn mcp_headless_over_stdio() {
    let mut child = Command::new(BIN).args(["mcp", "--headless"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        for m in [
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"draw_shape","arguments":{"shape":"polygon","cx":100,"cy":100,"radius":50,"sides":5,"fill":"#00aa00"}}}),
            json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"screenshot","arguments":{"scale":0.25}}}),
        ] {
            writeln!(stdin, "{m}").unwrap();
        }
    }
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let replies: Vec<Value> = String::from_utf8(out.stdout).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(replies.iter().map(|r| r["id"].as_u64().unwrap()).collect::<Vec<_>>(), [1, 2, 3, 4]);
    assert_eq!(replies[0]["result"]["serverInfo"]["name"], "vectorcraft");
    assert_eq!(replies[2]["result"]["isError"], false);
    assert_eq!(replies[3]["result"]["content"][0]["type"], "image");
}

/// `mcp --automation-read-root` / `--automation-write-root` (#832): the flags are checked before the
/// server starts, and the process then reads and writes only inside its roots.
#[test]
fn mcp_automation_roots() {
    let base = tmp("roots");
    for d in ["in", "out", "outside"] {
        std::fs::create_dir_all(base.join(d)).unwrap();
    }
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="30"><rect width="20" height="10"/></svg>"#;
    std::fs::write(base.join("in/a.svg"), svg).unwrap();
    std::fs::write(base.join("outside/b.svg"), svg).unwrap();
    let dir = |d: &str| base.join(d).to_string_lossy().into_owned();
    let fails = |args: &[&str], why: &str| {
        let out = Command::new(BIN).args(args).stdin(Stdio::null()).output().unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success() && err.contains(why), "{args:?}: {err}");
    };
    fails(&["mcp", "--automation-read-root", &dir("missing")], "cannot open the automation read root");
    fails(&["mcp", "--automation-read-root", &dir("in"), "--automation-read-root", &dir("out")], "given twice");
    fails(&["mcp", "--automation-write-root"], "needs a folder");
    fails(&["mcp", "--connect", "127.0.0.1:1", &format!("--automation-write-root={}", dir("out"))], "a running app enforces its own roots");

    let mut child = Command::new(BIN)
        .args(["mcp", "--automation-read-root", &dir("in"), &format!("--automation-write-root={}", dir("out"))])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let call = |id: u64, name: &str, args: Value| json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":args}});
    let file = |d: &str, f: &str| base.join(d).join(f).to_string_lossy().into_owned();
    {
        let mut stdin = child.stdin.take().unwrap();
        for m in [
            call(1, "open_file", json!({"path": file("in", "a.svg")})),
            call(2, "open_file", json!({"path": file("outside", "b.svg")})),
            call(3, "export", json!({"path": file("out", "a.png")})),
            call(4, "export", json!({"path": file("in", "a.png")})),
            call(5, "save_file", json!({"path": file("outside", "a.vectorcraft")})),
        ] {
            writeln!(stdin, "{m}").unwrap();
        }
    }
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let replies: Vec<Value> = String::from_utf8(out.stdout).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let errors: Vec<bool> = replies.iter().map(|r| r["result"]["isError"] == true).collect();
    assert_eq!(errors, [false, true, false, true, true], "{replies:?}");
    assert!(replies[1]["result"]["content"][0]["text"].as_str().unwrap().contains("outside the read root"));
    assert!(base.join("out/a.png").is_file() && !base.join("in/a.png").exists() && !base.join("outside/a.vectorcraft").exists());
}

/// End-to-end protocol sweep over stdio against the compiled binary. The in-process
/// tests drive `Server::handle_line` directly, so the binary's own wiring (argument
/// parsing, logger install, stdout pollution, notification order) is untested: this
/// drives the real `mcp --headless` process through prompts, completions, resource
/// templates, logging and the error taxonomy instead.
#[test]
fn mcp_protocol_over_stdio() {
    use std::io::{BufRead, BufReader};
    let mut child =
        Command::new(BIN).args(["mcp", "--headless"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut notes = vec![];

    fn send(stdin: &mut std::process::ChildStdin, v: Value) {
        writeln!(stdin, "{v}").unwrap();
    }
    // Next reply with this id; notifications (no id) are stashed for later.
    fn recv(lines: &mut std::io::Lines<BufReader<std::process::ChildStdout>>, notes: &mut Vec<Value>, id: u64) -> Value {
        loop {
            let line = lines.next().unwrap().unwrap();
            let v: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(v["jsonrpc"], "2.0", "{v}");
            if v.get("id").and_then(|i| i.as_u64()) == Some(id) {
                return v;
            }
            notes.push(v);
        }
    }

    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}),
    );
    let r = recv(&mut lines, &mut notes, 1);
    assert_eq!(r["result"]["serverInfo"]["name"], "vectorcraft");
    assert!(r["result"]["capabilities"]["prompts"].is_object(), "{r}");
    send(&mut stdin, json!({"jsonrpc":"2.0","method":"notifications/initialized"}));

    send(&mut stdin, json!({"jsonrpc":"2.0","id":2,"method":"prompts/list","params":{}}));
    let r = recv(&mut lines, &mut notes, 2);
    assert!(r["result"]["prompts"].as_array().is_some_and(|p| !p.is_empty()), "{r}");
    send(&mut stdin, json!({"jsonrpc":"2.0","id":3,"method":"prompts/get","params":{"name":"poster","arguments":{"brief":"a jazz festival"}}}));
    let r = recv(&mut lines, &mut notes, 3);
    assert!(r["result"]["messages"][0]["content"]["text"].as_str().is_some_and(|t| t.contains("a jazz festival")), "{r}");

    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","id":4,"method":"completion/complete","params":{"ref":{"type":"ref/prompt","name":"export-set"},"argument":{"name":"formats","value":"sv"}}}),
    );
    let r = recv(&mut lines, &mut notes, 4);
    assert!(r["result"]["completion"]["values"].as_array().is_some_and(|v| v.contains(&json!("svg"))), "{r}");

    send(&mut stdin, json!({"jsonrpc":"2.0","id":5,"method":"resources/list","params":{}}));
    assert!(recv(&mut lines, &mut notes, 5)["result"]["resources"].as_array().is_some_and(|v| v.len() == 3));
    send(&mut stdin, json!({"jsonrpc":"2.0","id":6,"method":"resources/templates/list","params":{}}));
    assert!(recv(&mut lines, &mut notes, 6)["result"]["resourceTemplates"].as_array().is_some_and(|v| v.len() >= 4));

    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"draw_shape","arguments":{"shape":"rectangle","x":10,"y":10,"width":60,"height":40}}}),
    );
    let r = recv(&mut lines, &mut notes, 7);
    assert_eq!(r["result"]["isError"], false, "{r}");
    let id: i64 = serde_json::from_str::<Value>(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap()["id"].as_i64().unwrap();

    send(&mut stdin, json!({"jsonrpc":"2.0","id":8,"method":"resources/read","params":{"uri": format!("vectorcraft://object/{id}")}}));
    let r = recv(&mut lines, &mut notes, 8);
    let body: Value = serde_json::from_str(r["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(body["id"].as_i64(), Some(id), "{body}");

    // Unknown resources (-32002) and bad template values (-32602) stay apart.
    send(&mut stdin, json!({"jsonrpc":"2.0","id":9,"method":"resources/read","params":{"uri":"vectorcraft://nope" }}));
    assert_eq!(recv(&mut lines, &mut notes, 9)["error"]["code"], -32002);
    send(&mut stdin, json!({"jsonrpc":"2.0","id":10,"method":"resources/read","params":{"uri":"vectorcraft://object/"}}));
    assert_eq!(recv(&mut lines, &mut notes, 10)["error"]["code"], -32602);

    // Large containers slice through run_command document.node: a truncated level reports
    // childCount, and a bad limit is a tool error, not a crash or a full read.
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","id":20,"method":"tools/call","params":{"name":"draw_shape","arguments":{"shape":"ellipse","x":90,"y":10,"width":30,"height":30}}}),
    );
    assert_eq!(recv(&mut lines, &mut notes, 20)["result"]["isError"], false);
    let run = |id: u64, command: &str, params: Value| json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"run_command","arguments":{"command":command,"params":params}}});
    send(&mut stdin, run(21, "select.all", json!({})));
    recv(&mut lines, &mut notes, 21);
    send(&mut stdin, run(22, "object.group", json!({})));
    let r = recv(&mut lines, &mut notes, 22);
    let group = serde_json::from_str::<Value>(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap()["id"].clone();
    send(&mut stdin, run(23, "document.node", json!({"id": group, "summary": true, "childLimit": 1})));
    let r = recv(&mut lines, &mut notes, 23);
    let sliced: Value = serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(sliced["children"].as_array().map(Vec::len), Some(1), "{sliced}");
    assert_eq!(sliced["childCount"], 2, "{sliced}");
    send(&mut stdin, run(24, "document.node", json!({"id": group, "summary": true, "depth": -1})));
    assert_eq!(recv(&mut lines, &mut notes, 24)["result"]["isError"], true);

    // Skeleton then drill: a depth-0 inspect is the layer skeleton with counts, and
    // document.find locates nodes without dumping the tree.
    send(&mut stdin, run(25, "document.inspect", json!({"depth": 0})));
    let r = recv(&mut lines, &mut notes, 25);
    let skel: Value = serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(skel["layers"][0].get("children").is_none(), "{skel}");
    assert!(skel["artboards"].as_array().is_some_and(|a| !a.is_empty()), "{skel}");
    send(&mut stdin, run(26, "document.find", json!({"kind": "ellipse"})));
    let r = recv(&mut lines, &mut notes, 26);
    let found: Value = serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(found["total"], 1, "{found}");
    assert!(found["matches"][0]["path"].as_array().is_some_and(|p| !p.is_empty()), "{found}");
    send(&mut stdin, run(27, "document.find", json!({})));
    assert_eq!(recv(&mut lines, &mut notes, 27)["result"]["isError"], true);

    // The binary installed its logger: records arrive as notifications/message, and only
    // after the client asked for them.
    send(&mut stdin, json!({"jsonrpc":"2.0","id":11,"method":"logging/setLevel","params":{"level":"debug"}}));
    assert!(recv(&mut lines, &mut notes, 11).get("error").is_none());
    send(&mut stdin, json!({"jsonrpc":"2.0","id":12,"method":"tools/list","params":{}}));
    assert!(recv(&mut lines, &mut notes, 12)["result"]["tools"].as_array().is_some_and(|v| !v.is_empty()));

    // Malformed input is a parse error, and the server keeps serving afterwards.
    stdin.write_all(b"{not json\n").unwrap();
    let parse_err: Value = loop {
        let line = lines.next().unwrap().unwrap();
        let v: Value = serde_json::from_str(&line).unwrap();
        if v.get("error").is_some() {
            break v;
        }
        notes.push(v);
    };
    assert_eq!(parse_err["error"]["code"], -32700);
    assert!(parse_err["id"].is_null(), "{parse_err}");
    send(&mut stdin, json!({"jsonrpc":"2.0","id":13,"method":"ping","params":{}}));
    assert!(recv(&mut lines, &mut notes, 13).get("error").is_none());
    drop(stdin);
    // Drain whatever the debug window queued; every line off the protocol stream parses.
    for line in lines.by_ref() {
        let line = line.unwrap();
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["jsonrpc"], "2.0", "{v}");
        notes.push(v);
    }
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    assert!(!String::from_utf8_lossy(&out.stderr).contains("panicked"));
    let messages = notes.iter().filter(|n| n["method"] == "notifications/message").count();
    assert!(messages > 0, "no log records after logging/setLevel: {notes:?}");
}

/// `vectorcraft-cli commands | head -1` panicked with "failed printing to
/// stdout: Broken pipe (os error 32)" and exit status 101.
#[test]
fn closed_stdout_ends_quietly() {
    use std::io::Read;
    let mut child = Command::new(BIN).arg("commands").stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let mut first = [0u8; 16];
    child.stdout.as_mut().unwrap().read_exact(&mut first).unwrap();
    drop(child.stdout.take());
    let out = child.wait_with_output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success() && !err.contains("panicked"), "{:?}: {err}", out.status);
}
