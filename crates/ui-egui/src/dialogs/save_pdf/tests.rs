use std::cell::RefCell;
use std::rc::Rc;

use serde_json::{Value, json};
use vectorcraft_engine::Session;

use super::super::tests_export::save_panel;
use super::*;
use crate::Services;
use craft::ui::dialog::{DialogHost, DialogId};
use craft::ui::patterns::form_dialogs::{HostedPdfExport, PdfExportWords};

type Log = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

/// The key the host registers Save PDF (and the preset editor) under.
const PDF_KEY: &str = "pdf-export";

/// An app with three artboards whose writer records what it gets.
fn app() -> (VectorcraftApp, Log) {
    let written = Log::default();
    let w = written.clone();
    let services = Services {
        write: Some(Box::new(move |p: &str, b: &[u8]| {
            w.borrow_mut().push((p.to_string(), b.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), services));
    app.run("file.new", json!({"width": 120, "height": 90, "artboards": 3})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    (app, written)
}

/// Run `f` on the window's dialog host and the UI context.
fn host<R>(app: &VectorcraftApp, f: impl FnOnce(&mut DialogHost, &egui::Context) -> R) -> R {
    let ctx = app.ui_ctx.clone().expect("a UI context");
    app.dialogs().expect("the dialog host").with(|h| f(h, &ctx))
}

/// The open dialogs, bottom to top, as (id, key).
fn open_dialogs(app: &VectorcraftApp) -> Vec<(DialogId, String)> {
    host(app, |h, _| h.open_dialogs().into_iter().map(|d| (d.id, d.key.unwrap_or_default())).collect())
}

/// The open Save PDF dialog (`pdf`) or the topmost other one, such as Custom….
fn dialog_id(app: &VectorcraftApp, pdf: bool) -> Option<DialogId> {
    open_dialogs(app).into_iter().rev().find(|(_, k)| (k == PDF_KEY) == pdf).map(|(id, _)| id)
}

fn pdf_id(app: &VectorcraftApp) -> DialogId {
    dialog_id(app, true).expect("Save PDF is open")
}

fn is_open(app: &VectorcraftApp) -> bool {
    dialog_id(app, true).is_some()
}

fn fields_of(app: &VectorcraftApp, id: DialogId) -> Value {
    host(app, |h, _| h.open_dialogs().into_iter().find(|d| d.id == id).and_then(|d| d.fields)).unwrap_or(Value::Null)
}

fn fields(app: &VectorcraftApp) -> Value {
    fields_of(app, pdf_id(app))
}

/// A field of Save PDF by its dotted name.
fn fget(app: &VectorcraftApp, name: &str) -> Value {
    name.split('.').try_fold(fields(app), |v, k| v.get(k).cloned()).unwrap_or(Value::Null)
}

/// Write a field as automation does; a refusal is the `Err`.
fn set_at(app: &VectorcraftApp, id: DialogId, name: &str, v: Value) -> Result<(), String> {
    host(app, |h, c| h.set_json_field(c, id, name, v)).map_err(|e| format!("{e:?}"))
}

fn set_field(app: &VectorcraftApp, name: &str, v: Value) {
    set_at(app, pdf_id(app), name, v).unwrap();
}

/// Write a field through automation's `ui.dialog.set`, which also takes the Custom… flattener options the app keeps.
fn dialog_set(app: &mut VectorcraftApp, name: &str, v: Value) {
    let ctx = app.ui_ctx.clone().expect("a UI context");
    let (req, _) = craft::automation::ControlRequest::new("ui.dialog.set", json!({"field": name, "value": v}));
    let craft::automation::Reply::Done(r) = crate::control::handle(app, &ctx, &req) else { panic!("ui.dialog.set {name} answered later") };
    assert_eq!(r["ok"], true, "ui.dialog.set {name}: {r}");
}

fn with_pdf<R>(app: &VectorcraftApp, f: impl FnOnce(&mut HostedPdfExport) -> R) -> R {
    let id = pdf_id(app);
    host(app, |h, _| h.get_mut_by_id::<HostedPdfExport>(id).map(f)).expect("a Save PDF dialog")
}

/// The Summary as the open dialog writes it.
fn summary(app: &VectorcraftApp) -> String {
    let words = app.host.words().set::<PdfExportWords>();
    with_pdf(app, |d| d.summary_text(&words))
}

/// One frame: the app's pass, the dialog layer's pump, then the save panels answer.
fn pump(app: &mut VectorcraftApp) {
    super::super::tests_export::frame(app);
}

/// OK of the dialog `id`: a held OK is answered in the first frame and the dialog closes in the second.
fn confirm_id(app: &mut VectorcraftApp, id: DialogId) {
    host(app, |h, c| h.confirm_by_id(c, id));
    pump(app);
    pump(app);
}

fn cancel_id(app: &mut VectorcraftApp, id: DialogId) {
    host(app, |h, c| h.cancel_by_id(c, id));
    pump(app);
}

/// OK of Save PDF; the write runs in the pump.
fn confirm(app: &mut VectorcraftApp) {
    let id = pdf_id(app);
    confirm_id(app, id);
}

fn cancel(app: &mut VectorcraftApp) {
    let id = pdf_id(app);
    cancel_id(app, id);
}

/// Press a control that has no automation field, by its name.
fn press(app: &mut VectorcraftApp, name: &str) {
    let ctx = app.ui_ctx.clone().expect("a UI context");
    // The click finds the control in the accessibility tree of a frame drawn first, as a user presses what the window shows.
    crate::testing::app_frame(app, &ctx, Default::default());
    let (click, _reply) = craft::automation::ControlRequest::new("ui.click", json!({"control": {"name": name}}));
    match crate::control::handle(app, &ctx, &click) {
        craft::automation::Reply::Done(r) => assert_eq!(r["ok"], true, "ui.click {name}: {r}"),
        // The click is queued as input: the window's frames after it take it and press the control.
        _ => {
            for _ in 0..2 {
                crate::testing::app_frame(app, &ctx, Default::default());
            }
        }
    }
    pump(app);
}

/// Save the Summary to `path` and return the text written.
fn save_summary_to(app: &mut VectorcraftApp, written: &Log, path: &str) -> String {
    app.run(SAVE_SUMMARY, json!({"path": path})).unwrap();
    String::from_utf8(written.borrow().last().unwrap().1.clone()).unwrap()
}

fn line_count(text: &str) -> usize {
    text.lines().count()
}

/// The flattener options the pending save carries, as JSON.
fn job_flattener(app: &VectorcraftApp) -> Option<Value> {
    let job = app.save_pdf.as_ref().and_then(|o| o.job.as_ref()).expect("a pending save");
    job.flattener.as_ref().map(|o| serde_json::to_value(o).unwrap())
}

/// `ui.pdfSummary.save` writes the Summary as the dialog shows it, as UTF-8 text with LF line
/// endings. The dialog stays open and the document does not change.
#[test]
fn save_summary_writes_the_summary_as_text() {
    let (mut app, written) = app();
    app.run("file.export.pdf", json!({})).unwrap();
    let before = summary(&app);
    set_field(&app, "thumbnails", json!(true));
    set_field(&app, "security.printing", json!("low"));
    let revision = app.session.active().unwrap().revision;
    let path = "/tmp/Ünïcode (1) summary.txt";
    let r = app.run(SAVE_SUMMARY, json!({"path": path})).unwrap();
    assert_eq!(r["path"], path, "the path is used as given");
    let w = written.borrow();
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].0, path);
    assert_eq!(r["bytes"], w[0].1.len());
    let text = String::from_utf8(w[0].1.clone()).expect("UTF-8");
    assert!(!text.contains('\r') && text.ends_with('\n'), "LF line endings: {text:?}");
    assert!(line_count(&text) > line_count(&before), "changed options and a warning add lines: {text}");
    assert!(is_open(&app), "the dialog stays open");
    assert_eq!(app.session.active().unwrap().revision, revision, "the document does not change");
    assert!(app.ui.status.contains(path), "{}", app.ui.status);
}

/// A text option whose value holds CRLF or CR line breaks is written with LF line breaks, so the
/// file has one kind of line ending.
#[test]
fn save_summary_writes_line_breaks_in_values_as_lf() {
    let (mut app, written) = app();
    app.run("file.export.pdf", json!({})).unwrap();
    set_field(&app, "output.outputCondition", json!("first\r\nsecond\rthird"));
    app.run(SAVE_SUMMARY, json!({"path": "/tmp/summary.txt"})).unwrap();
    let text = String::from_utf8(written.borrow()[0].1.clone()).unwrap();
    assert!(!text.contains('\r'), "LF line endings: {text:?}");
    assert!(text.contains("first\nsecond\nthird\n"), "{text:?}");
}

/// The preset editor's Summary is saved too, measured against the app's default preset, and the
/// editor stays open.
#[test]
fn save_summary_writes_the_preset_editors_summary() {
    let (mut app, written) = app();
    open_preset(&mut app, &json!({})).unwrap();
    let untouched = save_summary_to(&mut app, &written, "/tmp/preset summary.txt");
    set_field(&app, "thumbnails", json!(true));
    let changed = save_summary_to(&mut app, &written, "/tmp/preset summary.txt");
    assert!(line_count(&changed) > line_count(&untouched), "{changed}");
    set_field(&app, "thumbnails", json!(false));
    assert_eq!(save_summary_to(&mut app, &written, "/tmp/preset summary.txt"), untouched);
    assert!(is_open(&app) && fields(&app).get("name").is_some(), "the editor stays open");
}

/// A path that is empty or not a string, and no Save PDF dialog open, are errors that write
/// nothing.
#[test]
fn save_summary_refuses_a_bad_path_and_a_closed_dialog() {
    let (mut app, written) = app();
    assert_eq!(app.run(SAVE_SUMMARY, json!({"path": "/tmp/s.txt"})).unwrap_err(), "the Save PDF dialog is not open");
    app.run("file.export.pdf", json!({})).unwrap();
    assert_eq!(app.run(SAVE_SUMMARY, json!({"path": ""})).unwrap_err(), "path is empty");
    assert_eq!(app.run(SAVE_SUMMARY, json!({"path": "  "})).unwrap_err(), "path is empty");
    assert_eq!(app.run(SAVE_SUMMARY, json!({"path": 5})).unwrap_err(), "`path` must be a string");
    assert!(written.borrow().is_empty());
    assert!(is_open(&app));
}

/// The command needs a path: omitted or null, it is an error that shows no save panel and writes
/// nothing.
#[test]
fn save_summary_requires_a_path() {
    let (mut app, written) = app();
    let asked: Rc<RefCell<usize>> = Rc::default();
    let a = asked.clone();
    save_panel(&mut app, move |pick| {
        *a.borrow_mut() += 1;
        Some(format!("/tmp/{}", pick.file_name.as_deref().unwrap_or_default()))
    });
    app.run("file.export.pdf", json!({})).unwrap();
    assert_eq!(app.run(SAVE_SUMMARY, json!({})).unwrap_err(), "missing `path`");
    assert_eq!(app.run(SAVE_SUMMARY, json!({"path": null})).unwrap_err(), "missing `path`");
    assert_eq!(*asked.borrow(), 0, "no save panel");
    assert!(written.borrow().is_empty());
    assert!(is_open(&app));
}

/// The pump hands the replaced saved preset to the dialog, so the Summary compares with the new
/// values.
#[test]
fn save_summary_compares_against_a_replaced_preset() {
    let (mut app, written) = app();
    app.run("pdf.preset.save", json!({"name": "Mine", "thumbnails": true})).unwrap();
    app.run("ui.savePdfDialog", json!({"preset": "Mine"})).unwrap();
    set_field(&app, "thumbnails", json!(false));
    pump(&mut app);
    let differs = save_summary_to(&mut app, &written, "/tmp/a.txt");
    app.run("pdf.preset.save", json!({"name": "Mine", "thumbnails": false})).unwrap();
    pump(&mut app);
    let matches = save_summary_to(&mut app, &written, "/tmp/b.txt");
    assert!(line_count(&differs) > line_count(&matches), "{differs}\n{matches}");
    cancel(&mut app);
    app.run("ui.savePdfDialog", json!({"preset": "Mine"})).unwrap();
    assert_eq!(matches, save_summary_to(&mut app, &written, "/tmp/c.txt"), "compared against the replaced preset");
}

/// The settings are checked again when the saved flattener preset the dialog names is deleted while
/// the dialog stays open: they are then refused, and nothing is written.
#[test]
fn save_summary_sees_a_deleted_flattener_preset() {
    let (mut app, written) = app();
    app.run("flattener.presets.save", json!({"name": "Mine Flat", "preset": "Low Resolution"})).unwrap();
    // The flattener preset is only editable where the save flattens: PDF 1.3.
    app.run("ui.savePdfDialog", json!({"compatibility": "1.3"})).unwrap();
    set_field(&app, "flattenerPreset", json!("Mine Flat"));
    app.run(SAVE_SUMMARY, json!({"path": "/tmp/a.txt"})).unwrap();
    app.run("flattener.presets.delete", json!({"name": "Mine Flat"})).unwrap();
    pump(&mut app);
    let e = app.run(SAVE_SUMMARY, json!({"path": "/tmp/b.txt"})).unwrap_err();
    assert!(e.contains("Mine Flat"), "the deleted preset is refused: {e}");
    assert_eq!(written.borrow().len(), 1, "nothing written for the deleted preset");
}

/// The settings are checked again when a loaded profile the dialog converts to is removed while the
/// dialog stays open: they are then refused, and nothing is written.
#[test]
fn save_summary_sees_a_removed_profile() {
    use vectorcraft_color::cms;
    let name = "Summary Stamp CMYK";
    // The built-in CMYK profile's file renamed `name`: padded with spaces, which names drop.
    let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
    let mut bytes = cms::icc_bytes(cms::DEVICE_CMYK).unwrap().to_vec();
    let (from, to) = (utf16(cms::DEVICE_CMYK), utf16(&format!("{name:<width$}", width = cms::DEVICE_CMYK.len())));
    let at = bytes.windows(from.len()).position(|w| w == from).unwrap();
    bytes[at..at + to.len()].copy_from_slice(&to);
    let path = vectorcraft_testkit::temp_dir("save-summary-profile").join("stamp.icc");
    std::fs::write(&path, bytes).unwrap();
    let (mut app, written) = app();
    assert_eq!(app.run("color.loadProfile", json!({"path": path.to_str().unwrap()})).unwrap()["name"], name);
    app.run("file.export.pdf", json!({})).unwrap();
    set_field(&app, "output.conversion", json!("destination"));
    set_field(&app, "output.destination", json!(name));
    app.run(SAVE_SUMMARY, json!({"path": "/tmp/a.txt"})).unwrap();
    app.run("color.removeProfile", json!({"name": name})).unwrap();
    pump(&mut app);
    let e = app.run(SAVE_SUMMARY, json!({"path": "/tmp/b.txt"})).unwrap_err();
    assert!(e.contains(name), "the removed profile is refused: {e}");
    assert_eq!(written.borrow().len(), 1, "nothing written for the removed profile");
}

/// With the desktop's file writer: a file already at the path is replaced whole, a folder's path
/// is an error that leaves the folder as it was, and under automation write roots a path outside
/// them is refused without writing.
#[test]
fn save_summary_writes_files_on_disk_within_the_automation_roots() {
    let base = std::env::temp_dir().join(format!("vc-pdf-summary-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    for d in ["out", "outside"] {
        std::fs::create_dir_all(base.join(d)).unwrap();
    }
    let roots = vectorcraft_engine::file_access::AutomationRoots::new(&[], &[&base.join("out")]).unwrap();
    let services = Services {
        write: Some(Box::new(|p: &str, b: &[u8]| vectorcraft_format::write_atomic(std::path::Path::new(p), b).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), services).with_automation_roots(roots.clone()));
    app.run("file.new", json!({"width": 120, "height": 90})).unwrap();
    app.run("file.export.pdf", json!({})).unwrap();
    // The Summary lists the changed options: one gives it a line.
    set_field(&app, "thumbnails", json!(true));
    let mut save = |path: &std::path::Path| {
        let p = json!({"path": path.to_string_lossy()});
        vectorcraft_engine::file_access::confine(roots.as_ref(), || app.run(SAVE_SUMMARY, p))
    };
    let file = base.join("out").join("Ünïcode (1), summary.txt");
    std::fs::write(&file, "an older file, longer than the summary\n".repeat(40)).unwrap();
    save(&file).unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(!text.contains("an older file") && text.ends_with('\n'), "replaced whole: {text:?}");
    assert!(save(&base.join("out")).is_err(), "a folder is not a file");
    assert!(base.join("out").is_dir() && file.is_file());
    let outside = base.join("outside").join("summary.txt");
    assert!(save(&outside).is_err(), "outside the write roots");
    assert!(!outside.exists());
    let _ = std::fs::remove_dir_all(&base);
}

/// The Summary section's Save Summary… button: the save panel suggests `<document> PDF
/// Summary.txt`, the file holds the text the dialog shows, and the dialog stays open. A canceled
/// panel writes nothing; a failed write is told over the dialog, which shows again when the
/// message closes. The web downloads the file under the suggested name.
#[test]
fn the_save_summary_button_writes_the_summary() {
    let (mut app, written) = app();
    let suggested: Rc<RefCell<Vec<String>>> = Rc::default();
    let s = suggested.clone();
    save_panel(&mut app, move |pick| {
        let name = pick.file_name.clone().unwrap_or_default();
        s.borrow_mut().push(name.clone());
        Some(format!("/tmp/{name}"))
    });
    app.run("file.export.pdf", json!({})).unwrap();
    let untouched = line_count(&summary(&app));
    set_field(&app, "thumbnails", json!(true));
    // The button is in the Summary section, which the user opens first.
    set_field(&app, "section", json!("summary"));
    press(&mut app, "Save Summary…");
    let title = app.session.active().unwrap().title();
    let stem = std::path::Path::new(&title).file_stem().unwrap().to_string_lossy().into_owned();
    let name = format!("{stem} PDF Summary.txt");
    assert_eq!(suggested.borrow()[..], *std::slice::from_ref(&name));
    {
        let w = written.borrow();
        assert_eq!(w.len(), 1, "written once");
        assert_eq!(w[0].0, format!("/tmp/{name}"));
        assert!(line_count(&String::from_utf8(w[0].1.clone()).unwrap()) > untouched);
    }
    assert!(is_open(&app), "the dialog stays open");
    // Canceled: nothing written, no status, the dialog stays.
    save_panel(&mut app, |_| None);
    app.ui.status.clear();
    press(&mut app, "Save Summary…");
    assert_eq!(written.borrow().len(), 1, "a canceled panel writes nothing");
    assert!(app.ui.status.is_empty(), "{}", app.ui.status);
    assert!(is_open(&app));
    // A failed write: a message over the dialog.
    save_panel(&mut app, |pick| Some(format!("/tmp/{}", pick.file_name.as_deref().unwrap_or_default())));
    app.services.write = Some(Box::new(|_: &str, _: &[u8]| Err("disk full".into())));
    press(&mut app, "Save Summary…");
    let open = open_dialogs(&app);
    assert_eq!(open.len(), 2, "{open:?}");
    assert!(open[0].1 == PDF_KEY && open[1].1 != PDF_KEY, "the message is above Save PDF: {open:?}");
    cancel_id(&mut app, open[1].0);
    assert_eq!(open_dialogs(&app).len(), 1, "Save PDF shows again");
    // The web downloads it under the suggested name.
    let downloads: Log = Log::default();
    let dl = downloads.clone();
    app.services.download = Some(Box::new(move |n: &str, b: &[u8]| {
        dl.borrow_mut().push((n.to_string(), b.to_vec()));
        Ok(())
    }));
    press(&mut app, "Save Summary…");
    let d = downloads.borrow();
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].0, name);
    assert!(line_count(&String::from_utf8(d[0].1.clone()).unwrap()) > untouched);
}

#[test]
fn confirm_runs_export_pdf_with_the_dialog_options() {
    let (mut app, written) = app();
    app.run("ui.savePdfDialog", json!({"path": "/tmp/out.pdf", "compatibility": "1.5"})).unwrap();
    set_field(&app, "artboards", json!("range"));
    set_field(&app, "range", json!("1,3"));
    set_field(&app, "compression.compressText", json!(false));
    set_field(&app, "viewAfterSaving", json!(true));
    confirm(&mut app);
    assert!(!is_open(&app), "closes after saving");
    let w = written.borrow();
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].0, "/tmp/out.pdf");
    assert!(w[0].1.starts_with(b"%PDF-1.5"), "compatibility reaches the writer");
    assert_eq!(vectorcraft_pdf::import(&w[0].1).unwrap().artboards.len(), 2, "range 1,3");
    assert!(!String::from_utf8_lossy(&w[0].1).contains("/FlateDecode"), "uncompressed content");
    let url = app.pending_url.take().expect("View PDF after Saving opens the file");
    assert!(url.starts_with("file:///") && url.ends_with("/tmp/out.pdf"), "the written file as a URL: {url}");
    assert!(app.ui.status.starts_with("Saved /tmp/out.pdf"), "{}", app.ui.status);
}

