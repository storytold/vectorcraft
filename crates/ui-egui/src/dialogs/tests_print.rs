//! File → Print: the Print dialog (every section draws, Print and Done, the preview's drag), the
//! print service it prints through (a mock here) and the PDF it saves without one.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::{Value, json};
use vectorcraft_engine::Session;

use super::print::{KIND, SECTIONS};
use crate::print::{PrintJob, PrintService, Printer};
use crate::state::Dialog;
use crate::{Services, VectorcraftApp, theme};

/// What the mock service was sent: (printer, title, PDF).
type Jobs = Rc<RefCell<Vec<(Option<String>, String, Vec<u8>)>>>;

/// What the app wrote: (path, bytes).
type Written = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

struct MockPrint(Jobs);

impl PrintService for MockPrint {
    fn printers(&mut self) -> Vec<Printer> {
        vec![Printer { name: "Office".into(), default: false }, Printer { name: "Proofer".into(), default: true }]
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
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 120, "height": 90, "artboards": 3})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    (app, jobs)
}

fn frame(app: &mut VectorcraftApp) {
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::show(app, ui.ctx()));
    out.textures_delta.clear();
}

fn dialog(app: &mut VectorcraftApp) -> &mut Dialog {
    app.ui.dialog.as_mut().expect("a dialog is open")
}

fn set(app: &mut VectorcraftApp, field: &str, value: Value) {
    dialog(app).fields.insert(field.into(), value);
}

fn pages(pdf: &[u8]) -> usize {
    vectorcraft_pdf::info(pdf, None).unwrap().pages.len()
}

#[test]
fn print_opens_the_dialog_and_every_section_draws() {
    let (mut app, _) = app();
    app.run("file.print", json!({})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND));
    assert_eq!(super::DialogKind::of(KIND), Some(super::DialogKind::Print));
    let d = dialog(&mut app);
    assert_eq!((d.str("printer"), d.bool("toFile")), ("Proofer".to_string(), false), "the system's default printer");
    assert_eq!(d.fields["copies"], 1, "the document's settings, defaults when never set up");
    for (s, mode) in SECTIONS.iter().flat_map(|s| [(s, "composite"), (s, "separations")]) {
        set(&mut app, "__section", json!(s));
        set(&mut app, "output", json!({"mode": mode}));
        frame(&mut app);
        assert!(app.ui.dialog.is_some(), "{s} closed the dialog");
    }
    // Tiling, a custom paper and settings that can't print draw too.
    set(&mut app, "scaling", json!("tileImageable"));
    set(&mut app, "media", json!("custom"));
    set(&mut app, "__sheet", json!(99));
    frame(&mut app);
    set(&mut app, "copies", json!(0));
    set(&mut app, "__section", json!("Summary"));
    frame(&mut app);
    assert!(app.ui.dialog.is_some());
    let mut changed = vec![json!({"option": "scaling", "value": "tileImageable"}), json!({"option": "copies", "value": 0})];
    super::print::labelled(&mut changed);
    assert_eq!((changed[0]["value"].clone(), changed[1]["value"].clone()), (json!("Tile Imageable Areas"), json!(0)));
    // The menu item and its shortcut.
    assert!(crate::menus::menu_entries(&app).iter().any(|e| e.command.as_deref() == Some("file.print") && e.enabled));
    assert_eq!(crate::shortcut_editor::default_command_shortcut("file.print"), Some("Cmd+P"));
}

#[test]
fn print_sends_every_page_to_the_chosen_printer_and_keeps_the_settings() {
    let (mut app, jobs) = app();
    app.run("file.print", json!({})).unwrap();
    set(&mut app, "printer", json!("Office"));
    set(&mut app, "copies", json!(2));
    set(&mut app, "marks", json!({"trim": true}));
    let undo = app.session.active().unwrap().history.undo.len();
    let r = super::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none(), "Print closes the dialog");
    assert_eq!((r["pages"].clone(), r["printed"].clone(), r["printer"].clone()), (json!(6), json!(true), json!("Office")));
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
    assert_eq!(dialog(&mut app).fields["copies"], 2);
}

