//! File › Scripts › Check Script… (`ui.checkScript`): the report on a script written for the reference
//! app, read without running it, in the framework's hosted report; File › Open of `.jsx`, `.jsxinc`
//! and `.jsxbin` files.

use serde_json::{Value, json};
use vectorcraft_engine::Session;

use super::picks::{poll, scripted};
use crate::{VectorcraftApp, dialogs, io, menus};

/// An app reading files from disk, whose window has a dialog host.
fn app() -> VectorcraftApp {
    let services = crate::Services { read: Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string()))), ..Default::default() };
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.adopt_context(&crate::testing::ui_context());
    app
}

/// The context of the app's window.
fn window(app: &VectorcraftApp) -> egui::Context {
    app.ui_ctx.clone().expect("the app's window")
}

/// One headless frame of the app's dialog host.
fn frame(app: &VectorcraftApp) {
    let host = app.dialogs().expect("the app has a dialog host");
    let mut out = window(app).run_ui(Default::default(), |ui| host.with(|h| h.run(ui)));
    out.textures_delta.clear();
}

/// Closes every open dialog as its Close button does.
fn close_all(app: &VectorcraftApp) {
    let ctx = window(app);
    app.dialogs().unwrap().with(|h| h.open_dialogs().iter().for_each(|d| assert!(h.cancel_by_id(&ctx, d.id))));
}

/// A script with a little of everything the report shows.
const SCRIPT: &str = concat!(
    "#target acme\n",
    "#include 'lib/missing.jsxinc'\n",
    "var doc = app.activeDocument;\n",
    "app.executeMenuCommand('acmecopyall');\n",
    "app.executeMenuCommand(name);\n",
    "var f = new File(p); f.open('w');\n",
    "if (x.typename == 'AcmeShape') {}\n",
    "var s = 'open;\n",
);

/// The fields of the open report: `lines` and `groups`.
fn fields(app: &VectorcraftApp) -> Value {
    let host = app.dialogs().expect("the app has a dialog host");
    host.with(|h| h.open_dialogs().into_iter().find_map(|d| d.fields)).expect("the report is shown")
}

/// The report's groups. PLAN: assumed `json_fields()["groups"]` is `[{heading, notes}]`.
fn groups(app: &VectorcraftApp) -> Vec<Value> {
    fields(app)["groups"].as_array().cloned().unwrap_or_default()
}

/// The heading of the first group: the script's name.
fn name(app: &VectorcraftApp) -> String {
    groups(app).first().and_then(|g| g["heading"].as_str()).unwrap_or_default().to_string()
}

#[test]
fn check_script_shows_the_report_and_copy_report_copies_it() {
    let mut app = app();
    let p = json!({"dataBase64": vectorcraft_format::base64_encode(SCRIPT.as_bytes()), "name": "Tidy.jsx"});
    let r = menus::run_ui_command(&mut app, "ui.checkScript", &p).unwrap().unwrap();
    assert_eq!((r["name"].as_str(), r["kind"].as_str()), (Some("Tidy.jsx"), Some("script")));
    assert_eq!(r["needs"], json!(["language", "dom", "files", "menuCommands"]));
    let shown = groups(&app);
    assert_eq!(name(&app), "Tidy.jsx");
    let text = fields(&app)["groups"].to_string();
    for heading in ["Outside the document", "Menu commands", "Document model", "Directives", "Includes", "Problems"] {
        assert!(text.contains(heading), "{heading} in {text}");
    }
    // The window draws every group.
    frame(&app);
    assert_eq!(groups(&app).len(), shown.len());
    // A script that does nothing has the name group only.
    close_all(&app);
    menus::run_ui_command(&mut app, "ui.checkScript", &json!({"data": "var a = 1;", "name": "Clean.jsx"})).unwrap().unwrap();
    assert_eq!(groups(&app).len(), 1);
    close_all(&app);
    assert!(app.dialogs().unwrap().with(|h| h.open_dialogs().is_empty()), "Close closes it");
    // The script's text does as well as its bytes.
    let by_text = menus::run_ui_command(&mut app, "ui.checkScript", &json!({"data": SCRIPT, "name": "Tidy.jsx"})).unwrap().unwrap();
    assert_eq!(by_text, r);
    // Nothing was copied until Copy is pressed, and the pump puts what it copied on the clipboard.
    let ctx = window(&app);
    dialogs::script_check::pump(&mut app, &ctx);
    assert!(app.clipboard_out.is_none());
}

#[test]
fn each_kind_of_file_draws_its_verdict() {
    for (name, bytes, kind) in [
        ("a.jsxbin", b"anything".as_slice(), "compiled"),
        ("a.jsx", b"import a from 'a';\n".as_slice(), "module"),
        ("a.jsx", b"\0\0\0".as_slice(), "notText"),
        ("a.jsx", b"".as_slice(), "script"),
    ] {
        let mut app = app();
        io::open_bytes(&mut app, name, bytes, None).unwrap();
        let text = fields(&app)["groups"].to_string();
        let plain = dialogs::script_check::need_label("language").is_some() && text.contains("Needs nothing beyond plain JavaScript");
        assert_eq!(plain, kind == "script", "{name} {kind}: {text}");
        if kind != "script" {
            assert_eq!(groups(&app).len(), 1, "{name}");
        }
        frame(&app);
    }
}

