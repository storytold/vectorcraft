//! `vectorcraft --control <port> --automation-read-root <dir> --automation-write-root <dir>` (#832):
//! control requests read and write only inside the roots; the person at the keyboard is not
//! confined.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_engine::file_access::AutomationRoots;

use crate::background::SharedWriteFn;
use crate::control::ControlRequest;
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
        assert!(r["result"].get("pending").is_none() && app.ui.dialog.is_none(), "no dialog opens: {r}");
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
        assert!(app.ui.dialog.is_none(), "{method} {params}: no dialog opens");
    }
    let r = request(&mut app, &tx, "engine.execute", execute("ui.shortcutImportDialog", base.join("in/a.kys")));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(crate::dialogs::shortcut_import::KIND));
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
    let sets = app.ui.action_sets.len();
    for command in ["actionSet.read", crate::panels::actions::LOAD_SET] {
        let r = request(&mut app, &tx, "engine.execute", execute(command, base.join("outside/b.aia")));
        assert!(r["error"].as_str().is_some_and(|e| e.contains("outside the read root")), "{command}: {r}");
    }
    assert_eq!(app.ui.action_sets.len(), sets, "no set added");
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
    let pixels = |v: &Value| image::load_from_memory(&vectorcraft_format::base64_decode(v.as_str().unwrap()).unwrap()).unwrap().to_rgba8();
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
    let pixels = |v: &Value| image::load_from_memory(&vectorcraft_format::base64_decode(v.as_str().unwrap()).unwrap()).unwrap().to_rgba8();
    let view = pixels(&request(&mut app, &tx, "ui.render", json!({"traceViews": true}))["result"]["pngBase64"]);
    let result = pixels(&request(&mut app, &tx, "ui.render", json!({}))["result"]["pngBase64"]);
    let (beside, middle) = (view.get_pixel(10, 10).0, view.get_pixel(50, 50).0);
    assert!(beside[0] < 200 && beside[2] > 245, "the source image beside the disc: {beside:?}");
    assert!(middle[..3].iter().all(|&c| c < 60), "the disc: {middle:?}");
    assert_eq!(result.get_pixel(10, 10).0, [255; 4], "the tracing result on white");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn frames_with_injected_input_are_confined() {
    let (mut app, tx, base) = confined("synthetic");
    let r = request(&mut app, &tx, "ui.key", json!({"key": "Escape"}));
    assert_eq!(r["ok"], true, "{r}");
    let ctx = egui::Context::default();
    let mut raw = egui::RawInput::default();
    app.raw_input_hook(&ctx, &mut raw);
    assert!(app.synthetic_frame, "the frame carrying the key");
    // The batch is acknowledged only after a settled UI pass took it (one step per pass).
    let mut out = ctx.run_ui(raw, |_| {});
    out.textures_delta.clear();
    let mut raw = egui::RawInput::default();
    app.raw_input_hook(&ctx, &mut raw);
    assert!(!app.synthetic_frame, "the frames after it are the person's");
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
        assert!(crate::opening::start(&mut app, path, svg.clone()).is_ok(), "{path} opens off the UI thread");
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
    app.ui.dialog = Some(crate::state::Dialog::new("preferences", json!({"fontsFolder": inside, "templatesFolder": outside})));

    // The person sees every path.
    let ctx = egui::Context::default();
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

/// A closed panel's former place, as `ui.docking_hidden` keeps it.
fn hidden_place() -> craft_ui::docking::Location<String> {
    serde_json::from_value(json!({"floating": null, "anchor": null, "placement": {"Tab": {"before": null}}, "size": null, "stack": null})).unwrap()
}

/// `ui.inspect` hides the path of a library loaded from outside the read root wherever the reply holds
/// its id, also when its panel is closed: here only the Color Guide's limit and a closed panel's place
/// name it.
#[test]
fn ui_inspect_hides_a_closed_librarys_path_outside_the_read_root() {
    let (mut app, tx, base) = confined("inspect-closed-lib");
    let theirs = format!("loaded/{}", s(base.join("outside/b.svg")));
    app.ui.color_guide_limit = theirs.clone();
    app.ui.docking_hidden.insert(format!("library:swatches:{theirs}"), hidden_place());
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    let ui = &r["result"]["ui"];
    assert_eq!(ui["color_guide_limit"], "loaded/b.svg", "{r}");
    assert!(ui["docking_hidden"].as_object().unwrap().contains_key("library:swatches:loaded/b.svg"), "{r}");
    let hidden = json!(s(base.join("outside"))).to_string();
    assert!(!r.to_string().contains(hidden.trim_matches('"')), "no path outside the root: {r}");
    assert_eq!(crate::control::inspect(&app, &egui::Context::default())["ui"]["color_guide_limit"], json!(theirs), "the person's");
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
        app.ui.docking_hidden.insert(format!("library:swatches:{id}"), hidden_place());
    }
    app.ui.color_guide_limit = y.clone();
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    let ui = &r["result"]["ui"];
    let ids: Vec<_> = ui["library_panels"].as_array().unwrap().iter().map(|p| p["id"].clone()).collect();
    assert_eq!(ids, [json!("loaded/b.svg"), json!("loaded/b.svg (2)")], "{r}");
    let keys: Vec<_> = ui["docking_hidden"].as_object().unwrap().keys().cloned().collect();
    assert_eq!(keys, ["library:swatches:loaded/b.svg", "library:swatches:loaded/b.svg (2)"], "no entry is dropped: {r}");
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

/// `ui.inspect`'s `scriptReport` names a file the person's script wrote outside the read root, and
/// the included file a failed run stopped in (also in its error line), by its file name; the person
/// sees the paths.
#[test]
fn ui_inspect_script_report_leaves_out_files_outside_the_read_root() {
    let (mut app, tx, base) = confined("inspect-script-report");
    let (kept, hidden) = (s(base.join("in/kept.svg")), s(base.join("outside/hidden.svg")));
    app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let source = format!(
        "vc.run('document.export', {{path: {kept}, format: 'svg'}}); vc.run('document.export', {{path: {hidden}, format: 'svg'}});",
        kept = json!(kept),
        hidden = json!(hidden)
    );
    let finish = |app: &mut VectorcraftApp, params: Value| {
        app.session.execute("script.run", &params).unwrap();
        for _ in 0..10_000 {
            if app.session.script_pump().is_some() {
                break;
            }
        }
    };
    finish(&mut app, json!({"source": source, "name": "t.js"}));
    let ctx = egui::Context::default();
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
    assert_eq!(person["scriptReport"]["file"], json!(s(base.join("outside/bad.jsxinc"))), "{person}");
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    assert_eq!(r["result"]["scriptReport"]["file"], "bad.jsxinc", "{r}");
    assert!(r["result"]["scriptReport"]["console"].to_string().contains("bad.jsxinc:2:"), "{r}");
    assert!(!r.to_string().contains(outside.trim_matches('"')), "no path outside the root: {r}");
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
    app.ui.docking_hidden.insert(format!("library:swatches:{theirs}"), hidden_place());
    let r = request(&mut app, &tx, "ui.inspect", json!({}));
    let ui = &r["result"]["ui"];
    assert_eq!((&ui["library_panels"][0]["id"], &ui["color_guide_limit"]), (&json!(theirs), &json!(theirs)), "{r}");
    assert!(ui["docking_hidden"].as_object().unwrap().contains_key(&format!("library:swatches:{theirs}")), "{r}");
}

/// Hidden logic-only ticks between the injection and its UI pass must not drain another step:
/// one step per settled pass, not per hidden tick.
#[test]
fn hidden_ticks_without_a_ui_pass_do_not_drain_more_injected_steps() {
    let (mut app, tx, base) = confined("gate");
    let r = request(&mut app, &tx, "ui.key", json!({"key": "A", "text": "x"}));
    assert_eq!(r["ok"], true, "{r}");
    let ctx = egui::Context::default();
    let mut raw = egui::RawInput::default();
    app.raw_input_hook(&ctx, &mut raw);
    assert!(!raw.events.is_empty(), "the first step is in the host's input");
    let queued = app.synthetic.len();
    for _ in 0..3 {
        let mut raw = egui::RawInput::default();
        app.raw_input_hook(&ctx, &mut raw);
        assert!(raw.events.is_empty(), "no new step while the previous batch is unapplied");
    }
    assert_eq!(app.synthetic.len(), queued, "the hidden ticks drained nothing");
    let _ = std::fs::remove_dir_all(base);
}

/// A UI pass that changes the document before taking an injected batch drops exactly that batch,
/// including the synthetic modifier latch it set: a later real event keeps the host's modifiers.
#[test]
fn a_stale_injected_batch_is_dropped_with_its_modifier_latch() {
    let (mut app, tx, base) = confined("discard");
    let r = request(&mut app, &tx, "ui.key", json!({"key": "A", "text": "x", "cmd": true}));
    assert_eq!(r["ok"], true, "{r}");
    let ctx = egui::Context::default();
    let mut raw = egui::RawInput::default();
    app.raw_input_hook(&ctx, &mut raw);
    assert!(app.synthetic_modifiers, "the batch carries Cmd");
    assert!(!raw.events.is_empty());
    // The document changed after injection (a control request or a click switching documents):
    // the batch no longer belongs to what the UI pass would act on.
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.raw_input_hook(&ctx, &mut raw);
    assert!(raw.events.is_empty(), "the stale events are removed from the host's input");
    assert!(!app.synthetic_frame && !app.synthetic_injected, "the batch is not applied");
    assert!(!app.synthetic_modifiers, "its modifier latch is cleared too");
    assert!(app.synthetic.is_empty(), "nothing further was queued");
    let _ = std::fs::remove_dir_all(base);
}

/// If the host's raw input no longer contains the synthetic batch exactly (another handler
/// rewrote or interleaved events), the app cannot separate stale synthetic clicks/text from the
/// person's input. It delivers nothing that pass and cancels the rest of the sequence.
#[test]
fn an_unidentifiable_stale_injected_batch_fails_closed() {
    let (mut app, tx, base) = confined("discard-unknown");
    let r = request(&mut app, &tx, "ui.key", json!({"key": "A", "text": "x", "cmd": true}));
    assert_eq!(r["ok"], true, "{r}");
    let ctx = egui::Context::default();
    let mut raw = egui::RawInput::default();
    app.raw_input_hook(&ctx, &mut raw);
    assert!(raw.events.len() > 1, "the batch has a modifier companion and the key");
    // Rewrite one event so the exact subsequence can no longer be found.
    let last = raw.events.len() - 1;
    raw.events[last] = egui::Event::Text("host".into());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.raw_input_hook(&ctx, &mut raw);
    assert!(raw.events.is_empty(), "unidentifiable stale input is never delivered");
    assert!(!app.synthetic_frame && !app.synthetic_injected, "the stale batch is not applied");
    assert!(!app.synthetic_modifiers, "its modifier latch is cleared too");
    assert!(app.synthetic.is_empty(), "the rest of the stale synthetic sequence is cancelled");
    let _ = std::fs::remove_dir_all(base);
}

/// A plug-in's permissions are approved only by a person who asked for the install. A control
/// request (or MCP, which comes through it) can't pass `grant`, can't answer the approval dialog or
/// rewrite what it installs, and a frame of synthetic input can't press its OK. The person can't
/// approve the question automation asked either: they start Install Plug-in… themselves, which asks
/// again, and its OK installs.
#[test]
fn automation_cannot_approve_a_plugin_install() {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = VectorcraftApp::new(Session::new(), Default::default()).with_control(rx);
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let id = "test.ui.approval.automation";
    let manifest = format!(r#"{{"id":"{id}","name":"Approval","version":"1","kind":"filter","permissions":["doc.read"]}}"#);
    let bytes = wat::parse_str(vectorcraft_plugins::wat::returning(&manifest, r#"{"objects":[]}"#)).unwrap();
    let data = vectorcraft_format::base64_encode(&bytes);
    let refused = |r: &Value, says: &str| assert!(r["error"].as_str().is_some_and(|e| e.contains(says)), "{r}");

    let r = request(&mut app, &tx, "engine.execute", json!({"command": "plugin.install", "params": {"dataBase64": data, "grant": ["doc.read"]}}));
    refused(&r, "--allow-plugin-grants");
    assert!(app.ui.dialog.is_none());

    let r = request(&mut app, &tx, "ui.menu.invoke", json!({"command": "ui.installPlugin", "params": {"dataBase64": data}}));
    assert_eq!(r["result"]["pending"], crate::dialogs::plugin::INSTALL_KIND, "{r}");
    assert_eq!(r["result"]["approval"], crate::dialogs::plugin::PERSON_MUST_START_INSTALL, "{r}");
    let r = request(&mut app, &tx, "ui.dialog.confirm", json!({}));
    refused(&r, ASKED);
    assert!(app.ui.dialog.is_some(), "the refused answer leaves it open");
    assert!(vectorcraft_plugins::registry::get(id).is_none());

    // What the dialog installs isn't in its fields.
    let r = request(&mut app, &tx, "ui.dialog.set", json!({"field": "__params", "value": {"dataBase64": "AAAA"}}));
    assert!(r.get("error").is_none(), "{r}");
    // What MCP's `answer_dialog` sends: `ui.dialog.set {fields}`, then `ui.dialog.confirm`.
    let r = request(&mut app, &tx, "ui.dialog.set", json!({"fields": [{"field": "__params", "value": {"dataBase64": "AAAA"}}]}));
    assert!(r.get("error").is_none(), "{r}");
    let r = request(&mut app, &tx, "ui.dialog.confirm", json!({}));
    refused(&r, ASKED);
    assert!(app.ui.dialog.is_some() && vectorcraft_plugins::registry::get(id).is_none());
    // Shown for long enough for a person's OK, it still refuses automation, and the person too.
    super::plugins::show_approval(&mut app);
    let r = request(&mut app, &tx, "ui.dialog.confirm", json!({}));
    refused(&r, ASKED);
    app.synthetic_frame = true;
    assert!(crate::dialogs::confirm(&mut app).unwrap_err().contains(ASKED));
    assert!(vectorcraft_plugins::registry::get(id).is_none());
    app.synthetic_frame = false;
    assert!(crate::dialogs::confirm(&mut app).unwrap_err().contains(ASKED), "a person's OK on automation's question");
    assert!(app.ui.dialog.is_some() && vectorcraft_plugins::registry::get(id).is_none() && vectorcraft_plugins::registry::granted(id).is_none());

    // The person starts Install Plug-in… with the file: a new question, which a person's OK approves
    // once it was shown for the delay, and which automation still can't.
    let r = app.run("ui.installPlugin", json!({"dataBase64": data})).unwrap();
    assert_eq!(r, json!({"pending": crate::dialogs::plugin::INSTALL_KIND}), "the person's question");
    assert_eq!(crate::dialogs::confirm(&mut app).unwrap()["pending"], crate::dialogs::plugin::INSTALL_KIND, "not shown yet");
    super::plugins::show_approval(&mut app);
    let r = request(&mut app, &tx, "ui.dialog.confirm", json!({}));
    refused(&r, "Only the person at the keyboard");
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(vectorcraft_plugins::registry::granted(id).map(|g| g.hash), Some(vectorcraft_plugins::permissions::sha256_hex(&bytes)));
    app.run("plugin.remove", json!({"id": id})).unwrap();
}

/// The refusal of an OK on an install question automation asked.
const ASKED: &str = "Automation asked to install this plug-in. To install it, choose Object › Plug-ins › Install Plug-in… yourself.";

/// The approval module `id` that lists `doc.read`, and its bytes as base64.
fn approval_module(id: &str) -> (Vec<u8>, String) {
    let manifest = format!(r#"{{"id":"{id}","name":"Approval","version":"1","kind":"filter","permissions":["doc.read"]}}"#);
    let bytes = wat::parse_str(vectorcraft_plugins::wat::returning(&manifest, r#"{"objects":[]}"#)).unwrap();
    let data = vectorcraft_format::base64_encode(&bytes);
    (bytes, data)
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
    let r = request(h.state_mut(), &tx, "ui.menu.invoke", json!({"command": "ui.installPlugin", "params": {"dataBase64": data}}));
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
    assert!(crate::dialogs::confirm(h.state_mut()).unwrap_err().contains(ASKED), "a confirm that skips the button");
    assert!(waiting(h.state(), id));
    h.get_by_label("Cancel").click();
    at(&mut h, 11.1);
    assert!(h.state().ui.dialog.is_none() && h.state().pending_plugin_install.is_none(), "Cancel");
    assert!(vectorcraft_plugins::registry::get(id).is_none() && vectorcraft_plugins::registry::granted(id).is_none());
}

/// A question automation asks in place of the one a person asked can't be approved: it keeps
/// automation's origin, though the person's question was shown and ready.
#[test]
fn automation_replacing_a_persons_install_question_does_not_take_its_approval() {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = VectorcraftApp::new(Session::new(), Default::default()).with_control(rx);
    let (mine, theirs) = ("test.ui.approval.mine", "test.ui.approval.theirs");
    app.run("ui.installPlugin", json!({"dataBase64": approval_module(mine).1})).unwrap();
    super::plugins::show_approval(&mut app);
    let r = request(&mut app, &tx, "ui.menu.invoke", json!({"command": "ui.installPlugin", "params": {"dataBase64": approval_module(theirs).1}}));
    assert_eq!(r["result"]["approval"], crate::dialogs::plugin::PERSON_MUST_START_INSTALL, "{r}");
    super::plugins::show_approval(&mut app);
    assert!(crate::dialogs::confirm(&mut app).unwrap_err().contains(ASKED));
    for id in [mine, theirs] {
        assert!(vectorcraft_plugins::registry::get(id).is_none() && vectorcraft_plugins::registry::granted(id).is_none(), "{id}");
    }
    // Asked again by automation with the same file, it is still automation's.
    let r = request(&mut app, &tx, "engine.execute", json!({"command": "ui.installPlugin", "params": {"dataBase64": approval_module(mine).1}}));
    assert_eq!(r["result"]["approval"], crate::dialogs::plugin::PERSON_MUST_START_INSTALL, "{r}");
    super::plugins::show_approval(&mut app);
    assert!(crate::dialogs::confirm(&mut app).unwrap_err().contains(ASKED));
    assert!(vectorcraft_plugins::registry::get(mine).is_none());
}

/// A file a person picks in the file dialog that a control request opened (Install Plug-in… or
/// File › Open with no path) is automation's request: its question can't be approved. The file
/// dialog a person opens asks a question they can approve. On the desktop the dialog answers later
/// ([`crate::picks`]); on the web the file arrives through the picker's inbox, apart from drops and
/// the page's script, which is automation.
#[test]
fn a_file_picked_in_a_dialog_automation_opened_stays_automations() {
    let base = std::env::temp_dir().join(format!("vc-ui-approval-picked-{}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    let (desktop, web) = ("test.ui.approval.picked", "test.ui.approval.picked.web");
    let (bytes, _) = approval_module(desktop);
    let wasm = base.join("picked.wasm");
    std::fs::write(&wasm, &bytes).unwrap();
    let mut services = Services::default();
    let (shown, _) = super::picks::off_the_ui_thread(&mut services);
    services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = VectorcraftApp::new(Session::new(), services).with_control(rx);
    let pick = |app: &mut VectorcraftApp| {
        shown.answer(&[&s(wasm.clone())]);
        crate::picks::poll(app, &egui::Context::default());
    };
    for command in ["ui.installPlugin", "file.open"] {
        let r = request(&mut app, &tx, "ui.menu.invoke", json!({"command": command, "params": {}}));
        assert!(app.file_dialog_open(), "{command}: {r}");
        pick(&mut app);
        assert!(super::plugins::waiting(&app, desktop), "{command}: {:?}", app.ui.status);
        super::plugins::show_approval(&mut app);
        assert!(crate::dialogs::confirm(&mut app).unwrap_err().contains(ASKED), "{command}");
        crate::dialogs::cancel(&mut app);
    }
    let _ = app.run("ui.installPlugin", json!({}));
    assert!(app.file_dialog_open());
    pick(&mut app);
    super::plugins::show_approval(&mut app);
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(vectorcraft_plugins::registry::granted(desktop).map(|g| g.hash), Some(vectorcraft_plugins::permissions::sha256_hex(&bytes)));
    app.run("plugin.remove", json!({"id": desktop})).unwrap();

    // The web: the browser's file picker answers through its own inbox, apart from the files a
    // person drops and the ones the page's script hands over.
    let (dropped, page) = <(crate::services::Inbox, crate::services::Inbox)>::default();
    let picked = crate::services::PickedInbox::default();
    let services = Services {
        open_async: Some(Box::new(|| {})),
        inbox: Some(dropped.clone()),
        picked_inbox: Some(picked.clone()),
        page_inbox: Some(page.clone()),
        ..Default::default()
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = VectorcraftApp::new(Session::new(), services).with_control(rx);
    let (by_drop, by_page) = ("test.ui.approval.picked.dropped", "test.ui.approval.picked.page");
    let file = |id: &str| (format!("{id}.wasm"), approval_module(id).0);
    let hand_over = |app: &mut VectorcraftApp, inbox: &crate::services::Inbox, id: &str| {
        inbox.lock().unwrap().push(file(id));
        app.drain_inbox();
    };
    // The picker answers with a file, or `None` when it is canceled.
    let answer = |app: &mut VectorcraftApp, id: Option<&str>| {
        picked.lock().unwrap().push(id.map(file));
        app.drain_inbox();
    };
    let r = request(&mut app, &tx, "ui.menu.invoke", json!({"command": "ui.installPlugin", "params": {}}));
    assert!(r.get("error").is_none(), "{r}");
    // A file the person drops before the picker answers stays the person's.
    hand_over(&mut app, &dropped, by_drop);
    super::plugins::show_approval(&mut app);
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(vectorcraft_plugins::registry::granted(by_drop).is_some());
    answer(&mut app, Some(web));
    super::plugins::show_approval(&mut app);
    assert!(crate::dialogs::confirm(&mut app).unwrap_err().contains(ASKED), "the web");
    crate::dialogs::cancel(&mut app);
    app.run("ui.installPlugin", json!({})).unwrap();
    answer(&mut app, Some(web));
    super::plugins::show_approval(&mut app);
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(vectorcraft_plugins::registry::granted(web).is_some());
    // Opening the picker again as the person before it answers automation leaves the answer
    // automation's.
    let (again, after) = ("test.ui.approval.picked.again", "test.ui.approval.picked.after");
    let r = request(&mut app, &tx, "ui.menu.invoke", json!({"command": "ui.installPlugin", "params": {}}));
    assert!(r.get("error").is_none(), "{r}");
    app.run("ui.installPlugin", json!({})).unwrap();
    answer(&mut app, Some(again));
    super::plugins::show_approval(&mut app);
    assert!(crate::dialogs::confirm(&mut app).unwrap_err().contains(ASKED), "opened again");
    crate::dialogs::cancel(&mut app);
    // Once the other picker is canceled, the next one the person opens answers for them again.
    answer(&mut app, None);
    app.run("ui.installPlugin", json!({})).unwrap();
    answer(&mut app, Some(after));
    super::plugins::show_approval(&mut app);
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(vectorcraft_plugins::registry::granted(after).is_some());
    // The page's script is automation.
    hand_over(&mut app, &page, by_page);
    super::plugins::show_approval(&mut app);
    assert!(crate::dialogs::confirm(&mut app).unwrap_err().contains(ASKED), "the page's script");
    crate::dialogs::cancel(&mut app);
    assert!(vectorcraft_plugins::registry::granted(by_page).is_none());
    for id in [web, by_drop, after] {
        app.run("plugin.remove", json!({"id": id})).unwrap();
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
    let (bytes, _) = approval_module(id);
    let wasm = base.join("opened.wasm");
    std::fs::write(&wasm, &bytes).unwrap();
    let services = Services { read: Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string()))), ..Default::default() };
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = VectorcraftApp::new(Session::new(), services).with_control(rx);
    let r = request(&mut app, &tx, "app.open", json!({"path": s(wasm.clone())}));
    assert!(r.get("error").is_none(), "{r}");
    assert!(super::plugins::waiting(&app, id));
    super::plugins::show_approval(&mut app);
    assert!(crate::dialogs::confirm(&mut app).unwrap_err().contains(ASKED));
    crate::dialogs::cancel(&mut app);
    crate::io::open_path(&mut app, &s(wasm.clone())).unwrap();
    super::plugins::show_approval(&mut app);
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(vectorcraft_plugins::registry::granted(id).is_some());
    app.run("plugin.remove", json!({"id": id})).unwrap();
    let _ = std::fs::remove_dir_all(base);
}

/// A module that takes the place of another with its id, and asks for nothing else: automation's
/// `replace: true` doesn't skip the question and its question can't be approved. The question a
/// person asked keeps what it installs out of reach of `ui.dialog.set`, refuses a control confirm,
/// and the person's own OK replaces.
#[test]
fn automation_cannot_approve_replacing_a_plugin() {
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

    for params in [json!({"dataBase64": data}), json!({"dataBase64": data, "replace": true})] {
        let r = request(&mut app, &tx, "ui.menu.invoke", json!({"command": "ui.installPlugin", "params": params}));
        assert_eq!(r["result"]["pending"], crate::dialogs::plugin::INSTALL_KIND, "{params}: {r}");
        assert_eq!(r["result"]["approval"], crate::dialogs::plugin::PERSON_MUST_START_INSTALL, "{params}: {r}");
        let pending = app.pending_plugin_install.as_ref().unwrap();
        assert_eq!(pending.message, "Install “Plain” in place of the plug-in with the same ID?");
        super::plugins::show_approval(&mut app);
        assert!(crate::dialogs::confirm(&mut app).unwrap_err().contains(ASKED), "{params}");
        let r = request(&mut app, &tx, "ui.dialog.confirm", json!({}));
        assert!(r["error"].as_str().is_some_and(|e| e.contains(ASKED)), "{r}");
        crate::dialogs::cancel(&mut app);
        assert_eq!(installed(), Some(first_hash.clone()), "{params}");
    }

    // The person's question: a control request can't point its OK at other bytes, nor press it.
    let r = app.run("ui.installPlugin", json!({"dataBase64": data})).unwrap();
    assert_eq!(r, json!({"pending": crate::dialogs::plugin::INSTALL_KIND}));
    let evil = vectorcraft_format::base64_encode(&plain(r#"{"objects":[{"name":"x"}]}"#));
    let r = request(&mut app, &tx, "ui.dialog.set", json!({"fields": [{"field": "__params", "value": {"dataBase64": evil, "replace": true}}]}));
    assert!(r.get("error").is_none(), "{r}");
    super::plugins::show_approval(&mut app);
    let r = request(&mut app, &tx, "ui.dialog.confirm", json!({}));
    assert!(r["error"].as_str().is_some_and(|e| e.contains("Only the person at the keyboard")), "{r}");
    assert_eq!(installed(), Some(first_hash.clone()));
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(installed(), Some(vectorcraft_plugins::permissions::sha256_hex(&other)), "the bytes the question showed");
    app.run("plugin.remove", json!({"id": id})).unwrap();
    // A module that asks for nothing still installs at once when automation asks.
    let free = "test.ui.approval.free";
    let module = wat::parse_str(vectorcraft_plugins::wat::returning(&format!(r#"{{"id":"{free}","name":"Free","kind":"filter"}}"#), "{}")).unwrap();
    let params = json!({"dataBase64": vectorcraft_format::base64_encode(&module)});
    let r = request(&mut app, &tx, "ui.menu.invoke", json!({"command": "ui.installPlugin", "params": params}));
    assert!(r["result"]["pending"].is_null() && app.ui.dialog.is_none(), "{r}");
    assert!(vectorcraft_plugins::registry::get(free).is_some());
    app.run("plugin.remove", json!({"id": free})).unwrap();
}

type DialogHarness = egui_kittest::Harness<'static, VectorcraftApp>;

/// Runs one UI pass of `h` for each batch of synthetic input the control path queued, each taken
/// through the host's raw-input hook, until none is left and the last batch is acknowledged.
fn take_injected(h: &mut DialogHarness) {
    let ctx = h.ctx.clone();
    loop {
        let mut raw = egui::RawInput::default();
        h.state_mut().raw_input_hook(&ctx, &mut raw);
        if raw.events.is_empty() && h.state().synthetic.is_empty() {
            break;
        }
        h.input_mut().events.extend(raw.events);
        h.step();
    }
}

/// The open install approval, with nothing installed or granted for `id`.
fn still_waiting(h: &DialogHarness, id: &str) -> bool {
    h.state().ui.dialog.as_ref().is_some_and(|d| d.kind == crate::dialogs::plugin::INSTALL_KIND)
        && vectorcraft_plugins::registry::get(id).is_none()
        && vectorcraft_plugins::registry::granted(id).is_none()
}

/// Keys and pointer input sent through the control path approve nothing at the install approval a
/// person asked for: Enter and Space press nothing with no button focused, and with OK focused they
/// and a `ui.click` on it press it and are refused. `ui.pointer` drives the canvas. A person's click
/// on the same button approves.
#[test]
fn injected_keys_and_pointer_input_cannot_approve_a_plugin_install() {
    use egui_kittest::kittest::{NodeT, Queryable};
    let refusal = "Only the person at the keyboard";
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = VectorcraftApp::new(Session::new(), Default::default()).with_control(rx);
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let id = "test.ui.approval.injected";
    let manifest = format!(r#"{{"id":"{id}","name":"Approval","version":"1","kind":"filter","permissions":["doc.read"]}}"#);
    let bytes = wat::parse_str(vectorcraft_plugins::wat::returning(&manifest, r#"{"objects":[]}"#)).unwrap();
    let data = vectorcraft_format::base64_encode(&bytes);
    let r = app.run("ui.installPlugin", json!({"dataBase64": data})).unwrap();
    assert_eq!(r, json!({"pending": crate::dialogs::plugin::INSTALL_KIND}), "the person's question");
    let mut fonts = false;
    let mut h: DialogHarness = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 800.0)).build_ui_state(
        move |ui, app: &mut VectorcraftApp| {
            if !std::mem::replace(&mut fonts, true) {
                crate::theme::install_fonts(ui.ctx());
                return;
            }
            crate::dialogs::show(app, ui.ctx());
        },
        app,
    );
    h.run_steps(4);

    for key in ["Enter", "Space"] {
        assert!(!h.get_by_label("OK").accesskit_node().is_focused(), "no button has the focus yet");
        h.state_mut().ui.status.clear();
        let r = request(h.state_mut(), &tx, "ui.key", json!({"key": key}));
        assert_eq!(r["ok"], true, "{r}");
        take_injected(&mut h);
        assert!(still_waiting(&h, id), "{key}");
        assert_eq!(h.state().ui.status, "", "{key} presses nothing");
    }

    for key in ["Enter", "Space"] {
        h.get_by_label("OK").focus();
        h.run_steps(2);
        assert!(h.get_by_label("OK").accesskit_node().is_focused());
        h.state_mut().ui.status.clear();
        let r = request(h.state_mut(), &tx, "ui.key", json!({"key": key}));
        assert_eq!(r["ok"], true, "{r}");
        take_injected(&mut h);
        assert!(still_waiting(&h, id), "{key} on a focused OK");
        assert!(h.state().ui.status.contains(refusal), "{key} pressed OK and was refused: {:?}", h.state().ui.status);
    }

    let at = h.get_by_label("OK").rect().center();
    h.state_mut().ui.status.clear();
    let r = request(h.state_mut(), &tx, "ui.click", json!({"x": at.x, "y": at.y}));
    assert_eq!(r["ok"], true, "{r}");
    take_injected(&mut h);
    assert!(still_waiting(&h, id), "ui.click");
    assert!(h.state().ui.status.contains(refusal), "ui.click pressed OK and was refused: {:?}", h.state().ui.status);

    let events = json!([{"kind": "down", "x": at.x, "y": at.y}, {"kind": "up", "x": at.x, "y": at.y}]);
    let r = request(h.state_mut(), &tx, "ui.pointer", json!({"events": events}));
    assert_eq!(r["ok"], true, "{r}");
    take_injected(&mut h);
    assert!(still_waiting(&h, id), "ui.pointer");

    h.get_by_label("OK").click();
    h.run_steps(2);
    assert!(h.state().ui.dialog.is_none(), "{:?}", h.state().ui.status);
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
