//! The Home icon shows the Home screen over open documents; Cmd+N opens the New Document dialog.

use craft::ui::dialog::StandardPrompt;
use serde_json::json;

use crate::tests::panel_harness::Panel;
use crate::{VectorcraftApp, canvas};

fn app_with_doc() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    app
}

/// Draw one canvas frame; whether it drew the document (else the Home screen).
fn shows_document(app: &mut VectorcraftApp) -> bool {
    // Only a view this frame lays out counts.
    if let Some(view) = app.view_mut() {
        view.rect = egui::Rect::NOTHING;
    }
    let ctx = crate::testing::ui_context();
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))), ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| canvas::show(app, ui));
    out.textures_delta.clear();
    app.canvas_view().is_some()
}

/// Gives the app a context and its window's dialog host, as its first frame does; whether a dialog is open on it.
fn dialogs_open(app: &mut VectorcraftApp, ctx: &egui::Context) -> bool {
    app.ui_ctx = Some(ctx.clone());
    app.dialogs().is_some_and(|host| host.with(|h| !h.open_dialogs().is_empty()))
}

#[test]
fn home_shows_over_open_documents_until_a_document_is_chosen() {
    let mut app = app_with_doc();
    assert!(shows_document(&mut app));
    app.run("app.home", json!({})).unwrap();
    assert!(!shows_document(&mut app), "Home replaces the canvas");
    assert_eq!(app.session.documents().len(), 1, "the document stays open");
    let ctx = crate::testing::ui_context();
    assert!(!dialogs_open(&mut app, &ctx), "Home is not the New Document dialog");
    // Choosing the document's tab returns to it.
    app.ui.home = None;
    assert!(shows_document(&mut app));
    // A new document (from Home's presets, New… or Open) replaces Home too.
    app.run("app.home", json!({})).unwrap();
    app.run("file.new", json!({})).unwrap();
    assert!(shows_document(&mut app));
    assert!(app.ui.home.is_none());
}

/// General › Show The Home Screen When No Documents Are Open (#394): off, an app with no document
/// shows an empty window instead of the Home screen, and the Home button still opens it.
#[test]
fn home_screen_without_documents_follows_its_preference() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    assert!(app.session.active().is_none());
    assert!(crate::menus::home_showing(&app), "Home with no document open (the default)");
    assert!(!shows_document(&mut app));
    app.session.execute("prefs.set", &json!({"key": "showHomeScreen", "value": false})).unwrap();
    assert!(!crate::menus::home_showing(&app), "off: an empty window, the Home button isn't lit");
    assert!(!shows_document(&mut app));
    app.run("app.home", json!({})).unwrap();
    assert!(crate::menus::home_showing(&app), "the Home button still shows the Home screen");
    app.run("file.new", json!({})).unwrap();
    assert!(shows_document(&mut app));
    assert!(app.ui.home.is_none());
    app.run("file.close", json!({})).unwrap();
    assert!(app.session.active().is_none());
    assert!(!crate::menus::home_showing(&app), "closing the last document leaves the window empty");
    app.session.execute("prefs.set", &json!({"key": "showHomeScreen", "value": true})).unwrap();
    assert!(crate::menus::home_showing(&app), "on again: Home");
}

