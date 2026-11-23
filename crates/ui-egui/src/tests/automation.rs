//! `vectorcraft --control <port> --automation-read-root <dir> --automation-write-root <dir>` (#832):
//! control requests read and write only inside the roots; the person at the keyboard is not
//! confined.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_engine::file_access::AutomationRoots;

use crate::background::SharedWriteFn;
use craft::automation::actions::ActionList;
use craft::automation::{ControlRequest, Reply};

use super::picks::{answer_dialog, open_dialogs};
use crate::dialogs::plugin::InstallOrigin;
use crate::{Services, VectorcraftApp};

/// A folder holding `in` (the read root, with `a.svg`), `out` (the write root) and `outside`
/// (with `b.svg`), and an app with the desktop's file services confined to `in` and `out`.
fn confined(tag: &str) -> (VectorcraftApp, std::sync::mpsc::Sender<ControlRequest>, PathBuf) {
    confined_with(tag, None)
}

/// [`confined`], with `write_shared` as the thread-safe writer that Background Save and Background
/// Export use.
fn confined_with(tag: &str, write_shared: Option<SharedWriteFn>) -> (VectorcraftApp, std::sync::mpsc::Sender<ControlRequest>, PathBuf) {
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
        write_shared,
        ..Default::default()
    };
    let roots = AutomationRoots::new(&[&base.join("in")], &[&base.join("out")]).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let app = crate::testing::windowed(VectorcraftApp::new(Session::new(), services).with_control(rx).with_automation_roots(roots));
    (app, tx, base)
}

fn s(p: PathBuf) -> String {
    p.to_string_lossy().into_owned()
}

/// The fields of the top dialog in a `ui.inspect` result's `dialogs` (the top one last).
fn top_dialog_fields(inspected: &Value) -> &Value {
    inspected["dialogs"].as_array().and_then(|dialogs| dialogs.last()).map_or(&Value::Null, |dialog| &dialog["fields"])
}

/// Sets `fields` on the app's top dialog, as a person's edits would.
fn set_top_fields(app: &mut VectorcraftApp, fields: &[(&str, Value)]) {
    let top = open_dialogs(app).pop().expect("an open dialog").id;
    answer_dialog(app, top, fields, false);
}

/// The Actions panel's list of the sets `sets` (`[{name, actions: [{name, steps}]}]`, as preferences
/// save them).
fn action_list(sets: Value) -> ActionList {
    crate::panels::actions::prefs::deserialize(sets).unwrap()
}

