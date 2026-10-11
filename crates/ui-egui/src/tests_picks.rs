//! File dialogs shown off the UI thread (Linux, #592) or as sheets on the window (macOS, #867):
//! what asked runs again with the answer, on the document it asked for; while a sheet is open
//! nothing closes or quits; and nothing outside `picks` shows a dialog in line.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::mpsc::{Sender, channel};

use egui::{Pos2, Rect, vec2};
use serde_json::{Value, json};
use vectorcraft_engine::Session;

use crate::panels::graphic_styles::GraphicStyleLibraries;
use crate::panels::library_panel::library_menu;
use crate::panels::swatches::SwatchLibraries;
use crate::picks::{self, PickRequest};
use crate::state::Dialog;
use crate::{FilePick, Services, VectorcraftApp};

const SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="30"><rect width="10" height="10"/></svg>"#;

/// What the status bar says when an answer comes for a document no longer open.
const CLOSED: &str = "The document was closed while its file dialog was open";

/// A dialog asked for, and where its answer goes.
type Asked = (PickRequest, Sender<Vec<String>>);

/// The dialogs showing.
#[derive(Clone, Default)]
pub(crate) struct Shown(Rc<RefCell<Vec<Asked>>>);

impl Shown {
    pub(crate) fn requests(&self) -> Vec<PickRequest> {
        self.0.borrow().iter().map(|(r, _)| r.clone()).collect()
    }
    /// The last dialog shown answers `paths`.
    pub(crate) fn answer(&self, paths: &[&str]) {
        let (_, tx) = self.0.borrow_mut().pop().expect("a dialog is showing");
        tx.send(paths.iter().map(|p| p.to_string()).collect()).unwrap();
    }
}

/// The paths files were written to.
pub(crate) type Written = Rc<RefCell<Vec<String>>>;

/// Give `services` dialogs shown off the UI thread (none in line: those fail the test), a reader of
/// [`SVG`] at any path and a writer that lists the paths written.
pub(crate) fn off_the_ui_thread(services: &mut Services) -> (Shown, Written) {
    let (shown, written) = (Shown::default(), Written::default());
    let (s, w) = (shown.clone(), written.clone());
    let in_line = |_: &FilePick| -> Option<String> { panic!("no dialog on the UI thread") };
    services.pick_open = Some(Box::new(in_line));
    services.pick_save = Some(Box::new(in_line));
    services.pick_open_multi = Some(Box::new(|| panic!("no dialog on the UI thread")));
    services.pick_folder = Some(Box::new(|| panic!("no dialog on the UI thread")));
    services.read = Some(Box::new(|_: &str| Ok(SVG.to_vec())));
    services.write = Some(Box::new(move |path: &str, _: &[u8]| {
        w.borrow_mut().push(path.to_string());
        Ok(())
    }));
    services.start_pick = Some(Box::new(move |request: PickRequest| {
        let (tx, rx) = channel();
        s.0.borrow_mut().push((request, tx));
        Some(rx)
    }));
    (shown, written)
}

/// A desktop app whose dialogs show off the UI thread, as sheets on the window when `sheets`
/// (macOS) or as windows of their own (Linux).
fn with_dialogs(sheets: bool) -> (VectorcraftApp, Shown, Written) {
    let mut services = Services { dialogs_are_sheets: sheets, ..Default::default() };
    let (shown, written) = off_the_ui_thread(&mut services);
    (VectorcraftApp::new(Session::new(), services), shown, written)
}

/// The macOS app: its dialogs are sheets.
fn app() -> (VectorcraftApp, Shown, Written) {
    with_dialogs(true)
}

fn poll(app: &mut VectorcraftApp) {
    picks::poll(app, &egui::Context::default());
}

fn dialog_kind(app: &VectorcraftApp) -> Option<&str> {
    app.ui.dialog.as_ref().map(|d| d.kind.as_str())
}

// ------------------------------------------------------------------------- driven like a person

/// Room for a dialog, a panel or a menu (the libraries menu lists every library).
const SCREEN: (f32, f32) = (1600.0, 6000.0);

/// One frame of `draw` with `events` → its texts and where they were drawn.
fn frame(
    ctx: &egui::Context,
    app: &mut VectorcraftApp,
    events: Vec<egui::Event>,
    draw: &impl Fn(&mut VectorcraftApp, &mut egui::Ui),
) -> Vec<(String, Rect)> {
    fn texts(s: &egui::Shape, out: &mut Vec<(String, Rect)>) {
        match s {
            egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.visual_bounding_rect())),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(SCREEN.0, SCREEN.1))), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| draw(app, ui));
    out.textures_delta.clear();
    let mut v = vec![];
    out.shapes.iter().for_each(|c| texts(&c.shape, &mut v));
    v
}

