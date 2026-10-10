//! `vectorcraft --control <port> --automation-read-root <dir> --automation-write-root <dir>` (#832):
//! control requests read and write only inside the roots; the person at the keyboard is not
//! confined.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_engine::file_access::AutomationRoots;

use crate::control::ControlRequest;
use crate::{Services, VectorcraftApp};

/// A folder holding `in` (the read root, with `a.svg`), `out` (the write root) and `outside`
/// (with `b.svg`), and an app with the desktop's file services confined to `in` and `out`.
fn confined(tag: &str) -> (VectorcraftApp, std::sync::mpsc::Sender<ControlRequest>, PathBuf) {
    let base = std::env::temp_dir().join(format!("vc-ui-roots-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    for d in ["in", "out", "outside"] {
        std::fs::create_dir_all(base.join(d)).unwrap();
    }
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="30"><rect width="20" height="10"/></svg>"#;
    std::fs::write(base.join("in/a.svg"), svg).unwrap();
    std::fs::write(base.join("outside/b.svg"), svg).unwrap();
    let services = Services {
        read: Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string()))),
        write: Some(Box::new(|p: &str, b: &[u8]| vectorcraft_format::write_atomic(Path::new(p), b).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    let roots = AutomationRoots::new(Some(&base.join("in")), Some(&base.join("out"))).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let app = VectorcraftApp::new(Session::new(), services).with_control(rx).with_automation_roots(roots);
    (app, tx, base)
}

fn s(p: PathBuf) -> String {
    p.to_string_lossy().into_owned()
}

/// One request through the control channel, as the app's frame drains it.
fn request(app: &mut VectorcraftApp, tx: &std::sync::mpsc::Sender<ControlRequest>, method: &str, params: Value) -> Value {
    let (req, reply) = ControlRequest::new(method, params);
    tx.send(req).unwrap();
    app.drain_control(&egui::Context::default());
    reply.try_recv().unwrap()
}

#[test]
fn control_requests_stay_inside_the_roots() {
    let (mut app, tx, base) = confined("control");
    let r = request(&mut app, &tx, "app.open", json!({"path": s(base.join("in/a.svg"))}));
    assert_eq!(r["ok"], true, "{r}");
    let r = request(&mut app, &tx, "app.open", json!({"path": s(base.join("outside/b.svg"))}));
    assert!(r["error"].as_str().unwrap().contains("outside the read root"), "{r}");
    let r = request(&mut app, &tx, "engine.execute", json!({"command": "file.place", "params": {"path": s(base.join("outside/b.svg"))}}));
    assert!(r["error"].as_str().unwrap().contains("outside the read root"), "{r}");

    let r = request(&mut app, &tx, "app.export", json!({"path": s(base.join("out/a.png")), "format": "png"}));
    assert_eq!(r["ok"], true, "{r}");
    assert!(base.join("out/a.png").is_file());
    for (method, params) in [
        ("app.export", json!({"path": s(base.join("outside/a.png")), "format": "png"})),
        ("app.save", json!({"path": s(base.join("outside/a.vectorcraft"))})),
        ("ui.render", json!({"path": s(base.join("outside/r.png"))})),
        // Refused at once, not when the frame comes.
        ("ui.screenshot", json!({"path": s(base.join("outside/w.png"))})),
    ] {
        let r = request(&mut app, &tx, method, params);
        assert!(r["error"].as_str().is_some_and(|e| e.contains("outside the write root")), "{method}: {r}");
    }
    let r = request(&mut app, &tx, "ui.render", json!({"path": s(base.join("out/r.png")), "data": true}));
    assert!(r["result"]["pngBase64"].is_string() && base.join("out/r.png").is_file(), "{r}");
    assert_eq!(std::fs::read_dir(base.join("outside")).unwrap().count(), 1, "nothing written outside");

    // The person at the keyboard isn't confined.
    assert!(app.run("file.open", json!({"path": s(base.join("outside/b.svg"))})).is_ok());
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn frames_with_injected_input_are_confined() {
    let (mut app, tx, base) = confined("synthetic");
    let r = request(&mut app, &tx, "ui.key", json!({"key": "Escape"}));
    assert_eq!(r["ok"], true, "{r}");
    let mut raw = egui::RawInput::default();
    app.raw_input_hook(&mut raw);
    assert!(app.synthetic_frame, "the frame carrying the key");
    let mut raw = egui::RawInput::default();
    app.raw_input_hook(&mut raw);
    assert!(!app.synthetic_frame, "the frames after it are the person's");
    let _ = std::fs::remove_dir_all(base);
}
