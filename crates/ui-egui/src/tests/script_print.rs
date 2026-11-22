//! A script's jobs for a printer (`Document.print` without a file): the run lists the print
//! service's printers, and its jobs are sent after it ends, only when the person at the keyboard
//! approves them with a click on Print.

use std::cell::RefCell;
use std::rc::Rc;

use egui::{Pos2, Rect, vec2};
use serde_json::{Value, json};
use vectorcraft_engine::Session;

use crate::dialogs::script_run::{PRINT_BY_CLICK, PRINT_BY_PERSON, PRINT_ENABLE_DELAY_SECS, PRINT_KIND, PRINTS_DROPPED};
use crate::print::{PrintJob, PrintService, Printer};
use crate::{Services, VectorcraftApp, dialogs, menus, script_runs};

/// What the service was sent: (printer, title, bytes).
type Sent = Rc<RefCell<Vec<(Option<String>, String, Vec<u8>)>>>;

/// A print service that records the jobs it is sent, with the printers `names` (the first one the
/// default), which lets scripts print when `scripts` is true.
struct Spool {
    sent: Sent,
    names: Rc<RefCell<Vec<String>>>,
    scripts: bool,
}

impl PrintService for Spool {
    fn printers(&mut self) -> Vec<Printer> {
        self.names.borrow().iter().enumerate().map(|(i, name)| Printer { name: name.clone(), default: i == 0 }).collect()
    }

    fn print(&mut self, job: &PrintJob) -> Result<String, String> {
        self.sent.borrow_mut().push((job.printer.map(str::to_string), job.title.to_string(), job.pdf.to_vec()));
        Ok(format!("Sent to {}", job.printer.unwrap_or("the default printer")))
    }

    fn scripts_may_print(&self) -> bool {
        self.scripts
    }
}

/// An app with a document, a print service listing Office (the default) and Lab, and no warning
/// before a script runs.
fn print_app(scripts: bool) -> (VectorcraftApp, Sent, Rc<RefCell<Vec<String>>>) {
    let sent = Sent::default();
    let names = Rc::new(RefCell::new(vec!["Office".to_string(), "Lab".to_string()]));
    let spool = Spool { sent: sent.clone(), names: names.clone(), scripts };
    let mut app = VectorcraftApp::new(Session::new(), Services { print: Some(Box::new(spool)), ..Default::default() });
    app.run("file.new", json!({})).unwrap();
    app.session.prefs.show_external_script_warning = false;
    (app, sent, names)
}

fn objects(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().doc.layers.iter().map(|l| l.children().map_or(0, Vec::len)).sum()
}

/// Runs `source` as File › Scripts › Other Script… runs it, to its end.
fn run(app: &mut VectorcraftApp, source: &str) {
    menus::run_ui_command(app, "ui.runScript", &json!({ "source": source, "name": "p.js" })).unwrap().unwrap();
    for _ in 0..10_000 {
        script_runs::poll(app);
        if !app.session.script_running() {
            return;
        }
    }
    panic!("the script did not end");
}

const PRINT_TO_LAB: &str =
    "var o = new PrintOptions(); o.printerName = 'Lab'; app.activeDocument.print(o); vc.run('shape.rectangle', {x: 0, y: 0, width: 10, height: 10});";

fn open_kind(app: &VectorcraftApp) -> Option<&str> {
    app.ui.dialog.as_ref().map(|d| d.kind.as_str())
}

/// A control request, handled as the app handles one: while `answering_control` is set.
fn control(app: &mut VectorcraftApp, method: &str, params: Value) -> Value {
    let (req, _) = crate::control::ControlRequest::new(method, params);
    app.ui.answering_control = true;
    let outcome = crate::control::handle(app, &egui::Context::default(), &req);
    app.ui.answering_control = false;
    let crate::control::Outcome::Done(r) = outcome else { panic!("expected an immediate reply") };
    r
}