/// Where text `label` was drawn (menu items pad theirs).
fn at(texts: &[(String, Rect)], label: &str) -> Pos2 {
    texts
        .iter()
        .find(|(t, _)| t.trim() == label)
        .unwrap_or_else(|| panic!("no `{label}` in {:?}", texts.iter().map(|t| &t.0).collect::<Vec<_>>()))
        .1
        .center()
}

/// Draw `draw` until it settles, then click its text `label`.
fn click(app: &mut VectorcraftApp, label: &str, draw: impl Fn(&mut VectorcraftApp, &mut egui::Ui)) {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&ctx, app, vec![], &draw);
    frame(&ctx, app, vec![], &draw);
    let pos = at(&frame(&ctx, app, vec![], &draw), label);
    let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&ctx, app, vec![egui::Event::PointerMoved(pos), button(true), button(false)], &draw);
}

/// The open dialog, drawn as the app draws it.
fn dialogs(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    crate::dialogs::show(app, ui.ctx());
}

/// The active document written to `Poster.vectorcraft` in a folder of its own named after `tag`
/// → (the folder, the file).
fn saved_document(app: &mut VectorcraftApp, tag: &str) -> (std::path::PathBuf, String) {
    let b64 = app.run("document.serialize", json!({})).unwrap()["dataBase64"].as_str().unwrap().to_string();
    let dir = std::env::temp_dir().join(format!("vc-picks-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("Poster.vectorcraft");
    std::fs::write(&file, vectorcraft_format::base64_decode(&b64).unwrap()).unwrap();
    (dir, file.to_string_lossy().into_owned())
}

// ------------------------------------------------------------------------------------- the tests

#[test]
fn file_open_opens_what_its_dialog_picks_when_it_answers() {
    let (mut app, shown, _) = app();
    // The dialog shows; File › Open gives up for now, without a "cancelled".
    assert!(app.run("file.open", json!({})).is_err());
    assert!(matches!(shown.requests().as_slice(), [PickRequest::Open(_)]));
    poll(&mut app);
    assert!(app.session.documents().is_empty() && app.ui.status.is_empty(), "{}", app.ui.status);
    // Another dialog waits for this one.
    assert!(app.run("file.open", json!({})).is_err());
    assert_eq!(shown.requests().len(), 1);
    assert!(app.ui.status.contains("file dialog is open"), "{}", app.ui.status);
    shown.answer(&["/art/a.svg"]);
    poll(&mut app);
    assert_eq!(app.session.documents().len(), 1);
    assert_eq!(app.session.active().and_then(|d| d.path.clone()).as_deref(), Some("/art/a.svg"));
    // Cancelled: nothing happens, and dialogs show again.
    assert!(app.run("file.open", json!({})).is_err());
    shown.answer(&[]);
    poll(&mut app);
    assert_eq!(app.session.documents().len(), 1);
    assert!(app.run("file.open", json!({})).is_err());
    assert_eq!(shown.requests().len(), 1);
}

#[test]
fn save_as_goes_on_with_the_path_picked() {
    let (mut app, shown, written) = app();
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
    assert!(app.run("file.saveAs", json!({})).is_err());
    assert!(matches!(shown.requests().as_slice(), [PickRequest::Save(_)]));
    shown.answer(&["/art/poster.svg"]);
    poll(&mut app);
    // SVG Options asks next, and its OK writes the file there.
    assert_eq!(dialog_kind(&app), Some("svgOptions"));
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(*written.borrow(), ["/art/poster.svg"]);
}

#[test]
fn a_dialogs_folder_button_fills_its_field_when_the_picker_answers() {
    let (mut app, shown, _) = app();
    let mut d = Dialog::new("exportForScreens", json!({"folder": ""}));
    app.ui.dialog = Some(d.clone());
    picks::folder_field(&mut app, &mut d, "folder");
    assert_eq!(shown.requests(), [PickRequest::Folder]);
    shown.answer(&["/out"]);
    poll(&mut app);
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.str("folder")).as_deref(), Some("/out"));
}

/// Separations Preview › Load Profile… asks off the UI thread, and the profile picked is loaded
/// when the dialog answers (here a missing file, whose error names it).
#[test]
fn load_profile_asks_off_the_ui_thread() {
    let (mut app, shown, _) = app();
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    click(&mut app, "Load Profile…", crate::panels::separations::show);
    assert!(matches!(shown.requests().as_slice(), [PickRequest::Open(p)] if p.filters.iter().any(|(_, e)| e.contains(&"icc"))));
    poll(&mut app);
    assert!(app.ui.status.is_empty(), "{}", app.ui.status);
    shown.answer(&["/no/such/profile.icc"]);
    poll(&mut app);
    assert!(app.ui.status.contains("/no/such/profile.icc"), "color.loadProfile ran on the path: {}", app.ui.status);
}