/// One request through the control channel, as the app's frame runs it on its window's context (one of
/// its own without a window), with the dialogs' pump after it. A reply that waits for injected input
/// comes once passes of that context have taken it.
fn request(app: &mut VectorcraftApp, tx: &std::sync::mpsc::Sender<ControlRequest>, method: &str, params: Value) -> Value {
    let ctx = app.ui_ctx.clone().unwrap_or_default();
    let (req, reply) = ControlRequest::new(method, params);
    tx.send(req).unwrap();
    for _ in 0..8 {
        app.run_control(&ctx);
        crate::dialogs::host::pump(app, &ctx);
        if let Ok(r) = reply.try_recv() {
            return r;
        }
        ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
    }
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

/// `app.save` (MCP's `save_file`) writes a native file with the save options it is given, without
/// asking for them.
#[test]
fn control_saves_take_the_native_save_options() {
    let (mut app, tx, base) = confined("saveopts");
    app.run("file.new", json!({"width": 100, "height": 50})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    let saved = |app: &mut VectorcraftApp, name: &str, mut params: Value| {
        let path = base.join("out").join(name);
        params["path"] = json!(s(path.clone()));
        let r = request(app, &tx, "app.save", params);
        assert_eq!(r["ok"], true, "{r}");
        assert!(r["result"].get("pending").is_none() && open_dialogs(app).is_empty(), "no dialog opens: {r}");
        std::fs::read(path).unwrap()
    };
    assert_eq!(saved(&mut app, "gz.vectorcraft", json!({"compress": true}))[..2], [0x1f, 0x8b]);
    let v3: Value = serde_json::from_slice(&saved(&mut app, "v3.vectorcraft", json!({"version": 3}))).unwrap();
    assert_eq!(v3["version"], 3);
    assert!(vectorcraft_format::preview(&saved(&mut app, "p.vectorcraft", json!({"preview": true}))).is_some());
    let _ = std::fs::remove_dir_all(base);
}

/// A save without a path writes the document's own file in the write root: `app.save` and
/// `file.save` leave its path out of the reply, since it is outside the read root, and keep a path
/// the client gives.
#[test]
fn control_saves_leave_out_the_documents_file_outside_the_read_root() {
    let (mut app, tx, base) = confined("save-own");
    app.run("file.new", json!({"width": 100, "height": 50})).unwrap();
    let own = s(base.join("out/doc.vectorcraft"));
    app.run("file.saveAs", json!({"path": own})).unwrap();
    for (method, params) in [("app.save", json!({})), ("engine.execute", json!({"command": "file.save", "params": {}}))] {
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        let r = request(&mut app, &tx, method, params);
        assert_eq!(r["ok"], true, "{method}: {r}");
        assert!(r["result"].get("path").is_none() && !r.to_string().contains(&s(base.join("out"))), "{method}: {r}");
        assert_eq!(app.session.active().map(|d| d.is_dirty()), Some(false), "{method}: saved");
    }
    let r = request(&mut app, &tx, "app.save", json!({"path": own}));
    assert_eq!(r["result"]["path"], json!(own), "{r}");
    let _ = std::fs::remove_dir_all(base);
}

/// In one session with a library loaded from outside the read root: the library keeps its file-name
/// id, a reply that names its path shows the file name, a save without a path leaves out the
/// document's file, and a save or export to a path the client gave in the write root keeps that path
/// in `path` and `files`, also in a `command.batch` step's result. A refused path outside the write root is still shown by its file name.
#[test]
fn control_saves_keep_the_given_path_beside_a_hidden_library() {
    let (mut app, tx, base) = confined("save-and-library");
    let library = s(base.join("outside/b.svg"));
    app.ui.color_guide_limit = format!("loaded/{library}");
    app.run("file.new", json!({"width": 100, "height": 50})).unwrap();
    let own = s(base.join("out/doc.vectorcraft"));
    app.run("file.saveAs", json!({"path": own})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    let r = request(&mut app, &tx, "app.save", json!({}));
    assert!(r["result"].get("path").is_none() && !r.to_string().contains(&s(base.join("out"))), "{r}");
    let r = request(&mut app, &tx, "app.save", json!({"path": own}));
    assert_eq!(r["result"]["path"], json!(own), "{r}");
    let split = s(base.join("out/split.vectorcraft"));
    let r = request(&mut app, &tx, "engine.execute", json!({"command": "file.save", "params": {"path": split, "separateArtboards": true}}));
    assert_eq!(r["result"]["path"], json!(split), "{r}");
    assert_eq!(r["result"]["files"][0], json!(split), "{r}");
    let batched = s(base.join("out/batched.vectorcraft"));
    let steps = json!({"commands": [{"command": "document.save", "params": {"path": batched}}]});
    let r = request(&mut app, &tx, "engine.execute", json!({"command": "command.batch", "params": steps}));
    assert_eq!(r["result"]["results"][0]["path"], json!(batched), "{r}");
    let svg = s(base.join("out/a.svg"));
    let r = request(&mut app, &tx, "app.export", json!({"path": svg, "wait": true}));
    assert_eq!(r["result"]["path"], json!(svg), "{r}");
    app.ui.status = format!("from {library}");
    let r = request(&mut app, &tx, "test.status", json!({}));
    assert_eq!(r["result"], "from b.svg", "{r}");
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    assert_eq!(r["result"]["ui"]["color_guide_limit"], "loaded/b.svg", "{r}");
    let refused = s(base.join("outside/c.vectorcraft"));
    let r = request(&mut app, &tx, "app.save", json!({"path": refused, "wait": true}));
    assert!(r["error"].as_str().is_some_and(|e| e.contains("outside the write root") && !e.contains(&refused)), "{r}");
    let _ = std::fs::remove_dir_all(base);
}

/// With Background Save and Background Export on, `app.save` and `app.export` with `wait: true`
/// (MCP's `save_file` in the app) reply with the write root's refusal, and write a file inside it
/// before they reply. Without `wait` the refusal reaches only the status bar.
#[test]
fn control_saves_with_wait_reply_with_the_write_roots_refusal() {
    let shared: SharedWriteFn = std::sync::Arc::new(|p: &str, b: &[u8]| vectorcraft_format::write_atomic(Path::new(p), b).map_err(|e| e.to_string()));
    let (mut app, tx, base) = confined_with("wait", Some(shared));
    app.run("file.new", json!({"width": 100, "height": 50})).unwrap();
    for (method, name) in [("app.save", "a.vectorcraft"), ("app.export", "a.svg")] {
        let r = request(&mut app, &tx, method, json!({"path": s(base.join("outside").join(name)), "wait": true}));
        assert!(r["error"].as_str().is_some_and(|e| e.contains("outside the write root")), "{method}: {r}");
        let path = base.join("out").join(name);
        let r = request(&mut app, &tx, method, json!({"path": s(path.clone()), "wait": true}));
        assert_eq!(r["ok"], true, "{method}: {r}");
        assert!(r["result"].get("background").is_none() && path.is_file(), "{method}: written before the reply: {r}");
    }
    let r = request(&mut app, &tx, "app.save", json!({"path": s(base.join("outside/b.vectorcraft"))}));
    assert_eq!(r["result"]["background"], true, "{r}");
    crate::background::wait_all(&mut app);
    assert!(app.ui.status.contains("outside the write root"), "{}", app.ui.status);
    assert_eq!(std::fs::read_dir(base.join("outside")).unwrap().count(), 1, "nothing written outside");
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
        assert!(open_dialogs(&app).is_empty() && app.shortcuts_dialog.is_none(), "{method} {params}: no dialog opens");
    }
    let r = request(&mut app, &tx, "engine.execute", execute("ui.shortcutImportDialog", base.join("in/a.kys")));
    assert_eq!(r["ok"], true, "{r}");
    assert!(app.shortcuts_dialog.is_some(), "Keyboard Shortcuts opens to review the set");
    let _ = std::fs::remove_dir_all(base);
}

/// `app.export` refuses a key no format takes before anything is written, with a path or without.
#[test]
fn control_export_refuses_an_option_no_format_takes() {
    let (mut app, tx, base) = confined("export-options");
    let r = request(&mut app, &tx, "app.open", json!({"path": s(base.join("in/a.svg"))}));
    assert_eq!(r["ok"], true, "{r}");
    let path = base.join("out/a.pdf");
    let refused = |r: &Value| r["error"].as_str().is_some_and(|e| e.contains("unknown parameter"));
    for mut params in [json!({"format": "pdf", "noSuchOption": true}), json!({"format": "pdf", "marks": {"useDocumentBleed": true}})] {
        let r = request(&mut app, &tx, "app.export", params.clone());
        assert!(refused(&r), "{r}");
        params["path"] = json!(s(path.clone()));
        params["wait"] = json!(true);
        let r = request(&mut app, &tx, "app.export", params);
        assert!(refused(&r), "{r}");
        assert!(!path.exists());
    }
    let r = request(&mut app, &tx, "app.export", json!({"format": "pdf", "path": s(path.clone()), "wait": true}));
    assert_eq!(r["ok"], true, "{r}");
    let _ = std::fs::remove_dir_all(base);
}

/// `app.export`, and `file.exportAs` run as a command, write no SVG of a document that links a
/// JavaScript file outside the read root, and the error names no file or folder; the person at the
/// keyboard exports it with the link as stored.
#[test]
fn control_svg_export_refuses_a_linked_javascript_file_outside_the_read_root() {
    let (mut app, tx, base) = confined("svg-script");
    app.run("file.new", json!({"width": 100, "height": 50})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    let script = s(base.join("outside/menu.js"));
    app.run("svgInteractivity.addFile", json!({"file": script})).unwrap();
    let path = s(base.join("out/a.svg"));
    for (method, params) in [
        ("app.export", json!({"path": path, "format": "svg", "wait": true})),
        ("app.export", json!({"path": s(base.join("out/a.svgz")), "wait": true})),
        ("app.export", json!({"format": "svg"})),
        ("engine.execute", json!({"command": "file.exportAs", "params": {"path": path, "format": "svg"}})),
    ] {
        let r = request(&mut app, &tx, method, params.clone());
        let e = r["error"].as_str().unwrap_or_default();
        assert!(e.contains("a linked JavaScript file is outside the read roots"), "{method} {params}: {r}");
        assert!(!e.contains("menu.js") && !e.contains(&s(base.clone())), "{method} {params}: {r}");
    }
    assert_eq!(std::fs::read_dir(base.join("out")).unwrap().count(), 0, "nothing written");
    // The person at the keyboard isn't confined.
    app.run("file.exportAs", json!({"path": path, "format": "svg"})).unwrap();
    let svg = std::fs::read_to_string(&path).unwrap();
    assert!(svg.contains(&format!("<script xlink:href=\"{script}\"/>")), "{svg}");
    let _ = std::fs::remove_dir_all(base);
}

/// `app.export`, and `file.exportAs` run as a command, write no file that carries the native
/// document (a PDF with Preserve Editing, a native file, a `.ai` or EPS file) of a document that
/// links a JavaScript file outside the read root, and the error names no file or folder; a PDF
/// without Preserve Editing is written, and the person at the keyboard exports the native file.
#[test]
fn control_native_export_refuses_a_linked_javascript_file_outside_the_read_root() {
    let (mut app, tx, base) = confined("native-script");
    app.run("file.new", json!({"width": 100, "height": 50})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    let script = s(base.join("outside/menu.js"));
    app.run("svgInteractivity.addFile", json!({"file": script})).unwrap();
    let out = |name: &str| s(base.join("out").join(name));
    for (method, params) in [
        ("app.export", json!({"path": out("a.pdf"), "wait": true})),
        ("app.export", json!({"path": out("a.ai"), "wait": true})),
        ("app.export", json!({"format": "eps"})),
        ("engine.execute", json!({"command": "file.exportAs", "params": {"path": out("a.vectorcraft"), "format": "vectorcraft"}})),
    ] {
        let r = request(&mut app, &tx, method, params.clone());
        let e = r["error"].as_str().unwrap_or_default();
        assert!(e.contains("a linked JavaScript file is outside the read roots"), "{method} {params}: {r}");
        assert!(!e.contains("menu.js") && !e.contains(&s(base.clone())), "{method} {params}: {r}");
    }
    assert_eq!(std::fs::read_dir(base.join("out")).unwrap().count(), 0, "nothing written");
    let r = request(&mut app, &tx, "app.export", json!({"path": out("b.pdf"), "preserveEditing": false, "wait": true}));
    assert_eq!(r["ok"], true, "{r}");
    // The person at the keyboard isn't confined.
    app.run("file.exportAs", json!({"path": out("a.vectorcraft"), "format": "vectorcraft"})).unwrap();
    let native = vectorcraft_format::load(&std::fs::read(out("a.vectorcraft")).unwrap()).unwrap();
    assert_eq!(native.svg_scripts, [script]);
    let _ = std::fs::remove_dir_all(base);
}

/// `app.export`, and `file.exportAs` run as a command, write no SVG that would name a linked
/// image's file outside the read root (Images: Link, Preserve Editing Capabilities), and the error
/// names no file or folder. Embedded pixels are written; the person at the keyboard exports the
/// link as stored.
#[test]
fn control_svg_export_refuses_a_linked_image_file_outside_the_read_root() {
    let (mut app, tx, base) = confined("svg-image");
    app.run("file.new", json!({"width": 100, "height": 50})).unwrap();
    let photo = s(base.join("outside/photo.png"));
    let mut png = vec![];
    image::RgbaImage::from_pixel(4, 4, image::Rgba([200, 40, 40, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    std::fs::write(&photo, png).unwrap();
    app.run("file.place", json!({"path": photo, "link": true, "at": [50, 25]})).unwrap();
    let path = s(base.join("out/a.svg"));
    for (method, params) in [
        ("app.export", json!({"path": path, "format": "svg", "images": "link", "wait": true})),
        ("app.export", json!({"path": s(base.join("out/a.svgz")), "images": "link", "wait": true})),
        ("app.export", json!({"format": "svg", "images": "link"})),
        ("app.export", json!({"format": "svg", "preserveEditing": true})),
        ("engine.execute", json!({"command": "file.exportAs", "params": {"path": path, "format": "svg", "images": "link"}})),
    ] {
        let r = request(&mut app, &tx, method, params.clone());
        let e = r["error"].as_str().unwrap_or_default();
        assert!(e.contains("a linked image file is outside the read roots"), "{method} {params}: {r}");
        assert!(!e.contains("photo.png") && !e.contains(&s(base.clone())), "{method} {params}: {r}");
    }
    assert_eq!(std::fs::read_dir(base.join("out")).unwrap().count(), 0, "nothing written");
    let r = request(&mut app, &tx, "app.export", json!({"path": path, "format": "svg", "wait": true}));
    assert_eq!(r["ok"], true, "{r}");
    let svg = std::fs::read_to_string(&path).unwrap();
    assert!(svg.contains("data:image/png;base64,") && !svg.contains(&photo), "{svg}");
    // The person at the keyboard isn't confined.
    app.run("file.exportAs", json!({"path": path, "format": "svg", "images": "link"})).unwrap();
    let svg = std::fs::read_to_string(&path).unwrap();
    assert!(svg.contains(&format!("xlink:href=\"{photo}\"")), "{svg}");
    let _ = std::fs::remove_dir_all(base);
}

/// `app.export` with a path writes the file when given an option only another format reads, and
/// its warnings name that option, as `document.export`'s do without a path. `file.exportAs` run
/// as a command does the same; an export that automation did not ask for gets no such note.
#[test]
fn control_export_notes_an_option_the_format_does_not_read() {
    let (mut app, tx, base) = confined("export-unused");
    let r = request(&mut app, &tx, "app.open", json!({"path": s(base.join("in/a.svg"))}));
    assert_eq!(r["ok"], true, "{r}");
    let note = json!("`quality` was not used: PDF files don't read it");
    let path = base.join("out/a.pdf");
    let r = request(&mut app, &tx, "app.export", json!({"format": "pdf", "quality": 80, "path": s(path.clone()), "wait": true}));
    assert_eq!(r["ok"], true, "{r}");
    assert!(path.is_file() && r["result"]["warnings"].as_array().is_some_and(|w| w.contains(&note)), "{r}");
    let r = request(&mut app, &tx, "app.export", json!({"format": "pdf", "quality": 80}));
    assert!(r["result"]["warnings"].as_array().is_some_and(|w| w.contains(&note)), "{r}");
    let r = request(&mut app, &tx, "app.export", json!({"format": "jpg", "quality": 80, "path": s(base.join("out/a.jpg")), "wait": true}));
    assert!(r["result"]["warnings"].as_array().is_some_and(|w| !w.iter().any(|w| w.to_string().contains("was not used"))), "{r}");
    let params = json!({"format": "pdf", "quality": 80, "path": s(base.join("out/b.pdf"))});
    let r = request(&mut app, &tx, "engine.execute", json!({"command": "file.exportAs", "params": params}));
    assert!(r["result"]["warnings"].as_array().is_some_and(|w| w.contains(&note)), "{r}");
    assert!(app.ui.status.contains("`quality` was not used"), "{}", app.ui.status);
    let r = crate::io::export(&mut app, Some("pdf"), Some(s(base.join("out/c.pdf"))), &json!({"quality": 80})).unwrap();
    assert!(r["warnings"].as_array().is_some_and(|w| !w.contains(&note)), "{r}");
    let _ = std::fs::remove_dir_all(base);
}

/// A Background Export replies before the file is written: its reply names the options the format
/// does not read at once, as a background save's does, and has no `warnings` when there are none.
#[test]
fn control_background_export_notes_an_option_the_format_does_not_read() {
    let shared: SharedWriteFn = std::sync::Arc::new(|p: &str, b: &[u8]| vectorcraft_format::write_atomic(Path::new(p), b).map_err(|e| e.to_string()));
    let (mut app, tx, base) = confined_with("export-unused-background", Some(shared));
    app.run("file.new", json!({"width": 100, "height": 50})).unwrap();
    let r = request(&mut app, &tx, "app.export", json!({"format": "pdf", "quality": 80, "path": s(base.join("out/a.pdf"))}));
    assert_eq!(r["result"]["background"], true, "{r}");
    assert_eq!(r["result"]["warnings"], json!(["`quality` was not used: PDF files don't read it"]), "{r}");
    crate::background::wait_all(&mut app);
    let r = request(&mut app, &tx, "app.export", json!({"format": "jpg", "quality": 80, "path": s(base.join("out/a.jpg"))}));
    assert_eq!(r["result"]["background"], true, "{r}");
    assert!(r["result"].get("warnings").is_none(), "{r}");
    crate::background::wait_all(&mut app);
    assert!(base.join("out/a.pdf").is_file() && base.join("out/a.jpg").is_file());
    let _ = std::fs::remove_dir_all(base);
}

/// `app.save`, and `file.save` and `file.exportForScreens` run as commands, refuse a key no save
/// format or no format takes before anything is written.
#[test]
fn control_save_refuses_an_option_no_format_takes() {
    let (mut app, tx, base) = confined("save-options");
    let r = request(&mut app, &tx, "app.open", json!({"path": s(base.join("in/a.svg"))}));
    assert_eq!(r["ok"], true, "{r}");
    let path = base.join("out/a.pdf");
    let refused = |r: &Value| r["error"].as_str().is_some_and(|e| e.contains("unknown parameter"));
    for params in [json!({"noSuchOption": true}), json!({"options": {"marks": {"useDocumentBleed": true}}})] {
        let mut params = params;
        params["path"] = json!(s(path.clone()));
        let r = request(&mut app, &tx, "engine.execute", json!({"command": "file.save", "params": params.clone()}));
        assert!(refused(&r), "{r}");
        assert!(!path.exists());
        params["wait"] = json!(true);
        let r = request(&mut app, &tx, "app.save", params);
        assert!(refused(&r), "{r}");
        assert!(!path.exists());
    }
    let folder = base.join("out/screens");
    for params in [json!({"noSuchOption": true}), json!({"formats": [{"format": "png", "noSuchOption": 1}]})] {
        let mut params = params;
        params["folder"] = json!(s(folder.clone()));
        let r = request(&mut app, &tx, "engine.execute", json!({"command": "file.exportForScreens", "params": params}));
        assert!(refused(&r), "{r}");
        assert!(!folder.exists());
    }
    let r = request(&mut app, &tx, "app.save", json!({"path": s(path.clone()), "wait": true}));
    assert_eq!(r["ok"], true, "{r}");
    assert!(path.exists());
    let _ = std::fs::remove_dir_all(base);
}

/// An action set another app saved (`.aia`) is read only from inside the read root, whichever way a
/// control request names it: `actionSet.read` and Load Actions….
#[test]
fn action_sets_are_read_only_inside_the_read_root() {
    let (mut app, tx, base) = confined("aia");
    let set = "/version 3\n/name [ 4 4d696e65 ]\n/action-1 { /name [ 1 41 ] }\n";
    std::fs::write(base.join("in/a.aia"), set).unwrap();
    std::fs::write(base.join("outside/b.aia"), set).unwrap();
    let execute = |command: &str, path: PathBuf| json!({"command": command, "params": {"path": s(path)}});
    let r = request(&mut app, &tx, "engine.execute", execute("actionSet.read", base.join("in/a.aia")));
    assert_eq!(r["result"]["set"], "Mine", "{r}");
    let r = request(&mut app, &tx, "engine.execute", execute(crate::panels::actions::LOAD_SET, base.join("in/a.aia")));
    assert_eq!(r["result"]["name"], "Mine", "{r}");
    let sets = app.ui.actions.sets().len();
    for command in ["actionSet.read", crate::panels::actions::LOAD_SET] {
        let r = request(&mut app, &tx, "engine.execute", execute(command, base.join("outside/b.aia")));
        assert!(r["error"].as_str().is_some_and(|e| e.contains("outside the read root")), "{command}: {r}");
    }
    assert_eq!(app.ui.actions.sets().len(), sets, "no set added");
    let _ = std::fs::remove_dir_all(base);
}

/// `ui.render` (MCP's `render_preview` and `screenshot` in the app) draws the artboard pixel for
/// pixel as `app.export` writes a PNG on white: two rectangles that meet at x 40.3 and a diagonal
/// edge.
#[test]
fn control_render_matches_a_png_export_on_white() {
    let (mut app, tx, base) = confined("render");
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="120" height="80"><rect x="0" y="10" width="40.3" height="50" fill="#204080"/><rect x="40.3" y="10" width="49.7" height="50" fill="#e0a020"/><path d="M10 75 L110 5 L115 30 Z" fill="#30a050"/></svg>"##;
    std::fs::write(base.join("in/edges.svg"), svg).unwrap();
    let r = request(&mut app, &tx, "app.open", json!({"path": s(base.join("in/edges.svg"))}));
    assert_eq!(r["ok"], true, "{r}");
    let pixels = |v: &Value| {
        craft_image_codecs::decode(&vectorcraft_format::base64_decode(v.as_str().unwrap()).unwrap())
            .map(|__cimg| {
                let (__w, __h) = __cimg.dimensions();
                match (__cimg.layout(), __cimg.sample_type()) {
                    (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8) => {
                        image::DynamicImage::ImageLuma8(image::GrayImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                    }
                    (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8) => {
                        image::DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                    }
                    (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8) => {
                        image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                    }
                    _ => image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(__w, __h, __cimg.to_rgba8()).unwrap()),
                }
            })
            .unwrap()
            .to_rgba8()
    };
    let r = request(&mut app, &tx, "app.export", json!({"format": "png", "background": "white"}));
    let export = pixels(&r["result"]["dataBase64"]);
    let r = request(&mut app, &tx, "ui.render", json!({}));
    let render = pixels(&r["result"]["pngBase64"]);
    assert_eq!(render.dimensions(), export.dimensions());
    let differ = render.pixels().zip(export.pixels()).filter(|(a, b)| a != b).count();
    assert_eq!(differ, 0, "{differ} pixels differ from the export");
    let _ = std::fs::remove_dir_all(base);
}

/// `ui.render {traceViews: true}` draws an Image Trace object in its view, as the canvas does:
/// in Source Image view the image's blue shows beside the traced disc, where `ui.render` without
/// it draws the tracing result on white.
#[test]
fn control_render_draws_the_trace_view_with_trace_views() {
    let (mut app, tx, base) = confined("traceviews");
    let png = base.join("in/disc.png");
    image::RgbImage::from_fn(100, 100, |x, y| {
        let (dx, dy) = (f64::from(x) + 0.5 - 50.0, f64::from(y) + 0.5 - 50.0);
        if dx * dx + dy * dy <= 900.0 { image::Rgb([0, 0, 0]) } else { image::Rgb([170, 210, 255]) }
    })
    .save(&png)
    .unwrap();
    let r = request(&mut app, &tx, "app.open", json!({"path": s(png)}));
    assert_eq!(r["ok"], true, "{r}");
    app.run("select.all", json!({})).unwrap();
    app.run("imageTrace.make", json!({"params": {"ignoreWhite": true}})).unwrap();
    app.run("imageTrace.setView", json!({"view": "sourceImage"})).unwrap();
    let pixels = |v: &Value| {
        craft_image_codecs::decode(&vectorcraft_format::base64_decode(v.as_str().unwrap()).unwrap())
            .map(|__cimg| {
                let (__w, __h) = __cimg.dimensions();
                match (__cimg.layout(), __cimg.sample_type()) {
                    (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8) => {
                        image::DynamicImage::ImageLuma8(image::GrayImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                    }
                    (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8) => {
                        image::DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                    }
                    (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8) => {
                        image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                    }
                    _ => image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(__w, __h, __cimg.to_rgba8()).unwrap()),
                }
            })
            .unwrap()
            .to_rgba8()
    };
    let view = pixels(&request(&mut app, &tx, "ui.render", json!({"traceViews": true}))["result"]["pngBase64"]);
    let result = pixels(&request(&mut app, &tx, "ui.render", json!({}))["result"]["pngBase64"]);
    let (beside, middle) = (view.get_pixel(10, 10).0, view.get_pixel(50, 50).0);
    assert!(beside[0] < 200 && beside[2] > 245, "the source image beside the disc: {beside:?}");
    assert!(middle[..3].iter().all(|&c| c < 60), "the disc: {middle:?}");
    assert_eq!(result.get_pixel(10, 10).0, [255; 4], "the tracing result on white");
    let _ = std::fs::remove_dir_all(base);
}

/// A pass that carries injected input runs inside the automation roots, whatever else it holds: a
/// file the person drops outside them is refused in it, and opened in the passes after.
#[test]
fn a_pass_with_injected_input_runs_inside_the_roots() {
    #[derive(Debug)]
    struct Dropped(PathBuf);
    impl egui::DroppedFile for Dropped {
        fn path(&self) -> &Path {
            &self.0
        }
        fn bytes(&self) -> Result<Vec<u8>, String> {
            std::fs::read(&self.0).map_err(|e| e.to_string())
        }
    }
    let (mut app, tx, base) = confined("synthetic");
    let ctx = crate::testing::ui_context();
    let outside = base.join("outside/b.svg");
    // The OS holds the file over the window in the pass before it drops it (`hovered`), as a desktop drag does.
    let pass = |app: &mut VectorcraftApp, hovered: bool, dropped: bool| {
        let mut raw = egui::RawInput::default();
        if hovered {
            raw.hovered_files.push(egui::HoveredFile { path: Some(outside.clone()), mime: String::new() });
        }
        if dropped {
            raw.dropped_files.push(std::sync::Arc::new(Dropped(outside.clone())));
        }
        crate::testing::app_frame(app, &ctx, raw);
        app.session.documents().len()
    };
    pass(&mut app, true, false);
    let (req, _reply) = ControlRequest::new("ui.key", json!({"key": "Escape"}));
    tx.send(req).unwrap();
    app.run_control(&ctx);
    assert_eq!(pass(&mut app, false, true), 0, "the pass carrying the key is confined");
    for _ in 0..4 {
        pass(&mut app, false, false);
    }
    pass(&mut app, true, false);
    assert_eq!(pass(&mut app, false, true), 1, "the person's pass is not");
    let _ = std::fs::remove_dir_all(base);
}

/// Every File › Open Recent Files row `ui.menu.list` returns, disabled ones included: (id, label),
/// the label of a disabled row starting "(disabled) ".
fn recent_rows(entries: &Value) -> Vec<(String, String)> {
    let rows = entries.as_array().map(Vec::as_slice).unwrap_or_default();
    rows.iter()
        .filter(|e| e["command"].as_str().is_some_and(|c| c.starts_with("file.openRecent")))
        .map(|e| {
            let label = e["label"].as_str().unwrap_or_default();
            let label = if e["enabled"] == true { label.to_string() } else { format!("(disabled) {label}") };
            (e["command"].as_str().unwrap_or_default().to_string(), label)
        })
        .collect()
}

#[test]
fn recent_files_outside_the_read_root_are_not_listed_or_opened() {
    let (mut app, tx, base) = confined("recent");
    std::fs::copy(base.join("in/a.svg"), base.join("in/c.svg")).unwrap();
    let (a, b, c) = (s(base.join("in/a.svg")), s(base.join("outside/b.svg")), s(base.join("in/c.svg")));
    app.ui.recent_files = vec![a.clone(), b.clone(), c.clone()];
    let rows = |pairs: &[(&str, &str)]| pairs.iter().map(|(id, l)| (id.to_string(), l.to_string())).collect::<Vec<_>>();

    // The person's menu lists all three.
    let person = serde_json::to_value(crate::menus::menu_entries(&app)).unwrap();
    assert_eq!(recent_rows(&person), rows(&[("file.openRecent1", "a.svg"), ("file.openRecent2", "b.svg"), ("file.openRecent3", "c.svg")]));
    assert_eq!(app.run("file.recentFiles", json!({})).unwrap(), json!([a, b, c]));

    // The client's listing leaves out the second, and the third keeps its id.
    let r = request(&mut app, &tx, "ui.menu.list", json!({}));
    assert_eq!(recent_rows(&r["result"]), rows(&[("file.openRecent1", "a.svg"), ("file.openRecent3", "c.svg")]), "{r}");
    let r = request(&mut app, &tx, "engine.execute", json!({"command": "file.recentFiles"}));
    assert_eq!(r["result"], json!([a, c]), "{r}");
    let r = request(&mut app, &tx, "ui.menu.invoke", json!({"command": "file.openRecent2"}));
    assert_eq!(r["error"], "no such recent file", "the reply names no path: {r}");
    assert!(app.session.documents().is_empty());
    let r = request(&mut app, &tx, "ui.menu.invoke", json!({"command": "file.openRecent3"}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(app.session.documents().len(), 1);
    assert_eq!(app.ui.recent_files, [c.clone(), a.clone(), b.clone()]);

    // A file outside the root that the person has open is still not brought forward.
    app.run("file.openRecent3", json!({})).unwrap();
    assert_eq!((app.session.documents().len(), &app.ui.recent_files[0]), (2, &b));
    let r = request(&mut app, &tx, "ui.menu.invoke", json!({"command": "file.openRecent1"}));
    assert_eq!(r["error"], "no such recent file", "{r}");

    // Rows past Number of Recent Files to Display stay out, and a write root alone lists none.
    app.session.prefs.recent_files_count = 2;
    let r = request(&mut app, &tx, "ui.menu.list", json!({}));
    assert_eq!(recent_rows(&r["result"]), rows(&[("file.openRecent2", "c.svg")]), "{r}");
    let write_only = AutomationRoots::new(&[], &[&base.join("out")]).unwrap();
    let listed = vectorcraft_engine::file_access::confine(write_only.as_ref(), || serde_json::to_value(crate::menus::menu_entries(&app)).unwrap());
    assert!(recent_rows(&listed).is_empty(), "{listed}");
    let _ = std::fs::remove_dir_all(base);
}

/// `ui.inspect` leaves out the paths outside the read root that the person's work put in it: files
/// still opening, recent files, a library panel's file and folder preferences, saved and in the
/// Preferences dialog.
#[test]
fn ui_inspect_leaves_out_paths_outside_the_read_root() {
    let (mut app, tx, base) = confined("inspect");
    std::fs::copy(base.join("in/a.svg"), base.join("in/c.svg")).unwrap();
    let (a, b, c) = (s(base.join("in/a.svg")), s(base.join("outside/b.svg")), s(base.join("in/c.svg")));
    let svg = std::fs::read(&a).unwrap();
    for path in [&a, &b, &c] {
        assert_eq!(crate::opening::start(&mut app, path, svg.clone())["background"], true, "{path} opens off the UI thread");
    }
    app.ui.recent_files = vec![a.clone(), b.clone(), c.clone()];
    let panel = |id: &str, path: &str| -> crate::panels::library_panel::LibraryPanel {
        serde_json::from_value(json!({"kind": "swatches", "id": id, "path": path})).unwrap()
    };
    let theirs = format!("loaded/{b}");
    app.ui.library_panels = vec![panel("mine", &a), panel(&theirs, &b)];
    app.ui.open_panel = Some(format!("library:swatches:{theirs}"));
    let (inside, outside) = (s(base.join("in")), s(base.join("outside")));
    app.ui.engine_prefs = json!({
        "fontsFolder": inside, "templatesFolder": outside, "recoveryFolder": outside, "scratchPrimary": "Startup",
        "scratchSecondary": "../private", "pluginsFolder": "",
    });
    crate::prefs_dialog::open(&mut app, None);
    // The person types the folders: their request is not confined.
    let (fill, _) = ControlRequest::new(
        "ui.dialog.set",
        json!({"fields": [{"field": "fontsFolder", "value": inside}, {"field": "templatesFolder", "value": outside}]}),
    );
    let Reply::Done(filled) = crate::control::handle(&mut app, &crate::testing::ui_context(), &fill) else { panic!("deferred") };
    assert_eq!(filled["ok"], true, "{filled}");

    // The person sees every path.
    let ctx = crate::testing::ui_context();
    let person = crate::control::inspect(&app, &ctx);
    assert_eq!(person["opening"], json!([a, b, c]));
    assert_eq!(person["ui"]["recent_files"], json!([a, b, c]));
    assert_eq!(person["ui"]["library_panels"][1]["path"], b);
    assert_eq!(person["ui"]["engine_prefs"]["templatesFolder"], outside);

    // The client sees the ones inside the read root, in the same order.
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    let ui = &r["result"]["ui"];
    assert_eq!(r["result"]["opening"], json!([a, c]), "{r}");
    assert_eq!(ui["recent_files"], json!([a, c]));
    assert_eq!(ui["library_panels"][0]["path"], a);
    assert_eq!((ui["library_panels"][1]["id"].as_str(), ui["library_panels"][1].get("path")), (Some("loaded/b.svg"), None), "the panel stays");
    assert_eq!(ui["open_panel"], "library:swatches:loaded/b.svg");
    let prefs = ui["engine_prefs"].as_object().unwrap();
    assert_eq!(
        (prefs["fontsFolder"].as_str(), prefs["scratchPrimary"].as_str(), prefs["pluginsFolder"].as_str()),
        (Some(inside.as_str()), Some("Startup"), Some(""))
    );
    assert!(["templatesFolder", "recoveryFolder", "scratchSecondary"].iter().all(|k| !prefs.contains_key(*k)), "{prefs:?}");
    let fields = ui["dialog"]["fields"].as_object().unwrap();
    assert_eq!((fields["fontsFolder"].as_str(), fields.get("templatesFolder")), (Some(inside.as_str()), None), "the Preferences dialog's copy too");
    let hidden = json!(outside).to_string();
    assert!(!r.to_string().contains(hidden.trim_matches('"')), "no path outside the root: {r}");
    let r = request(&mut app, &tx, "ui.docking", json!({}));
    assert_eq!(r["result"]["flyout"], "library:swatches:loaded/b.svg", "{r}");
    assert_eq!(crate::panel_docking::inspect(&app, &ctx)["flyout"], json!(format!("library:swatches:{theirs}")), "the person's");
    assert_eq!(app.ui.recent_files, [a.clone(), b.clone(), c.clone()], "the UI state is not changed");
    assert_eq!(app.opening.pending.len(), 3, "and no open is canceled");

    // A write root alone lets the client read none of them.
    let write_only = AutomationRoots::new(&[], &[&base.join("out")]).unwrap();
    let listed = vectorcraft_engine::file_access::confine(write_only.as_ref(), || crate::control::inspect(&app, &ctx));
    assert_eq!((&listed["opening"], &listed["ui"]["recent_files"]), (&json!([]), &json!([])), "{listed}");
    let _ = std::fs::remove_dir_all(base);
}

/// `ui.inspect` leaves out the file of a saved proof setup that an open Insert Menu Item dialog
/// picked when it is outside the read root, and shows one inside it.
#[test]
fn ui_inspect_leaves_out_an_insert_menu_item_path_outside_the_read_root() {
    let (mut app, tx, base) = confined("inspect-insert-menu-item");
    let load = crate::dialogs::proof_setup::LOAD;
    let (inside, outside) = (s(base.join("in/Press.vcproof")), s(base.join("outside/Press.vcproof")));
    crate::dialogs::action_insert_menu_item::open(&mut app, None, None);
    let pick = |app: &mut VectorcraftApp, path: &str| {
        set_top_fields(app, &[("command", json!(load)), ("params", json!({"path": path}))]);
    };
    pick(&mut app, &outside);
    let ctx = crate::testing::ui_context();
    assert_eq!(top_dialog_fields(&crate::control::inspect(&app, &ctx))["params"]["path"], outside, "the person sees it");
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    let fields = top_dialog_fields(&r["result"]);
    assert_eq!((fields["command"].as_str(), fields["params"].get("path")), (Some(load), None), "{r}");
    assert_eq!(top_dialog_fields(&crate::control::inspect(&app, &ctx))["params"]["path"], outside, "the dialog is not changed");
    pick(&mut app, &inside);
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    assert_eq!(top_dialog_fields(&r["result"])["params"]["path"], inside, "{r}");
    let _ = std::fs::remove_dir_all(base);
}

/// `ui.inspect` checks a folder preference as it is written: the read root's path with a space before
/// it is a relative path, outside the root, and is left out.
#[test]
fn ui_inspect_checks_a_folder_preference_as_written() {
    let (mut app, tx, base) = confined("inspect-folder-space");
    let inside = s(base.join("in"));
    app.ui.engine_prefs = json!({"fontsFolder": inside, "templatesFolder": format!(" {inside}")});
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    let prefs = r["result"]["ui"]["engine_prefs"].as_object().unwrap();
    assert_eq!(prefs.get("fontsFolder").and_then(Value::as_str), Some(inside.as_str()), "{r}");
    assert!(!prefs.contains_key("templatesFolder"), "{r}");
    let _ = std::fs::remove_dir_all(base);
}

/// `ui.inspect` names the file selected in JavaScript Files by its file name alone when it is
/// outside the read root, and keeps one inside it; the person sees the whole path.
#[test]
fn ui_inspect_hides_a_selected_javascript_file_outside_the_read_root() {
    let (mut app, tx, base) = confined("inspect-javascript-files");
    let (inside, outside) = (s(base.join("in/menu.js")), s(base.join("outside/menu.js")));
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    for f in [&inside, &outside] {
        app.run("svgInteractivity.addFile", json!({ "file": f })).unwrap();
    }
    crate::dialogs::javascript_files::open(&mut app).unwrap();
    set_top_fields(&mut app, &[("selected", json!(outside))]);
    let selected = |v: &Value| top_dialog_fields(v)["selected"].clone();
    let ctx = crate::testing::ui_context();
    assert_eq!(selected(&crate::control::inspect(&app, &ctx)), json!(outside), "the person's");
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    assert_eq!(selected(&r["result"]), "menu.js", "{r}");
    assert!(!r.to_string().contains(&json!(outside).to_string().trim_matches('"').to_string()), "{r}");
    set_top_fields(&mut app, &[("selected", json!(inside))]);
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    assert_eq!(selected(&r["result"]), json!(inside), "{r}");
    assert_eq!(selected(&crate::control::inspect(&app, &ctx)), json!(inside), "the dialog is not changed");
    let _ = std::fs::remove_dir_all(base);
}

/// `ui.inspect` checks a folder preference as the app uses it, trimmed, as well: with the working
/// folder as the read root, a path outside it with a space before it reads as a name in the working
/// folder, and is still left out, because the app trims it and uses the path outside.
#[test]
fn ui_inspect_checks_a_folder_preference_as_trimmed() {
    let (_, _, base) = confined("inspect-folder-trimmed");
    let cwd = std::env::current_dir().unwrap();
    let roots = AutomationRoots::new(&[&cwd], &[&base.join("out")]).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = VectorcraftApp::new(Session::new(), Services::default()).with_control(rx).with_automation_roots(roots);
    let outside = s(base.join("outside"));
    // Without its drive, the path starts with a separator: trimmed, it is absolute on any platform.
    let rooted = match outside.as_bytes() {
        [drive, b':', ..] if drive.is_ascii_alphabetic() => &outside[2..],
        _ => outside.as_str(),
    };
    let spaced = format!(" {rooted}");
    app.ui.engine_prefs = json!({"fontsFolder": spaced, "pluginsFolder": ""});
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    let prefs = r["result"]["ui"]["engine_prefs"].as_object().unwrap();
    assert_eq!(prefs.get("pluginsFolder").and_then(Value::as_str), Some(""), "{r}");
    assert!(!prefs.contains_key("fontsFolder"), "{r}");
    let _ = std::fs::remove_dir_all(base);
}

/// `ui.inspect` hides a hidden library's id and path in every string it holds: status text, the
/// command palette query and a script's report show the id and file name shown, and the path appears
/// nowhere in the reply.
#[test]
fn ui_inspect_hides_a_hidden_library_path_in_every_string() {
    let (mut app, tx, base) = confined("inspect-status-id");
    let path = s(base.join("outside/b.svg"));
    let theirs = format!("loaded/{path}");
    app.ui.color_guide_limit = theirs.clone();
    app.ui.status = format!("Loaded {path} (3 swatches), kept {path}.bak");
    app.ui.palette_query = format!("\"{theirs}\"");
    let source = format!("vc.log({})", json!(format!("read {path}.")));
    app.session.execute("script.run", &json!({"source": source, "name": "t.js"})).unwrap();
    for _ in 0..10_000 {
        if app.session.script_pump().is_some() {
            break;
        }
    }
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    assert_eq!(r["result"]["ui"]["color_guide_limit"], "loaded/b.svg", "{r}");
    assert_eq!(r["result"]["ui"]["status"], "Loaded b.svg (3 swatches), kept b.svg.bak", "{r}");
    assert_eq!(r["result"]["ui"]["palette_query"], "\"loaded/b.svg\"", "{r}");
    assert_eq!(r["result"]["scriptReport"]["log"], json!(["read b.svg."]), "{r}");
    let outside = json!(s(base.join("outside"))).to_string();
    assert!(!r.to_string().contains(outside.trim_matches('"')), "no path outside the root: {r}");
    let _ = std::fs::remove_dir_all(base);
}

/// The reply to a request that names a hidden library by the id `ui.inspect` shows hides that
/// library's id wherever the reply holds it, also between spaces and brackets and where a character
/// that continues a path follows it.
#[test]
fn a_reply_hides_a_library_the_request_named_by_its_shown_id_everywhere() {
    let (mut app, tx, base) = confined("named-hidden-reply");
    let theirs = format!("loaded/{}", s(base.join("outside/b.svg")));
    app.ui.color_guide_limit = theirs.clone();
    app.ui.status = format!("Not added ( {theirs} ); kept {theirs}_old");
    let r = request(&mut app, &tx, "ui.inspect", json!({"library": "loaded/b.svg"}));
    assert_eq!(r["result"]["ui"]["status"], "Not added ( loaded/b.svg ); kept loaded/b.svg_old", "{r}");
    let outside = json!(s(base.join("outside"))).to_string();
    assert!(!r.to_string().contains(outside.trim_matches('"')), "no path outside the root: {r}");
    let _ = std::fs::remove_dir_all(base);
}

/// An error to a request that names a hidden library by the id `ui.inspect` shows names it by that id,
/// and its path appears nowhere in the reply.
#[test]
fn an_error_for_a_hidden_library_named_by_its_shown_id_shows_that_id() {
    let (mut app, tx, base) = confined("named-hidden-error");
    app.ui.color_guide_limit = format!("loaded/{}", s(base.join("outside/b.svg")));
    let r = request(&mut app, &tx, "engine.execute", json!({"command": "ui.colorGuideLimit", "params": {"library": "loaded/b.svg"}}));
    assert_eq!(r["ok"], false, "{r}");
    assert!(r["error"].as_str().is_some_and(|e| e.contains("loaded/b.svg")), "{r}");
    let outside = json!(s(base.join("outside"))).to_string();
    assert!(!r.to_string().contains(outside.trim_matches('"')), "no path outside the root: {r}");
    let _ = std::fs::remove_dir_all(base);
}

/// `ui.inspect` leaves out of a recorded action's steps a path outside the read root under any path
/// param (`diffPath`, `file`, a name ending in `Files`), and an absolute path or a `file:` URL outside
/// it (the scheme in any case) under any other param or in any list. A `file:` URL inside it stays.
#[test]
fn ui_inspect_leaves_out_recorded_paths_under_any_key() {
    let (mut app, tx, base) = confined("inspect-action-keys");
    let (a, b) = (s(base.join("in/a.svg")), s(base.join("outside/b.svg")));
    let to_url = |p: &str| if p.starts_with('/') { format!("file://{p}") } else { format!("file:///{}", p.replace('\\', "/")) };
    let (url, url_in) = (to_url(&b), to_url(&a));
    app.ui.actions = action_list(json!([{
        "name": "set",
        "actions": [{"name": "files", "steps": [
            ["document.compare", {"diffPath": b, "target": a}],
            ["file.open", {"file": b}],
            ["type.fonts", {"fontFiles": [a, b, url_in]}],
            ["x.y", {"target": b, "url": url, "upper": url.replacen("file:", "FILE:", 1), "other": "keep"}],
            ["x.z", {"list": [b, "keep"]}],
            ["x.w", {"share": "file://other-host/share/b.svg", "other": "keep"}],
            ["x.v", {"file": url_in}],
        ]}],
    }]));
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    let steps = &r["result"]["ui"]["action_sets"][0]["actions"][0]["steps"];
    let want = json!([
        ["document.compare", {"target": a}],
        ["file.open", {}],
        ["type.fonts", {"fontFiles": [a, url_in]}],
        ["x.y", {"other": "keep"}],
        ["x.z", {"list": ["keep"]}],
        ["x.w", {"other": "keep"}],
        ["x.v", {"file": url_in}],
    ]);
    assert_eq!(steps, &want, "{r}");
    let outside = json!(s(base.join("outside"))).to_string();
    assert!(!r.to_string().contains(outside.trim_matches('"')), "no path outside the root: {r}");
    let _ = std::fs::remove_dir_all(base);
}

/// `ui.inspect` hides a library loaded from outside the read root in the steps of a recorded action
/// (its `library` and `panel` params) and in the action's name, also when nothing else names it.
#[test]
fn ui_inspect_hides_a_hidden_library_in_recorded_steps() {
    let (mut app, tx, base) = confined("inspect-action-steps");
    let theirs = format!("loaded/{}", s(base.join("outside/b.svg")));
    app.ui.actions = action_list(json!([{
        "name": "set",
        "actions": [{"name": theirs, "steps": [
            ["swatch.library.add", {"library": theirs}],
            ["window.panel", {"panel": format!("library:swatches:{theirs}")}],
        ]}],
    }]));
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    let action = &r["result"]["ui"]["action_sets"][0]["actions"][0];
    assert_eq!(action["name"], "loaded/b.svg", "{r}");
    assert_eq!(
        action["steps"],
        json!([["swatch.library.add", {"library": "loaded/b.svg"}], ["window.panel", {"panel": "library:swatches:loaded/b.svg"}]]),
        "{r}"
    );
    let _ = std::fs::remove_dir_all(base);
}

/// `ui.inspect` leaves out of a recorded action's steps the file paths (`path`, `folder`, a name ending
/// in `Folder`, `paths`) outside the read root, also in a batch step's commands, and keeps the ones
/// inside it.
#[test]
fn ui_inspect_leaves_out_recorded_paths_outside_the_read_root() {
    let (mut app, tx, base) = confined("inspect-action-paths");
    let (a, b) = (s(base.join("in/a.svg")), s(base.join("outside/b.svg")));
    app.ui.actions = action_list(json!([{
        "name": "set",
        "actions": [{"name": "files", "steps": [
            ["file.open", {"path": a}],
            ["file.open", {"path": b}],
            ["file.place", {"paths": [a, b]}],
            ["batch.run", {"folder": s(base.join("outside"))}],
            ["action.batch", {"destinationFolder": s(base.join("outside"))}],
            ["command.batch", {"commands": [{"command": "file.open", "params": {"path": b}}]}],
        ]}],
    }]));
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    let steps = &r["result"]["ui"]["action_sets"][0]["actions"][0]["steps"];
    let want = json!([
        ["file.open", {"path": a}],
        ["file.open", {}],
        ["file.place", {"paths": [a]}],
        ["batch.run", {}],
        ["action.batch", {}],
        ["command.batch", {"commands": [{"command": "file.open", "params": {}}]}],
    ]);
    assert_eq!(steps, &want, "{r}");
    assert_eq!(app.ui.actions.actions()[0].steps.len(), 6, "the UI state is not changed");
    let _ = std::fs::remove_dir_all(base);
}

/// `ui.inspect` hides a library loaded from outside the read root whose id holds a path written with
/// either separator, also one saved on another platform.
#[test]
fn ui_inspect_hides_a_library_path_with_either_separator() {
    let (mut app, tx, base) = confined("inspect-separators");
    app.ui.color_guide_limit = r"loaded/C:\Users\someone\b.svg".to_string();
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    assert_eq!(r["result"]["ui"]["color_guide_limit"], "loaded/b.svg", "{r}");
    app.ui.color_guide_limit = "loaded//home/someone/c.svg".to_string();
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    assert_eq!(r["result"]["ui"]["color_guide_limit"], "loaded/c.svg", "{r}");
    let _ = std::fs::remove_dir_all(base);
}

/// A hidden library does not take the id of a library loaded without a path under the same file
/// name: it shows as `loaded/<file name> (2)`, and a request that names the other library reaches it.
#[test]
fn ui_inspect_keeps_a_hidden_library_off_a_loaded_librarys_id() {
    let (mut app, tx, base) = confined("inspect-loaded-id");
    let svg = std::fs::read_to_string(base.join("in/a.svg")).unwrap();
    let loaded = app.session.execute("swatch.library.load", &json!({"data": svg, "name": "b.svg"})).unwrap();
    assert_eq!(loaded["library"], "loaded/b.svg");
    let theirs = format!("loaded/{}", s(base.join("outside/b.svg")));
    app.ui.color_guide_limit = theirs;
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    assert_eq!(r["result"]["ui"]["color_guide_limit"], "loaded/b.svg (2)", "{r}");
    let _ = std::fs::remove_dir_all(base);
}

/// Hidden libraries with the same file name that are shown for the first time in the same reply are
/// numbered in the order of their paths, as the library commands number them.
#[test]
fn hidden_libraries_are_numbered_in_the_order_of_their_paths() {
    let (mut app, tx, base) = confined("hidden-load-order");
    let gpl = "GIMP Palette\nName: p\n0 0 0 Black\n";
    for dir in ["z", "a"] {
        std::fs::create_dir_all(base.join("outside").join(dir)).unwrap();
        let path = base.join("outside").join(dir).join("b.gpl");
        std::fs::write(&path, gpl).unwrap();
        app.session.execute("swatch.library.load", &json!({"path": s(path)})).unwrap();
    }
    app.ui.color_guide_limit = format!("loaded/{}", s(base.join("outside").join("z").join("b.gpl")));
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    assert_eq!(r["result"]["ui"]["color_guide_limit"], "loaded/b.gpl (2)", "{r}");
    let _ = std::fs::remove_dir_all(base);
}

/// A hidden library keeps the id a reply showed for it when another library with the same file name is
/// loaded later.
#[test]
fn a_hidden_library_keeps_its_id_when_another_with_its_name_loads() {
    let (mut app, tx, base) = confined("hidden-stable-id");
    let first = format!("loaded/{}", s(base.join("outside").join("a").join("b.gpl")));
    let panel: crate::panels::library_panel::LibraryPanel = serde_json::from_value(json!({"kind": "swatches", "id": first})).unwrap();
    app.ui.library_panels = vec![panel];
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    assert_eq!(r["result"]["ui"]["library_panels"][0]["id"], "loaded/b.gpl", "{r}");
    std::fs::create_dir_all(base.join("outside").join("z")).unwrap();
    let path = base.join("outside").join("z").join("b.gpl");
    std::fs::write(&path, "GIMP Palette\nName: p\n0 0 0 Black\n").unwrap();
    app.session.execute("swatch.library.load", &json!({"path": s(path.clone())})).unwrap();
    app.ui.color_guide_limit = format!("loaded/{}", s(path));
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    assert_eq!(r["result"]["ui"]["library_panels"][0]["id"], "loaded/b.gpl", "{r}");
    assert_eq!(r["result"]["ui"]["color_guide_limit"], "loaded/b.gpl (2)", "{r}");
    let _ = std::fs::remove_dir_all(base);
}

/// An error from a panic hides a hidden library's path, also when the request stopped referring to
/// the library before it panicked.
#[test]
fn a_panic_reply_hides_a_library_hidden_before_the_request() {
    let (mut app, tx, base) = confined("panic-hidden");
    let path = s(base.join("outside/b.svg"));
    app.ui.color_guide_limit = format!("loaded/{path}");
    app.ui.status = format!("Loaded {path}");
    let r = request(&mut app, &tx, "test.panic", json!({"forget": true}));
    assert_eq!(r["error"], "internal error: failed on Loaded b.svg (please report this bug)", "{r}");
    let _ = std::fs::remove_dir_all(base);
}

/// An error shows a path outside the read root that the request sent by its file name, whether or not
/// a hidden library was loaded from that path, so that it reads the same either way. A path the
/// request sends does not show the app's own text that holds it, in an error or a result, and a result
/// shows that path by its file name as an error does.
#[test]
fn an_error_hides_a_sent_path_whether_or_not_a_library_was_loaded_from_it() {
    let (mut app, tx, base) = confined("client-echo");
    let path = s(base.join("outside/b.svg"));
    // Another hidden library with the same file name comes first, so this one shows as `b.svg (2)`.
    let first = format!("loaded/{}", s(base.join("outside/a/b.svg")));
    let panel: crate::panels::library_panel::LibraryPanel = serde_json::from_value(json!({"kind": "swatches", "id": first})).unwrap();
    app.ui.library_panels = vec![panel];
    app.ui.color_guide_limit = format!("loaded/{path}");
    app.ui.status = format!("Loaded {path}");
    let unloaded = [(s(base.join("outside/y/b.svg")), "b.svg"), (s(base.join("outside/c.svg")), "c.svg")];
    for (sent, name) in [(path.clone(), "b.svg")].into_iter().chain(unloaded) {
        let r = request(&mut app, &tx, "app.open", json!({"path": sent}));
        assert!(r["error"].as_str().is_some_and(|e| e.ends_with(&format!(": {name}"))), "{r}");
    }
    let r = request(&mut app, &tx, "test.status", json!({"guess": path}));
    assert_eq!(r["result"], "Loaded b.svg", "{r}");
    let r = request(&mut app, &tx, "test.panic", json!({"guess": path}));
    assert_eq!(r["error"], "internal error: failed on Loaded b.svg (please report this bug)", "{r}");
    let _ = std::fs::remove_dir_all(base);
}

/// A screenshot's error, sent after its request was handled, hides a hidden library's path and the path outside the
/// read root that the request sent, as the capture's writer hides it.
#[test]
fn a_screenshot_reply_hides_paths_like_any_reply() {
    let (mut app, _tx, base) = confined("deferred-reply");
    let path = s(base.join("outside/b.svg"));
    app.ui.color_guide_limit = format!("loaded/{path}");
    let sent = s(base.join("outside/shot.png"));
    let roots = AutomationRoots::new(&[&base.join("in")], &[&base.join("out")]).unwrap();
    let sent_path = json!({"path": sent});
    // The app holds its roots for the whole process, so its hiding is read with them in force.
    let (library, failed) = vectorcraft_engine::file_access::confine(roots.as_ref(), || {
        let hiding = crate::control::deferred_hiding(&app);
        (hiding.hide_error(&format!("from {path}"), &sent_path), hiding.hide_error(&format!("could not write {sent}"), &sent_path))
    });
    assert_eq!(library, "from b.svg");
    assert_eq!(failed, "could not write shot.png");
    let _ = std::fs::remove_dir_all(base);
}

/// A reply whose only trace of a hidden library's path is a `file:` URL with every separator escaped
/// shows the file name alone.
#[test]
fn a_reply_hides_a_path_with_escaped_separators() {
    let (mut app, tx, base) = confined("escaped-separators");
    let path = s(base.join("outside/b.svg"));
    app.ui.color_guide_limit = format!("loaded/{path}");
    app.ui.status = format!("from file:{}", path.replace('/', "%2F").replace('\\', "%5C"));
    let r = request(&mut app, &tx, "test.status", json!({}));
    assert_eq!(r["result"], "from b.svg", "{r}");
    let _ = std::fs::remove_dir_all(base);
}

/// `ui.inspect` leaves out of a recorded action's steps a path that starts with `./` or `../`, `~` and
/// a path under `~` outside the read root under any param, as a key too, and in any list. A URL and
/// other text with a separator stay.
#[test]
fn ui_inspect_leaves_out_relative_paths_in_recorded_steps() {
    let (mut app, tx, base) = confined("inspect-relative");
    app.ui.actions = action_list(json!([{
        "name": "set",
        "actions": [{"name": "files", "steps": [
            ["x.y", {"target": "~/b.svg", "rel": "../outside/b.svg", "home": "~", "text": "a/b", "url": "https://example.com/a"}],
            ["x.z", {"list": [".\\b.svg", "a/b"], "../k": 1}],
        ]}],
    }]));
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    let steps = &r["result"]["ui"]["action_sets"][0]["actions"][0]["steps"];
    let want = json!([["x.y", {"text": "a/b", "url": "https://example.com/a"}], ["x.z", {"list": ["a/b"]}]]);
    assert_eq!(steps, &want, "{r}");
    let _ = std::fs::remove_dir_all(base);
}

/// `ui.inspect` leaves out of a recorded `prefs.set` step the value of a folder preference outside the
/// read root, given as `{key, value}` or in `values`, also in a batch step's commands at any depth: a
/// bare name or a subfolder (in the working folder, outside the root), a path with spaces around it and
/// a `file:` URL. A folder inside the root, an empty value, a scratch disk choice, other preferences and
/// another command's params stay, and the UI state is not changed.
#[test]
fn ui_inspect_leaves_out_folder_preferences_in_recorded_prefs_steps() {
    let (mut app, tx, base) = confined("inspect-prefs-steps");
    let (inside, outside) = (s(base.join("in")), s(base.join("outside")));
    let url = if outside.starts_with('/') { format!("file://{outside}") } else { format!("file:///{}", outside.replace('\\', "/")) };
    let folders = ["fontsFolder", "pluginsFolder", "scratchPrimary", "scratchSecondary", "recoveryFolder", "templatesFolder"];
    let mut steps: Vec<Value> = folders.iter().map(|k| json!(["prefs.set", {"key": k, "value": "scratch"}])).collect();
    steps.push(json!(["prefs.set", {"values": folders.iter().map(|k| (k.to_string(), json!("tmp/scratch"))).collect::<serde_json::Map<_, _>>()}]));
    steps.push(json!(["prefs.set", {"values": {
        "scratchPrimary": format!(" {outside} "), "fontsFolder": url, "pluginsFolder": format!(" {url}"), "recoveryFolder": "Startup",
        "templatesFolder": inside, "scratchSecondary": " None ", "uiAccent": "green",
    }}]));
    steps.push(json!(["prefs.set", {"key": "scratchPrimary", "value": "Startup"}]));
    steps.push(json!(["prefs.set", {"key": "recoveryFolder", "value": ""}]));
    steps.push(json!(["prefs.set", {"key": "fontsFolder", "value": inside}]));
    let batch = |commands: Value| json!({"commands": commands});
    let inner = batch(json!([{"command": "prefs.set", "params": {"values": {"scratchSecondary": "tmp/scratch", "uiAccent": "red"}}}]));
    steps.push(json!([
        "command.batch",
        batch(json!([
            {"command": "prefs.set", "params": {"key": "scratchPrimary", "value": "scratch"}},
            {"command": "command.batch", "params": inner},
        ]))
    ]));
    steps.push(json!(["x.y", {"key": "scratchPrimary", "value": "scratch"}]));
    app.ui.actions = action_list(json!([{"name": "set", "actions": [{"name": "prefs", "steps": steps}]}]));
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    let shown = &r["result"]["ui"]["action_sets"][0]["actions"][0]["steps"];
    let mut want: Vec<Value> = folders.iter().map(|k| json!(["prefs.set", {"key": k}])).collect();
    want.push(json!(["prefs.set", {"values": {}}]));
    want.push(json!(["prefs.set", {"values": {"templatesFolder": inside, "scratchSecondary": " None ", "uiAccent": "green"}}]));
    want.push(json!(["prefs.set", {"key": "scratchPrimary", "value": "Startup"}]));
    want.push(json!(["prefs.set", {"key": "recoveryFolder", "value": ""}]));
    want.push(json!(["prefs.set", {"key": "fontsFolder", "value": inside}]));
    let inner = batch(json!([{"command": "prefs.set", "params": {"values": {"uiAccent": "red"}}}]));
    want.push(json!([
        "command.batch",
        batch(json!([
            {"command": "prefs.set", "params": {"key": "scratchPrimary"}},
            {"command": "command.batch", "params": inner},
        ]))
    ]));
    want.push(json!(["x.y", {"key": "scratchPrimary", "value": "scratch"}]));
    assert_eq!(shown, &json!(want), "{r}");
    assert!(!r.to_string().contains("tmp/scratch") && !r.to_string().contains(&outside.replace('\\', "\\\\")), "{r}");
    assert_eq!(app.ui.actions.actions()[0].steps[0].params, json!({"key": "fontsFolder", "value": "scratch"}), "the UI state is not changed");
    let _ = std::fs::remove_dir_all(base);
}

/// Two libraries loaded from outside the read root under the same file name, only the second of them
/// in a library panel: `ui.inspect` and `swatch.library.list` show the second under the same id, and
/// that id finds it.
#[test]
fn ui_inspect_and_library_commands_show_a_hidden_library_under_one_id() {
    let (mut app, tx, base) = confined("one-id");
    let (x, y) = (base.join("outside/x/Kit.gpl"), base.join("outside/y/Kit.gpl"));
    for (path, colors) in [(&x, "0 0 0 Black\n"), (&y, "0 0 0 Black\n255 255 255 White\n")] {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, format!("GIMP Palette\nName: Kit\n{colors}")).unwrap();
        app.session.execute("swatch.library.load", &json!({"path": s(path.clone())})).unwrap();
    }
    let second = format!("loaded/{}", s(y.clone()));
    app.ui.library_panels = vec![serde_json::from_value(json!({"kind": "swatches", "id": second, "path": s(y.clone())})).unwrap()];

    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    assert_eq!(r["result"]["ui"]["library_panels"][0]["id"], "loaded/Kit.gpl (2)", "{r}");
    let r = request(&mut app, &tx, "engine.execute", json!({"command": "swatch.library.list", "params": {}}));
    let libs = r["result"]["libraries"].as_array().unwrap_or_else(|| panic!("{r}"));
    let loaded: Vec<&Value> = libs.iter().filter(|l| l["category"] == "loaded").map(|l| &l["id"]).collect();
    assert_eq!(loaded, [&json!("loaded/Kit.gpl"), &json!("loaded/Kit.gpl (2)")], "{r}");
    let r = request(&mut app, &tx, "engine.execute", json!({"command": "swatch.library.get", "params": {"library": "loaded/Kit.gpl (2)"}}));
    assert_eq!(r["result"]["swatches"].as_array().map(Vec::len), Some(2), "the second library: {r}");
    assert!(!r.to_string().contains("outside"), "{r}");
    let _ = std::fs::remove_dir_all(base);
}

/// A library panel command given a hidden library's shown id opens the library and answers with
/// that id, not the library's path.
#[test]
fn a_library_panel_command_answers_with_a_hidden_librarys_shown_id() {
    let (mut app, tx, base) = confined("panel-shown-id");
    let path = base.join("outside/x/Kit.gpl");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "GIMP Palette\nName: Kit\n0 0 0 Black\n").unwrap();
    app.session.execute("swatch.library.load", &json!({"path": s(path.clone())})).unwrap();
    let r = request(&mut app, &tx, "engine.execute", json!({"command": "window.swatchLibrary", "params": {"library": "loaded/Kit.gpl"}}));
    assert_eq!(r["ok"], true, "{r}");
    assert!(r.to_string().contains("loaded/Kit.gpl") && !r.to_string().contains("outside"), "{r}");
    assert_eq!(app.ui.library_panels.last().map(|p| p.id.clone()), Some(format!("loaded/{}", s(path))), "the app's own id");
    // The panel id it answers with names the panel in the panel commands.
    let panel = r["result"]["panel"].clone();
    assert_eq!(panel, "library:swatches:loaded/Kit.gpl", "{r}");
    let r = request(&mut app, &tx, "engine.execute", json!({"command": "ui.libraryPanel", "params": {"panel": panel, "view": "largeList"}}));
    assert_eq!((&r["ok"], &r["result"]["panel"]), (&json!(true), &panel), "{r}");
    assert!(!r.to_string().contains("outside"), "{r}");
    let r = request(
        &mut app,
        &tx,
        "engine.execute",
        json!({"command": "window.panel.move", "params": {"panel": "swatches", "anchor": panel, "zone": "tab"}}),
    );
    assert_eq!(r["ok"], true, "{r}");
    assert!(!r.to_string().contains("outside"), "{r}");
    let r = request(&mut app, &tx, "engine.execute", json!({"command": "window.panel.close", "params": {"panel": panel}}));
    assert_eq!(r["ok"], true, "{r}");
    assert!(!r.to_string().contains("outside"), "{r}");
    // Closed, the panel isn't found, and the error names it by the id given.
    let r = request(&mut app, &tx, "engine.execute", json!({"command": "ui.libraryPanel", "params": {"panel": panel, "view": "largeList"}}));
    assert_eq!(r["ok"], false, "{r}");
    assert!(!r.to_string().contains("outside"), "{r}");
    let _ = std::fs::remove_dir_all(base);
}

/// A closed panel's former place, as the dock workspace keeps it.
fn hidden_place() -> craft::behavior::dock::Location<String> {
    serde_json::from_value(json!({"floating": null, "anchor": null, "placement": {"Tab": {"before": null}}, "size": null, "stack": null})).unwrap()
}

/// Closes `panel` into the dock workspace's closed panels, with [`hidden_place`] as its former place.
fn hide_panel(app: &mut VectorcraftApp, panel: String) {
    let ws = &app.ui.dock_workspace;
    let mut hidden = ws.hidden().to_vec();
    hidden.push((panel, hidden_place()));
    let next = craft::behavior::dock::Workspace::from_parts(ws.layout().clone(), hidden, ws.floated().to_vec());
    std::sync::Arc::make_mut(&mut app.ui.dock_workspace).replace(next);
}

/// The closed panels of a `ui.inspect` result's UI state, in order.
fn hidden_panels(ui: &Value) -> Vec<String> {
    let hidden = ui["dock"]["hidden"].as_array().map(Vec::as_slice).unwrap_or_default();
    hidden.iter().filter_map(|entry| entry[0].as_str().map(str::to_owned)).collect()
}

/// `ui.inspect` hides the path of a library loaded from outside the read root wherever the reply holds
/// its id, also when its panel is closed: here only the Color Guide's limit and a closed panel's place
/// name it.
#[test]
fn ui_inspect_hides_a_closed_librarys_path_outside_the_read_root() {
    let (mut app, tx, base) = confined("inspect-closed-lib");
    let theirs = format!("loaded/{}", s(base.join("outside/b.svg")));
    app.ui.color_guide_limit = theirs.clone();
    hide_panel(&mut app, format!("library:swatches:{theirs}"));
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    let ui = &r["result"]["ui"];
    assert_eq!(ui["color_guide_limit"], "loaded/b.svg", "{r}");
    assert!(hidden_panels(ui).iter().any(|p| p == "library:swatches:loaded/b.svg"), "{r}");
    let hidden = json!(s(base.join("outside"))).to_string();
    assert!(!r.to_string().contains(hidden.trim_matches('"')), "no path outside the root: {r}");
    assert_eq!(crate::control::inspect(&app, &crate::testing::ui_context())["ui"]["color_guide_limit"], json!(theirs), "the person's");
    let _ = std::fs::remove_dir_all(base);
}

/// Two libraries loaded from outside the read root with the same file name stay two in `ui.inspect`:
/// the second one shows as `loaded/<file name> (2)`, the same in every place the reply names it.
#[test]
fn ui_inspect_keeps_two_hidden_libraries_with_one_file_name_apart() {
    let (mut app, tx, base) = confined("inspect-same-name");
    let (x, y) = (format!("loaded/{}", s(base.join("outside/x/b.svg"))), format!("loaded/{}", s(base.join("outside/y/b.svg"))));
    let panel = |id: &str| -> crate::panels::library_panel::LibraryPanel { serde_json::from_value(json!({"kind": "swatches", "id": id})).unwrap() };
    app.ui.library_panels = vec![panel(&x), panel(&y)];
    for id in [&x, &y] {
        hide_panel(&mut app, format!("library:swatches:{id}"));
    }
    app.ui.color_guide_limit = y.clone();
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    let ui = &r["result"]["ui"];
    let ids: Vec<_> = ui["library_panels"].as_array().unwrap().iter().map(|p| p["id"].clone()).collect();
    assert_eq!(ids, [json!("loaded/b.svg"), json!("loaded/b.svg (2)")], "{r}");
    assert_eq!(hidden_panels(ui), ["library:swatches:loaded/b.svg", "library:swatches:loaded/b.svg (2)"], "no entry is dropped: {r}");
    assert_eq!(ui["color_guide_limit"], "loaded/b.svg (2)", "the same id everywhere");
    let _ = std::fs::remove_dir_all(base);
}

/// `ui.inspect` and `ui.docking` show a hidden library under the same id, also when another hidden
/// library with the same file name is in only one of the replies (here as the Color Guide's limit).
#[test]
fn ui_inspect_and_ui_docking_give_a_hidden_library_the_same_id() {
    let (mut app, tx, base) = confined("inspect-docking-same-id");
    let (x, y) = (format!("loaded/{}", s(base.join("outside/x/b.svg"))), format!("loaded/{}", s(base.join("outside/y/b.svg"))));
    let panel: crate::panels::library_panel::LibraryPanel = serde_json::from_value(json!({"kind": "swatches", "id": y})).unwrap();
    app.ui.library_panels = vec![panel];
    app.ui.open_panel = Some(format!("library:swatches:{y}"));
    app.ui.color_guide_limit = x.clone();
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    let ui = &r["result"]["ui"];
    assert_eq!(
        (ui["color_guide_limit"].as_str(), ui["open_panel"].as_str()),
        (Some("loaded/b.svg"), Some("library:swatches:loaded/b.svg (2)")),
        "{r}"
    );
    let d = request(&mut app, &tx, "ui.docking", json!({}));
    assert_eq!(d["result"]["flyout"], ui["open_panel"], "{d}");
    let _ = std::fs::remove_dir_all(base);
}

/// `ui.inspect` rewrites only the ids of libraries the UI state refers to: a script's report line and
/// a command palette query that look like a library id, with no such library, stay as written, and
/// the query does not take the file name of a real library outside the root.
#[test]
fn ui_inspect_keeps_text_that_only_looks_like_a_library_id() {
    let (mut app, tx, base) = confined("inspect-lookalike");
    let real = format!("loaded/{}", s(base.join("outside/x/d.svg")));
    let panel: crate::panels::library_panel::LibraryPanel = serde_json::from_value(json!({"kind": "swatches", "id": real})).unwrap();
    app.ui.library_panels = vec![panel];
    let query = format!("library:swatches:loaded/{}", s(base.join("outside/d.svg")));
    app.ui.palette_query = query.clone();
    let text = format!("loaded/{}", s(base.join("outside/c.svg")));
    let source = format!("vc.log({})", json!(text));
    app.session.execute("script.run", &json!({"source": source, "name": "t.js"})).unwrap();
    for _ in 0..10_000 {
        if app.session.script_pump().is_some() {
            break;
        }
    }
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    assert_eq!(r["result"]["scriptReport"]["log"], json!([text]), "{r}");
    assert_eq!(r["result"]["ui"]["palette_query"], json!(query), "{r}");
    assert_eq!(r["result"]["ui"]["library_panels"][0]["id"], "loaded/d.svg", "{r}");
    let _ = std::fs::remove_dir_all(base);
}

/// `ui.inspect`'s `scriptReport` names a file the person's script wrote outside the read root (in
/// the folder the person allowed it), and the included file a failed run stopped in (also in its
/// error line), by its file name; the person sees the paths.
#[test]
fn ui_inspect_script_report_leaves_out_files_outside_the_read_root() {
    let (mut app, tx, base) = confined("inspect-script-report");
    let (kept, hidden) = (s(base.join("in/kept.svg")), s(base.join("outside/hidden.svg")));
    app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let source = format!(
        "Folder.selectDialog('Folder'); vc.run('document.export', {{path: {kept}, format: 'svg'}}); vc.run('document.export', {{path: {hidden}, format: 'svg'}});",
        kept = json!(kept),
        hidden = json!(hidden)
    );
    let finish = |app: &mut VectorcraftApp, params: Value| {
        app.session.execute("script.run", &params).unwrap();
        for _ in 0..10_000 {
            if app.session.script_picker().is_some() {
                let paths = vec![s(base.clone())];
                assert!(app.session.script_pick(vectorcraft_engine::script::files::PickAnswer::Allow { paths, always: false }));
            }
            if app.session.script_pump().is_some() {
                break;
            }
        }
    };
    finish(&mut app, json!({"source": source, "name": "t.js"}));
    let ctx = crate::testing::ui_context();
    let outside = json!(s(base.join("outside"))).to_string();
    let person = crate::control::inspect(&app, &ctx);
    assert_eq!(person["scriptReport"]["written"], json!([kept, hidden]), "{person}");
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    assert_eq!(r["result"]["scriptReport"]["written"], json!([kept, "hidden.svg"]), "{r}");
    assert!(!r.to_string().contains(outside.trim_matches('"')), "no path outside the root: {r}");
    assert_eq!(app.session.script_report().unwrap()["written"], json!([kept, hidden]), "the report is not changed");

    // A run that fails in a file it includes.
    std::fs::write(base.join("outside/run.jsx"), "#include \"bad.jsxinc\"\n").unwrap();
    std::fs::write(base.join("outside/bad.jsxinc"), "\nthrow new Error('stopped here');\n").unwrap();
    finish(&mut app, json!({"path": s(base.join("outside/run.jsx"))}));
    let person = crate::control::inspect(&app, &ctx);
    // The report names the file by its real path (on macOS the temp folder is reached through a link), as people
    // read it (on Windows without the `\\?\` prefix).
    let bad = vectorcraft_engine::cmd::findfiles::plain(std::fs::canonicalize(base.join("outside/bad.jsxinc")).unwrap());
    assert_eq!(person["scriptReport"]["file"], json!(s(bad)), "{person}");
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    assert_eq!(r["result"]["scriptReport"]["file"], "bad.jsxinc", "{r}");
    assert!(r["result"]["scriptReport"]["console"].to_string().contains("bad.jsxinc:2:"), "{r}");
    assert!(!r.to_string().contains(outside.trim_matches('"')), "no path outside the root: {r}");
    let _ = std::fs::remove_dir_all(base);
}

/// `ui.inspect` rewrites a hidden library's id only where a string is that whole id: a readable
/// library's id that begins with it stays, and a script's report line and a command palette query
/// that end with it show the id shown.
#[test]
fn ui_inspect_rewrites_only_whole_hidden_library_ids() {
    let (mut app, tx, base) = confined("inspect-whole-id");
    let hidden = format!("loaded/{}", s(base.join("i")));
    let readable = format!("loaded/{}", s(base.join("in/a.svg")));
    assert!(readable.starts_with(&hidden));
    let panel = |id: &str| -> crate::panels::library_panel::LibraryPanel { serde_json::from_value(json!({"kind": "swatches", "id": id})).unwrap() };
    app.ui.library_panels = vec![panel(&hidden), panel(&readable)];
    app.ui.palette_query = format!("library:swatches:{hidden}");
    let text = format!("found {hidden}");
    let source = format!("vc.log({})", json!(text));
    app.session.execute("script.run", &json!({"source": source, "name": "t.js"})).unwrap();
    for _ in 0..10_000 {
        if app.session.script_pump().is_some() {
            break;
        }
    }
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    let ids: Vec<_> = r["result"]["ui"]["library_panels"].as_array().unwrap().iter().map(|p| p["id"].clone()).collect();
    assert_eq!(ids, [json!("loaded/i"), json!(readable)], "{r}");
    assert_eq!(r["result"]["scriptReport"]["log"], json!(["found loaded/i"]), "{r}");
    assert_eq!(r["result"]["ui"]["palette_query"], "library:swatches:loaded/i", "{r}");
    // A request that names the hidden library by the id shown keeps the readable id too.
    let r = request(&mut app, &tx, "ui.inspect", json!({"library": "loaded/i"}));
    let ids: Vec<_> = r["result"]["ui"]["library_panels"].as_array().unwrap().iter().map(|p| p["id"].clone()).collect();
    assert_eq!(ids, [json!("loaded/i"), json!(readable)], "{r}");
    let _ = std::fs::remove_dir_all(base);
}

/// A client can name a hidden library by the id `ui.inspect` shows: `ui.set {panel}` with that panel id
/// reopens the closed panel of the library, and the reply does not show its path.
#[test]
fn ui_set_reopens_a_hidden_librarys_panel_by_its_shown_id() {
    let (mut app, tx, base) = confined("set-hidden-panel");
    let theirs = format!("loaded/{}", s(base.join("outside/b.svg")));
    let real = format!("library:swatches:{theirs}");
    hide_panel(&mut app, real.clone());
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    let shown = "library:swatches:loaded/b.svg";
    assert!(hidden_panels(&r["result"]["ui"]).iter().any(|p| p == shown), "{r}");
    let r = request(&mut app, &tx, "ui.set", json!({"panel": shown}));
    assert_eq!(r["ok"], true, "{r}");
    assert!(!app.ui.dock_workspace.hidden().iter().any(|(p, _)| p == &real), "the closed panel is reopened: {r}");
    let layout = crate::panel_docking::layout_of(&app.ui);
    assert!(layout.contains(&real) && !layout.contains(&shown.to_string()), "no second panel: {layout:?}");
    let hidden = json!(s(base.join("outside"))).to_string();
    assert!(!r.to_string().contains(hidden.trim_matches('"')), "no path outside the root: {r}");
    // A docking command takes the shown id as its destination too.
    let r = request(
        &mut app,
        &tx,
        "engine.execute",
        json!({"command": "window.panel.move", "params": {"panel": "layers", "anchor": shown, "zone": "tab"}}),
    );
    assert_eq!(r["ok"], true, "{r}");
    assert!(!r.to_string().contains(hidden.trim_matches('"')), "no path outside the root: {r}");
    // So does `ui.dock`'s move, whose destination is `target`.
    let r = request(
        &mut app,
        &tx,
        "engine.execute",
        json!({"command": "ui.dock", "params": {"operation": "move", "panel": "properties", "target": shown, "zone": "tab"}}),
    );
    assert_eq!(r["ok"], true, "{r}");
    let layout = crate::panel_docking::layout_of(&app.ui);
    assert!(layout.contains(&real) && !layout.contains(&shown.to_string()), "no second panel: {layout:?}");
    assert!(!r.to_string().contains(hidden.trim_matches('"')), "no path outside the root: {r}");
    // An error that quotes the panel shows the id the client gave, and other text is not rewritten.
    let r = request(&mut app, &tx, "ui.set", json!({"panel": "library:bogus:loaded/b.svg"}));
    assert_eq!(r["ok"], false, "{r}");
    assert!(!r.to_string().contains(hidden.trim_matches('"')), "no path outside the root: {r}");
    let r = request(&mut app, &tx, "ui.set", json!({"status": "loaded/b.svg"}));
    assert_eq!((r["ok"].as_bool(), app.ui.status.as_str()), (Some(true), "loaded/b.svg"), "{r}");
    let _ = std::fs::remove_dir_all(base);
}

/// A shortcut for showing the panel of a library outside the read root, saved in a shortcut set, is
/// keyed by the panel id shown, also when nothing else refers to the library.
#[test]
fn ui_inspect_hides_a_library_panels_path_in_shortcut_overrides() {
    let (mut app, tx, base) = confined("shortcut-hidden-panel");
    let real = format!("panel:library:swatches:loaded/{}", s(base.join("outside/b.svg")));
    let mut overrides = craft::keymap::Overrides::new();
    overrides.insert(real, vec!["Cmd+Alt+Shift+F7".to_owned()]);
    app.ui.shortcut_sets.save_as("Mine", overrides, &crate::shortcut_editor::BUILTINS).unwrap();
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    let shown = "panel:library:swatches:loaded/b.svg";
    // The sets are saved in the keymap's own form: a list of named sets, each a list of overrides.
    let sets = r["result"]["ui"]["shortcut_sets"]["sets"].as_array().cloned().unwrap_or_default();
    let mine = sets.iter().find(|set| set["name"] == "Mine").map_or(Value::Null, |set| set["overrides"].clone());
    let keys = mine.as_array().and_then(|o| o.iter().find(|entry| entry["command"] == shown)).map(|entry| entry["keys"].clone());
    assert_eq!(keys, Some(json!(["Cmd+Alt+Shift+F7"])), "{r}");
    let hidden = json!(s(base.join("outside"))).to_string();
    assert!(!r.to_string().contains(hidden.trim_matches('"')), "no path outside the root: {r}");
    let _ = std::fs::remove_dir_all(base);
}

/// A client without automation roots sees every loaded library's full id in `ui.inspect`.
#[test]
fn ui_inspect_shows_full_library_ids_without_a_read_root() {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = VectorcraftApp::new(Session::new(), Default::default()).with_control(rx);
    let theirs = format!("loaded/{}", s(std::env::temp_dir().join("vc-ui-roots-unconfined").join("b.svg")));
    let panel: crate::panels::library_panel::LibraryPanel = serde_json::from_value(json!({"kind": "swatches", "id": theirs})).unwrap();
    app.ui.library_panels = vec![panel];
    app.ui.color_guide_limit = theirs.clone();
    hide_panel(&mut app, format!("library:swatches:{theirs}"));
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    let ui = &r["result"]["ui"];
    assert_eq!((&ui["library_panels"][0]["id"], &ui["color_guide_limit"]), (&json!(theirs), &json!(theirs)), "{r}");
    assert!(hidden_panels(ui).contains(&format!("library:swatches:{theirs}")), "{r}");
}

/// A plug-in's permissions are approved only by a person who asked for the install. A control
/// request (or MCP, which comes through it) can't pass `grant`, can't answer the approval dialog or
/// set what it installs, and the question it asked keeps OK disabled for the person too: they start
/// Install Plug-in… themselves, which asks again, and its OK installs.
#[test]
fn automation_cannot_approve_a_plugin_install() {
    use super::plugins::{approve, at, ok_enabled, waiting, window};
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = VectorcraftApp::new(Session::new(), Default::default()).with_control(rx);
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let mut h = window(app);
    let id = "test.ui.approval.automation";
    let (bytes, data) = approval_module(id);
    let refused = |r: &Value| assert_eq!(r["ok"], false, "{r}");

    let r = request_drawn(&mut h, &tx, "engine.execute", json!({"command": "plugin.install", "params": {"dataBase64": data, "grant": ["doc.read"]}}));
    assert!(r["error"].as_str().is_some_and(|e| e.contains("--allow-plugin-grants")), "{r}");
    assert!(open_dialogs(h.state()).is_empty());

    let r = request_drawn(&mut h, &tx, "engine.execute", json!({"command": "ui.installPlugin", "params": {"dataBase64": data}}));
    assert_eq!(r["result"]["pending"], crate::dialogs::plugin::INSTALL_KIND, "{r}");
    assert_eq!(r["result"]["approval"], crate::dialogs::plugin::PERSON_MUST_START_INSTALL, "{r}");
    refused(&request_drawn(&mut h, &tx, "ui.dialog.confirm", json!({})));
    assert!(waiting(h.state(), id), "the refused answer leaves it open");

    // What the question installs isn't among its fields, and no field can be set: alone, or as MCP's
    // `answer_dialog` sends it, `ui.dialog.set {fields}` then `ui.dialog.confirm`.
    refused(&request_drawn(&mut h, &tx, "ui.dialog.set", json!({"field": "__params", "value": {"dataBase64": "AAAA"}})));
    refused(&request_drawn(&mut h, &tx, "ui.dialog.set", json!({"fields": [{"field": "__params", "value": {"dataBase64": "AAAA"}}]})));
    refused(&request_drawn(&mut h, &tx, "ui.dialog.confirm", json!({})));
    assert!(waiting(h.state(), id));
    // Shown for long enough for a person's OK, it still refuses automation, and the person too.
    let time = h.input_mut().time.unwrap_or(0.0);
    at(&mut h, time + 1.0);
    at(&mut h, time + 2.0);
    refused(&request_drawn(&mut h, &tx, "ui.dialog.confirm", json!({})));
    assert!(!ok_enabled(&h), "a person's OK on automation's question");
    assert!(waiting(h.state(), id));
    // Automation may still close its question.
    assert_eq!(request_drawn(&mut h, &tx, "ui.dialog.cancel", json!({}))["ok"], true);
    assert!(open_dialogs(h.state()).is_empty() && h.state().pending_plugin_install.is_none());

    // The person starts Install Plug-in… with the file: a new question, which a person's OK approves
    // once it was shown for the delay, and which automation still can't.
    let r = h.state_mut().run("ui.installPlugin", json!({"dataBase64": data})).unwrap();
    assert_eq!(r, json!({"pending": crate::dialogs::plugin::INSTALL_KIND}), "the person's question");
    refused(&request_drawn(&mut h, &tx, "ui.dialog.confirm", json!({})));
    assert!(waiting(h.state(), id));
    approve(&mut h);
    assert_eq!(vectorcraft_plugins::registry::granted(id).map(|g| g.hash), Some(vectorcraft_plugins::permissions::sha256_hex(&bytes)));
    h.state_mut().run("plugin.remove", json!({"id": id})).unwrap();
}

/// An app with a control channel and a new document.
fn controlled() -> (VectorcraftApp, std::sync::mpsc::Sender<ControlRequest>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()).with_control(rx));
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    (app, tx)
}

/// The fields of the top dialog, as automation lists them.
fn dialog_fields(app: &VectorcraftApp) -> Option<Value> {
    open_dialogs(app).pop().and_then(|dialog| dialog.fields)
}

/// The objects on the document's first layer.
fn objects(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().doc.layers[0].children().map_or(0, |c| c.len())
}

/// The left edge and width of the first object on the document's first layer.
fn first_object(app: &VectorcraftApp) -> (f64, f64) {
    let b = app.session.doc().unwrap().doc.layers[0].children().unwrap()[0].geometric_bounds().unwrap();
    (b.x0, b.width())
}

/// Each of `sets` sent as `ui.dialog.set` is refused, sets no field, and runs nothing.
fn refused_sets(app: &mut VectorcraftApp, tx: &std::sync::mpsc::Sender<ControlRequest>, sets: &[Value]) {
    for set in sets {
        let before = (dialog_fields(app), objects(app));
        let r = request(app, tx, "ui.dialog.set", set.clone());
        assert_eq!(r["ok"], false, "{set}: {r}");
        assert_eq!((dialog_fields(app), objects(app)), before, "{set}: no field changes, the dialog stays open and nothing ran");
    }
}

/// A confirmation a person opened runs the command it was opened with: `ui.dialog.set` can't
/// change the command or its params, alone or in a list with `action: "confirm"`, and OK then runs
/// the command it asked about.
#[test]
fn dialog_set_cannot_redirect_person_opened_confirm() {
    let (mut app, tx) = controlled();
    let rect = json!({"x": 10, "y": 10, "width": 20, "height": 20});
    crate::dialogs::confirm::ask(&mut app, "Add a rectangle?", "", "shape.rectangle", rect);
    let other = json!({"x": 0, "y": 0, "width": 90, "height": 90});
    refused_sets(
        &mut app,
        &tx,
        &[
            json!({"field": "__command", "value": "file.close"}),
            json!({"field": "__params", "value": other}),
            json!({"fields": [{"field": "detail", "value": "Sure?"}, {"field": "__command", "value": "file.close"}], "action": "confirm"}),
            json!({"fields": [{"field": "__params", "value": other}, {"field": "detail", "value": "Sure?"}], "action": "confirm"}),
        ],
    );
    let command = crate::dialogs::confirm::open_question(&app).map(|(_, _, command)| command);
    assert_eq!(command.as_deref(), Some("shape.rectangle"), "it asks about its command still");
    let r = request(&mut app, &tx, "ui.dialog.confirm", json!({}));
    assert_eq!(r["ok"], true, "{r}");
    assert!(open_dialogs(&app).is_empty() && objects(&app) == 1, "OK ran the command it asked about");
    assert_eq!(first_object(&app), (10.0, 20.0));
}

/// The generic parameter dialog runs its command with its fixed parameters: `ui.dialog.set` can
/// change the parameters it shows, not the command, its fixed parameters or its heading.
#[test]
fn dialog_set_cannot_redirect_parameter_dialog() {
    let (mut app, tx) = controlled();
    let params = json!({"__params": {"x": 10, "y": 10}, "width": 20, "height": 20}).as_object().cloned().unwrap();
    crate::dialogs::command::open(&mut app, "shape.rectangle", "Rectangle", params).unwrap();
    refused_sets(
        &mut app,
        &tx,
        &[
            json!({"field": "__command", "value": "file.close"}),
            json!({"field": "__params", "value": {"x": 500, "y": 500}}),
            json!({"field": "__label", "value": "Close"}),
            json!({"fields": [{"field": "width", "value": 30}, {"field": "__command", "value": "file.close"}], "action": "confirm"}),
        ],
    );
    let id = open_dialogs(&app).pop().expect("the parameter dialog").id;
    let r = request(&mut app, &tx, "ui.dialog.set", json!({"field": "width", "value": 30, "action": "confirm"}));
    assert_eq!(r["ok"], true, "{r}");
    // The dialog holds its OK for the pump; the OK the pump accepted closes it in the dialog's next pass, which
    // `request`'s bare frames don't draw.
    let (ctx, host) = (app.ui_ctx.clone().expect("a window context"), app.dialogs().expect("the window's dialog host"));
    if host.with(|h| h.held_state(id) == Some(craft::ui::dialog::HeldOk::Accepted) && h.confirm_by_id(&ctx, id)) {
        crate::dialogs::host::pump(&mut app, &ctx);
    }
    assert!(open_dialogs(&app).is_empty() && objects(&app) == 1, "OK ran the dialog's command");
    assert_eq!(first_object(&app), (10.0, 30.0), "with its fixed x and the width set");
}

/// A plug-in command's dialog runs that command of that plug-in: `ui.dialog.set` can't change the
/// command or the plug-in it runs.
#[test]
fn dialog_set_cannot_redirect_plugin_command() {
    let (mut app, tx) = controlled();
    let id = "test.ui.redirect";
    let manifest = format!(
        r#"{{"id":"{id}","name":"Redirect","version":"1","kind":"filter","commands":[{{"id":"a","label":"A",
        "params":{{"amount":{{"type":"number","min":0,"max":10,"default":1}}}}}},{{"id":"b","label":"B"}}]}}"#
    );
    let wasm = wat::parse_str(vectorcraft_plugins::wat::returning(&manifest, r#"{"objects":[]}"#)).unwrap();
    app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm)})).unwrap();
    crate::menus::invoke(&mut app, "plugin.test.ui.redirect.a", json!({}));
    refused_sets(
        &mut app,
        &tx,
        &[
            json!({"field": "__command", "value": "plugin.test.ui.redirect.b"}),
            json!({"field": "__plugin", "value": "test.ui.other"}),
            json!({"fields": [{"field": "amount", "value": 2}, {"field": "__command", "value": ""}], "action": "confirm"}),
        ],
    );
    let r = request(&mut app, &tx, "ui.dialog.set", json!({"field": "amount", "value": 2}));
    assert_eq!(r["ok"], true, "{r}");
    app.run("plugin.remove", json!({"id": id})).unwrap();
}

