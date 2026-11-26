//! File → Print: the Print dialog hosted on the window (Print and Done, the preview's drag), the
//! print service it prints through (a mock here) and the PDF it saves without one.

use std::cell::RefCell;
use std::rc::Rc;

use craft::print::ui::Printer;
use serde_json::{Value, json};
use vectorcraft_engine::Session;

use crate::print::{PrintJob, PrintService};
use crate::{Services, VectorcraftApp};

/// What the mock service was sent: (printer, title, PDF).
type Jobs = Rc<RefCell<Vec<(Option<String>, String, Vec<u8>)>>>;

/// What the app wrote: (path, bytes).
type Written = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

struct MockPrint(Jobs);

impl PrintService for MockPrint {
    fn printers(&mut self) -> Vec<Printer> {
        vec![Printer::new("Office"), Printer::new("Proofer").is_default(true)]
    }

    fn print(&mut self, job: &PrintJob) -> Result<String, String> {
        self.0.borrow_mut().push((job.printer.map(str::to_string), job.title.to_string(), job.pdf.to_vec()));
        Ok(format!("Sent to {}", job.printer.unwrap_or("the default printer")))
    }
}

/// An app with three 120 × 90 artboards and a rectangle, printing through the mock.
fn app() -> (VectorcraftApp, Jobs) {
    let jobs = Jobs::default();
    let services = Services { print: Some(Box::new(MockPrint(jobs.clone()))), ..Default::default() };
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), services));
    app.run("file.new", json!({"width": 120, "height": 90, "artboards": 3})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    (app, jobs)
}

/// The app's egui context, over which its window's dialogs run.
fn ctx(app: &VectorcraftApp) -> egui::Context {
    app.ui_ctx.clone().expect("the test app has a context")
}

/// The topmost open dialog: its id and its json fields.
fn top(app: &VectorcraftApp) -> Option<(egui::Id, Value)> {
    app.dialogs()?.with(|host| host.open_dialogs().pop()).map(|d| (d.id, d.fields.unwrap_or_default()))
}

/// Whether a dialog is open.
pub(super) fn is_open(app: &VectorcraftApp) -> bool {
    top(app).is_some()
}

/// The json fields of the topmost open dialog.
pub(super) fn fields(app: &VectorcraftApp) -> Value {
    top(app).expect("a dialog is open").1
}

/// Sets a field of the topmost open dialog as the user editing it would; the dialog's refusal is the error.
pub(super) fn try_set(app: &mut VectorcraftApp, name: &str, value: Value) -> Result<(), String> {
    let ctx = ctx(app);
    let (id, _) = top(app).expect("a dialog is open");
    app.dialogs().expect("a window host").with(|host| host.set_json_field(&ctx, id, name, value))
}

/// Sets a field that the dialog accepts.
pub(super) fn set(app: &mut VectorcraftApp, name: &str, value: Value) {
    try_set(app, name, value).unwrap();
}

/// One frame of the window: the host shows the dialogs, and the frame's dialog work brings their answers to the app.
pub(super) fn pump(app: &mut VectorcraftApp) {
    let ctx = ctx(app);
    crate::testing::app_frame(app, &ctx, Default::default());
}

/// Presses the topmost dialog's default button (Print, or Done when `choice` says so), then runs its answer.
pub(super) fn confirm(app: &mut VectorcraftApp) {
    let ctx = ctx(app);
    let (id, _) = top(app).expect("a dialog is open");
    // A refused answer is read from the dialog staying open.
    let _ = app.dialogs().expect("a window host").with(|host| host.confirm_by_id(&ctx, id));
    // The held OK is answered in the first frame and the dialog closes in the second.
    pump(app);
    pump(app);
}

/// One control request, answered as the control channel answers it: at once, or, for synthetic input (`ui.text`,
/// `ui.key`), once the frames after it have processed the input.
pub(super) fn call_raw(app: &mut VectorcraftApp, method: &str, params: Value) -> Value {
    let ctx = ctx(app);
    let (req, _reply) = craft::automation::ControlRequest::new(method, params);
    let reply = match crate::control::handle(app, &ctx, &req) {
        craft::automation::Reply::Done(reply) => reply,
        craft::automation::Reply::AfterInput => {
            for _ in 0..2 {
                crate::testing::app_frame(app, &ctx, Default::default());
            }
            json!({"ok": true, "result": null})
        }
        _ => panic!("{method} is answered at once or after its input"),
    };
    pump(app);
    reply
}