/// The Swatches libraries menu's Other Library… asks off the UI thread; the file picked (another
/// document) loads as a library and opens in the library panel.
#[test]
fn other_swatch_library_asks_off_the_ui_thread() {
    let (mut app, shown, _) = app();
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    app.run("swatch.new", json!({"name": "Signal", "color": "#ff3300"})).unwrap();
    let (dir, file) = saved_document(&mut app, "swatches");
    click(&mut app, "Other Library…", library_menu::<SwatchLibraries>);
    assert!(matches!(shown.requests().as_slice(), [PickRequest::Open(_)]));
    poll(&mut app);
    assert!(app.ui.library_panel.is_none() && app.ui.status.is_empty(), "{}", app.ui.status);
    shown.answer(&[&file]);
    poll(&mut app);
    let open = app.ui.library_panel.clone().unwrap_or_else(|| panic!("no library open: {}", app.ui.status));
    assert_eq!(open.kind, "swatches");
    let (_, lib) = vectorcraft_engine::cmd::swatchlib::library(&app.session, &open.id).unwrap();
    assert!(lib.swatch("Signal").is_some(), "{}", open.id);
    let _ = std::fs::remove_dir_all(dir);
}

/// The Graphic Styles libraries menu's Other Library… asks off the UI thread; the file picked
/// (another document) loads as a library and opens in the library panel.
#[test]
fn other_graphic_style_library_asks_off_the_ui_thread() {
    let (mut app, shown, _) = app();
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap()["id"].clone();
    app.run("graphicStyle.new", json!({"name": "Poster Look", "id": id})).unwrap();
    let (dir, file) = saved_document(&mut app, "styles");
    click(&mut app, "Other Library…", library_menu::<GraphicStyleLibraries>);
    assert!(matches!(shown.requests().as_slice(), [PickRequest::Open(_)]));
    shown.answer(&[&file]);
    poll(&mut app);
    let open = app.ui.library_panel.clone().unwrap_or_else(|| panic!("no library open: {}", app.ui.status));
    assert_eq!(open.kind, "graphicStyles");
    let (_, lib) = vectorcraft_engine::cmd::stylelib::library(&app.session, &open.id).unwrap();
    assert!(lib.style("Poster Look").is_some(), "{}", open.id);
    let _ = std::fs::remove_dir_all(dir);
}

/// The Keyboard Shortcuts dialog's Import… and Export… ask off the UI thread: the set imported
/// replaces the dialog's working set (kept on OK), Export… writes it where the dialog answers, and
/// an answer once the dialog is closed does nothing.
#[test]
fn keyboard_shortcuts_import_and_export_ask_off_the_ui_thread() {
    let (mut app, shown, written) = app();
    let set = crate::shortcut_editor::export_json("Custom", &BTreeMap::from([("view.grid".to_string(), "Ctrl+Alt+G".to_string())]));
    let (_, want) = crate::shortcut_editor::import_json(&set).unwrap();
    let bytes = serde_json::to_vec(&set).unwrap();
    app.services.read = Some(Box::new(move |_: &str| Ok(bytes.clone())));
    app.run("edit.keyboardShortcuts", json!({})).unwrap();
    click(&mut app, "Import…", dialogs);
    assert!(matches!(shown.requests().as_slice(), [PickRequest::Open(_)]));
    shown.answer(&["/keys/mine.json"]);
    poll(&mut app);
    let d = app.ui.dialog.as_ref().expect("the dialog stays open");
    assert_eq!((d.kind.as_str(), &d.fields["overrides"]), ("shortcuts", &serde_json::to_value(&want).unwrap()));
    assert!(d.str("__message").starts_with("Imported"), "{}", d.str("__message"));
    assert!(app.ui.shortcut_overrides.is_empty(), "kept only on OK");
    click(&mut app, "Export…", dialogs);
    assert!(matches!(shown.requests().as_slice(), [PickRequest::Save(_)]));
    shown.answer(&["/keys/out.json"]);
    poll(&mut app);
    assert_eq!(*written.borrow(), ["/keys/out.json"]);
    // Closed before the dialog answers: nothing is written, and the status says why.
    click(&mut app, "Export…", dialogs);
    app.ui.dialog = None;
    shown.answer(&["/keys/late.json"]);
    poll(&mut app);
    assert_eq!(*written.borrow(), ["/keys/out.json"]);
    assert_eq!(app.ui.status, picks::dialog_gone());
    // The command waits for its dialog too: no data in place of the file.
    assert_eq!(app.run("shortcuts.export", json!({})), Err("cancelled".to_string()));
    shown.answer(&["/keys/command.json"]);
    poll(&mut app);
    assert_eq!(*written.borrow(), ["/keys/out.json", "/keys/command.json"]);
}