/// The `__` fields Transform Each keeps for itself are not controls: `ui.dialog.set` refuses them as
/// it refuses a field the dialog does not hold, and sets nothing.
#[test]
fn dialog_set_rejects_unknown_hidden_field() {
    let (mut app, tx) = controlled();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 60, "height": 20})).unwrap();
    app.run("select.all", json!({})).unwrap();
    crate::menus::invoke(&mut app, "object.transformEach", json!({}));
    let sets: Vec<Value> =
        ["__anything", "__constrainApplied", "__ratioBase", "nosuch"].iter().map(|field| json!({"field": field, "value": true})).collect();
    refused_sets(&mut app, &tx, &sets);
}

/// The dialog controls held under a `__` name are still set: Transform Each's link, and the
/// Preferences search through the routing of `ui.dialog.set` to Preferences.
#[test]
fn dialog_set_allows_hidden_controls() {
    let (mut app, tx) = controlled();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 60, "height": 20})).unwrap();
    app.run("select.all", json!({})).unwrap();
    crate::menus::invoke(&mut app, "object.transformEach", json!({}));
    let before = dialog_fields(&app);
    let r = request(&mut app, &tx, "ui.dialog.set", json!({"field": "__constrain", "value": true}));
    assert_eq!(r["ok"], true, "{r}");
    assert_ne!(dialog_fields(&app), before, "the link is set");
    let r = request(&mut app, &tx, "ui.dialog.cancel", json!({}));
    assert_eq!(r["ok"], true, "{r}");

    crate::prefs_dialog::open(&mut app, None);
    let r = request(&mut app, &tx, "ui.dialog.set", json!({"field": "__search", "value": "grid"}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(crate::prefs_dialog::fields(&app).unwrap()["__search"], "grid");
}

/// Save PDF's and Print's OK export and print: `ui.dialog.set` can't add a preset name to turn OK
/// into saving a preset, alone or with `action: "confirm"`. Saving a preset is the preset editor's.
#[test]
fn dialog_set_cannot_open_save_preset() {
    let (mut app, tx) = controlled();
    let saved = |app: &VectorcraftApp| (app.session.prefs.pdf_presets.len(), app.session.prefs.print_presets.len());
    for open in ["ui.savePdfDialog", "file.print"] {
        app.run(open, json!({})).unwrap();
        // The pump gives the dialog the fields it derives (Print's inks and summary) before they are compared.
        let ctx = app.ui_ctx.clone().unwrap_or_default();
        crate::dialogs::host::pump(&mut app, &ctx);
        let presets = saved(&app);
        refused_sets(
            &mut app,
            &tx,
            &[
                json!({"field": "__savePresetAs", "value": "Proofs"}),
                json!({"fields": [{"field": "__savePresetAs", "value": "Proofs"}], "action": "confirm"}),
            ],
        );
        assert_eq!(saved(&app), presets, "{open}: no preset saved");
        let r = request(&mut app, &tx, "ui.dialog.cancel", json!({}));
        assert_eq!(r["ok"], true, "{open}: {r}");
    }
}

/// What the install question automation asked says instead of taking an OK.
const ASKED: &str = "Automation asked to install this plug-in. To install it, choose Object › Plug-ins › Install Plug-in… yourself.";

/// The approval module `id` that lists `doc.read`, and its bytes as base64.
fn approval_module(id: &str) -> (Vec<u8>, String) {
    let manifest = format!(r#"{{"id":"{id}","name":"Approval","version":"1","kind":"filter","permissions":["doc.read"]}}"#);
    let bytes = wat::parse_str(vectorcraft_plugins::wat::returning(&manifest, r#"{"objects":[]}"#)).unwrap();
    let data = vectorcraft_format::base64_encode(&bytes);
    (bytes, data)
}

/// Who asked the install question waiting for an answer.
fn install_origin(app: &VectorcraftApp) -> Option<InstallOrigin> {
    app.pending_plugin_install.as_ref().map(|pending| pending.origin)
}

/// An install question that a control request asked keeps OK disabled in the person's window, even
/// once it was shown for the delay: a person's click, Enter or Space install nothing, and the dialog
/// says to start Install Plug-in…. Cancel still closes it at once.
#[test]
fn a_person_cannot_approve_an_install_question_automation_asked() {
    use super::plugins::{at, click_ok, ok_enabled, waiting, window};
    use egui_kittest::kittest::Queryable;
    let id = "test.ui.approval.asked";
    let (tx, rx) = std::sync::mpsc::channel();
    let app = VectorcraftApp::new(Session::new(), Default::default()).with_control(rx);
    let mut h = window(app);
    let (_, data) = approval_module(id);
    let r = request_drawn(&mut h, &tx, "engine.execute", json!({"command": "ui.installPlugin", "params": {"dataBase64": data}}));
    assert_eq!(r["result"]["approval"], crate::dialogs::plugin::PERSON_MUST_START_INSTALL, "{r}");
    at(&mut h, 10.0);
    at(&mut h, 10.0);
    at(&mut h, 10.6);
    assert!(!ok_enabled(&h), "OK stays disabled past the delay");
    assert!(h.query_by_label(ASKED).is_some(), "the dialog says what to do");
    click_ok(&mut h, 10.7);
    at(&mut h, 10.8);
    assert!(waiting(h.state(), id), "a person's click");
    for (key, time) in [(egui::Key::Enter, 10.9), (egui::Key::Space, 11.0)] {
        h.key_press(key);
        at(&mut h, time);
        assert!(waiting(h.state(), id), "{key:?}");
    }
    let r = request_drawn(&mut h, &tx, "ui.dialog.confirm", json!({}));
    assert_eq!(r["ok"], false, "a confirm that skips the button: {r}");
    assert!(waiting(h.state(), id));
    h.get_by_label("Cancel").click();
    at(&mut h, 11.1);
    assert!(open_dialogs(h.state()).is_empty() && h.state().pending_plugin_install.is_none(), "Cancel");
    assert!(vectorcraft_plugins::registry::get(id).is_none() && vectorcraft_plugins::registry::granted(id).is_none());
}

/// A question automation asks in place of the one a person asked can't be approved: it keeps
/// automation's origin, though the person's question was shown and ready.
#[test]
fn automation_replacing_a_persons_install_question_does_not_take_its_approval() {
    let (mut app, tx) = controlled();
    let (mine, theirs) = ("test.ui.approval.mine", "test.ui.approval.theirs");
    app.run("ui.installPlugin", json!({"dataBase64": approval_module(mine).1})).unwrap();
    assert_eq!(install_origin(&app), Some(InstallOrigin::Person));
    let install = |data: String| json!({"command": "ui.installPlugin", "params": {"dataBase64": data}});
    let r = request(&mut app, &tx, "engine.execute", install(approval_module(theirs).1));
    assert_eq!(r["result"]["approval"], crate::dialogs::plugin::PERSON_MUST_START_INSTALL, "{r}");
    assert_eq!(install_origin(&app), Some(InstallOrigin::Automation), "the question is automation's");
    // Asked again by automation with the person's file, it is still automation's.
    let r = request(&mut app, &tx, "engine.execute", install(approval_module(mine).1));
    assert_eq!(r["result"]["approval"], crate::dialogs::plugin::PERSON_MUST_START_INSTALL, "{r}");
    assert_eq!(install_origin(&app), Some(InstallOrigin::Automation));
    assert_eq!(request(&mut app, &tx, "ui.dialog.confirm", json!({}))["ok"], false);
    for id in [mine, theirs] {
        assert!(vectorcraft_plugins::registry::get(id).is_none() && vectorcraft_plugins::registry::granted(id).is_none(), "{id}");
    }
}

/// A file a person picks in the file dialog that a control request opened (Install Plug-in… or
/// File › Open with no path) is automation's request: its question can't be approved. The file
/// dialog a person opens asks a question they can approve. A file a person drops is theirs; one the
/// web page's script hands over is automation's.
#[test]
fn a_file_picked_in_a_dialog_automation_opened_stays_automations() {
    use super::plugins::waiting;
    let base = std::env::temp_dir().join(format!("vc-ui-approval-picked-{}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    let picked = "test.ui.approval.picked";
    let wasm = base.join("picked.wasm");
    std::fs::write(&wasm, approval_module(picked).0).unwrap();
    let services = Services { read: Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string()))), ..Default::default() };
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = VectorcraftApp::new(Session::new(), services).with_control(rx);
    let picker = super::picks::scripted(&mut app);
    let ctx = app.ui_ctx.clone().expect("the picker's window");
    let pick = |app: &mut VectorcraftApp| {
        picker.answer(&[&s(wasm.clone())]);
        super::picks::poll(app, &ctx);
    };
    for command in ["ui.installPlugin", "file.open"] {
        let r = request(&mut app, &tx, "ui.menu.invoke", json!({"command": command, "params": {}}));
        assert!(app.file_dialog_open(), "{command}: {r}");
        pick(&mut app);
        assert!(waiting(&app, picked), "{command}: {:?}", app.ui.status);
        assert_eq!(install_origin(&app), Some(InstallOrigin::Automation), "{command}");
        assert_eq!(request(&mut app, &tx, "ui.dialog.cancel", json!({}))["ok"], true, "{command}");
    }
    let _ = app.run("ui.installPlugin", json!({}));
    assert!(app.file_dialog_open());
    pick(&mut app);
    assert!(waiting(&app, picked));
    assert_eq!(install_origin(&app), Some(InstallOrigin::Person), "the person's file dialog");

    let (dropped, page) = <(crate::services::Inbox, crate::services::Inbox)>::default();
    let services = Services { inbox: Some(dropped.clone()), page_inbox: Some(page.clone()), ..Default::default() };
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), services));
    for (inbox, id, origin) in
        [(&dropped, "test.ui.approval.picked.dropped", InstallOrigin::Person), (&page, "test.ui.approval.picked.page", InstallOrigin::Automation)]
    {
        inbox.lock().unwrap().push((format!("{id}.wasm"), approval_module(id).0));
        app.drain_inbox();
        assert!(waiting(&app, id), "{id}");
        assert_eq!(install_origin(&app), Some(origin), "{id}");
    }
    let _ = std::fs::remove_dir_all(base);
}