/// A control request that must succeed; its result.
pub(super) fn call(app: &mut VectorcraftApp, method: &str, params: Value) -> Value {
    let r = call_raw(app, method, params);
    assert_eq!(r["ok"], true, "{method}: {r}");
    r["result"].clone()
}

/// Presses the control named `name` in a frame drawn first, as a user presses what the window shows, then runs the
/// frame's dialog work.
pub(super) fn press(app: &mut VectorcraftApp, name: &str) {
    press_control(app, json!({ "name": name }));
}

/// [`press`] of the control `control` names (`{name, role?, within?, index?}`, as `ui.click` takes it).
pub(super) fn press_control(app: &mut VectorcraftApp, control: Value) {
    let ctx = ctx(app);
    crate::testing::app_frame(app, &ctx, Default::default());
    let name = control["name"].clone();
    let (click, _reply) = craft::automation::ControlRequest::new("ui.click", json!({ "control": control }));
    match crate::control::handle(app, &ctx, &click) {
        craft::automation::Reply::Done(r) => assert_eq!(r["ok"], true, "ui.click {name}: {r}"),
        // The click is queued as input: the frames after it press the control.
        _ => {
            for _ in 0..2 {
                crate::testing::app_frame(app, &ctx, Default::default());
            }
        }
    }
    pump(app);
}

fn pages(pdf: &[u8]) -> usize {
    vectorcraft_pdf::info(pdf, None).unwrap().pages.len()
}

#[test]
fn print_sends_every_page_to_the_chosen_printer_and_keeps_the_settings() {
    let (mut app, jobs) = app();
    app.run("file.print", json!({})).unwrap();
    pump(&mut app);
    assert_eq!(fields(&app)["printer"], "Proofer", "the system's default printer");
    set(&mut app, "printer", json!("Office"));
    set(&mut app, "copies", json!(2));
    set(&mut app, "marks.trim", json!(true));
    let undo = app.session.active().unwrap().history.undo.len();
    confirm(&mut app);
    assert!(!is_open(&app), "Print closes the dialog");
    let jobs = jobs.borrow();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].0.as_deref(), Some("Office"));
    assert_eq!(pages(&jobs[0].2), 6, "3 artboards × 2 copies");
    assert_eq!(app.ui.status, "Sent to Office");
    // The settings are kept with the document as one undo step.
    let st = app.session.active().unwrap();
    assert_eq!(st.history.undo.len(), undo + 1);
    let saved = st.doc.print_setup.as_ref().unwrap();
    assert_eq!((saved["copies"].clone(), saved["marks"]["trim"].clone(), saved["marks"]["weight"].clone()), (json!(2), json!(true), json!(0.25)));
    // The next Print starts from them.
    app.run("file.print", json!({})).unwrap();
    pump(&mut app);
    assert_eq!(fields(&app)["copies"], 2);
}

#[test]
fn done_keeps_the_settings_without_printing_and_bad_settings_keep_the_dialog() {
    let (mut app, jobs) = app();
    app.run("file.print", json!({})).unwrap();
    pump(&mut app);
    set(&mut app, "artboards", json!("range"));
    set(&mut app, "range", json!("7"));
    // Artboard 7 doesn't exist: Print and Done are both refused.
    confirm(&mut app);
    assert!(is_open(&app), "Print refused");
    set(&mut app, "choice", json!("done"));
    confirm(&mut app);
    assert!(is_open(&app), "Done refused");
    assert!(app.session.active().unwrap().doc.print_setup.is_none(), "nothing kept");
    assert_ne!(fields(&app)["choice"], "done", "Done's press doesn't stick to the dialog");
    set(&mut app, "range", json!("2-3"));
    set(&mut app, "choice", json!("done"));
    confirm(&mut app);
    assert!(!is_open(&app));
    assert!(jobs.borrow().is_empty(), "Done doesn't print");
    let saved = app.session.active().unwrap().doc.print_setup.clone().unwrap();
    assert_eq!((saved["artboards"].clone(), saved["range"].clone()), (json!("range"), json!("2-3")));
    // Agents print without the dialog; the default printer when none is named.
    let r = app.run("file.print", json!({"settings": {"copies": 3}})).unwrap();
    assert_eq!(r["pages"], 6);
    assert_eq!(jobs.borrow()[0].0, None);
    assert_eq!(pages(&jobs.borrow()[0].2), 6, "artboards 2-3 × 3 copies");
    let p = app.run("print.printers", json!({})).unwrap();
    assert_eq!(
        (p["printers"][1]["name"].clone(), p["printers"][1]["default"].clone(), p["service"].clone()),
        (json!("Proofer"), json!(true), json!(true))
    );
    assert!(!crate::menus::enabled(&app, "print.printerSetup"), "the mock has no settings to open");
}