/// Cmd+Alt+N, pressed through the shortcut handler, makes a document with the settings New
/// Document opens on (the Print category's first preset while Recent is empty, then the first
/// Recent entry), named as the dialog names it, and does not open the dialog.
#[test]
fn cmd_alt_n_makes_a_document_without_the_new_document_dialog() {
    use vectorcraft_doc::ColorMode;
    use vectorcraft_engine::cmd::newdoc;
    let ctx = crate::testing::ui_context();
    let press = |app: &mut VectorcraftApp| {
        // The first frame of a context builds the keymap the key stage reads.
        crate::tests_synthetic::key_frame(app, &ctx, vec![]);
        let modifiers = egui::Modifiers::COMMAND | egui::Modifiers::ALT;
        let ev = |pressed| egui::Event::Key { key: egui::Key::N, physical_key: None, pressed, repeat: false, modifiers };
        // A keyboard's tap: a second press with no release between is a repeat.
        crate::tests_synthetic::key_frame(app, &ctx, vec![ev(true), ev(false)]);
    };
    let made = |app: &VectorcraftApp| {
        let d = &app.session.active().expect("a document").doc;
        let r = d.artboards[0].rect;
        (d.title.clone(), r.width(), r.height(), d.color_mode, d.raster_effects_ppi)
    };
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    assert!(app.session.active().is_none());
    let first = newdoc::category(&app.session, "Print").and_then(|presets| presets.into_iter().next()).expect("a Print preset");
    let name = app.session.peek_untitled();
    press(&mut app);
    assert_eq!(app.session.documents().len(), 1);
    assert!(!dialogs_open(&mut app, &ctx), "the New Document dialog does not open");
    assert_eq!(made(&app), (name, first.width, first.height, ColorMode::Cmyk, 300.0), "the Print category's first preset, as New Document opens on");
    // Then the first Recent entry, the 400 × 300 pt document's settings.
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let name = app.session.peek_untitled();
    press(&mut app);
    assert_eq!(app.session.documents().len(), 3);
    assert!(!dialogs_open(&mut app, &ctx));
    assert_eq!(made(&app), (name, 400.0, 300.0, ColorMode::Rgb, 72.0));
    // Run as a command while a dialog is open, it leaves that dialog open.
    app.run("file.newDialog", json!({})).unwrap();
    assert!(dialogs_open(&mut app, &ctx), "file.newDialog puts its dialog on the host");
    app.run("file.newWithoutDialog", json!({})).unwrap();
    assert_eq!(app.session.documents().len(), 4);
    assert!(dialogs_open(&mut app, &ctx));
}

/// The Home screen of `app` in a window tall enough for its Learn section.
fn home_screen(app: VectorcraftApp) -> Panel {
    let mut home = Panel::new(app, canvas::show);
    home.room(egui::vec2(1000.0, 2400.0));
    home.frame();
    home
}

/// The Home screen lists the first recent files (#663); opening one that can't be opened says so.
#[test]
fn home_lists_recent_files_and_a_click_opens_one() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    let dir = std::env::temp_dir().join("vectorcraft-home-recent");
    let missing = dir.join("Gone.svg").to_string_lossy().into_owned();
    app.ui.recent_files = (0..9).map(|i| dir.join(format!("Drawing {i}.svg")).to_string_lossy().into_owned()).collect();
    app.ui.recent_files.insert(0, missing);
    // Each row reads its name and then its folder.
    let row = |name: &str| format!("{name}, {}", dir.display());
    let mut home = home_screen(app);
    assert!(home.shows(&row("Gone.svg")) && home.shows(&row("Drawing 4.svg")), "the first recent files are listed");
    assert!(!home.shows(&row("Drawing 5.svg")), "the first six only");
    home.double_click(&row("Gone.svg"));
    let app = home.app();
    assert!(app.session.documents().is_empty());
    assert!(!app.ui.status.is_empty(), "a file that can't be opened says so");
}

/// The Home screen has a Learn section: every tutorial of the Learn panel under its skill. A
/// tutorial shows the Learn panel at its steps and no document is opened; the Quick skill guide
/// shows its list of skills.
#[test]
fn home_has_a_learn_section_that_opens_the_learn_panel() {
    use crate::panels::learn::{self, Tab, View, catalog};
    let mut home = home_screen(VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default()));
    assert!(home.shows("Learn"));
    for skill in catalog::SKILLS {
        assert!(home.shows(skill.title), "{} is shown", skill.title);
        for tutorial in skill.tutorials {
            assert!(home.shows(tutorial.title), "{} is shown", tutorial.title);
        }
    }
    assert!(!learn::visible(&home.app()), "the panel is not shown before a choice");
    home.click("Draw paths with the Pen");
    let app = home.app();
    assert!(learn::visible(&app), "a tutorial shows the Learn panel");
    assert_eq!(app.ui.learn, View { tab: Tab::Skills, skill: Some("draw"), tutorial: Some("pen-paths") });
    assert!(app.session.documents().is_empty(), "no document is opened");
    assert!(crate::menus::home_showing(&app), "Home stays");
    drop(app);
    home.click("Quick skill guide");
    assert_eq!(home.app().ui.learn, View { tab: Tab::Skills, skill: None, tutorial: None }, "the guide shows the list of skills");
}