/// A `.wasm` that a control request opens by path (`app.open`) asks a question that can't be
/// approved; one the person opens can be.
#[test]
fn a_plugin_automation_opened_by_path_asks_a_question_no_one_approves() {
    let base = std::env::temp_dir().join(format!("vc-ui-approval-open-{}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    let id = "test.ui.approval.opened";
    let wasm = base.join("opened.wasm");
    std::fs::write(&wasm, approval_module(id).0).unwrap();
    let services = Services { read: Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string()))), ..Default::default() };
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), services).with_control(rx));
    let r = request(&mut app, &tx, "app.open", json!({"path": s(wasm.clone())}));
    assert!(r.get("error").is_none(), "{r}");
    assert!(super::plugins::waiting(&app, id));
    assert_eq!(install_origin(&app), Some(InstallOrigin::Automation));
    assert_eq!(request(&mut app, &tx, "ui.dialog.confirm", json!({}))["ok"], false);
    assert_eq!(request(&mut app, &tx, "ui.dialog.cancel", json!({}))["ok"], true);
    crate::io::open_path(&mut app, &s(wasm.clone())).unwrap();
    assert!(super::plugins::waiting(&app, id));
    assert_eq!(install_origin(&app), Some(InstallOrigin::Person), "the person's open");
    let _ = std::fs::remove_dir_all(base);
}