/// A dialog's folder button answered after the dialog closed fills nothing, and the status says
/// why.
#[test]
fn a_folder_for_a_closed_dialog_is_dropped() {
    let (mut app, shown, _) = app();
    let mut d = Dialog::new("exportForScreens", json!({"folder": ""}));
    app.ui.dialog = Some(d.clone());
    picks::folder_field(&mut app, &mut d, "folder");
    app.ui.dialog = None;
    shown.answer(&["/out"]);
    poll(&mut app);
    assert_eq!(dialog_kind(&app), None);
    assert_eq!(app.ui.status, picks::dialog_gone());
}

/// Export… in the PDF, Print, Transparency Flattener and Perspective Grid Presets dialogs asks off
/// the UI thread and writes the selected preset where the dialog answers; the dialog stays open.
#[test]
fn preset_dialogs_export_asks_off_the_ui_thread() {
    use vectorcraft_engine::cmd::{flatten, pdfcmds, perspgrid, printpresets};
    let cases = [
        ("pdf.preset.save", json!({"name": "Mine"}), "ui.pdfPresetsDialog", pdfcmds::PRESET_FORMAT),
        ("print.presets.save", json!({"name": "Mine", "settings": {"copies": 2}}), "ui.printPresetsDialog", printpresets::PRESET_FORMAT),
        ("flattener.presets.save", json!({"name": "Mine", "balance": 20}), "ui.flattenerPresetsDialog", flatten::PRESET_FORMAT),
        ("perspective.presets.save", json!({"name": "Mine"}), "ui.perspectivePresetsDialog", perspgrid::PRESET_FORMAT),
    ];
    for (save, params, open, ext) in cases {
        let (mut app, shown, written) = app();
        app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
        app.run(save, params).unwrap();
        app.run(open, json!({"selected": "Mine"})).unwrap();
        let kind = dialog_kind(&app).unwrap().to_string();
        click(&mut app, "Export…", dialogs);
        assert!(matches!(shown.requests().as_slice(), [PickRequest::Save(_)]), "{open}: {:?}", shown.requests());
        assert!(written.borrow().is_empty(), "{open}");
        let path = format!("/presets/mine.{ext}");
        shown.answer(&[&path]);
        poll(&mut app);
        assert_eq!(*written.borrow(), [path], "{open}: {}", app.ui.status);
        assert_eq!(dialog_kind(&app), Some(kind.as_str()), "{open}");
    }
}

/// Save As answers on the document that asked for it: the one made active meanwhile isn't saved,
/// and the one that asked is active again.
#[test]
fn save_as_saves_the_document_that_asked() {
    let (mut app, shown, written) = app();
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    app.run("file.new", json!({"width": 200, "height": 160})).unwrap();
    let uids: Vec<u64> = app.session.documents().iter().map(|d| d.uid).collect();
    app.run("document.activate", json!({"index": 0})).unwrap();
    assert!(app.run("file.saveAs", json!({})).is_err());
    app.run("document.activate", json!({"index": 1})).unwrap();
    shown.answer(&["/art/a.svg"]);
    poll(&mut app);
    assert_eq!(app.session.active().map(|d| d.uid), Some(uids[0]));
    assert_eq!(dialog_kind(&app), Some("svgOptions"));
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(*written.borrow(), ["/art/a.svg"]);
    let paths: Vec<Option<String>> = app.session.documents().iter().map(|d| d.path.clone()).collect();
    assert_eq!(paths, [Some("/art/a.svg".to_string()), None]);
    assert_eq!(app.session.active().map(|d| d.uid), Some(uids[0]));
}

/// An answer for a document closed meanwhile (here through the engine, as an agent can) is dropped:
/// nothing is written, no other document is saved, and the status says why.
#[test]
fn an_answer_for_a_closed_document_is_dropped() {
    let (mut app, shown, written) = app();
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    app.run("file.new", json!({"width": 200, "height": 160})).unwrap();
    app.run("document.activate", json!({"index": 0})).unwrap();
    assert!(app.run("file.saveAs", json!({})).is_err());
    app.session.execute("file.close", &json!({"index": 0})).unwrap();
    app.sync_views();
    shown.answer(&["/art/a.svg"]);
    poll(&mut app);
    assert_eq!(app.ui.status, CLOSED);
    assert!(written.borrow().is_empty());
    assert_eq!(dialog_kind(&app), None, "no SVG Options");
    assert_eq!(app.session.documents().iter().map(|d| d.path.clone()).collect::<Vec<_>>(), [None]);
    assert!(!app.file_dialog_open());
}