/// Home shown over open documents has the Learn section too; a tutorial chosen there leaves the
/// documents and the active one as they are.
#[test]
fn home_learn_section_over_open_documents_leaves_them_as_they_are() {
    use crate::panels::learn::{self, Tab, View};
    let mut app = app_with_doc();
    app.run("file.new", json!({})).unwrap();
    app.run("document.activate", json!({"index": 0})).unwrap();
    app.run("app.home", json!({})).unwrap();
    let mut home = home_screen(app);
    home.click("Add a gradient");
    let app = home.app();
    assert!(learn::visible(&app));
    assert_eq!(app.ui.learn, View { tab: Tab::Skills, skill: Some("color"), tutorial: Some("gradient") });
    assert_eq!((app.session.documents().len(), app.session.active_index()), (2, Some(0)), "the documents stay as they are");
    assert!(crate::menus::home_showing(&app), "Home stays over the documents");
}

/// A file that is already open is not opened again, however it is asked for: Home › Recent Files,
/// File › Open Recent, File › Open, the Finder and a drop all focus its tab, unsaved edits and all
/// (#3455).
#[test]
fn reopening_an_open_file_focuses_its_tab() {
    let services = crate::Services {
        read: Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string()))),
        write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), services);
    let d = std::env::temp_dir().join(format!("vc-home-reopen-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let path = d.join("doc.vectorcraft").to_string_lossy().to_string();
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 1, "y": 2, "width": 10, "height": 10})).unwrap();
    app.run("file.saveAs", json!({"path": path})).unwrap();
    // An edit not saved yet.
    app.run("shape.rectangle", json!({"x": 20, "y": 2, "width": 10, "height": 10})).unwrap();
    let back = |app: &mut VectorcraftApp| {
        app.run("file.new", json!({})).unwrap();
        assert_eq!(app.session.documents().len(), 2);
    };
    let focused = |app: &VectorcraftApp, how: &str| {
        assert_eq!(app.session.documents().len(), 2, "{how}: no second copy");
        assert_eq!(app.session.active_index(), Some(0), "{how}");
        assert_eq!(app.session.doc().unwrap().doc.layers[0].children().unwrap().len(), 2, "{how}: the unsaved edit is kept");
        app.session.doc().unwrap().uid
    };
    back(&mut app);
    // Home › Recent Files, File › Open Recent, File › Open… and the Finder.
    let r = crate::io::open_reporting(&mut app, &path).unwrap();
    assert_eq!(r["activated"], true);
    let uid = focused(&app, "open_reporting");
    app.run("document.activate", json!({"index": 1})).unwrap();
    app.run("file.close", json!({})).unwrap();
    back(&mut app);
    // File › Open with a path (an open dialog's answer, `app.open`), and a path spelled differently.
    let other = d.join(".").join("doc.vectorcraft").to_string_lossy().to_string();
    assert_eq!(app.run("file.open", json!({"path": other})).unwrap()["activated"], true);
    assert_eq!(focused(&app, "file.open"), uid);
    app.run("document.activate", json!({"index": 1})).unwrap();
    app.run("file.close", json!({})).unwrap();
    back(&mut app);
    // A drop.
    crate::place::drop_files(&mut app, vec![(crate::place::DropTarget::Open, ("doc.vectorcraft".into(), Some(path.clone()), vec![]))]);
    assert_eq!(focused(&app, "drop"), uid);
    let _ = std::fs::remove_dir_all(d);
}

