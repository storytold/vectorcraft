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

/// A shortcut set another app saved (`.kys`) is read only from inside the read root, whichever way a
/// control request names it: `shortcutSet.read`, Import Shortcut Set, Keyboard Shortcuts' Import…
/// and File › Open.
#[test]
fn shortcut_sets_are_read_only_inside_the_read_root() {
    let (mut app, tx, base) = confined("kys");
    let set = "/Menus {\n /Acme\\ Undo { /Modifiers 0 /Key 48 }\n}\n";
    std::fs::write(base.join("in/a.kys"), set).unwrap();
    std::fs::write(base.join("outside/b.kys"), set).unwrap();
    let execute = |command: &str, path: PathBuf| json!({"command": command, "params": {"path": s(path)}});
    let r = request(&mut app, &tx, "engine.execute", execute("shortcutSet.read", base.join("in/a.kys")));
    assert_eq!(r["result"]["entries"][0]["name"], "Acme Undo", "{r}");
    for (method, params) in [
        ("engine.execute", execute("shortcutSet.read", base.join("outside/b.kys"))),
        ("engine.execute", execute("ui.shortcutImportDialog", base.join("outside/b.kys"))),
        ("engine.execute", execute("shortcuts.import", base.join("outside/b.kys"))),
        ("app.open", json!({"path": s(base.join("outside/b.kys"))})),
    ] {
        let r = request(&mut app, &tx, method, params.clone());
        assert!(r["error"].as_str().is_some_and(|e| e.contains("outside the read root")), "{method} {params}: {r}");
        assert!(app.ui.dialog.is_none(), "{method} {params}: no dialog opens");
    }
    let r = request(&mut app, &tx, "engine.execute", execute("ui.shortcutImportDialog", base.join("in/a.kys")));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(crate::dialogs::shortcut_import::KIND));
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

/// Edit Original and the Attributes panel's Browser button hand a file or URL to another app: a
/// control request (or MCP, which comes through it) can't run them, roots or not.
#[test]
fn automation_cannot_hand_files_or_urls_to_other_apps() {
    let base = std::env::temp_dir().join(format!("vc-ui-handoff-{}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    let png = base.join("linked.png");
    image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 0, 255])).save(&png).unwrap();
    let opened = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let (u, f) = (opened.clone(), opened.clone());
    let services = Services {
        open_url: Some(Box::new(move |p: &str| u.borrow_mut().push(p.to_string()))),
        open_file: Some(Box::new(move |p: &str| {
            f.borrow_mut().push(p.to_string());
            Ok(())
        })),
        ..Default::default()
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = VectorcraftApp::new(Session::new(), services).with_control(rx);
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let image = app.run("file.place", json!({"path": s(png.clone())})).unwrap()["ids"][0].as_u64().unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap();
    app.run("attributes.set", json!({"url": "https://example.com/a"})).unwrap();
    for (command, params) in [("attributes.openUrl", json!({})), ("links.editOriginal", json!({"id": image}))] {
        let r = request(&mut app, &tx, "engine.execute", json!({"command": command, "params": params}));
        assert!(r["error"].as_str().is_some_and(|e| e.contains("only the person at the keyboard")), "{command}: {r}");
    }
    assert!(opened.borrow().is_empty());
    // The person at the keyboard can.
    app.run("attributes.openUrl", json!({})).unwrap();
    app.run("links.editOriginal", json!({"id": image})).unwrap();
    assert_eq!(opened.borrow().len(), 2);
    let _ = std::fs::remove_dir_all(base);
}