/// A button whose action asks for an open dialog and, once that answers, a save dialog shows both
/// off the UI thread, then finishes with both paths.
#[test]
fn a_buttons_second_dialog_shows_off_the_ui_thread_too() {
    let (mut app, shown, _) = app();
    let (from, done) = (Rc::new(RefCell::new(None::<String>)), Rc::new(RefCell::new(vec![])));
    let (f, d) = (from.clone(), done.clone());
    picks::button(&mut app, move |app| {
        if f.borrow().is_none() {
            let picked = picks::open(app, &FilePick::default());
            *f.borrow_mut() = picked;
        }
        let Some(source) = f.borrow().clone() else { return };
        if let Some(to) = picks::save(app, &FilePick::named("copy.svg")) {
            d.borrow_mut().push((source, to));
        }
    });
    assert!(matches!(shown.requests().as_slice(), [PickRequest::Open(_)]));
    shown.answer(&["/art/in.svg"]);
    poll(&mut app);
    assert_eq!(from.borrow().as_deref(), Some("/art/in.svg"));
    assert_eq!(shown.requests(), [PickRequest::Save(FilePick::named("copy.svg"))]);
    assert!(app.file_dialog_open() && done.borrow().is_empty());
    shown.answer(&["/art/out.svg"]);
    poll(&mut app);
    assert_eq!(*done.borrow(), [("/art/in.svg".to_string(), "/art/out.svg".to_string())]);
    assert!(!app.file_dialog_open());
}

/// While a dialog's sheet is on the window (macOS) nothing quits or closes: Quit, Close and Close
/// All say a file dialog is open and close nothing; once it answers or is cancelled they work
/// again.
#[test]
fn nothing_closes_or_quits_while_a_sheet_is_open() {
    let (mut app, shown, _) = app();
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    app.run("file.new", json!({"width": 200, "height": 160})).unwrap();
    assert!(!app.file_dialog_open() && !app.file_sheet_open());
    assert!(app.run("file.open", json!({})).is_err());
    assert!(app.file_dialog_open() && app.file_sheet_open());
    let refused = |app: &mut VectorcraftApp| {
        for (id, p) in [("app.quit", json!({})), ("file.close", json!({})), ("file.close", json!({"index": 0})), ("file.closeAll", json!({}))] {
            assert_eq!(app.run(id, p.clone()), Err(picks::busy().to_string()), "{id} {p}");
        }
        assert_eq!(app.session.documents().len(), 2);
        assert_ne!(app.ui.status, "quit");
        assert_eq!(dialog_kind(app), None, "no Save Changes either");
    };
    refused(&mut app);
    // Answered: the file opens, and documents close again.
    shown.answer(&["/art/c.svg"]);
    poll(&mut app);
    assert!(!app.file_dialog_open());
    assert_eq!(app.session.documents().len(), 3);
    app.run("file.close", json!({})).unwrap();
    assert_eq!(app.session.documents().len(), 2);
    // Cancelled: the same.
    assert!(app.run("file.saveAs", json!({})).is_err());
    refused(&mut app);
    shown.answer(&[]);
    poll(&mut app);
    assert!(!app.file_dialog_open());
    app.run("file.closeAll", json!({})).unwrap();
    assert!(app.session.documents().is_empty());
    assert!(app.run("file.open", json!({})).is_err());
    assert!(app.run("app.quit", json!({})).is_err());
    shown.answer(&[]);
    poll(&mut app);
    app.run("app.quit", json!({})).unwrap();
    assert_eq!(app.ui.status, "quit");
}

/// On Linux the dialog is a window of its own, not a sheet: Close and Quit go on while it is open,
/// and its answer for the document closed meanwhile is dropped, nothing written.
#[test]
fn on_linux_documents_close_while_a_dialog_is_open() {
    let (mut app, shown, written) = with_dialogs(false);
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    app.run("file.new", json!({"width": 200, "height": 160})).unwrap();
    app.run("document.activate", json!({"index": 0})).unwrap();
    assert!(app.run("file.saveAs", json!({})).is_err());
    assert!(app.file_dialog_open() && !app.file_sheet_open());
    app.run("file.close", json!({})).unwrap();
    assert_eq!(app.session.documents().len(), 1);
    shown.answer(&["/art/a.svg"]);
    poll(&mut app);
    assert_eq!(app.ui.status, CLOSED);
    assert!(written.borrow().is_empty());
    assert_eq!(dialog_kind(&app), None);
    // Quit goes on too.
    assert!(app.run("file.open", json!({})).is_err());
    assert!(app.file_dialog_open());
    app.run("app.quit", json!({})).unwrap();
    assert_eq!(app.ui.status, "quit");
}

