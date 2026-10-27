//! `script.check`: scripts written for the reference app checked without running them, from a path,
//! text or base64; the files their `#include` lines name read from the script's folder when asked,
//! through the automation read root; and the limits.

use serde_json::{Value, json};

use crate::*;

fn check(p: Value) -> Value {
    Session::new().execute("script.check", &p).unwrap_or_else(|e| panic!("script.check {p}: {e}"))
}

/// A fresh folder for the files of test `name`.
fn folder(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vc-script-check-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("scripts/lib")).unwrap();
    dir
}

fn statuses(r: &Value) -> Vec<&str> {
    r["includes"].as_array().unwrap().iter().map(|i| i["status"].as_str().unwrap()).collect()
}

#[test]
fn a_script_is_reported_without_running_it() {
    let src = concat!(
        "#target acme-12\n",
        "var doc = app.activeDocument;\n",
        "app.executeMenuCommand('acmecopyall');\n",
        "var f = new File(p); f.open('w');\n",
        "alert('done');\n",
    );
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let undo = s.doc().unwrap().history.undo.len();
    let r = s.execute("script.check", &json!({"data": src, "name": "/Users/me/Tidy Layers.jsx"})).unwrap();
    assert_eq!((r["name"].as_str(), r["kind"].as_str(), r["lines"].as_u64()), (Some("Tidy Layers.jsx"), Some("script"), Some(5)));
    assert_eq!(r["needs"], json!(["language", "dom", "files", "menuCommands"]));
    assert_eq!(r["targets"], json!([{"value": "acme-12", "app": "acme", "version": "12", "line": 1}]));
    assert_eq!(r["directives"], json!([{"name": "target", "value": "acme-12", "line": 1, "form": "#"}]));
    assert_eq!(r["menuCommands"], json!({"strings": [{"value": "acmecopyall", "count": 1, "lines": [3]}], "dynamic": 0}));
    assert_eq!(r["outside"], json!([{"id": "filesWritten", "count": 1, "lines": [4], "modes": ["w"]}]));
    assert_eq!(r["dom"], json!({"members": [{"path": "app.activeDocument", "count": 1}], "types": []}));
    let notify = r["features"].as_array().unwrap().iter().find(|f| f["id"] == "notify").unwrap();
    assert_eq!(notify, &json!({"id": "notify", "count": 1, "lines": [5]}));
    assert_eq!((r["problems"].clone(), r["includes"].clone(), r["truncated"].clone()), (json!([]), json!([]), json!(false)));
    // Nothing ran: the document has no new undo step, and the check isn't recorded.
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    assert!(!find_command("script.check").unwrap().journal);
}

