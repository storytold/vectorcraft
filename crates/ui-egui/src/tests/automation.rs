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