/// File › Open acts on no document: its answer opens the file even when the document active at
/// the time closed meanwhile (Linux).
#[test]
fn an_open_answer_goes_on_after_the_active_document_closes() {
    let (mut app, shown, _) = with_dialogs(false);
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    assert!(app.run("file.open", json!({})).is_err());
    app.run("file.close", json!({})).unwrap();
    assert!(app.session.documents().is_empty());
    shown.answer(&["/art/b.svg"]);
    poll(&mut app);
    assert_eq!(app.session.documents().iter().map(|d| d.path.clone()).collect::<Vec<_>>(), [Some("/art/b.svg".to_string())]);
    assert_ne!(app.ui.status, CLOSED);
}

/// Save Changes' Save, answered by its Save dialog, saves and closes the document it asked about,
/// even when an earlier document closed meanwhile (Linux), then asks about the next one.
#[test]
fn save_changes_answers_for_its_own_document() {
    let (mut app, shown, written) = with_dialogs(false);
    let modified = |app: &mut VectorcraftApp| app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    // X clean, A modified and untitled, C modified with a file.
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    modified(&mut app);
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    modified(&mut app);
    app.session.active_mut().unwrap().path = Some("/art/c.vectorcraft".into());
    let uids: Vec<u64> = app.session.documents().iter().map(|d| d.uid).collect();
    app.run("file.closeAll", json!({})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.fields["uid"].clone()), Some(json!(uids[1])), "asks about A");
    // Save: A's Save dialog shows, and X closes meanwhile.
    assert!(crate::dialogs::confirm(&mut app).is_err());
    assert!(matches!(shown.requests().as_slice(), [PickRequest::Save(_)]));
    app.run("file.close", json!({"index": 0})).unwrap();
    shown.answer(&["/art/a.vectorcraft"]);
    poll(&mut app);
    assert_eq!(*written.borrow(), ["/art/a.vectorcraft"], "{}", app.ui.status);
    assert_eq!(app.session.documents().iter().map(|d| d.uid).collect::<Vec<_>>(), [uids[2]], "A closed, C untouched");
    let d = app.ui.dialog.as_ref().expect("Save Changes asks about C next");
    assert_eq!((d.kind.as_str(), &d.fields["uid"]), (crate::unsaved::KIND, &json!(uids[2])));
}

/// A dialog asked for while another is open is refused, and the status bar says so rather than
/// "cancelled".
#[test]
fn a_refused_dialog_says_why() {
    let (mut app, shown, _) = app();
    assert!(app.run("file.open", json!({})).is_err());
    poll(&mut app);
    assert!(app.ui.status.is_empty(), "{}", app.ui.status);
    crate::menus::invoke(&mut app, "file.open", json!({}));
    assert_eq!(shown.requests().len(), 1);
    poll(&mut app);
    assert_eq!(app.ui.status, picks::busy());
    shown.answer(&[]);
    poll(&mut app);
    // The next dialog's own "cancelled" is cleared as before.
    crate::menus::invoke(&mut app, "file.open", json!({}));
    poll(&mut app);
    assert!(app.ui.status.is_empty(), "{}", app.ui.status);
}

/// After a panic inside what asks (a bug the frame's guard catches and recovers from), no entry is
/// left behind: the next command's answer runs that command, not the one that panicked.
#[test]
fn a_panic_inside_an_entry_leaves_none_behind() {
    let (mut app, shown, written) = app();
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    let r = vectorcraft_engine::guard::catch_panic(|| picks::button(&mut app, |_| panic!("a bug")));
    assert!(r.is_err());
    app.picks.recover();
    assert!(app.picks.is_entry_free());
    assert!(app.run("file.open", json!({})).is_err());
    shown.answer(&["/art/b.svg"]);
    poll(&mut app);
    assert_eq!(app.session.documents().len(), 2, "File › Open ran again");
    assert!(written.borrow().is_empty());
}

/// One whole frame of the app (its logic and its window); `close` presses the window's close
/// button.
fn app_frame(ctx: &egui::Context, app: &mut VectorcraftApp, close: bool) -> Vec<egui::ViewportCommand> {
    let mut raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 900.0))), ..Default::default() };
    if close {
        raw.viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
    }
    let mut out = ctx.run_ui(raw, |ui| {
        app.logic(ui.ctx());
        app.ui(ui);
    });
    out.textures_delta.clear();
    out.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.commands.clone()).unwrap_or_default()
}

