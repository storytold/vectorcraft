//! File › Scripts › Check Script… (`ui.checkScript`): the report on a script written for the reference
//! app, read without running it, and its Copy Report; File › Open of `.jsx`, `.jsxinc` and `.jsxbin`
//! files.

use serde_json::{Value, json};
use vectorcraft_engine::Session;

use crate::{VectorcraftApp, dialogs, io, menus, theme};

/// An app reading files from disk.
fn app() -> VectorcraftApp {
    let services = crate::Services { read: Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string()))), ..Default::default() };
    VectorcraftApp::new(Session::new(), services)
}

/// One headless frame of the dialog layer.
fn frame(app: &mut VectorcraftApp) {
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    theme::apply(&ctx, Default::default());
    let mut out = ctx.run_ui(Default::default(), |ui| dialogs::show(app, ui.ctx()));
    out.textures_delta.clear();
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

fn report(app: &VectorcraftApp) -> &Value {
    let d = app.ui.dialog.as_ref().expect("the report is shown");
    assert_eq!(d.kind, dialogs::script_check::KIND);
    &d.fields["report"]
}

#[test]
fn file_scripts_holds_check_script_above_other_script() {
    let tree = menus::menu_tree();
    let (_, file) = tree.iter().find(|(t, _)| *t == "File").unwrap();
    let scripts = file.iter().find_map(|i| if let menus::Item::Sub("Scripts", items) = i { Some(items) } else { None }).unwrap();
    assert!(
        matches!(scripts.as_slice(), [menus::Item::Cmd("Check Script…", "ui.checkScript", _), menus::Item::Todo("Other Script…", _)]),
        "{scripts:?}"
    );
}

#[test]
fn check_script_shows_the_report_and_copy_report_copies_it() {
    let mut app = app();
    let p = json!({"dataBase64": vectorcraft_format::base64_encode(SCRIPT.as_bytes()), "name": "Tidy.jsx"});
    let r = menus::run_ui_command(&mut app, "ui.checkScript", &p).unwrap().unwrap();
    assert_eq!(report(&app), &r);
    assert_eq!((r["name"].as_str(), r["kind"].as_str()), (Some("Tidy.jsx"), Some("script")));
    assert_eq!(r["needs"], json!(["language", "dom", "files", "menuCommands"]));
    // The window draws every section.
    frame(&mut app);
    assert_eq!(report(&app), &r);
    // Copy Report (the button beside Close) copies the report as JSON and leaves the window open.
    app.ui.dialog.as_mut().unwrap().fields.insert("discard".into(), json!(true));
    dialogs::confirm(&mut app).unwrap();
    let copied: Value = serde_json::from_str(app.clipboard_out.as_deref().unwrap()).unwrap();
    assert_eq!(copied, r);
    assert!(app.ui.dialog.as_ref().unwrap().fields.get("discard").is_none());
    assert_eq!(app.ui.status, "Copied the script report");
    // Close closes it.
    dialogs::cancel(&mut app);
    assert!(app.ui.dialog.is_none());
    // The script's text does as well as its bytes.
    let by_text = menus::run_ui_command(&mut app, "ui.checkScript", &json!({"data": SCRIPT, "name": "Tidy.jsx"})).unwrap().unwrap();
    assert_eq!(by_text, r);
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
        assert_eq!(report(&app)["kind"], kind, "{name}");
        frame(&mut app);
        assert!(app.ui.dialog.is_some());
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
    let r = report(&opened);
    assert_eq!(r["includes"], json!([{"value": "lib/net.jsxinc", "line": 1, "status": "checked", "path": "lib/net.jsxinc"}]));
    assert_eq!(r["needs"], json!(["language", "hostOnly"]));
    assert!(opened.session.documents().is_empty(), "no document opened");
    // `script.check` reads the script up to its limit, and the app's reader, which reads a whole file,
    // isn't asked.
    assert_eq!(reads.get(), 0);
    assert_eq!(opened.ui.recent_files.first(), Some(&path), "File › Open lists it in Open Recent Files");
    // Check Script… with a path does the same; without one it asks for a script.
    let mut app = app();
    menus::run_ui_command(&mut app, "ui.checkScript", &json!({ "path": path })).unwrap().unwrap();
    assert_eq!(report(&app)["includes"][0]["status"], "checked");
    let asked = std::rc::Rc::new(std::cell::RefCell::new(vec![]));
    let seen = asked.clone();
    let picked = path.clone();
    app.services.pick_open = Some(Box::new(move |pick: &crate::FilePick| {
        seen.borrow_mut().push(pick.filters.clone());
        Some(picked.clone())
    }));
    app.ui.dialog = None;
    menus::run_ui_command(&mut app, "ui.checkScript", &json!({})).unwrap().unwrap();
    assert_eq!(asked.borrow().as_slice(), [crate::FilePick::filters([("Scripts", &["jsx", "jsxinc", "jsxbin"][..])])]);
    assert_eq!(report(&app)["name"], "main.jsx");
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
    assert_eq!(report(&opened)["name"], "later.jsx");
    assert_eq!(reads.get(), 0);
    assert_eq!(opened.ui.recent_files.first(), Some(&path));
    let _ = std::fs::remove_dir_all(dir);
}

/// Check Script… asks for its script off the UI thread, and acts on no document: the answer shows the
/// report even when the document that was active closed meanwhile.
#[test]
fn check_script_asks_off_the_ui_thread_and_needs_no_document() {
    let dir = std::env::temp_dir().join(format!("vc-ui-script-pick-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("picked.jsx");
    std::fs::write(&path, "alert('picked');\n").unwrap();
    let mut services = crate::Services { dialogs_are_sheets: false, ..Default::default() };
    let (shown, _) = crate::tests_picks::off_the_ui_thread(&mut services);
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    assert!(app.run("ui.checkScript", json!({})).is_err(), "nothing yet: the dialog is open");
    assert!(
        matches!(shown.requests().as_slice(), [crate::picks::PickRequest::Open(pick)] if pick.filters == crate::FilePick::filters([("Scripts", &["jsx", "jsxinc", "jsxbin"][..])]))
    );
    app.run("file.close", json!({})).unwrap();
    shown.answer(&[&path.to_string_lossy()]);
    crate::picks::poll(&mut app, &egui::Context::default());
    assert_eq!(report(&app)["name"], "picked.jsx", "{}", app.ui.status);
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