#[test]
fn print_does_not_send_or_update_the_other_drawing() {
    let (mut app, jobs) = app();
    app.run("file.new", json!({"width": 180, "height": 100, "name": "Other drawing"})).unwrap();
    app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 50, "height": 30})).unwrap();
    let activate = |app: &mut VectorcraftApp, index| app.run("document.activate", json!({"index": index})).unwrap();
    let undo = app.session.active().unwrap().history.undo.len();

    activate(&mut app, 0);
    app.run("file.print", json!({})).unwrap();
    pump(&mut app);
    set(&mut app, "copies", json!(2));
    activate(&mut app, 1);
    confirm(&mut app);

    assert!(jobs.borrow().is_empty());
    let other = app.session.active().unwrap();
    assert!(other.doc.print_setup.is_none());
    assert_eq!(other.history.undo.len(), undo);
    assert!(is_open(&app));
    assert_eq!(fields(&app)["copies"], 2);
    activate(&mut app, 0);
    confirm(&mut app);
    assert_eq!(jobs.borrow().len(), 1);
    assert_eq!(pages(&jobs.borrow()[0].2), 6);
    assert!(!is_open(&app));
}

/// A print queue that puts each page on the paper as it is, as `lp` does.
struct UprightQueue(Jobs);

impl PrintService for UprightQueue {
    fn printers(&mut self) -> Vec<Printer> {
        vec![]
    }

    fn print(&mut self, job: &PrintJob) -> Result<String, String> {
        MockPrint(self.0.clone()).print(job)
    }

    fn turns_landscape(&self) -> bool {
        false
    }
}

/// The printers whose settings the setup service was asked to open.
type Setups = Rc<RefCell<Vec<Option<String>>>>;

/// A print service that opens printer settings, recording each printer it was asked for.
struct SetupPrint(Jobs, Setups);

impl PrintService for SetupPrint {
    fn printers(&mut self) -> Vec<Printer> {
        MockPrint(self.0.clone()).printers()
    }

    fn print(&mut self, job: &PrintJob) -> Result<String, String> {
        MockPrint(self.0.clone()).print(job)
    }

    fn has_setup(&self) -> bool {
        true
    }

    fn setup(&mut self, printer: Option<&str>) -> Result<(), String> {
        self.1.borrow_mut().push(printer.map(str::to_string));
        Ok(())
    }
}

#[test]
fn setup_opens_the_settings_of_the_printer_chosen() {
    let setups = Setups::default();
    let services = Services { print: Some(Box::new(SetupPrint(Jobs::default(), setups.clone()))), ..Default::default() };
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), services));
    app.run("file.new", json!({"width": 120, "height": 90})).unwrap();
    app.run("file.print", json!({})).unwrap();
    pump(&mut app);
    press(&mut app, "Setup…");
    set(&mut app, "printer", json!("Office"));
    press(&mut app, "Setup…");
    assert_eq!(*setups.borrow(), [Some("Proofer".to_string()), Some("Office".to_string())], "the default printer, then the one chosen");
    // PDF File has no printer settings.
    set(&mut app, "printer", json!("PDF File"));
    // The frame after the change draws the row with PDF File's controls.
    pump(&mut app);
    pump(&mut app);
    let r = call_raw(&mut app, "ui.click", json!({"control": {"name": "Setup…"}}));
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(setups.borrow().len(), 2);
    // A service without printer settings shows no Setup….
    let (mut app, _) = self::app();
    app.run("file.print", json!({})).unwrap();
    pump(&mut app);
    let r = call_raw(&mut app, "ui.click", json!({"control": {"name": "Setup…"}}));
    assert_eq!(r["ok"], false, "{r}");
}