/// The window's close button doesn't close the window while a dialog's sheet is on it: the status
/// says a file dialog is open. On Linux, and once the sheet answers, it closes.
#[test]
fn the_window_stays_open_while_a_sheet_is_on_it() {
    for sheets in [true, false] {
        let (mut app, shown, _) = with_dialogs(sheets);
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
        app_frame(&ctx, &mut app, false);
        assert!(app.run("file.open", json!({})).is_err());
        let commands = app_frame(&ctx, &mut app, true);
        assert_eq!(commands.contains(&egui::ViewportCommand::CancelClose), sheets, "sheets: {sheets}");
        assert_eq!(app.ui.status == picks::busy(), sheets, "sheets: {sheets}: {}", app.ui.status);
        assert_eq!(app.session.documents().len(), 1);
        shown.answer(&[]);
        app_frame(&ctx, &mut app, false);
        assert!(!app_frame(&ctx, &mut app, true).contains(&egui::ViewportCommand::CancelClose), "sheets: {sheets}");
    }
}

/// `ui.inspect`'s `fileDialog` is the kind of dialog open (open, save, place, folder), null once it
/// answers.
#[test]
fn inspect_tells_the_file_dialog_open() {
    let (mut app, shown, _) = app();
    let ctx = egui::Context::default();
    let shows = |app: &VectorcraftApp| crate::control::inspect(app, &ctx)["fileDialog"].clone();
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    assert_eq!(shows(&app), Value::Null);
    /// What asks for a kind of dialog.
    type Ask = fn(&mut VectorcraftApp);
    let asks: [(&str, Ask); 4] = [
        ("open", |app| assert!(app.run("file.open", json!({})).is_err())),
        ("save", |app| assert!(app.run("file.saveAs", json!({})).is_err())),
        ("place", |app| assert!(app.run("file.place", json!({})).is_err())),
        ("folder", |app| {
            let mut d = Dialog::new("exportForScreens", json!({"folder": ""}));
            app.ui.dialog = Some(d.clone());
            picks::folder_field(app, &mut d, "folder");
        }),
    ];
    for (kind, ask) in asks {
        ask(&mut app);
        assert!(app.file_dialog_open(), "{kind}");
        assert_eq!(shows(&app), json!(kind));
        shown.answer(&[]);
        poll(&mut app);
        assert!(!app.file_dialog_open(), "{kind}");
        assert_eq!(shows(&app), Value::Null, "{kind}");
    }
}

// ------------------------------------------------------------------------ nothing asks in line

/// `src` with comments and the insides of string and char literals blanked: only code is left.
fn code_only(src: &str) -> String {
    let b: Vec<char> = src.chars().collect();
    let ident = |i: usize| i < b.len() && (b[i].is_alphanumeric() || b[i] == '_');
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        let next = b.get(i + 1).copied();
        if c == '/' && next == Some('/') {
            while i < b.len() && b[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            let mut depth = 0;
            while i < b.len() {
                if b[i] == '/' && b.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if b[i] == '*' && b.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            out.push(' ');
        } else if c == 'r' && (i == 0 || !ident(i - 1) || (b[i - 1] == 'b' && (i < 2 || !ident(i - 2)))) && matches!(next, Some('"' | '#')) {
            // A raw string r"…", r#"…"# (or a raw identifier r#name, kept).
            let hashes = b[i + 1..].iter().take_while(|&&c| c == '#').count();
            if b.get(i + 1 + hashes) != Some(&'"') {
                out.push(c);
                i += 1;
                continue;
            }
            let close: Vec<char> = std::iter::once('"').chain(std::iter::repeat_n('#', hashes)).collect();
            i += 2 + hashes;
            while i < b.len() && !b[i..].starts_with(&close) {
                i += 1;
            }
            i += close.len();
            out.push_str("\"\"");
        } else if c == '"' {
            i += 1;
            while i < b.len() && b[i] != '"' {
                i += if b[i] == '\\' { 2 } else { 1 };
            }
            i += 1;
            out.push_str("\"\"");
        } else if c == '\'' && next == Some('\\') {
            // An escaped char literal.
            i += 2;
            while i < b.len() && b[i] != '\'' {
                i += 1;
            }
            i += 1;
            out.push_str("' '");
        } else if c == '\'' && b.get(i + 2) == Some(&'\'') {
            i += 3;
            out.push_str("' '");
        } else {
            // A lifetime's quote, or code.
            out.push(c);
            i += 1;
        }
    }
    out
}

/// `code` (from [`code_only`]) without its `#[cfg(test)]` items: test modules and their
/// declarations, test helpers.
fn without_tests(code: &str) -> String {
    let mut out = String::new();
    let mut rest = code;
    while let Some(at) = rest.find("#[cfg(test)]") {
        out.push_str(&rest[..at]);
        let item = &rest[at..];
        // The item ends at its first `;` outside braces, or with the braces it opens.
        let mut depth = 0usize;
        let mut end = item.len();
        for (k, ch) in item.char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        end = k + 1;
                        break;
                    }
                }
                ';' if depth == 0 => {
                    end = k + 1;
                    break;
                }
                _ => {}
            }
        }
        rest = &item[end..];
    }
    out.push_str(rest);
    out
}