/// An app with `doc.vectorcraft` saved in a fresh folder `name`, open in the first tab, and an
/// untitled document active in the second → (app, folder, document path).
fn saved_and_another(name: &str) -> (VectorcraftApp, std::path::PathBuf, String) {
    let services = crate::Services {
        read: Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string()))),
        write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), services);
    let d = std::env::temp_dir().join(format!("vc-reopen-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let path = d.join("doc.vectorcraft").to_string_lossy().to_string();
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("file.saveAs", json!({"path": path})).unwrap();
    app.run("file.new", json!({})).unwrap();
    assert_eq!((app.session.documents().len(), app.session.active_index()), (2, Some(1)));
    (app, d, path)
}

/// Home shown over the document asked for gives way to it, as it does to any other tab (#5369).
#[test]
fn reopening_the_active_document_closes_home() {
    let (mut app, d, path) = saved_and_another("home");
    app.run("document.activate", json!({"index": 0})).unwrap();
    app.run("app.home", json!({})).unwrap();
    assert!(crate::menus::home_showing(&app));
    assert_eq!(crate::io::open_reporting(&mut app, &path).unwrap()["activated"], true);
    assert!(!crate::menus::home_showing(&app));
    let _ = std::fs::remove_dir_all(d);
}

/// Automation confined to a folder can't reach an open document saved outside it (#5369).
#[test]
fn a_confined_open_does_not_reach_an_open_document_outside_its_roots() {
    use vectorcraft_engine::file_access;
    let (mut app, d, path) = saved_and_another("roots");
    let inside = d.join("inside");
    std::fs::create_dir_all(&inside).unwrap();
    let roots = file_access::AutomationRoots::new(&[inside.as_path()], &[]).unwrap();
    assert!(file_access::confine(roots.as_ref(), || app.run("file.open", json!({"path": path}))).is_err());
    assert_eq!((app.session.documents().len(), app.session.active_index()), (2, Some(1)));
    let _ = std::fs::remove_dir_all(d);
}

/// Opens a prompt on the app's window host in a frame, standing in for a dialog that acts on the active document.
fn open_prompt(app: &mut VectorcraftApp, ctx: &egui::Context) {
    app.ui_ctx = Some(ctx.clone());
    let host = app.dialogs().expect("the window's dialog host");
    host.with(|h| h.open(StandardPrompt::custom("A dialog")));
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| host.with(|h| h.run(ui)));
    out.textures_delta.clear();
}

/// Closes the prompt [`open_prompt`] opened, as Cancel would.
fn close_prompt(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let host = app.dialogs().expect("the window's dialog host");
    host.with(|h| h.cancel::<StandardPrompt>(ctx));
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| host.with(|h| h.run(ui)));
    out.textures_delta.clear();
}

/// While a dialog acts on the active document, an open document asked for again comes to the
/// front only once the dialog closes, so the dialog's OK acts on its own document (#5369).
#[test]
fn reopening_while_a_dialog_is_open_waits_for_it_to_close() {
    let (mut app, d, path) = saved_and_another("dialog");
    let ctx = crate::testing::ui_context();
    open_prompt(&mut app, &ctx);
    assert_eq!(crate::io::open_reporting(&mut app, &path).unwrap(), serde_json::Value::Null);
    crate::opening::poll(&mut app);
    assert_eq!((app.session.documents().len(), app.session.active_index()), (2, Some(1)), "still the dialog's document");
    close_prompt(&mut app, &ctx);
    crate::opening::poll(&mut app);
    assert_eq!((app.session.documents().len(), app.session.active_index()), (2, Some(0)));
    let _ = std::fs::remove_dir_all(d);
}

/// A document asked for under automation roots while a dialog is open comes to the front, or
/// opens again, in those roots once the dialog closes, not in the scope of the frame that runs it
/// (#5369): closed meanwhile and now a link to a file outside the roots, it is not read.
#[cfg(unix)]
#[test]
fn a_queued_reopen_keeps_the_roots_it_was_asked_in() {
    use vectorcraft_engine::file_access;
    let (mut app, d, _) = saved_and_another("queued-roots");
    let (inside, outside) = (d.join("inside"), d.join("outside"));
    std::fs::create_dir_all(&inside).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let path = inside.join("doc.vectorcraft").to_string_lossy().to_string();
    app.run("document.activate", json!({"index": 0})).unwrap();
    app.run("file.saveAs", json!({"path": path})).unwrap();
    app.run("document.activate", json!({"index": 1})).unwrap();
    let ctx = crate::testing::ui_context();
    open_prompt(&mut app, &ctx);
    let roots = file_access::AutomationRoots::new(&[inside.as_path()], &[]).unwrap();
    assert_eq!(file_access::confine(roots.as_ref(), || crate::io::open_reporting(&mut app, &path)).unwrap(), serde_json::Value::Null);
    // Closed, and its file swapped for a link to one outside the roots.
    app.session.execute("file.close", &json!({"index": 0})).unwrap();
    let away = outside.join("doc.vectorcraft");
    std::fs::rename(&path, &away).unwrap();
    std::os::unix::fs::symlink(&away, &path).unwrap();
    close_prompt(&mut app, &ctx);
    crate::opening::poll(&mut app);
    assert_eq!(app.session.documents().len(), 1, "the file outside the roots is not opened");
    let _ = std::fs::remove_dir_all(d);
}