/// A module that takes the place of another with its id, and asks for nothing else: automation's
/// `replace: true` doesn't skip the question and its question can't be approved. The question a
/// person asked keeps what it installs out of reach of `ui.dialog.set`, refuses a control confirm,
/// and the person's own OK replaces.
#[test]
fn automation_cannot_approve_replacing_a_plugin() {
    use super::plugins::{approve, window};
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = VectorcraftApp::new(Session::new(), Default::default()).with_control(rx);
    let id = "test.ui.approval.replaced";
    let plain = |output: &str| {
        wat::parse_str(vectorcraft_plugins::wat::returning(&format!(r#"{{"id":"{id}","name":"Plain","kind":"filter"}}"#), output)).unwrap()
    };
    let (first, other) = (plain(r#"{"objects":[]}"#), plain(r#"{"objects":[],"error":null}"#));
    let installed = || vectorcraft_plugins::registry::get(id).map(|p| p.hash().to_string());
    let first_hash = vectorcraft_plugins::permissions::sha256_hex(&first);
    let data = vectorcraft_format::base64_encode(&other);
    app.run("ui.installPlugin", json!({"dataBase64": vectorcraft_format::base64_encode(&first)})).unwrap();
    assert_eq!(installed(), Some(first_hash.clone()));
    let mut h = window(app);

    for params in [json!({"dataBase64": data}), json!({"dataBase64": data, "replace": true})] {
        let r = request_drawn(&mut h, &tx, "engine.execute", json!({"command": "ui.installPlugin", "params": params}));
        assert_eq!(r["result"]["pending"], crate::dialogs::plugin::INSTALL_KIND, "{params}: {r}");
        assert_eq!(r["result"]["approval"], crate::dialogs::plugin::PERSON_MUST_START_INSTALL, "{params}: {r}");
        let pending = h.state().pending_plugin_install.as_ref().unwrap();
        assert_eq!(pending.message, "Install “Plain” in place of the plug-in with the same ID?");
        assert_eq!(install_origin(h.state()), Some(InstallOrigin::Automation), "{params}");
        let r = request_drawn(&mut h, &tx, "ui.dialog.confirm", json!({}));
        assert_eq!(r["ok"], false, "{params}: {r}");
        assert_eq!(request_drawn(&mut h, &tx, "ui.dialog.cancel", json!({}))["ok"], true, "{params}");
        assert_eq!(installed(), Some(first_hash.clone()), "{params}");
    }

    // The person's question: a control request can't point its OK at other bytes, nor press it.
    let r = h.state_mut().run("ui.installPlugin", json!({"dataBase64": data})).unwrap();
    assert_eq!(r, json!({"pending": crate::dialogs::plugin::INSTALL_KIND}));
    let evil = vectorcraft_format::base64_encode(&plain(r#"{"objects":[{"name":"x"}]}"#));
    let r = request_drawn(&mut h, &tx, "ui.dialog.set", json!({"fields": [{"field": "__params", "value": {"dataBase64": evil, "replace": true}}]}));
    assert_eq!(r["ok"], false, "{r}");
    let r = request_drawn(&mut h, &tx, "ui.dialog.confirm", json!({}));
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(installed(), Some(first_hash.clone()));
    approve(&mut h);
    assert_eq!(installed(), Some(vectorcraft_plugins::permissions::sha256_hex(&other)), "the bytes the question showed");
    h.state_mut().run("plugin.remove", json!({"id": id})).unwrap();
    // A module that asks for nothing still installs at once when automation asks.
    let free = "test.ui.approval.free";
    let module = wat::parse_str(vectorcraft_plugins::wat::returning(&format!(r#"{{"id":"{free}","name":"Free","kind":"filter"}}"#), "{}")).unwrap();
    let params = json!({"dataBase64": vectorcraft_format::base64_encode(&module)});
    let r = request_drawn(&mut h, &tx, "engine.execute", json!({"command": "ui.installPlugin", "params": params}));
    assert!(r["result"]["pending"].is_null() && open_dialogs(h.state()).is_empty(), "{r}");
    assert!(vectorcraft_plugins::registry::get(free).is_some());
    h.state_mut().run("plugin.remove", json!({"id": free})).unwrap();
}

/// [`request`] for an app the window draws: its passes take the injected input and answer the
/// dialogs, and the reply comes once they have.
fn request_drawn(h: &mut super::plugins::Window, tx: &std::sync::mpsc::Sender<ControlRequest>, method: &str, params: Value) -> Value {
    let ctx = h.ctx.clone();
    let (req, reply) = ControlRequest::new(method, params);
    tx.send(req).unwrap();
    for _ in 0..64 {
        h.state_mut().run_control(&ctx);
        h.step();
        if let Ok(r) = reply.try_recv() {
            return r;
        }
    }
    reply.try_recv().unwrap()
}

/// Keys and pointer input sent through the control path approve nothing at the install approval a
/// person asked for: Enter and Space press nothing with no button focused, and with OK focused they
/// and a `ui.click` on it press it and are refused. `ui.pointer` drives the canvas. A person's click
/// on the same button approves.
#[test]
fn injected_keys_and_pointer_input_cannot_approve_a_plugin_install() {
    use super::plugins::{approve, at, waiting, window};
    use egui_kittest::kittest::{NodeT, Queryable};
    let refusal = "Only the person at the keyboard";
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = VectorcraftApp::new(Session::new(), Default::default()).with_control(rx);
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let id = "test.ui.approval.injected";
    let (bytes, data) = approval_module(id);
    let mut h = window(app);
    let r = h.state_mut().run("ui.installPlugin", json!({"dataBase64": data})).unwrap();
    assert_eq!(r, json!({"pending": crate::dialogs::plugin::INSTALL_KIND}), "the person's question");
    // Shown past the delay, so its OK takes input.
    let time = h.input_mut().time.unwrap_or(0.0);
    at(&mut h, time + 1.0);
    at(&mut h, time + 2.0);

    for key in ["Enter", "Space"] {
        assert!(!h.get_by_label("OK").accesskit_node().is_focused(), "no button has the focus yet");
        h.state_mut().ui.status.clear();
        let r = request_drawn(&mut h, &tx, "ui.key", json!({"key": key}));
        assert_eq!(r["ok"], true, "{r}");
        assert!(waiting(h.state(), id), "{key}");
        assert_eq!(h.state().ui.status, "", "{key} presses nothing");
    }

    for key in ["Enter", "Space"] {
        h.get_by_label("OK").focus();
        h.run_steps(2);
        assert!(h.get_by_label("OK").accesskit_node().is_focused());
        h.state_mut().ui.status.clear();
        let r = request_drawn(&mut h, &tx, "ui.key", json!({"key": key}));
        assert_eq!(r["ok"], true, "{r}");
        assert!(waiting(h.state(), id), "{key} on a focused OK");
        assert!(h.state().ui.status.contains(refusal), "{key} pressed OK and was refused: {:?}", h.state().ui.status);
    }

    let at_ok = h.get_by_label("OK").rect().center();
    h.state_mut().ui.status.clear();
    let r = request_drawn(&mut h, &tx, "ui.click", json!({"x": at_ok.x, "y": at_ok.y}));
    assert_eq!(r["ok"], true, "{r}");
    assert!(waiting(h.state(), id), "ui.click");
    assert!(h.state().ui.status.contains(refusal), "ui.click pressed OK and was refused: {:?}", h.state().ui.status);

    let events = json!([{"kind": "down", "x": at_ok.x, "y": at_ok.y}, {"kind": "up", "x": at_ok.x, "y": at_ok.y}]);
    let r = request_drawn(&mut h, &tx, "ui.pointer", json!({"events": events}));
    assert_eq!(r["ok"], true, "{r}");
    assert!(waiting(h.state(), id), "ui.pointer");

    approve(&mut h);
    assert!(open_dialogs(h.state()).is_empty(), "{:?}", h.state().ui.status);
    assert_eq!(vectorcraft_plugins::registry::granted(id).map(|g| g.hash), Some(vectorcraft_plugins::permissions::sha256_hex(&bytes)));
    h.state_mut().run("plugin.remove", json!({"id": id})).unwrap();
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
    let f = opened.clone();
    let services = Services {
        open_file: Some(Box::new(move |p: &str| {
            f.borrow_mut().push(p.to_string());
            Ok(())
        })),
        ..Default::default()
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = VectorcraftApp::new(Session::new(), services).with_control(rx);
    // The URLs a frame of the app asks the window to open: a link opens in the pass after the dialogs' pump.
    let ctx = crate::testing::ui_context();
    let urls = |app: &mut VectorcraftApp| -> Vec<String> {
        let out = crate::testing::app_frame(app, &ctx, egui::RawInput::default());
        let opens =
            out.platform_output.commands.iter().filter_map(|c| if let egui::OutputCommand::OpenUrl(u) = c { Some(u.url.clone()) } else { None });
        opens.collect()
    };
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let image = app.run("file.place", json!({"path": s(png.clone())})).unwrap()["ids"][0].as_u64().unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap();
    app.run("attributes.set", json!({"url": "https://example.com/a"})).unwrap();
    for (command, params) in [("attributes.openUrl", json!({})), ("links.editOriginal", json!({"id": image}))] {
        let r = request(&mut app, &tx, "engine.execute", json!({"command": command, "params": params}));
        assert!(r["error"].as_str().is_some_and(|e| e.contains("only the person at the keyboard")), "{command}: {r}");
    }
    assert!(opened.borrow().is_empty() && urls(&mut app).is_empty());
    // The person at the keyboard can.
    app.run("attributes.openUrl", json!({})).unwrap();
    app.run("links.editOriginal", json!({"id": image})).unwrap();
    assert_eq!(urls(&mut app), ["https://example.com/a"]);
    assert_eq!(opened.borrow().len(), 1);
    let _ = std::fs::remove_dir_all(base);
}