/// A Save PDF that fails keeps the dialog open (#6861); saving again, once it works, closes it.
#[test]
fn a_failed_save_says_why_in_the_dialog() {
    let (mut app, written) = app();
    app.run("ui.savePdfDialog", json!({"path": "/tmp/out.pdf"})).unwrap();
    set_field(&app, "artboards", json!("range"));
    set_field(&app, "range", json!("9"));
    confirm(&mut app);
    assert!(written.borrow().is_empty(), "nothing was written");
    assert!(is_open(&app), "the dialog stays open");
    set_field(&app, "range", json!("1"));
    confirm(&mut app);
    assert!(!is_open(&app));
    assert_eq!(written.borrow().len(), 1);
}

/// Opened for a page of the art's bounds (Export As with Use Artboards off), the dialog has no
/// artboard choice, and OK writes one page of the art's bounds with its options, a saved preset
/// and View PDF after Saving included.
#[test]
fn a_page_of_the_arts_bounds_has_no_artboards_section_and_takes_the_options() {
    let (mut app, written) = app();
    app.run("pdf.preset.save", json!({"name": "Mine", "compatibility": "1.5"})).unwrap();
    app.run("ui.savePdfDialog", json!({"path": "/tmp/artboards.pdf"})).unwrap();
    assert_eq!(fget(&app, "artboards"), "all");
    cancel(&mut app);
    app.run("ui.savePdfDialog", json!({"path": "/tmp/art.pdf", "preset": "Mine", "useArtboards": false})).unwrap();
    assert_eq!(fget(&app, "artboards"), "artBounds");
    set_field(&app, "viewAfterSaving", json!(true));
    confirm(&mut app);
    assert!(!is_open(&app), "closes after saving");
    let w = written.borrow();
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].0, "/tmp/art.pdf");
    assert!(w[0].1.starts_with(b"%PDF-1.5"), "the saved preset reaches the writer");
    let pdf = vectorcraft_pdf::import(&w[0].1).unwrap();
    assert_eq!(pdf.artboards.len(), 1);
    let size = pdf.artboards[0].rect.size();
    assert!((50.0..52.0).contains(&size.width) && (40.0..42.0).contains(&size.height), "the rectangle's bounds with its stroke: {size:?}");
    let url = app.pending_url.take().expect("View PDF after Saving opens the file");
    assert!(url.ends_with("/tmp/art.pdf"), "{url}");
}