#[test]
fn a_landscape_job_goes_upright_to_a_queue_that_does_not_turn_it() {
    let sizes = |pdf: &[u8]| vectorcraft_pdf::info(pdf, None).unwrap().pages.iter().map(|p| (p.width, p.height)).collect::<Vec<_>>();
    let landscape = json!({"settings": {"autoRotate": false, "orientation": "landscape", "artboards": "range", "range": "1"}});
    // The desktop's queue: Letter upright, the art turned on it.
    let jobs = Jobs::default();
    let services = Services { print: Some(Box::new(UprightQueue(jobs.clone()))), ..Default::default() };
    let mut desktop = VectorcraftApp::new(Session::new(), services);
    desktop.run("file.new", json!({"width": 120, "height": 90})).unwrap();
    desktop.run("file.print", landscape.clone()).unwrap();
    assert_eq!(sizes(&jobs.borrow()[0].2), [(612.0, 792.0)]);
    // A service that turns pages itself (the browser's dialog) gets the landscape page.
    let (mut app, jobs) = app();
    app.run("file.print", landscape).unwrap();
    assert_eq!(sizes(&jobs.borrow()[0].2), [(792.0, 612.0)]);
}

#[test]
fn without_a_print_service_print_saves_a_pdf() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Services::default()));
    let picker = crate::tests_picks::scripted(&mut app);
    let written: Written = Rc::default();
    let w = written.clone();
    app.services.write = Some(Box::new(move |p: &str, b: &[u8]| {
        w.borrow_mut().push((p.to_string(), b.to_vec()));
        Ok(())
    }));
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    app.run("file.print", json!({})).unwrap();
    pump(&mut app);
    confirm(&mut app);
    picker.answer_saved("/tmp/Untitled-1.pdf", None);
    let ctx = ctx(&app);
    crate::tests_picks::poll(&mut app, &ctx);
    let w = written.borrow();
    assert_eq!(w.len(), 1);
    assert!(w[0].0.ends_with(".pdf"), "{}", w[0].0);
    assert_eq!(pages(&w[0].1), 1);
    assert!(app.ui.status.starts_with("Saved /tmp/"), "{}", app.ui.status);
    // Printing straight to a path writes it through the app too.
    drop(w);
    let r = app.run("file.print", json!({"path": "/tmp/job.pdf", "settings": {"copies": 2}})).unwrap();
    assert_eq!((r["path"].clone(), r["pages"].clone()), (json!("/tmp/job.pdf"), json!(2)));
    assert_eq!(written.borrow()[1].0, "/tmp/job.pdf");
    assert!(app.run("print.printerSetup", json!({})).is_err());
    assert_eq!(app.run("print.printers", json!({})).unwrap(), json!({"printers": [], "service": false, "setup": false}));
}

#[test]
fn copies_per_sheet_prints_a_grid_of_each_artboard() {
    let (mut app, jobs) = app();
    app.run("file.print", json!({})).unwrap();
    pump(&mut app);
    set(&mut app, "copiesPerSheet", json!(true));
    set(&mut app, "gutter", json!(9));
    confirm(&mut app);
    assert_eq!(pages(&jobs.borrow()[0].2), 3, "a sheet per artboard");
    let saved = app.session.active().unwrap().doc.print_setup.clone().unwrap();
    assert_eq!((saved["copiesPerSheet"].clone(), saved["gutter"].clone()), (json!(true), json!(9.0)));
    // Each 120 × 90 artboard fills landscape Letter 6 across and 6 down, a gutter apart.
    let pv = app.session.execute("print.preview", &json!({})).unwrap();
    let copies = &pv["sheets"][0]["copies"];
    assert_eq!(copies.as_array().map(Vec::len), Some(36));
    let (first, second) = (copies[0][2].as_f64().unwrap(), copies[1][0].as_f64().unwrap());
    assert!((second - first - 9.0).abs() < 0.02, "{first} then {second}");
}