/// The calls in `code` (from [`code_only`]) of the host's in-line dialogs: [`Services`]'
/// `pick_open`, `pick_save`, `pick_folder` and `pick_open_multi` used other than to ask whether
/// the host has them (`.is_some()`, `.is_none()`).
fn in_line_dialogs(code: &str) -> Vec<String> {
    let mut found = vec![];
    for name in ["pick_open", "pick_save", "pick_folder", "pick_open_multi"] {
        let field = format!(".{name}");
        let mut rest = code;
        while let Some(at) = rest.find(&field) {
            rest = &rest[at + field.len()..];
            // Another field whose name starts with this one's (`pick_open_multi`).
            if rest.starts_with(|c: char| c.is_alphanumeric() || c == '_') {
                continue;
            }
            // What follows, over line breaks.
            let next: String = rest.chars().filter(|c| !c.is_whitespace()).take(40).collect();
            if !next.starts_with(".is_some()") && !next.starts_with(".is_none()") {
                found.push(format!("{field}{next}"));
            }
        }
    }
    found
}

/// Nothing but `picks` shows the host's file dialogs: everything else asks through it, which shows
/// them off the UI thread when what asks runs as an entry (#867). Asking whether the host has one
/// is fine.
#[test]
fn only_picks_shows_the_hosts_dialogs() {
    // The scan finds the in-line call Load Profile… made, split over lines, and not presence checks
    // or comments.
    let before = r#"if widgets::flat_button(ui, tl!("Load Profile…"), 110.0).clicked()
            && let Some(path) = app
                .services
                .pick_open
                .as_mut()
                .and_then(|f| f(&crate::FilePick { filters: vec![("ICC Profiles", &["icc", "icm"])], ..Default::default() }))
        {"#;
    assert_eq!(in_line_dialogs(&code_only(before)).len(), 1);
    assert_eq!(in_line_dialogs(&code_only("let s = &mut app.services; let p = (s.pick_save.as_mut()?)(&pick);")).len(), 1);
    assert_eq!(in_line_dialogs(&code_only("let Some(f) = &mut s.pick_open else { return };")).len(), 1);
    let fine = "let paths = if app.services.pick_open_multi.is_some() { 1 } else { 2 }; // app.services.pick_open.as_mut()\n\
                let t = \"services.pick_folder.take()\"; let c = '\"'; let u = s.pick_folder.is_none();";
    assert_eq!(in_line_dialogs(&code_only(fine)), Vec::<String>::new());
    assert_eq!(
        without_tests(&code_only("fn a() {}\n#[cfg(test)]\nmod tests;\n#[cfg(test)]\nmod t { fn b() { x.pick_open.take(); } }\nfn c() {}")),
        "fn a() {}\n\n\nfn c() {}"
    );

    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    // Each source file's code without its tests, by its path under `src`.
    let mut scanned = BTreeMap::new();
    let mut stack = vec![dir.clone()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
            if path.is_dir() {
                stack.push(path);
            } else if name.ends_with(".rs") && !name.starts_with("tests") && name != "picks.rs" {
                let key = path.strip_prefix(&dir).unwrap().to_string_lossy().replace('\\', "/");
                scanned.insert(key, without_tests(&code_only(&std::fs::read_to_string(&path).unwrap())));
            }
        }
    }
    // Code after a test item is still read: lib.rs declares its test modules first, and the
    // shortcut editor's mirror macro has a test-only block.
    for (file, code) in [
        ("lib.rs", "fnfile_dialog_open"),
        ("shortcut_editor.rs", "fnimport_into_dialog"),
        ("ui_fonts.rs", "fnpainted_text"),
        ("place.rs", ".pick_open_multi.is_some()"),
        ("panels/separations.rs", "picks::open("),
    ] {
        let flat: String = scanned.get(file).unwrap_or_else(|| panic!("{file} not read")).chars().filter(|c| !c.is_whitespace()).collect();
        assert!(flat.contains(code), "{file} lost `{code}`");
    }
    let found: Vec<String> = scanned.iter().flat_map(|(file, code)| in_line_dialogs(code).into_iter().map(move |c| format!("{file}: {c}"))).collect();
    assert!(found.is_empty(), "file dialogs shown in line, not through `picks`: {found:#?}");
}