/// The open dialog, drawn as the app draws it at `time` on the app's clock and egui's, with
/// `events` → the texts drawn, with where.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, events: Vec<egui::Event>) -> Vec<(String, Rect)> {
    fn texts(s: &egui::Shape, out: &mut Vec<(String, Rect)>) {
        match s {
            egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.visual_bounding_rect())),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    app.now_override = Some(time);
    let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0)));
    let raw = egui::RawInput { screen_rect, time: Some(time), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| dialogs::show(app, ui.ctx()));
    out.textures_delta.clear();
    let mut v = vec![];
    out.shapes.iter().for_each(|c| texts(&c.shape, &mut v));
    v
}

/// Where the button `label` was drawn: the last text that reads it (the heading reads Print too).
fn button(texts: &[(String, Rect)], label: &str) -> Pos2 {
    texts.iter().rev().find(|(t, _)| t.trim() == label).unwrap_or_else(|| panic!("no `{label}` in {texts:?}")).1.center()
}

/// A press and release of the primary button at `pos`.
fn click(pos: Pos2) -> Vec<egui::Event> {
    let press = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    vec![egui::Event::PointerMoved(pos), press(true), press(false)]
}

/// A press of `key` with no modifiers.
fn key(key: egui::Key) -> Vec<egui::Event> {
    let event = |pressed| egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
    vec![event(true), event(false)]
}

/// Draws the open question from `time` until it settles, then clicks its `label` button once
/// [`PRINT_ENABLE_DELAY_SECS`] have passed since it was first drawn.
fn click_after_delay(app: &mut VectorcraftApp, time: f64, label: &str) {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(app, &ctx, time, vec![]);
    frame(app, &ctx, time + 0.1, vec![]);
    let later = time + PRINT_ENABLE_DELAY_SECS + 0.1;
    let pos = button(&frame(app, &ctx, later, vec![]), label);
    frame(app, &ctx, later + 0.1, click(pos));
}

/// The job is sent after the run's changes are kept, once, and only when the person at the
/// keyboard clicks Print: Print from a control request or synthetic input is refused, as a confirm
/// without the click is.
#[test]
fn a_scripts_print_job_is_sent_only_when_the_person_clicks_print() {
    let (mut app, sent, _) = print_app(true);
    run(&mut app, PRINT_TO_LAB);
    assert_eq!(objects(&app), 1, "the run's changes are kept");
    assert_eq!(open_kind(&app), Some(PRINT_KIND));
    assert!(sent.borrow().is_empty(), "nothing is sent before the person answers");
    let r = control(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(r["error"], PRINT_BY_PERSON, "{r}");
    app.synthetic_frame = true;
    assert_eq!(dialogs::confirm(&mut app).unwrap_err(), PRINT_BY_PERSON);
    app.synthetic_frame = false;
    assert_eq!(dialogs::confirm(&mut app).unwrap_err(), PRINT_BY_CLICK);
    assert_eq!(open_kind(&app), Some(PRINT_KIND), "the question stays open");
    assert!(sent.borrow().is_empty());
    click_after_delay(&mut app, 10.0, "Print");
    assert_eq!(open_kind(&app), None);
    {
        let sent = sent.borrow();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0.as_deref(), Some("Lab"));
        assert!(sent[0].2.starts_with(b"%PDF"));
    }
    assert_eq!(app.ui.status, "Sent to Lab");
    for _ in 0..5 {
        script_runs::poll(&mut app);
    }
    assert_eq!(sent.borrow().len(), 1, "a job is sent once");
    assert_eq!(open_kind(&app), None);
}