#[test]
fn file_open_of_a_script_checks_it_with_what_it_includes() {
    let dir = std::env::temp_dir().join(format!("vc-ui-script-check-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(dir.join("main.jsx"), "#include 'lib/net.jsxinc'\n").unwrap();
    std::fs::write(dir.join("lib/net.jsxinc"), "var s = new Socket();\n").unwrap();
    let path = dir.join("main.jsx").to_string_lossy().into_owned();
    let mut opened = app();
    let reads = std::rc::Rc::new(std::cell::Cell::new(0));
    let counted = reads.clone();
    opened.services.read = Some(Box::new(move |p: &str| {
        counted.set(counted.get() + 1);
        std::fs::read(p).map_err(|e| e.to_string())
    }));
    io::open_path(&mut opened, &path).unwrap();
    let text = fields(&opened)["groups"].to_string();
    assert!(text.contains("lib/net.jsxinc"), "the include is listed: {text}");
    for need in ["language", "hostOnly"] {
        assert!(text.contains(dialogs::script_check::need_label(need).unwrap()), "{need} in {text}");
    }
    assert!(opened.session.documents().is_empty(), "no document opened");
    // `script.check` reads the script up to its limit, and the app's reader, which reads a whole file,
    // isn't asked.
    assert_eq!(reads.get(), 0);
    assert_eq!(opened.ui.recent_files.first(), Some(&path), "File › Open lists it in Open Recent Files");
    // Check Script… with a path does the same; without one it asks for a script.
    let mut app = app();
    menus::run_ui_command(&mut app, "ui.checkScript", &json!({ "path": path })).unwrap().unwrap();
    assert!(fields(&app)["groups"].to_string().contains("lib/net.jsxinc"));
    let picker = scripted(&mut app);
    let pending = menus::run_ui_command(&mut app, "ui.checkScript", &json!({})).unwrap().unwrap();
    assert_eq!(pending["pending"], "fileDialog");
    let requests = picker.requests();
    assert_eq!(requests.len(), 1);
    let filters: Vec<(&str, Vec<&str>)> =
        requests[0].filters.iter().map(|f| (f.name.as_str(), f.extensions.iter().map(String::as_str).collect())).collect();
    assert_eq!(filters, [("Scripts", vec!["jsx", "jsxinc", "jsxbin"])]);
    picker.answer(&[&path]);
    let ctx = window(&app);
    poll(&mut app, &ctx);
    assert_eq!(name(&app), "main.jsx");
    let _ = std::fs::remove_dir_all(dir);
}

/// Where a document opens off the UI thread (File › Open, Open Recent and drops on the desktop), a
/// script shows its report at once, and the app's reader isn't asked for it.
#[test]
fn a_script_opens_at_once_where_documents_open_off_the_ui_thread() {
    let dir = std::env::temp_dir().join(format!("vc-ui-script-background-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("later.jsx");
    std::fs::write(&path, "alert('later');\n").unwrap();
    let path = path.to_string_lossy().into_owned();
    let mut opened = app();
    let reads = std::rc::Rc::new(std::cell::Cell::new(0));
    let counted = reads.clone();
    opened.services.read = Some(Box::new(move |p: &str| {
        counted.set(counted.get() + 1);
        std::fs::read(p).map_err(|e| e.to_string())
    }));
    assert_eq!(io::open_file(&mut opened, &path, true).unwrap(), Value::Null);
    assert!(opened.opening.pending.is_empty(), "nothing is read off the UI thread");
    assert_eq!(name(&opened), "later.jsx");
    assert_eq!(reads.get(), 0);
    assert_eq!(opened.ui.recent_files.first(), Some(&path));
    let _ = std::fs::remove_dir_all(dir);
}

/// Check Script… asks for its script through the picker, and acts on no document: the answer shows the
/// report even when the document that was active closed meanwhile.
#[test]
fn check_script_asks_off_the_ui_thread_and_needs_no_document() {
    let dir = std::env::temp_dir().join(format!("vc-ui-script-pick-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("picked.jsx");
    std::fs::write(&path, "alert('picked');\n").unwrap();
    let mut app = VectorcraftApp::new(Session::new(), crate::Services::default());
    let picker = scripted(&mut app);
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    let pending = menus::run_ui_command(&mut app, "ui.checkScript", &json!({})).unwrap().unwrap();
    assert_eq!(pending["pending"], "fileDialog");
    let requests = picker.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].filters.first().map(|f| f.name.as_str()), Some("Scripts"));
    app.run("file.close", json!({})).unwrap();
    picker.answer(&[&path.to_string_lossy()]);
    let ctx = window(&app);
    poll(&mut app, &ctx);
    assert_eq!(name(&app), "picked.jsx", "{}", app.ui.status);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn every_id_a_report_holds_has_a_label() {
    use dialogs::script_check::{need_label, outside_label, problem_label, status_label};
    use vectorcraft_engine::cmd::scripts::{IncludeStatus, Need, OUTSIDE, PROBLEMS};
    for need in Need::ALL {
        assert!(need_label(need.id()).is_some(), "{need:?}");
    }
    for status in IncludeStatus::ALL {
        assert!(status_label(status.id()).is_some(), "{status:?}");
    }
    for id in OUTSIDE {
        assert!(outside_label(id).is_some(), "{id}");
    }
    for code in PROBLEMS {
        assert!(problem_label(code).is_some(), "{code}");
    }
}