/// `useArtboards` other than true, false or null is refused, as `document.exportPdf` refuses it,
/// and no dialog opens.
#[test]
fn a_use_artboards_that_is_not_true_or_false_is_refused() {
    let (mut app, _) = app();
    for bad in [json!("false"), json!(0), json!({})] {
        let e = app.run("ui.savePdfDialog", json!({"path": "/tmp/x.pdf", "useArtboards": bad})).unwrap_err();
        assert!(e.contains("useArtboards must be true or false"), "{bad}: {e}");
        assert!(!is_open(&app), "{bad}: no dialog opens");
    }
    for ok in [json!(true), json!(null)] {
        app.run("ui.savePdfDialog", json!({"path": "/tmp/x.pdf", "useArtboards": ok})).unwrap();
        assert_ne!(fget(&app, "artboards"), "artBounds", "{ok}: the artboards' dialog");
        cancel(&mut app);
    }
}

/// With Background Export on, a page of the art's bounds is written in the background, as Export
/// As writes its other formats.
#[test]
fn a_page_of_the_arts_bounds_is_written_in_the_background() {
    let written = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(String, Vec<u8>)>::new()));
    let w = written.clone();
    let services = Services {
        write_shared: Some(std::sync::Arc::new(move |p: &str, b: &[u8]| {
            w.lock().unwrap().push((p.to_string(), b.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), services));
    app.session.prefs.background_export = true;
    app.run("file.new", json!({"width": 120, "height": 90})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    app.run("ui.savePdfDialog", json!({"path": "/tmp/bg.pdf", "useArtboards": false})).unwrap();
    confirm(&mut app);
    assert!(!is_open(&app));
    crate::background::wait_all(&mut app);
    let w = written.lock().unwrap();
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].0, "/tmp/bg.pdf");
    assert_eq!(vectorcraft_pdf::import(&w[0].1).unwrap().artboards.len(), 1);
}

/// A document without a title of its own, saved to a file, is titled after that file in the PDF,
/// and viewers show the file name, with or without Use Artboards; one with its own title shows
/// that title.
#[test]
fn a_page_of_the_arts_bounds_shows_the_title_only_when_it_is_the_documents_own() {
    let (mut app, written) = app();
    app.session.doc_mut().unwrap().path = Some("/docs/Poster.vectorcraft".into());
    for artboards in [true, false] {
        app.run("ui.savePdfDialog", json!({"path": "/tmp/t.pdf", "useArtboards": artboards, "compression": {"compressText": false}})).unwrap();
        confirm(&mut app);
    }
    std::sync::Arc::make_mut(&mut app.session.doc_mut().unwrap().doc).title = "Spring Sale".into();
    app.run("ui.savePdfDialog", json!({"path": "/tmp/t.pdf", "useArtboards": false, "compression": {"compressText": false}})).unwrap();
    confirm(&mut app);
    let w = written.borrow();
    assert_eq!(w.len(), 3);
    let texts: Vec<String> = w.iter().map(|(_, b)| String::from_utf8_lossy(b).into_owned()).collect();
    for (pdf, artboards) in texts[..2].iter().zip([true, false]) {
        assert!(pdf.contains("/Title(Poster)"), "useArtboards {artboards}");
        assert!(!pdf.contains("/DisplayDocTitle true"), "useArtboards {artboards}");
    }
    assert!(texts[2].contains("/Title(Spring Sale)") && texts[2].contains("/DisplayDocTitle true"));
}

#[test]
fn warnings_reach_the_status_and_errors_keep_the_dialog_open() {
    let (mut app, written) = app();
    app.run("ui.savePdfDialog", json!({"path": "/tmp/w.pdf"})).unwrap();
    set_field(&app, "security.printing", json!("low"));
    confirm(&mut app);
    assert!(app.ui.status.contains("1 note:") && app.ui.status.contains("permissions"), "{}", app.ui.status);
    assert!(app.pending_url.is_none(), "not opened unless asked");
    app.run("ui.savePdfDialog", json!({"path": "/tmp/bad.pdf"})).unwrap();
    set_field(&app, "artboards", json!("range"));
    set_field(&app, "range", json!("7"));
    confirm(&mut app);
    assert!(is_open(&app), "a bad range keeps the dialog open");
    assert_eq!(written.borrow().len(), 1);
    cancel(&mut app);
    // Without a path, bad options are refused before a save dialog asks for one.
    let asked = Rc::new(RefCell::new(0));
    let a = asked.clone();
    save_panel(&mut app, move |pick| {
        *a.borrow_mut() += 1;
        Some(format!("/tmp/picked-{}", pick.file_name.as_deref().unwrap_or_default()))
    });
    app.run("ui.savePdfDialog", json!({"range": "7"})).unwrap();
    set_field(&app, "artboards", json!("range"));
    confirm(&mut app);
    assert_eq!(*asked.borrow(), 0, "no save dialog for a bad range");
    assert!(is_open(&app));
    set_field(&app, "range", json!("2"));
    confirm(&mut app);
    assert_eq!(*asked.borrow(), 1);
    let path = written.borrow().last().unwrap().0.clone();
    assert!(path.starts_with("/tmp/picked-") && path.ends_with(".pdf"), "{path}");
    // Agents can export without the dialog.
    let r = app.run("file.export.pdf", json!({"path": "/tmp/direct.pdf", "range": "2"})).unwrap();
    assert_eq!(r["path"], "/tmp/direct.pdf");
    assert!(r["warnings"].is_array());
}

/// A placed image whose pixel data can't be decoded is left out of the PDF, and the status bar
/// shows the note raster exports give.
#[test]
fn an_image_that_couldnt_be_decoded_is_named_in_the_status() {
    let (mut app, written) = app();
    // A PNG whose header reads but whose pixel data is cut short.
    let mut png = vec![];
    {
        let __dyn: image::DynamicImage = (image::RgbaImage::from_pixel(64, 32, image::Rgba([200, 30, 30, 255]))).into();
        let (__w, __h) = (__dyn.width(), __dyn.height());
        let (__layout, __sample, __raw) = match __dyn {
            image::DynamicImage::ImageLuma8(b) => (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLumaA8(b) => (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgb8(b) => (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgba8(b) => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLuma16(b) => (
                craft_image_codecs::ChannelLayout::Gray,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageLumaA16(b) => (
                craft_image_codecs::ChannelLayout::GrayA,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgb16(b) => (
                craft_image_codecs::ChannelLayout::Rgb,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgba16(b) => (
                craft_image_codecs::ChannelLayout::Rgba,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            other => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, other.into_rgba8().into_raw()),
        };
        let __cimg = craft_image_codecs::Image::from_raw(__w, __h, __layout, __sample, __raw).unwrap();
        craft_image_codecs::encode(&__cimg, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default())
            .map(|__b| png.extend_from_slice(&__b))
    }
    .unwrap();
    // The image is placed on the first artboard.
    vectorcraft_testkit::fixtures::place_cut_png(&mut app.session, &png, json!({"at": [60, 50]}));
    app.run("ui.savePdfDialog", json!({"path": "/tmp/cut.pdf"})).unwrap();
    confirm(&mut app);
    assert_eq!(app.ui.status, "Saved /tmp/cut.pdf with 1 note: an image couldn't be decoded and is missing from the picture");
    assert_eq!(written.borrow().len(), 1);
}

#[test]
fn presets_reset_the_settings_and_fields_fall_back_to_defaults() {
    let (mut app, _) = app();
    app.run("ui.savePdfDialog", json!({"compatibility": "1.4", "marks": {"trim": true}})).unwrap();
    assert_eq!(fget(&app, "compatibility"), "1.4");
    assert_eq!(fget(&app, "marks.weight"), 0.25, "the rest of a section keeps its defaults");
    set_field(&app, "preset", json!(vectorcraft_pdf::default_preset_name(APP)));
    assert_eq!(fget(&app, "compatibility"), "1.7");
    assert_eq!(fget(&app, "marks.trim"), false);
    cancel(&mut app);
    assert!(app.run("ui.savePdfDialog", json!({"compatibility": "1.0"})).is_err(), "bad options are refused");
    assert!(!is_open(&app));
}

/// The bytes and status a save writes after `setup` and the given field writes.
fn written_with(setup: impl FnOnce(&mut VectorcraftApp), options: &[(&str, Value)]) -> (Vec<u8>, String) {
    let (mut app, written) = app();
    setup(&mut app);
    app.run("ui.savePdfDialog", json!({"path": "/tmp/option.pdf"})).unwrap();
    for (name, v) in options {
        set_field(&app, name, v.clone());
    }
    confirm(&mut app);
    let bytes = written.borrow().first().expect("a file was written").1.clone();
    (bytes, app.ui.status.clone())
}

/// One option of each group that changes the file reaches it: marks and bleed with the
/// non-printing layer, and the PDF layers.
#[test]
fn a_sections_option_reaches_the_pdf() {
    let (bytes, status) = written_with(
        |app| {
            app.run("document.setup", json!({"bleed": 9})).unwrap();
            let notes = app.run("layer.new", json!({})).unwrap()["id"].clone();
            app.run("layer.setProps", json!({"id": notes, "printable": false})).unwrap();
            app.run("shape.ellipse", json!({"x": 60, "y": 40, "width": 20, "height": 20})).unwrap();
        },
        &[("bleed.useDocument", json!(true)), ("marks.trim", json!(true)), ("includeNonPrinting", json!(true))],
    );
    assert!(!status.contains("note"), "marks and bleed are applied: {status}");
    let page = &vectorcraft_pdf::info(&bytes, None).unwrap().pages[0];
    let trim = page.boxes.iter().find(|(b, _)| *b == vectorcraft_pdf::CropTo::Trim).unwrap().1;
    assert_eq!((trim.width(), trim.x0), (120.0, 9.0 + 18.0 + 0.25), "the trim box is the artboard, inside the bleed and marks");
    let mut ellipse = false;
    vectorcraft_pdf::import(&bytes).unwrap().walk(|n| {
        let b = n.path_data().and_then(|p| p.bounds()).unwrap_or_default();
        ellipse |= (b.width() - 20.0).abs() < 0.1 && (b.height() - 20.0).abs() < 0.1;
    });
    assert!(ellipse, "the non-printing layer is included");

    let (bytes, status) = written_with(
        |app| {
            app.run("layer.new", json!({"name": "Notes"})).unwrap();
            app.run("shape.ellipse", json!({"x": 60, "y": 40, "width": 20, "height": 20})).unwrap();
        },
        &[("createLayers", json!(true))],
    );
    assert!(!status.contains("note"), "{status}");
    let names: Vec<_> = vectorcraft_pdf::import(&bytes).unwrap().layers.iter().filter_map(|l| l.name.clone()).collect();
    // The empty pages of the other artboards come in as page layers.
    assert_eq!(names[..2], ["Layer 1", "Notes"]);
}

/// Passwords given when opening reach the encrypted file and never a preset.
#[test]
fn confirming_preserves_passwords_supplied_when_opening_the_dialog() {
    for (open, owner) in [("open", ""), ("", "owner"), ("open", "owner")] {
        let (mut app, written) = app();
        app.run(
            "ui.savePdfDialog",
            json!({"path": "/tmp/locked.pdf", "security": {"openPassword": open, "permissionsPassword": owner, "printing": "none"}}),
        )
        .unwrap();
        assert!(!summary(&app).contains("Password"), "summaries omit passwords");
        // The preset row's New… opens its name field; OK saves the preset under the suggested name.
        press(&mut app, "New…");
        press(&mut app, "OK");
        let saved = &app.session.prefs.pdf_presets[0].settings.security;
        assert!(saved.open_password.is_empty() && saved.permissions_password.is_empty(), "presets forget passwords");

        confirm(&mut app);
        assert!(!is_open(&app));
        let written = written.borrow();
        let bytes = &written[0].1;
        assert!(String::from_utf8_lossy(bytes).contains("/Encrypt"));
        if open.is_empty() {
            assert_eq!(vectorcraft_pdf::info(bytes, None).unwrap().pages.len(), 3);
        } else {
            assert_eq!(vectorcraft_pdf::info(bytes, None).unwrap_err(), vectorcraft_pdf::PdfError::NeedsPassword);
        }
        assert_eq!(vectorcraft_pdf::info(bytes, Some("wrong")).unwrap_err(), vectorcraft_pdf::PdfError::WrongPassword);
        for password in [open, owner].into_iter().filter(|p| !p.is_empty()) {
            assert_eq!(vectorcraft_pdf::info(bytes, Some(password)).unwrap().pages.len(), 3);
        }
    }
}

#[test]
fn the_security_section_takes_passwords_that_presets_drop() {
    let (mut app, _) = app();
    app.run("ui.savePdfDialog", json!({})).unwrap();
    set_field(&app, "security.openPassword", json!("open"));
    set_field(&app, "security.permissionsPassword", json!("own"));
    set_field(&app, "security.copy", json!(false));
    let s = with_pdf(&app, |d| d.settings().clone());
    assert_eq!((s.security.open_password.as_str(), s.security.permissions_password.as_str()), ("open", "own"));
    assert_eq!(fget(&app, "security.copy"), false);
    set_field(&app, "standard", json!("pdfA2b"));
    set_field(&app, "preset", json!(vectorcraft_pdf::default_preset_name(APP)));
    let s = with_pdf(&app, |d| d.settings().clone());
    assert!(s.security.open_password.is_empty() && s.security.permissions_password.is_empty(), "presets carry no passwords");
    assert_eq!(fget(&app, "security.copy"), true);
}

#[test]
fn choosing_a_pdfx_standard_sets_its_version_and_writes_the_standard() {
    let (mut app, written) = app();
    app.run("ui.savePdfDialog", json!({"path": "/tmp/x.pdf", "createLayers": true})).unwrap();
    set_field(&app, "standard", json!("pdfX1a"));
    assert_eq!((fget(&app, "compatibility"), fget(&app, "createLayers"), fget(&app, "preserveEditing")), (json!("1.3"), json!(false), json!(false)));
    confirm(&mut app);
    let pdf = String::from_utf8_lossy(&written.borrow()[0].1).into_owned();
    assert!(pdf.starts_with("%PDF-1.3") && pdf.contains("/GTS_PDFXConformance(PDF/X-1a:2001)") && pdf.contains("/S/GTS_PDFX"));
    // PDF/X-4 keeps layers and is PDF 1.6.
    app.run("ui.savePdfDialog", json!({"compatibility": "2.0", "createLayers": true})).unwrap();
    set_field(&app, "standard", json!("pdfX4"));
    assert_eq!((fget(&app, "compatibility"), fget(&app, "createLayers")), (json!("1.6"), json!(true)));
}

#[test]
fn pdf_1_3_takes_a_flattener_preset_in_advanced() {
    let (mut app, written) = app();
    app.run("ui.savePdfDialog", json!({"path": "/tmp/old.pdf", "compatibility": "1.3"})).unwrap();
    set_field(&app, "flattenerPreset", json!("Low Resolution"));
    assert_eq!((fget(&app, "compatibility"), fget(&app, "flattenerPreset")), (json!("1.3"), json!("Low Resolution")));
    confirm(&mut app);
    assert!(written.borrow()[0].1.starts_with(b"%PDF-1.3"));
    // A preset keeps the flattener preset.
    app.run("pdf.preset.save", json!({"name": "Old", "compatibility": "1.3", "flattenerPreset": "Low Resolution"})).unwrap();
    let saved = app.session.prefs.pdf_presets.iter().find(|p| p.name == "Old").unwrap();
    assert_eq!((saved.settings.compatibility, saved.settings.flattener_preset.as_str()), (vectorcraft_pdf::Compatibility::Pdf13, "Low Resolution"));
}

#[test]
fn save_pdf_flattens_transparent_document_without_modal_prompt() {
    let (mut app, written) = app();
    // Add transparent artwork to the document
    app.run("transparency.set", json!({"opacity": 50})).unwrap();

    app.run("ui.savePdfDialog", json!({"path": "/tmp/transparent.pdf", "compatibility": "1.3"})).unwrap();
    assert!(is_open(&app));

    // Clicking Save PDF goes ahead without stopping at a confirmation prompt
    confirm(&mut app);
    assert!(!is_open(&app), "Save PDF closes without a modal prompt");
    assert_eq!(written.borrow().len(), 1);
    assert!(written.borrow()[0].1.starts_with(b"%PDF-1.3"));
}

#[test]
fn summary_warnings_include_flattener_resolutions_when_flattening_transparency() {
    let (mut app, _) = app();
    app.run("transparency.set", json!({"opacity": 50})).unwrap();

    app.run("ui.savePdfDialog", json!({"path": "/tmp/out.pdf", "compatibility": "1.3"})).unwrap();
    pump(&mut app);
    let text = summary(&app);
    assert!(text.contains("1200") && text.contains("300"), "{text}");

    // With a custom flattener override, the resolution in the warning reflects the options
    dialog_set(&mut app, "flattener.lineArtPpi", json!(600));
    dialog_set(&mut app, "flattener.gradientPpi", json!(200));
    pump(&mut app);
    let text = summary(&app);
    assert!(text.contains("600") && text.contains("200"), "{text}");

    // An opaque document is not flattened, so there is nothing to warn about.
    let (mut plain, _) = app_without_transparency();
    plain.run("ui.savePdfDialog", json!({"path": "/tmp/out.pdf", "compatibility": "1.3"})).unwrap();
    pump(&mut plain);
    assert!(!summary(&plain).contains("1200"));
}

/// The default app: nothing in it is transparent.
fn app_without_transparency() -> (VectorcraftApp, Log) {
    app()
}

/// A second rectangle, half transparent, over the first one.
fn transparent_overlap(app: &mut VectorcraftApp) {
    app.run("select.none", json!({})).unwrap();
    app.run("shape.rectangle", json!({"x": 30, "y": 20, "width": 50, "height": 40})).unwrap();
    app.run("transparency.set", json!({"opacity": 50})).unwrap();
}

/// The images a written PDF reopens with.
fn images(pdf: &[u8]) -> usize {
    let doc = vectorcraft_pdf::import(pdf).unwrap();
    let mut n = 0;
    for l in &doc.layers {
        l.walk(&mut |x| n += matches!(x.kind, vectorcraft_doc::NodeKind::Image(_)) as usize);
    }
    n
}

/// Save PDF on `params` (and `path`), showing Advanced.
fn open_advanced(app: &mut VectorcraftApp, params: Value) {
    let mut p = params;
    p["path"] = json!("/tmp/out.pdf");
    app.run("ui.savePdfDialog", p).unwrap();
    set_field(app, "section", json!("advanced"));
}

/// Press Custom… and return the dialog it opens over Save PDF.
fn open_custom(app: &mut VectorcraftApp) -> DialogId {
    press(app, "Custom…");
    dialog_id(app, false).expect("the Custom… dialog opens")
}

/// A number a dialog shows, whichever width it is stored in.
fn number(v: &Value) -> Option<f64> {
    v.as_f64()
}

#[test]
fn custom_starts_from_the_preset_and_cancel_goes_back_unchanged() {
    let (mut app, _) = app();
    open_advanced(&mut app, json!({"compatibility": "1.3", "flattenerPreset": "low"}));
    let before = fields(&app);
    let child = open_custom(&mut app);
    let low = serde_json::to_value(FlattenOptions::preset("Low Resolution").unwrap()).unwrap();
    let shown = fields_of(&app, child);
    assert_eq!(number(&shown["balance"]), number(&low["balance"]), "Low Resolution's");
    assert_eq!(number(&shown["gradientPpi"]), number(&low["gradientPpi"]));
    // Low Resolution doesn't clip, but the PDF 1.3 writer does.
    assert_eq!(shown["clipComplexRegions"], json!(true), "shown as written");
    set_at(&app, child, "balance", json!(10)).unwrap();
    cancel_id(&mut app, child);
    assert!(is_open(&app) && dialog_id(&app, false).is_none());
    assert_eq!(fields(&app), before, "Cancel leaves Save PDF as it was");
}

#[test]
fn custom_options_are_written_until_a_preset_is_chosen() {
    let (mut app, written) = app();
    transparent_overlap(&mut app);
    // Choosing a preset drops the custom options.
    open_advanced(&mut app, json!({"compatibility": "1.3"}));
    let child = open_custom(&mut app);
    set_at(&app, child, "balance", json!(0)).unwrap();
    confirm_id(&mut app, child);
    assert!(is_open(&app) && dialog_id(&app, false).is_none(), "OK goes back to Save PDF");
    assert_eq!(job_flattener(&app).map(|o| number(&o["balance"])), Some(Some(0.0)));
    set_field(&app, "flattenerPreset", json!("Low Resolution"));
    pump(&mut app);
    assert_eq!(job_flattener(&app), None);
    cancel(&mut app);
    // Until then they are written.
    open_advanced(&mut app, json!({"compatibility": "1.3"}));
    let child = open_custom(&mut app);
    set_at(&app, child, "balance", json!(0)).unwrap();
    confirm_id(&mut app, child);
    confirm(&mut app);
    assert!(images(&written.borrow()[0].1) > 0, "balance 0 rasterizes the flattened art");
    open_advanced(&mut app, json!({"compatibility": "1.3"}));
    confirm(&mut app);
    assert_eq!(images(&written.borrow()[1].1), 0, "High Resolution keeps it vector");
}

/// What OK of Custom… leaves on Save PDF: options equal to the preset's leave its name, options
/// that drop overprints stay dropped, and alpha shows as the PDF 1.3 writer drops it.
#[test]
fn custom_confirm_rules() {
    let (mut app, _) = app();
    for overprint in ["preserve", "discard"] {
        open_advanced(&mut app, json!({"compatibility": "1.3", "flattenerPreset": "medium", "advanced": {"overprint": overprint}}));
        let child = open_custom(&mut app);
        assert_eq!(fields_of(&app, child)["preserveOverprints"], json!(overprint == "preserve"), "{overprint}");
        confirm_id(&mut app, child);
        assert!(is_open(&app), "{overprint}");
        assert_eq!(job_flattener(&app), None, "{overprint}");
        assert_eq!(fget(&app, "flattenerPreset"), "Medium Resolution", "{overprint}");
        cancel(&mut app);
    }

    let params = json!({"compatibility": "1.3", "advanced": {"overprint": "preserve"}, "flattener": {"preserveOverprints": false}});
    open_advanced(&mut app, params);
    let child = open_custom(&mut app);
    assert_eq!(fields_of(&app, child)["preserveOverprints"], json!(false), "shown as written");
    confirm_id(&mut app, child);
    assert_eq!(job_flattener(&app).unwrap()["preserveOverprints"], json!(false));
    cancel(&mut app);

    let (mut app, written) = app_with_overlap();
    open_advanced(&mut app, json!({"compatibility": "1.3", "flattener": {"balance": 0, "preserveAlpha": true}}));
    let child = open_custom(&mut app);
    assert_eq!(fields_of(&app, child)["preserveAlpha"], json!(false), "shown as written");
    confirm_id(&mut app, child);
    assert_eq!(job_flattener(&app).unwrap()["preserveAlpha"], json!(false));
    confirm(&mut app);
    assert!(written.borrow()[0].1.starts_with(b"%PDF-1.3") && images(&written.borrow()[0].1) > 0);
}

fn app_with_overlap() -> (VectorcraftApp, Log) {
    let mut t = app();
    transparent_overlap(&mut t.0);
    t
}

#[test]
fn bad_custom_options_keep_the_dialog_open() {
    let (mut app, _) = app();
    open_advanced(&mut app, json!({"compatibility": "1.3"}));
    let before = fields(&app);
    let child = open_custom(&mut app);
    // Refused on the write, or answered with an error on OK: either way the dialog stays.
    if set_at(&app, child, "gradientPpi", json!(9000)).is_ok() {
        confirm_id(&mut app, child);
    }
    assert_eq!(dialog_id(&app, false), Some(child), "Custom… stays open");
    assert_eq!(fields(&app), before, "Save PDF's options are unchanged");
}

/// Custom… keeps what it opened with when its saved preset is deleted (or made again) while it is
/// open.
#[test]
fn a_deleted_saved_preset_keeps_what_custom_opened_with() {
    let (mut app, written) = app();
    app.run("flattener.presets.save", json!({"name": "Mine", "balance": 20})).unwrap();
    open_advanced(&mut app, json!({"compatibility": "1.3", "flattenerPreset": "Mine"}));
    let child = open_custom(&mut app);
    assert_eq!(number(&fields_of(&app, child)["balance"]), Some(20.0), "the saved preset's");
    app.run("flattener.presets.delete", json!({"name": "Mine"})).unwrap();
    pump(&mut app);
    set_at(&app, child, "balance", json!(30)).unwrap();
    confirm_id(&mut app, child);
    assert_eq!((fget(&app, "flattenerPreset"), number(&job_flattener(&app).unwrap()["balance"])), (json!(""), Some(30.0)));
    confirm(&mut app);
    assert!(written.borrow()[0].1.starts_with(b"%PDF-1.3"));

    // Whichever value Overprint shows it with, although the default preset keeps overprints.
    for (kept, overprint) in [(false, "preserve"), (true, "discard")] {
        app.run("flattener.presets.save", json!({"name": "Mine", "balance": 20, "preserveOverprints": kept})).unwrap();
        open_advanced(&mut app, json!({"compatibility": "1.3", "flattenerPreset": "Mine", "advanced": {"overprint": overprint}}));
        let child = open_custom(&mut app);
        assert_eq!(fields_of(&app, child)["preserveOverprints"], json!(false), "shown as written ({overprint})");
        app.run("flattener.presets.delete", json!({"name": "Mine"})).unwrap();
        pump(&mut app);
        confirm_id(&mut app, child);
        assert_eq!(job_flattener(&app).unwrap()["preserveOverprints"], json!(kept), "{overprint}");
        cancel(&mut app);
    }

    // Deleted and made again under the same name with the other overprint value.
    app.run("flattener.presets.save", json!({"name": "Mine", "balance": 20, "preserveOverprints": false})).unwrap();
    open_advanced(&mut app, json!({"compatibility": "1.3", "flattenerPreset": "Mine", "advanced": {"overprint": "preserve"}}));
    let child = open_custom(&mut app);
    app.run("flattener.presets.delete", json!({"name": "Mine"})).unwrap();
    app.run("flattener.presets.save", json!({"name": "Mine", "balance": 20, "preserveOverprints": true})).unwrap();
    pump(&mut app);
    confirm_id(&mut app, child);
    assert_eq!(job_flattener(&app).unwrap()["preserveOverprints"], json!(false), "the option it opened with wins");
}

/// A flattener preset named in another case opens as the listed name, not as [Custom].
#[test]
fn flattener_preset_shows_a_name_given_in_any_case() {
    let (mut app, _) = app();
    app.run("flattener.presets.save", json!({"name": "Mine", "balance": 20})).unwrap();
    for (given, shown) in [("MINE", "Mine"), ("HIGH RESOLUTION", "High Resolution")] {
        open_advanced(&mut app, json!({"compatibility": "1.3", "flattenerPreset": given}));
        assert_eq!(fget(&app, "flattenerPreset"), shown, "{given}");
        assert_eq!(job_flattener(&app), None, "{given}: not custom");
        cancel(&mut app);
    }
}

#[test]
fn save_pdf_opens_with_the_flattener_options_given_and_refuses_bad_ones() {
    let (mut app, _) = app();
    app.run("ui.savePdfDialog", json!({"compatibility": "1.3", "flattener": {"lineArtPpi": 600}})).unwrap();
    assert_eq!(job_flattener(&app).map(|o| number(&o["lineArtPpi"])), Some(Some(600.0)));
    cancel(&mut app);
    for bad in [json!({"bogus": 1}), json!(5), json!({"gradientPpi": 9000})] {
        assert!(app.run("ui.savePdfDialog", json!({"flattener": bad})).is_err(), "{bad}");
        assert!(!is_open(&app), "{bad}");
    }
}

/// The number and length fields of the sections that name them: the page range, the bleed sides and the font
/// subset threshold.
#[test]
fn the_range_bleed_and_subset_fields_are_named() {
    let (mut app, _) = app();
    app.run("file.export.pdf", json!({})).unwrap();
    let sections: [(&str, &[&str]); 3] =
        [("general", &["Range"]), ("marks", &["Offset", "Top", "Bottom", "Left", "Right"]), ("advanced", &["Subset fonts below"])];
    for (section, expected) in sections {
        set_field(&app, "section", json!(section));
        pump(&mut app);
        let ctx = app.ui_ctx.clone().expect("a UI context");
        let out = crate::testing::app_frame(&mut app, &ctx, Default::default());
        let names = super::super::tests_print::field_names(&out);
        for name in expected {
            assert!(names.iter().any(|n| n == name), "{section}: {name} in {names:?}");
        }
    }
}