#[test]
fn a_path_text_and_base64_give_one_report() {
    let dir = folder("forms");
    let src = "#include 'lib/x.jsxinc'\nvar s = 'open;\n";
    let path = dir.join("scripts/Tidy.jsx");
    std::fs::write(&path, src).unwrap();
    let by_path = check(json!({"path": path.to_string_lossy()}));
    assert_eq!(by_path["name"], "Tidy.jsx");
    assert_eq!(by_path["problems"], json!([{"code": "unterminatedString", "line": 2}]));
    assert_eq!(check(json!({"data": src, "name": "Tidy.jsx"})), by_path);
    assert_eq!(check(json!({"dataBase64": vectorcraft_format::base64_encode(src.as_bytes()), "name": "Tidy.jsx"})), by_path);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn includes_are_read_from_the_scripts_folder_only_when_asked() {
    let dir = folder("includes");
    std::fs::write(dir.join("scripts/main.jsx"), "#include 'lib/net.jsxinc'\n#include '../outside.jsxinc'\n#include 'lib/none.jsxinc'\n").unwrap();
    std::fs::write(dir.join("scripts/lib/net.jsxinc"), "var s = new Socket();\n").unwrap();
    std::fs::write(dir.join("outside.jsxinc"), "alert('outside');\n").unwrap();
    let path = dir.join("scripts/main.jsx").to_string_lossy().into_owned();
    let r = check(json!({"path": path}));
    assert_eq!(statuses(&r), ["notRead", "notRead", "notRead"]);
    assert_eq!(r["needs"], json!(["language"]));
    let r = check(json!({"path": path, "includes": true}));
    assert_eq!(statuses(&r), ["checked", "notFollowed", "missing"]);
    assert_eq!(r["includes"][0]["path"], "lib/net.jsxinc");
    assert_eq!(r["needs"], json!(["language", "hostOnly"]));
    assert_eq!(r["outside"], json!([{"id": "network", "count": 1, "lines": []}]), "an included file's lines aren't the script's");
    // Text has no folder to read from.
    let r = check(json!({"data": "#include 'lib/net.jsxinc'\n", "includes": true}));
    assert_eq!(statuses(&r), ["notRead"]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_name_given_with_a_path_names_the_reply_and_the_includes_follow_the_path() {
    let dir = folder("named");
    std::fs::write(dir.join("scripts/main.jsx"), "#include 'helper.jsx'\n").unwrap();
    std::fs::write(dir.join("scripts/helper.jsx"), "var s = new Socket();\n").unwrap();
    let path = dir.join("scripts/main.jsx").to_string_lossy().into_owned();
    let r = check(json!({"path": path, "name": "helper.jsx", "includes": true}));
    assert_eq!(r["name"], "helper.jsx");
    assert_eq!(statuses(&r), ["checked"]);
    assert_eq!(r["needs"], json!(["language", "hostOnly"]));
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
fn links_out_of_the_folder_and_the_automation_read_root_keep_reads_inside() {
    let dir = folder("root");
    std::fs::write(dir.join("scripts/main.jsx"), "#include 'lib/link.jsxinc'\n#include 'lib/ok.jsxinc'\n").unwrap();
    std::fs::write(dir.join("scripts/lib/ok.jsxinc"), "alert(1);\n").unwrap();
    std::fs::write(dir.join("secret.jsxinc"), "var s = new Socket();\n").unwrap();
    std::os::unix::fs::symlink(dir.join("secret.jsxinc"), dir.join("scripts/lib/link.jsxinc")).unwrap();
    std::fs::write(dir.join("other.jsx"), "alert(2);\n").unwrap();
    let main = dir.join("scripts/main.jsx").to_string_lossy().into_owned();
    // A link that leads out of the script's folder isn't followed.
    let r = check(json!({"path": main, "includes": true}));
    assert_eq!(statuses(&r), ["notFollowed", "checked"]);
    assert_eq!(r["outside"], json!([]));
    // Under a read root, the script and its includes are read inside it, and a script outside isn't.
    let roots = crate::file_access::AutomationRoots::new(&[&dir.join("scripts")], &[]).unwrap().unwrap();
    let mut s = Session::new();
    let r = crate::file_access::confine(Some(&roots), || s.execute("script.check", &json!({"path": main, "includes": true}))).unwrap();
    assert_eq!(statuses(&r), ["notFollowed", "checked"]);
    let other = dir.join("other.jsx").to_string_lossy().into_owned();
    let e = crate::file_access::confine(Some(&roots), || s.execute("script.check", &json!({"path": other}))).unwrap_err();
    assert!(e.to_string().contains("outside the read root"), "{e}");
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
fn pipes_and_folders_are_not_read() {
    let dir = folder("pipes");
    let pipe = dir.join("scripts/lib/pipe.jsxinc");
    assert!(std::process::Command::new("mkfifo").arg(&pipe).status().unwrap().success());
    std::fs::write(dir.join("scripts/main.jsx"), "#include 'lib/pipe.jsxinc'\n#include 'lib'\n").unwrap();
    let main = dir.join("scripts/main.jsx").to_string_lossy().into_owned();
    let pipe = pipe.to_string_lossy().into_owned();
    // Opening a pipe that has no writer waits for one: the checks run on a thread with a time limit.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut s = Session::new();
        let included = s.execute("script.check", &json!({"path": main, "includes": true})).map(|r| r["includes"].clone());
        let direct = s.execute("script.check", &json!({ "path": pipe })).map_err(|e| e.to_string());
        let _ = tx.send((included.map_err(|e| e.to_string()), direct));
    });
    let (included, direct) = rx.recv_timeout(std::time::Duration::from_secs(20)).expect("the checks return");
    let statuses: Vec<String> = included.unwrap().as_array().unwrap().iter().map(|i| i["status"].as_str().unwrap().to_string()).collect();
    assert_eq!(statuses, ["missing", "missing"]);
    assert!(direct.is_err_and(|e| e.contains("not a file")));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn kinds_limits_and_bad_params() {
    let kind = |p: Value| check(p)["kind"].as_str().unwrap().to_string();
    assert_eq!(kind(json!({"data": "@JSXBIN@ES@2.0@AcmeBody", "name": "a.jsx"})), "compiled");
    assert_eq!(kind(json!({"data": "alert(1);", "name": "a.jsxbin"})), "compiled");
    assert_eq!(kind(json!({"data": "import a from 'a';\nexport default a;\n"})), "module");
    assert_eq!(kind(json!({"dataBase64": vectorcraft_format::base64_encode(b"var a;\0")})), "notText");
    // Past 8 MiB as text and as base64: only the head is decoded.
    let big = "a".repeat((8 << 20) + 1);
    assert_eq!(kind(json!({"data": big})), "tooLarge");
    assert_eq!(kind(json!({"dataBase64": vectorcraft_format::base64_encode(big.as_bytes())})), "tooLarge");
    assert_eq!(kind(json!({"data": &big[1..]})), "script");
    let mut s = Session::new();
    for (p, want) in [
        (json!({}), "give `path`, `data` or `dataBase64`"),
        (json!({"dataBase64": "@@@"}), "bad dataBase64"),
        (json!({"path": std::env::temp_dir().join("vc-no-such-script.jsx").to_string_lossy()}), "vc-no-such-script.jsx"),
    ] {
        let e = s.execute("script.check", &p).unwrap_err().to_string();
        assert!(e.contains(want), "{p}: {e}");
    }
}