/// Enter does not print, and Print takes no click until [`PRINT_ENABLE_DELAY_SECS`] after the
/// question was first drawn; a click after that prints.
#[test]
fn enter_or_an_early_click_does_not_print() {
    let (mut app, sent, _) = print_app(true);
    run(&mut app, PRINT_TO_LAB);
    assert_eq!(open_kind(&app), Some(PRINT_KIND));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&mut app, &ctx, 10.0, vec![]);
    let pos = button(&frame(&mut app, &ctx, 10.1, vec![]), "Print");
    for (time, events) in [(10.2, click(pos)), (10.3, key(egui::Key::Enter)), (10.4, key(egui::Key::Enter))] {
        frame(&mut app, &ctx, time, events);
        assert_eq!(open_kind(&app), Some(PRINT_KIND), "at {time}");
    }
    let later = 10.0 + PRINT_ENABLE_DELAY_SECS + 0.1;
    for (time, events) in [(later, key(egui::Key::Enter)), (later + 0.1, key(egui::Key::Enter))] {
        frame(&mut app, &ctx, time, events);
        assert_eq!(open_kind(&app), Some(PRINT_KIND), "Enter at {time}");
    }
    // Tab gives Print the keyboard focus: Enter on it does not print either.
    frame(&mut app, &ctx, later + 0.15, key(egui::Key::Tab));
    frame(&mut app, &ctx, later + 0.16, key(egui::Key::Enter));
    assert_eq!(open_kind(&app), Some(PRINT_KIND), "Enter on the focused button");
    assert!(sent.borrow().is_empty(), "nothing is sent");
    frame(&mut app, &ctx, later + 0.2, click(pos));
    assert_eq!(open_kind(&app), None);
    assert_eq!(sent.borrow().len(), 1);
}

/// Space on Print, once Tab gave it the keyboard focus after [`PRINT_ENABLE_DELAY_SECS`], prints:
/// a keyboard user can approve the jobs without a mouse.
#[test]
fn space_on_the_focused_print_button_prints() {
    let (mut app, sent, _) = print_app(true);
    run(&mut app, PRINT_TO_LAB);
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&mut app, &ctx, 10.0, vec![]);
    let later = 10.0 + PRINT_ENABLE_DELAY_SECS + 0.1;
    frame(&mut app, &ctx, later, vec![]);
    frame(&mut app, &ctx, later + 0.1, key(egui::Key::Tab));
    frame(&mut app, &ctx, later + 0.2, key(egui::Key::Space));
    assert_eq!(open_kind(&app), None);
    assert_eq!(sent.borrow().len(), 1);
}

/// While a text field has the keyboard when the run ends, the question waits, and nothing is sent;
/// it opens at the first poll after the field lets the keyboard go.
#[test]
fn the_print_question_waits_while_a_text_field_has_the_keyboard() {
    let (mut app, sent, _) = print_app(true);
    let ctx = egui::Context::default();
    let id = egui::Id::unique("script-print-typing");
    let mut text = String::new();
    let mut draw = |time: f64, focus: bool| {
        let raw = egui::RawInput { time: Some(time), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut text).id(id));
            if focus {
                r.request_focus();
            }
        });
        out.textures_delta.clear();
    };
    draw(1.0, true);
    draw(1.1, false);
    assert!(ctx.text_edit_focused());
    app.ui_ctx = Some(ctx.clone());
    run(&mut app, PRINT_TO_LAB);
    for _ in 0..3 {
        script_runs::poll(&mut app);
    }
    assert_eq!(open_kind(&app), None, "the question waits");
    assert!(app.session.script_prints_waiting(), "the job waits");
    ctx.memory_mut(|m| m.surrender_focus(id));
    script_runs::poll(&mut app);
    assert_eq!(open_kind(&app), Some(PRINT_KIND));
    assert!(sent.borrow().is_empty());
}