#[test]
fn done_keeps_the_settings_without_printing_and_bad_settings_keep_the_dialog() {
    let (mut app, jobs) = app();
    app.run("file.print", json!({})).unwrap();
    set(&mut app, "artboards", json!("range"));
    set(&mut app, "range", json!("7"));
    set(&mut app, "discard", json!(true));
    assert!(super::confirm(&mut app).is_err(), "artboard 7 doesn't exist");
    assert!(app.session.active().unwrap().doc.print_setup.is_none(), "nothing kept");
    assert!(!dialog(&mut app).fields.contains_key("discard"), "Done's press doesn't stick to the dialog");
    set(&mut app, "range", json!("2-3"));
    set(&mut app, "discard", json!(true));
    super::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
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
    let written: Written = Rc::default();
    let w = written.clone();
    let services = Services {
        pick_save: Some(Box::new(|pick: &crate::FilePick| Some(format!("/tmp/{}", pick.name)))),
        write: Some(Box::new(move |p: &str, b: &[u8]| {
            w.borrow_mut().push((p.to_string(), b.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    app.run("file.print", json!({})).unwrap();
    let d = dialog(&mut app);
    assert!(d.bool("toFile"), "no printers here: PDF File");
    frame(&mut app);
    let r = super::confirm(&mut app).unwrap();
    assert_eq!((r["printed"].clone(), r["pages"].clone()), (json!(false), json!(1)));
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
    set(&mut app, "copiesPerSheet", json!(true));
    set(&mut app, "gutter", json!(9));
    frame(&mut app);
    let r = super::confirm(&mut app).unwrap();
    assert_eq!(r["pages"], 3, "a sheet per artboard");
    assert_eq!(pages(&jobs.borrow()[0].2), 3);
    let saved = app.session.active().unwrap().doc.print_setup.clone().unwrap();
    assert_eq!((saved["copiesPerSheet"].clone(), saved["gutter"].clone()), (json!(true), json!(9.0)));
    // Each 120 × 90 artboard fills landscape Letter 6 across and 6 down, 9 pt apart.
    let pv = app.session.execute("print.preview", &json!({})).unwrap();
    assert_eq!(pv["sheets"][0]["copies"].as_array().map(Vec::len), Some(36));
    assert_eq!(pv["sheets"][0]["copies"][1], json!([142.5, 13.5, 262.5, 103.5]));
}

#[test]
fn dragging_the_preview_moves_the_placement_on_the_paper() {
    let (mut app, _) = app();
    // The page as the preview shows it, for each way the paper can be turned.
    for (settings, page_move) in [
        (json!({}), (10.0, 5.0)),
        (json!({"autoRotate": false, "orientation": "portraitFlipped"}), (10.0, 5.0)),
        (json!({"output": {"emulsion": "down"}, "transverse": true}), (-4.0, 7.0)),
        (json!({"scaling": "custom", "scale": {"width": 50, "height": 200}}), (6.0, -8.0)),
        // Copies per sheet: the drag moves the whole grid.
        (json!({"copiesPerSheet": true, "gutter": 9}), (10.0, 5.0)),
    ] {
        let before = app.session.execute("print.preview", &json!({ "settings": settings })).unwrap();
        let sheet = &before["sheets"][0];
        let mut d = Dialog::new(KIND, settings.clone());
        super::print::move_placement(&mut d, sheet, page_move);
        let moved = super::print::settings(&d);
        let after = app.session.execute("print.preview", &json!({ "settings": moved })).unwrap();
        let (t0, t1) = (&before["sheets"][0]["trim"], &after["sheets"][0]["trim"]);
        let delta = (t1[0].as_f64().unwrap() - t0[0].as_f64().unwrap(), t1[1].as_f64().unwrap() - t0[1].as_f64().unwrap());
        assert!((delta.0 - page_move.0).abs() < 0.02 && (delta.1 - page_move.1).abs() < 0.02, "{settings}: moved {delta:?}, dragged {page_move:?}");
    }
}

#[test]
fn the_control_channel_fills_and_confirms_the_dialog() {
    let (mut app, jobs) = app();
    let ctx = egui::Context::default();
    let call = |app: &mut VectorcraftApp, method: &str, params: Value| {
        let (req, _) = crate::control::ControlRequest::new(method, params);
        let crate::control::Outcome::Done(r) = crate::control::handle(app, &ctx, &req) else { panic!("not done") };
        assert_eq!(r["ok"], true, "{method}: {r}");
        r["result"].clone()
    };
    call(&mut app, "engine.execute", json!({"command": "file.print"}));
    call(&mut app, "ui.dialog.set", json!({"field": "copies", "value": 3}));
    call(&mut app, "ui.dialog.set", json!({"field": "artboards", "value": "ignore"}));
    let r = call(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(r["pages"], 3);
    assert_eq!(pages(&jobs.borrow()[0].2), 3);
    // Done through the channel, and Cancel.
    call(&mut app, "engine.execute", json!({"command": "file.print"}));
    call(&mut app, "ui.dialog.set", json!({"field": "copies", "value": 4}));
    call(&mut app, "ui.dialog.cancel", json!({}));
    assert_eq!(app.session.active().unwrap().doc.print_setup.as_ref().unwrap()["copies"], 3, "Cancel keeps nothing");
    call(&mut app, "engine.execute", json!({"command": "file.print"}));
    call(&mut app, "ui.dialog.set", json!({"field": "discard", "value": true}));
    call(&mut app, "ui.dialog.set", json!({"field": "copies", "value": 5}));
    call(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(app.session.active().unwrap().doc.print_setup.as_ref().unwrap()["copies"], 5);
    assert_eq!(jobs.borrow().len(), 1);
}

/// Frames of the dialog layer with the accessibility tree on.
struct Screen {
    ctx: egui::Context,
    time: f64,
}

impl Screen {
    fn new() -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install_fonts(&ctx);
        Self { ctx, time: 0.0 }
    }

    fn run(&mut self, app: &mut VectorcraftApp, events: Vec<egui::Event>) -> egui::FullOutput {
        self.time += 0.1;
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0));
        let input = egui::RawInput { events, time: Some(self.time), screen_rect: Some(screen), ..Default::default() };
        let mut out = self.ctx.run_ui(input, |ui| super::show(app, ui.ctx()));
        out.textures_delta.clear();
        out
    }

    /// The Scale link as screen readers find it: its node, whether it is on, and whether it is dimmed.
    fn link(&mut self, app: &mut VectorcraftApp) -> (egui::accesskit::NodeId, Option<egui::accesskit::Toggled>, bool) {
        self.run(app, vec![]);
        let out = self.run(app, vec![]);
        let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
        let link =
            nodes.iter().find(|(_, n)| n.label() == Some("Constrain Width and Height Proportions") && n.role() == egui::accesskit::Role::Button);
        let (id, node) = link.expect("the Scale link");
        (*id, node.toggled(), node.is_disabled())
    }
}

/// Scale W and H of the open dialog, and its link.
fn scale(app: &mut VectorcraftApp) -> (f64, f64, bool) {
    let s = &dialog(app).fields["scale"];
    (s["width"].as_f64().unwrap_or(f64::NAN), s["height"].as_f64().unwrap_or(f64::NAN), s["linked"] == true)
}

#[test]
fn the_scale_link_keeps_scale_width_and_height_equal() {
    use egui::accesskit::{Action, ActionRequest, Toggled, TreeId};
    let (mut app, _) = app();
    app.run("file.print", json!({})).unwrap();
    let mut screen = Screen::new();
    // On for a new job, and dimmed with the fields until a scaling uses them.
    let (link, on, dimmed) = screen.link(&mut app);
    assert_eq!((on, dimmed), (Some(Toggled::True), true));
    set(&mut app, "scaling", json!("custom"));
    assert!(!screen.link(&mut app).2, "Custom scales by W and H");
    // Linked, a new W sets H, and a new H sets W.
    dialog(&mut app).fields["scale"]["width"] = json!(40);
    screen.run(&mut app, vec![]);
    assert_eq!(scale(&mut app), (40.0, 40.0, true));
    dialog(&mut app).fields["scale"]["height"] = json!(250);
    screen.run(&mut app, vec![]);
    assert_eq!(scale(&mut app), (250.0, 250.0, true));
    // A click turns it off: screen readers hear it off, and each field changes alone.
    let request = |action| egui::Event::AccessKitActionRequest(ActionRequest { action, target_node: link, target_tree: TreeId::ROOT, data: None });
    screen.run(&mut app, vec![request(Action::Click)]);
    assert_eq!(screen.link(&mut app).1, Some(Toggled::False));
    dialog(&mut app).fields["scale"]["width"] = json!(50);
    screen.run(&mut app, vec![]);
    assert_eq!(scale(&mut app), (50.0, 250.0, false));
    // Space on the focused link turns it on and sets H to W; Enter turns it off again.
    let key = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    screen.run(&mut app, vec![request(Action::Focus)]);
    screen.run(&mut app, vec![key(egui::Key::Space)]);
    assert_eq!(scale(&mut app), (50.0, 50.0, true));
    assert_eq!(screen.link(&mut app).1, Some(Toggled::True));
    screen.run(&mut app, vec![request(Action::Focus)]);
    screen.run(&mut app, vec![key(egui::Key::Enter)]);
    assert_eq!(scale(&mut app), (50.0, 50.0, false));
    // Done keeps the link with the document's settings, and the next Print opens with it.
    dialog(&mut app).fields["scale"]["height"] = json!(75);
    set(&mut app, "discard", json!(true));
    super::confirm(&mut app).unwrap();
    let saved = app.session.active().unwrap().doc.print_setup.clone().unwrap();
    assert_eq!(saved["scale"], json!({"width": 50.0, "height": 75.0, "linked": false}));
    app.run("file.print", json!({})).unwrap();
    assert_eq!(scale(&mut app), (50.0, 75.0, false));
}

#[test]
fn a_print_preset_keeps_the_scale_link() {
    let (mut app, _) = app();
    app.run("print.presets.save", json!({"name": "Apart", "settings": {"scaling": "custom", "scale": {"width": 30, "height": 90}}})).unwrap();
    app.run("file.print", json!({})).unwrap();
    assert_eq!(scale(&mut app), (100.0, 100.0, true));
    // Choosing the preset loads its unlinked W and H as they are.
    set(&mut app, "preset", json!("Apart"));
    frame(&mut app);
    assert_eq!(scale(&mut app), (30.0, 90.0, false));
    // A scale set without the link and with W and H apart turns it off, as print.setup reads it.
    set(&mut app, "preset", json!("[Default]"));
    frame(&mut app);
    assert_eq!(scale(&mut app), (100.0, 100.0, true));
    set(&mut app, "scale", json!({"width": 20, "height": 60}));
    frame(&mut app);
    assert_eq!(scale(&mut app), (20.0, 60.0, false));
    // `linked: null` keeps the link as it was, off here.
    set(&mut app, "scale", json!({"width": 30, "height": 70, "linked": null}));
    frame(&mut app);
    assert_eq!(dialog(&mut app).fields["scale"], json!({"width": 30, "height": 70, "linked": false}));
}

#[test]
fn a_scale_set_through_the_dialog_keeps_the_percentage_it_changes() {
    let (mut app, _) = app();
    app.run("file.print", json!({})).unwrap();
    // A new H with the link on, set before any frame: W follows it, and Done saves both.
    set(&mut app, "scale", json!({"width": 100, "height": 250, "linked": true}));
    set(&mut app, "discard", json!(true));
    super::confirm(&mut app).unwrap();
    let saved = app.session.active().unwrap().doc.print_setup.clone().unwrap();
    assert_eq!(saved["scale"], json!({"width": 250.0, "height": 250.0, "linked": true}));
    // A scale that gives only W keeps the link and H as they were, so H follows W.
    app.run("file.print", json!({})).unwrap();
    set(&mut app, "scale", json!({"width": 40}));
    frame(&mut app);
    assert_eq!(scale(&mut app), (40.0, 40.0, true));
    // A new W and H apart with `linked: null` keep the link on: they stay as set, and confirming
    // refuses them, as print.setup does, and saves nothing.
    set(&mut app, "scale", json!({"width": 50, "height": 200, "linked": null}));
    frame(&mut app);
    frame(&mut app);
    assert_eq!(scale(&mut app), (50.0, 200.0, true));
    set(&mut app, "discard", json!(true));
    assert!(super::confirm(&mut app).is_err());
    let saved = app.session.active().unwrap().doc.print_setup.clone().unwrap();
    assert_eq!(saved["scale"], json!({"width": 250.0, "height": 250.0, "linked": true}));
}

/// An accessibility action on the node `target_node`.
fn request(action: egui::accesskit::Action, target_node: egui::accesskit::NodeId) -> egui::Event {
    use egui::accesskit::{ActionRequest, TreeId};
    egui::Event::AccessKitActionRequest(ActionRequest { action, target_node, target_tree: TreeId::ROOT, data: None })
}

/// Type `typed` into Scale W, which shows `shown` (the first field that does, before H), and Tab out.
fn type_w(screen: &mut Screen, app: &mut VectorcraftApp, shown: &str, typed: &str) {
    let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    screen.run(app, vec![]);
    let out = screen.run(app, vec![]);
    let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
    let w = nodes.iter().find(|(_, n)| n.role() == egui::accesskit::Role::TextInput && n.value().is_some_and(|v| v.starts_with(shown)));
    screen.run(app, vec![request(egui::accesskit::Action::Focus, w.expect("the Scale W field").0)]);
    screen.run(app, vec![key(egui::Key::A, egui::Modifiers::COMMAND), egui::Event::Text(typed.into())]);
    screen.run(app, vec![key(egui::Key::Tab, egui::Modifiers::NONE)]);
}

#[test]
fn a_scale_set_after_a_typed_percentage_follows_the_link() {
    let (mut app, _) = app();
    app.run("file.print", json!({})).unwrap();
    let mut screen = Screen::new();
    set(&mut app, "scaling", json!("custom"));
    dialog(&mut app).fields["scale"]["width"] = json!(40);
    screen.run(&mut app, vec![]);
    assert_eq!(scale(&mut app), (40.0, 40.0, true));
    // W typed with the link on sets H as well.
    type_w(&mut screen, &mut app, "40", "60");
    assert_eq!(scale(&mut app), (60.0, 60.0, true));
    // A new H set before the next frame: W follows it, and Done saves both.
    dialog(&mut app).fields["scale"]["height"] = json!(50);
    set(&mut app, "discard", json!(true));
    super::confirm(&mut app).unwrap();
    let saved = app.session.active().unwrap().doc.print_setup.clone().unwrap();
    assert_eq!(saved["scale"], json!({"width": 50.0, "height": 50.0, "linked": true}));
    // W typed with the link off changes W alone; turning the link on then sets H to W.
    app.run("file.print", json!({})).unwrap();
    let (link, _, _) = screen.link(&mut app);
    screen.run(&mut app, vec![request(egui::accesskit::Action::Click, link)]);
    assert_eq!(scale(&mut app), (50.0, 50.0, false));
    type_w(&mut screen, &mut app, "50", "70");
    assert_eq!(scale(&mut app), (70.0, 50.0, false));
    screen.run(&mut app, vec![request(egui::accesskit::Action::Click, link)]);
    screen.run(&mut app, vec![]);
    assert_eq!(scale(&mut app), (70.0, 70.0, true));
}

#[test]
fn the_dimmed_scale_link_ignores_the_keyboard() {
    use egui::accesskit::{Action, ActionRequest, TreeId};
    let (mut app, _) = app();
    app.run("file.print", json!({})).unwrap();
    let mut screen = Screen::new();
    set(&mut app, "scaling", json!("custom"));
    let (link, _, _) = screen.link(&mut app);
    let focus =
        egui::Event::AccessKitActionRequest(ActionRequest { action: Action::Focus, target_node: link, target_tree: TreeId::ROOT, data: None });
    let space = egui::Event::Key { key: egui::Key::Space, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    screen.run(&mut app, vec![focus]);
    // Scaling None dims the link while it has the keyboard: Space leaves it as it was.
    set(&mut app, "scaling", json!("none"));
    screen.run(&mut app, vec![space]);
    screen.run(&mut app, vec![]);
    assert_eq!(scale(&mut app), (100.0, 100.0, true));
}

/// The node screen readers call `label` with role `role`, and whether it is dimmed.
fn node(screen: &mut Screen, app: &mut VectorcraftApp, label: &str, role: egui::accesskit::Role) -> Option<(egui::accesskit::NodeId, bool)> {
    screen.run(app, vec![]);
    let out = screen.run(app, vec![]);
    let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
    nodes.iter().find(|(_, n)| n.label() == Some(label) && n.role() == role).map(|(id, n)| (*id, n.is_disabled()))
}

#[test]
fn a_spot_inks_icon_converts_it_to_process_and_back() {
    use egui::accesskit::{Action, ActionRequest, Role, TreeId};
    let (mut app, _) = app();
    app.run("swatch.new", json!({"name": "Gold", "color": {"model": "cmyk", "c": 0, "m": 0.2, "y": 0.8, "k": 0.1}, "spot": true})).unwrap();
    app.run("file.print", json!({})).unwrap();
    set(&mut app, "__section", json!("Output"));
    set(&mut app, "output", json!({"mode": "separations"}));
    let mut screen = Screen::new();
    let click = |target_node| {
        egui::Event::AccessKitActionRequest(ActionRequest { action: Action::Click, target_node, target_tree: TreeId::ROOT, data: None })
    };
    // Only the spot ink has the icon.
    assert!(node(&mut screen, &mut app, "Convert Cyan to process", Role::Button).is_none());
    let (convert, dimmed) = node(&mut screen, &mut app, "Convert Gold to process", Role::Button).expect("Gold's icon");
    assert!(!dimmed);
    assert_eq!(node(&mut screen, &mut app, "Gold", Role::CheckBox).map(|n| n.1), Some(false));
    screen.run(&mut app, vec![click(convert)]);
    assert_eq!(dialog(&mut app).fields["output"]["inks"], json!([{"name": "Gold", "print": true, "convertToProcess": true}]));
    // Converted, it has no plate: its Print box is dimmed, and the icon turns it back.
    assert_eq!(node(&mut screen, &mut app, "Gold", Role::CheckBox).map(|n| n.1), Some(true));
    let (keep, _) = node(&mut screen, &mut app, "Keep Gold as spot", Role::Button).expect("the process icon");
    screen.run(&mut app, vec![click(keep)]);
    assert_eq!(dialog(&mut app).fields["output"]["inks"], json!([{"name": "Gold", "print": true, "convertToProcess": false}]));
    // Convert All Spot Colors to Process converts it and dims the icon, keeping its own choice.
    dialog(&mut app).fields["output"]["spotsToProcess"] = json!(true);
    let (keep, dimmed) = node(&mut screen, &mut app, "Keep Gold as spot", Role::Button).expect("the process icon");
    assert!(dimmed);
    screen.run(&mut app, vec![click(keep)]);
    assert_eq!(dialog(&mut app).fields["output"]["inks"], json!([{"name": "Gold", "print": true, "convertToProcess": false}]));
}