#[test]
fn dragging_the_preview_moves_the_placement_on_the_paper() {
    let (mut app, _) = app();
    // The page as the preview shows it, for each way the paper can be turned, and the Print Tiling tool's pages.
    for (settings, (dx, dy), tolerance) in [
        (json!({}), (10.0, 5.0), 0.02),
        (json!({"autoRotate": false, "orientation": "portraitFlipped"}), (10.0, 5.0), 0.02),
        (json!({"output": {"emulsion": "down"}, "transverse": true}), (-4.0, 7.0), 0.02),
        (json!({"scaling": "custom", "scale": {"width": 50, "height": 200}}), (6.0, -8.0), 0.02),
        // Copies per sheet: the drag moves the whole grid.
        (json!({"copiesPerSheet": true, "gutter": 9}), (10.0, 5.0), 0.02),
        (json!({"tileOrigin": {"placed": true, "x": -20, "y": -10}}), (10.0, 5.0), 0.05),
        (
            json!({"transverse": true, "scaling": "custom", "scale": {"width": 50, "height": 200}, "tileOrigin": {"placed": true, "x": -20, "y": -10}}),
            (10.0, 5.0),
            0.05,
        ),
    ] {
        let before = app.session.execute("print.preview", &json!({ "settings": settings })).unwrap();
        let (dx, dy) = (dx, dy);
        let mut dialog_fields = super::print::fields_of(&serde_json::from_value(settings.clone()).unwrap()).unwrap();
        let placement = super::print::settings(&dialog_fields)["placement"].clone();
        super::print::move_placement(&mut dialog_fields, &before["sheets"][0], (dx, dy));
        let moved = super::print::settings(&dialog_fields);
        if settings.get("tileOrigin").is_some() {
            assert_eq!(moved["placement"], placement, "the placement stays");
        }
        let after = app.session.execute("print.preview", &json!({ "settings": moved })).unwrap();
        let (t0, t1) = (&before["sheets"][0]["trim"], &after["sheets"][0]["trim"]);
        let delta = (t1[0].as_f64().unwrap() - t0[0].as_f64().unwrap(), t1[1].as_f64().unwrap() - t0[1].as_f64().unwrap());
        assert!((delta.0 - dx).abs() < tolerance && (delta.1 - dy).abs() < tolerance, "{settings}: moved {delta:?}, dragged {:?}", (dx, dy));
    }
}

