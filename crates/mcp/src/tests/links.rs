//! Linked images over MCP, headless: a document opened without its linked file reports it, and
//! `links.relink` / `links.check` reach it through `run_command`.

use serde_json::{Value, json};

use crate::{Headless, call_tool};

fn run(h: &mut Headless, command: &str, params: Value) -> Value {
    let r = call_tool(h, "run_command", &json!({"command": command, "params": params}));
    assert!(!r.is_error, "{command}: {r:?}");
    serde_json::from_str(r.content[0]["text"].as_str().unwrap()).unwrap()
}

#[test]
fn headless_links_report_missing_files_and_relink() {
    let dir = std::env::temp_dir().join(format!("vectorcraft-mcp-links-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = |n: &str| dir.join(n).to_string_lossy().into_owned();
    let mut h = Headless::new();
    run(&mut h, "file.new", json!({"width": 300, "height": 200}));
    let png = run(&mut h, "document.serialize", json!({"format": "png"}));
    std::fs::write(file("a.png"), vectorcraft_format::base64_decode(png["dataBase64"].as_str().unwrap()).unwrap()).unwrap();
    std::fs::write(file("b.png"), vectorcraft_format::base64_decode(png["dataBase64"].as_str().unwrap()).unwrap()).unwrap();
    let id = run(&mut h, "file.place", json!({"path": file("a.png")}))["ids"][0].clone();
    run(&mut h, "document.save", json!({"path": file("doc.vectorcraft")}));
    std::fs::remove_file(file("a.png")).unwrap();
    let r = run(&mut h, "document.open", json!({"path": file("doc.vectorcraft")}));
    assert_eq!(r["missingLinks"][0]["ids"], json!([id]), "{r}");
    assert_eq!(run(&mut h, "links.check", json!({}))["missing"], 1);
    let r = run(&mut h, "links.relink", json!({"ids": [id], "path": file("b.png")}));
    assert_eq!(r["relinked"], json!([id]));
    assert_eq!(run(&mut h, "links.check", json!({}))["links"][0]["status"], "ok");
    let _ = std::fs::remove_dir_all(dir);
}
