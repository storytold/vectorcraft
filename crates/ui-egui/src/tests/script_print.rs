//! A script's jobs for a printer (`Document.print` without a file): the run lists the print
//! service's printers, and its jobs are sent after it ends, only when the person at the keyboard
//! approves them with a click on Print.

use std::cell::RefCell;
use std::rc::Rc;

use craft::ui::dialog::DialogResult;
use egui::{Pos2, Rect, vec2};
use serde_json::json;
use vectorcraft_engine::Session;

use super::script_dialogs::{cancel, confirm, fields, is_open, pump, set};
use crate::dialogs::script_run::{self, PRINT_BY_PERSON, PRINT_ENABLE_DELAY_SECS, PRINT_KIND, PRINTS_DROPPED, PrintAnswer, ScriptPrint};
use crate::print::{PrintJob, PrintService, Printer};
use crate::{Services, VectorcraftApp, menus, script_runs};

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
        self.names.borrow().iter().enumerate().map(|(i, name)| Printer::new(name.clone()).is_default(i == 0)).collect()
    }

    fn print(&mut self, job: &PrintJob) -> Result<String, String> {
        self.sent.borrow_mut().push((job.printer.map(str::to_string), job.title.to_string(), job.pdf.to_vec()));
        Ok(format!("Sent to {}", job.printer.unwrap_or("the default printer")))
    }

    fn scripts_may_print(&self) -> bool {
        self.scripts
    }
}

/// An app in a window, with a document, a print service listing Office (the default) and Lab, and
/// no warning before a script runs.
fn print_app(scripts: bool) -> (VectorcraftApp, Sent, Rc<RefCell<Vec<String>>>) {
    let sent = Sent::default();
    let names = Rc::new(RefCell::new(vec!["Office".to_string(), "Lab".to_string()]));
    let spool = Spool { sent: sent.clone(), names: names.clone(), scripts };
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Services { print: Some(Box::new(spool)), ..Default::default() }));
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

fn asking(app: &VectorcraftApp) -> bool {
    is_open::<ScriptPrint>(app)
}

/// One pass of the dialog host at `time` with `events`, then the frame's pump → the texts drawn,
/// with where.
fn frame(app: &mut VectorcraftApp, time: f64, events: Vec<egui::Event>) -> Vec<(String, Rect)> {
    fn texts(s: &egui::Shape, out: &mut Vec<(String, Rect)>) {
        match s {
            egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.visual_bounding_rect())),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let ctx = app.ui_ctx.clone().unwrap();
    let dialogs = app.dialogs().unwrap();
    let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0)));
    let raw = egui::RawInput { screen_rect, time: Some(time), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| dialogs.with(|host| host.run(ui)));
    out.textures_delta.clear();
    pump(app);
    let mut v = vec![];
    out.shapes.iter().for_each(|c| texts(&c.shape, &mut v));
    v
}

/// Where the button `label` was drawn: the last text that reads it (the heading reads Print too).
fn button(texts: &[(String, Rect)], label: &str) -> Pos2 {
    texts.iter().rev().find(|(t, _)| t.trim() == label).unwrap_or_else(|| panic!("no `{label}` in {texts:?}")).1.center()
}

/// A click of the primary button at `pos` from `time`: hover, press and release in frames of their
/// own, as a person's click comes.
fn click(app: &mut VectorcraftApp, time: f64, pos: Pos2) {
    let press = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    for (i, event) in [egui::Event::PointerMoved(pos), press(true), press(false)].into_iter().enumerate() {
        frame(app, time + 0.01 * i as f64, vec![event]);
    }
}

/// A press of `key` with no modifiers at `time`, let go in a frame of its own, as a person lets go
/// of it.
fn key(app: &mut VectorcraftApp, time: f64, key: egui::Key) {
    let event = |pressed| egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
    frame(app, time, vec![event(true)]);
    frame(app, time + 0.01, vec![event(false)]);
}

/// Draws the open question from `time` until it settles, then clicks its `label` button once
/// [`PRINT_ENABLE_DELAY_SECS`] have passed since it was first drawn.
fn click_after_delay(app: &mut VectorcraftApp, time: f64, label: &str) {
    frame(app, time, vec![]);
    frame(app, time + 0.1, vec![]);
    let later = time + PRINT_ENABLE_DELAY_SECS + 0.1;
    let pos = button(&frame(app, later, vec![]), label);
    click(app, later + 0.1, pos);
}