/// Cancel, from anyone, drops the jobs and keeps the run's changes.
#[test]
fn cancel_drops_a_scripts_print_jobs_and_keeps_its_changes() {
    let (mut app, sent, _) = print_app(true);
    run(&mut app, &format!("app.activeDocument.print(); {PRINT_TO_LAB}"));
    assert_eq!(app.session.script_prints().len(), 2);
    let r = control(&mut app, "ui.dialog.cancel", json!({}));
    assert!(r.get("error").is_none_or(Value::is_null), "{r}");
    assert_eq!(open_kind(&app), None);
    assert!(!app.session.script_prints_waiting());
    assert_eq!(app.ui.status, PRINTS_DROPPED);
    script_runs::poll(&mut app);
    assert_eq!(open_kind(&app), None, "the question does not come back");
    assert!(sent.borrow().is_empty());
    assert_eq!(objects(&app), 1);
    // The Cancel button, at once or later, does the same.
    run(&mut app, PRINT_TO_LAB);
    assert_eq!(open_kind(&app), Some(PRINT_KIND));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&mut app, &ctx, 10.0, vec![]);
    let pos = button(&frame(&mut app, &ctx, 10.1, vec![]), "Cancel");
    frame(&mut app, &ctx, 10.2, click(pos));
    assert_eq!(open_kind(&app), None);
    assert!(!app.session.script_prints_waiting());
    assert_eq!(app.ui.status, PRINTS_DROPPED);
    run(&mut app, PRINT_TO_LAB);
    click_after_delay(&mut app, 20.0, "Cancel");
    assert_eq!(open_kind(&app), None);
    assert!(!app.session.script_prints_waiting());
    assert!(sent.borrow().is_empty());
    assert_eq!(objects(&app), 3);
}

/// A run that fails asks nothing and sends nothing; a printer that went away before Print is
/// refused and no job is sent.
#[test]
fn a_failed_run_or_a_printer_that_went_away_sends_nothing() {
    let (mut app, sent, names) = print_app(true);
    run(&mut app, &format!("{PRINT_TO_LAB} throw new Error('late');"));
    assert_eq!(open_kind(&app), None);
    assert!(!app.session.script_prints_waiting());
    assert_eq!(objects(&app), 0);
    run(&mut app, &format!("app.activeDocument.print(); {PRINT_TO_LAB}"));
    assert_eq!(open_kind(&app), Some(PRINT_KIND));
    names.borrow_mut().retain(|n| n != "Lab");
    click_after_delay(&mut app, 10.0, "Print");
    assert_eq!(app.ui.status, "The printer Lab is no longer installed: the script's print jobs were not sent");
    assert!(sent.borrow().is_empty(), "neither job is sent");
    assert_eq!(open_kind(&app), None);
}

/// Without a service that lets scripts print (the web's), or without any, printing to a printer
/// from a script throws and nothing is asked or sent.
#[test]
fn a_script_cannot_print_to_a_printer_without_a_service_that_allows_it() {
    let (mut app, sent, _) = print_app(false);
    run(&mut app, PRINT_TO_LAB);
    assert!(app.ui.status.contains("Printing to a printer is not available here"), "{}", app.ui.status);
    assert_eq!(open_kind(&app), None);
    assert!(sent.borrow().is_empty());
    let mut bare = VectorcraftApp::new(Session::new(), Services::default());
    bare.run("file.new", json!({})).unwrap();
    bare.session.prefs.show_external_script_warning = false;
    run(&mut bare, "vc.log(app.printerList.length);");
    assert!(bare.ui.status.contains("Application.printerList is not available here"), "{}", bare.ui.status);
    // A run the app started lists the service's printers; a later run without one does not.
    let (mut app, _, _) = print_app(true);
    run(&mut app, "var l = app.printerList; vc.log([l.length, l[0].name, l[1].name]);");
    assert_eq!(app.session.script_report().unwrap()["log"], json!([[2, "Office", "Lab"]]));
    assert!(app.session.script_printers.is_none());
    // A run that does not start leaves no list behind for a later one.
    assert!(app.run("script.run", json!({"source": "1", "path": "x.js"})).is_err());
    assert!(app.session.script_printers.is_none());
}