#[test]
fn the_control_channel_fills_and_confirms_the_dialog() {
    let (mut app, jobs) = app();
    call(&mut app, "engine.execute", json!({"command": "file.print"}));
    call(&mut app, "ui.dialog.set", json!({"field": "copies", "value": 3}));
    call(&mut app, "ui.dialog.set", json!({"field": "artboards", "value": "ignore"}));
    call(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(pages(&jobs.borrow()[0].2), 3);
    // Cancel keeps nothing.
    call(&mut app, "engine.execute", json!({"command": "file.print"}));
    call(&mut app, "ui.dialog.set", json!({"field": "copies", "value": 4}));
    call(&mut app, "ui.dialog.cancel", json!({}));
    assert_eq!(app.session.active().unwrap().doc.print_setup.as_ref().unwrap()["copies"], 3, "Cancel keeps nothing");
    // Done through the channel.
    call(&mut app, "engine.execute", json!({"command": "file.print"}));
    call(&mut app, "ui.dialog.set", json!({"field": "choice", "value": "done"}));
    call(&mut app, "ui.dialog.set", json!({"field": "copies", "value": 5}));
    call(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(app.session.active().unwrap().doc.print_setup.as_ref().unwrap()["copies"], 5);
    assert_eq!(jobs.borrow().len(), 1);
}

/// The Summary section's Pages, Options and Warnings are each named in the window's accessibility tree, with their
/// summary text as the value.
#[test]
fn the_summary_names_its_pages_options_and_warnings() {
    let (mut app, _) = app();
    app.run("file.print", json!({})).unwrap();
    pump(&mut app);
    set(&mut app, "section", json!("Summary"));
    pump(&mut app);
    let ctx = ctx(&app);
    let out = crate::testing::app_frame(&mut app, &ctx, Default::default());
    let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
    for name in ["Pages", "Options", "Warnings"] {
        assert!(nodes.iter().any(|(_, n)| n.label() == Some(name) && n.value().is_some_and(|v| !v.is_empty())), "{name} has no summary");
    }
}

/// Scale W and H of the open dialog, and its link.
fn scale(app: &VectorcraftApp) -> (f64, f64, bool) {
    let f = fields(app);
    let s = &f["scale"];
    (s["width"].as_f64().unwrap_or(f64::NAN), s["height"].as_f64().unwrap_or(f64::NAN), s["linked"] == true)
}

#[test]
fn the_scale_link_keeps_scale_width_and_height_equal() {
    let (mut app, _) = app();
    app.run("file.print", json!({})).unwrap();
    pump(&mut app);
    assert_eq!(scale(&app), (100.0, 100.0, true), "on for a new job");
    set(&mut app, "scaling", json!("custom"));
    // Linked, a new W sets H, and a new H sets W.
    set(&mut app, "scale", json!({"width": 40}));
    pump(&mut app);
    assert_eq!(scale(&app), (40.0, 40.0, true));
    set(&mut app, "scale", json!({"height": 250}));
    pump(&mut app);
    assert_eq!(scale(&app), (250.0, 250.0, true));
    // Unlinked, each field changes alone.
    set(&mut app, "scale", json!({"width": 50, "height": 250, "linked": false}));
    pump(&mut app);
    set(&mut app, "scale", json!({"width": 60}));
    pump(&mut app);
    assert_eq!(scale(&app), (60.0, 250.0, false));
    set(&mut app, "scale", json!({"height": 75}));
    pump(&mut app);
    assert_eq!(scale(&app), (60.0, 75.0, false));
    // Done keeps the link with the document's settings, and the next Print opens with it.
    set(&mut app, "choice", json!("done"));
    confirm(&mut app);
    let saved = app.session.active().unwrap().doc.print_setup.clone().unwrap();
    assert_eq!(saved["scale"], json!({"width": 60.0, "height": 75.0, "linked": false}));
    app.run("file.print", json!({})).unwrap();
    pump(&mut app);
    assert_eq!(scale(&app), (60.0, 75.0, false));
}

#[test]
fn a_print_preset_keeps_the_scale_link() {
    let (mut app, _) = app();
    app.run("print.presets.save", json!({"name": "Apart", "settings": {"scaling": "custom", "scale": {"width": 30, "height": 90}}})).unwrap();
    app.run("file.print", json!({})).unwrap();
    pump(&mut app);
    assert_eq!(scale(&app), (100.0, 100.0, true));
    // Choosing the preset loads its unlinked W and H as they are.
    set(&mut app, "preset", json!("Apart"));
    pump(&mut app);
    assert_eq!(scale(&app), (30.0, 90.0, false));
    // A scale set without the link and with W and H apart turns it off, as print.setup reads it.
    set(&mut app, "preset", json!("[Default]"));
    pump(&mut app);
    assert_eq!(scale(&app), (100.0, 100.0, true));
    // Scale takes input only while Scaling is Custom or tiles.
    set(&mut app, "scaling", json!("custom"));
    set(&mut app, "scale", json!({"width": 20, "height": 60}));
    pump(&mut app);
    assert_eq!(scale(&app), (20.0, 60.0, false));
    // `linked: null` keeps the link as it was, off here.
    set(&mut app, "scale", json!({"width": 30, "height": 70, "linked": null}));
    pump(&mut app);
    assert_eq!(fields(&app)["scale"], json!({"width": 30, "height": 70, "linked": false}));
}

#[test]
fn a_scale_set_through_the_dialog_keeps_the_percentage_it_changes() {
    let (mut app, _) = app();
    app.run("file.print", json!({})).unwrap();
    pump(&mut app);
    // A new H with the link on: W follows it, and Done saves both. Scale takes input only while Scaling is Custom.
    set(&mut app, "scaling", json!("custom"));
    set(&mut app, "scale", json!({"width": 100, "height": 250, "linked": true}));
    pump(&mut app);
    set(&mut app, "choice", json!("done"));
    confirm(&mut app);
    let saved = app.session.active().unwrap().doc.print_setup.clone().unwrap();
    assert_eq!(saved["scale"], json!({"width": 250.0, "height": 250.0, "linked": true}));
    // A scale that gives only W keeps the link and H as they were, so H follows W.
    app.run("file.print", json!({})).unwrap();
    pump(&mut app);
    set(&mut app, "scale", json!({"width": 40}));
    pump(&mut app);
    assert_eq!(scale(&app), (40.0, 40.0, true));
    // A new W and H apart with `linked: null` keep the link on: they stay as set, and confirming
    // refuses them, as print.setup does, and saves nothing.
    set(&mut app, "scale", json!({"width": 50, "height": 200, "linked": null}));
    pump(&mut app);
    pump(&mut app);
    assert_eq!(scale(&app), (50.0, 200.0, true));
    set(&mut app, "choice", json!("done"));
    confirm(&mut app);
    assert!(is_open(&app), "refused");
    let saved = app.session.active().unwrap().doc.print_setup.clone().unwrap();
    assert_eq!(saved["scale"], json!({"width": 250.0, "height": 250.0, "linked": true}));
}

#[test]
fn a_spot_inks_icon_converts_it_to_process_and_back() {
    let (mut app, _) = app();
    app.run("swatch.new", json!({"name": "Gold", "color": {"model": "cmyk", "c": 0, "m": 0.2, "y": 0.8, "k": 0.1}, "spot": true})).unwrap();
    app.run("file.print", json!({})).unwrap();
    pump(&mut app);
    set(&mut app, "section", json!("Output"));
    set(&mut app, "output.mode", json!("separations"));
    pump(&mut app);
    let inks = |app: &VectorcraftApp| fields(app)["output"]["inks"].clone();
    // Only the spot ink has the icon.
    let r = call_raw(&mut app, "ui.click", json!({"control": {"name": "Convert Cyan to process"}}));
    assert_eq!(r["ok"], false, "{r}");
    press(&mut app, "Convert Gold to process");
    assert_eq!(inks(&app), json!([{"name": "Gold", "print": true, "convertToProcess": true}]));
    press(&mut app, "Keep Gold as spot");
    assert_eq!(inks(&app), json!([{"name": "Gold", "print": true, "convertToProcess": false}]));
    // Convert All Spot Colors to Process dims the icon, which keeps its own choice.
    set(&mut app, "output.spotsToProcess", json!(true));
    pump(&mut app);
    call_raw(&mut app, "ui.click", json!({"control": {"name": "Keep Gold as spot"}}));
    pump(&mut app);
    assert_eq!(inks(&app), json!([{"name": "Gold", "print": true, "convertToProcess": false}]));
}

/// The names screen readers give the number and text fields in `out`'s accessibility tree: a field's own
/// name, else the text of a label it is labeled by, else "".
pub(super) fn field_names(out: &egui::FullOutput) -> Vec<String> {
    use egui::accesskit::Role;
    let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
    let text_of = |id| nodes.iter().find(|(n, _)| *n == id).and_then(|(_, n)| n.label().or(n.value())).map(str::to_string);
    nodes
        .iter()
        .filter(|(_, n)| matches!(n.role(), Role::TextInput | Role::SpinButton))
        .map(|(_, n)| n.label().map(str::to_string).or_else(|| n.labelled_by().iter().find_map(|l| text_of(*l))).unwrap_or_default())
        .collect()
}

/// Every number and length field of the dialog's sections has a name that tells it from the others in its row.
#[test]
fn every_number_and_length_field_is_named() {
    let (mut app, _) = app();
    app.run("file.print", json!({})).unwrap();
    pump(&mut app);
    let sections: [(&str, &[&str]); 4] = [
        ("General", &["Copies", "Range", "Width", "Height", "Overlap", "Tile Range", "Gutter", "Scale 1", "Scale 2"]),
        ("Marks and Bleed", &["Offset", "Top", "Bottom", "Left", "Right"]),
        ("Graphics", &["Flatness"]),
        ("Advanced", &["Unprintable margin"]),
    ];
    for (section, expected) in sections {
        set(&mut app, "section", json!(section));
        pump(&mut app);
        let ctx = ctx(&app);
        let out = crate::testing::app_frame(&mut app, &ctx, Default::default());
        let names = field_names(&out);
        for name in expected {
            assert!(names.iter().any(|n| n == name), "{section}: {name} in {names:?}");
        }
    }
}