/// The job is sent after the run's changes are kept, once, and only when the person at the
/// keyboard clicks Print: Print from automation is refused and the question opens again, and
/// `ui.dialog.set` and `ui.dialog.confirm` cannot print.
#[test]
fn a_scripts_print_job_is_sent_only_when_the_person_clicks_print() {
    let (mut app, sent, _) = print_app(true);
    run(&mut app, PRINT_TO_LAB);
    assert_eq!(objects(&app), 1, "the run's changes are kept");
    assert!(asking(&app));
    assert!(sent.borrow().is_empty(), "nothing is sent before the person answers");
    let jobs = fields(&app, PRINT_KIND)["jobs"].clone();
    assert_eq!(jobs[0]["printer"], "Lab");
    assert_eq!(jobs.as_array().map(Vec::len), Some(1));
    for (field, value) in [("jobs", json!([])), ("printer", json!("Office"))] {
        let e = set(&app, PRINT_KIND, field, value).unwrap_err();
        assert!(e.contains(&format!("unknown field `{field}`")), "{field}: {e}");
    }
    assert_eq!(set(&app, PRINT_KIND, "answer", json!("print")).unwrap_err(), PRINT_BY_PERSON);
    assert!(!confirm(&mut app, PRINT_KIND), "confirm takes only Cancel");
    assert!(asking(&app), "the question stays open");
    // A Print that a pass automation drove gave.
    script_run::with_host(&app, |host, ctx| host.cancel::<ScriptPrint>(ctx)).unwrap();
    let _ = script_run::with_host(&app, |host, _| host.take::<ScriptPrint>());
    app.ui.answering_control = true;
    script_run::settle_print(&mut app, DialogResult::Ok(PrintAnswer { print: true }));
    app.ui.answering_control = false;
    assert_eq!(app.ui.status, PRINT_BY_PERSON);
    assert!(asking(&app), "the question opens again");
    assert!(sent.borrow().is_empty());
    click_after_delay(&mut app, 10.0, "Print");
    assert!(!asking(&app));
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
    assert!(!asking(&app));
}

/// Enter does not print, and Print takes no click until [`PRINT_ENABLE_DELAY_SECS`] after the
/// question was first drawn; a click after that prints.
#[test]
fn enter_or_an_early_click_does_not_print() {
    let (mut app, sent, _) = print_app(true);
    run(&mut app, PRINT_TO_LAB);
    assert!(asking(&app));
    frame(&mut app, 10.0, vec![]);
    frame(&mut app, 10.05, vec![]);
    let pos = button(&frame(&mut app, 10.1, vec![]), "Print");
    click(&mut app, 10.2, pos);
    assert!(asking(&app), "an early click");
    for time in [10.3, 10.4] {
        key(&mut app, time, egui::Key::Enter);
        assert!(asking(&app), "at {time}");
    }
    let later = 10.0 + PRINT_ENABLE_DELAY_SECS + 0.1;
    for time in [later, later + 0.1] {
        key(&mut app, time, egui::Key::Enter);
        assert!(asking(&app), "Enter at {time}");
    }
    // Tab gives a button the keyboard focus: Enter on it does not print either.
    key(&mut app, later + 0.15, egui::Key::Tab);
    key(&mut app, later + 0.17, egui::Key::Enter);
    assert!(asking(&app), "Enter on the focused button");
    // Nor does an Enter pressed and let go within one frame.
    let enter = |pressed| egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
    frame(&mut app, later + 0.185, vec![enter(true), enter(false)]);
    assert!(asking(&app), "Enter pressed and let go in one frame");
    assert!(sent.borrow().is_empty(), "nothing is sent");
    // The question has settled where it stays; Print is clicked there.
    let pos = button(&frame(&mut app, later + 0.19, vec![]), "Print");
    click(&mut app, later + 0.2, pos);
    assert!(!asking(&app));
    assert_eq!(sent.borrow().len(), 1);
}

/// While a text field has the keyboard when the run ends, the question waits, and nothing is sent;
/// it opens at the first poll after the field lets the keyboard go.
#[test]
fn the_print_question_waits_while_a_text_field_has_the_keyboard() {
    let (mut app, sent, _) = print_app(true);
    let ctx = app.ui_ctx.clone().unwrap();
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
    run(&mut app, PRINT_TO_LAB);
    for _ in 0..3 {
        script_runs::poll(&mut app);
    }
    assert!(!asking(&app), "the question waits");
    assert!(app.session.script_prints_waiting(), "the job waits");
    ctx.memory_mut(|m| m.surrender_focus(id));
    script_runs::poll(&mut app);
    assert!(asking(&app));
    assert!(sent.borrow().is_empty());
}

/// Cancel, from anyone, drops the jobs and keeps the run's changes.
#[test]
fn cancel_drops_a_scripts_print_jobs_and_keeps_its_changes() {
    let (mut app, sent, _) = print_app(true);
    run(&mut app, &format!("app.activeDocument.print(); {PRINT_TO_LAB}"));
    assert_eq!(app.session.script_prints().len(), 2);
    cancel(&mut app, PRINT_KIND);
    assert!(!asking(&app));
    assert!(!app.session.script_prints_waiting());
    assert_eq!(app.ui.status, PRINTS_DROPPED);
    script_runs::poll(&mut app);
    assert!(!asking(&app), "the question does not come back");
    assert!(sent.borrow().is_empty());
    assert_eq!(objects(&app), 1);
    // `answer: "cancel"` with `ui.dialog.confirm` does the same.
    run(&mut app, PRINT_TO_LAB);
    set(&app, PRINT_KIND, "answer", json!("cancel")).unwrap();
    assert!(confirm(&mut app, PRINT_KIND));
    assert!(!asking(&app));
    assert!(!app.session.script_prints_waiting());
    // The Cancel button, at once or later, does the same.
    run(&mut app, PRINT_TO_LAB);
    assert!(asking(&app));
    frame(&mut app, 10.0, vec![]);
    let pos = button(&frame(&mut app, 10.1, vec![]), "Cancel");
    click(&mut app, 10.2, pos);
    assert!(!asking(&app));
    assert!(!app.session.script_prints_waiting());
    assert_eq!(app.ui.status, PRINTS_DROPPED);
    // The app draws on: the closed question's layer is gone by the next frame.
    frame(&mut app, 10.3, vec![]);
    run(&mut app, PRINT_TO_LAB);
    assert!(asking(&app));
    click_after_delay(&mut app, 20.0, "Cancel");
    assert!(!asking(&app));
    assert!(!app.session.script_prints_waiting());
    assert!(sent.borrow().is_empty());
    assert_eq!(objects(&app), 4);
}

/// A run that fails asks nothing and sends nothing; a printer that went away before Print is
/// refused and no job is sent.
#[test]
fn a_failed_run_or_a_printer_that_went_away_sends_nothing() {
    let (mut app, sent, names) = print_app(true);
    run(&mut app, &format!("{PRINT_TO_LAB} throw new Error('late');"));
    assert!(!asking(&app));
    assert!(!app.session.script_prints_waiting());
    assert_eq!(objects(&app), 0);
    run(&mut app, &format!("app.activeDocument.print(); {PRINT_TO_LAB}"));
    assert!(asking(&app));
    names.borrow_mut().retain(|n| n != "Lab");
    click_after_delay(&mut app, 10.0, "Print");
    assert_eq!(app.ui.status, "The printer Lab is no longer installed: the script's print jobs were not sent");
    assert!(sent.borrow().is_empty(), "neither job is sent");
    assert!(!asking(&app));
}

/// Without a service that lets scripts print (the web's), or without any, printing to a printer
/// from a script throws and nothing is asked or sent.
#[test]
fn a_script_cannot_print_to_a_printer_without_a_service_that_allows_it() {
    let (mut app, sent, _) = print_app(false);
    run(&mut app, PRINT_TO_LAB);
    assert!(app.ui.status.contains("Printing to a printer is not available here"), "{}", app.ui.status);
    assert!(!asking(&app));
    assert!(sent.borrow().is_empty());
    let mut bare = crate::testing::windowed(VectorcraftApp::new(Session::new(), Services::default()));
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

/// Space on Print, once Tab gave it the keyboard focus after [`PRINT_ENABLE_DELAY_SECS`], prints:
/// a keyboard user can approve the jobs without a mouse.
#[test]
fn space_on_the_focused_print_button_prints() {
    let (mut app, sent, _) = print_app(true);
    run(&mut app, PRINT_TO_LAB);
    frame(&mut app, 10.0, vec![]);
    let later = 10.0 + PRINT_ENABLE_DELAY_SECS + 0.1;
    frame(&mut app, later, vec![]);
    key(&mut app, later + 0.1, egui::Key::Tab);
    key(&mut app, later + 0.2, egui::Key::Space);
    assert!(!asking(&app));
    assert_eq!(sent.borrow().len(), 1);
}
