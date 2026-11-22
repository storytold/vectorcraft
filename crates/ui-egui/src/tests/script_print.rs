//! A script's jobs for a printer (`Document.print` without a file): the run lists the print
//! service's printers, and its jobs are sent after it ends, only when the person at the keyboard
//! approves them.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::{Value, json};
use vectorcraft_engine::Session;

use crate::dialogs::script_run::{PRINT_BY_PERSON, PRINT_KIND, PRINTS_DROPPED};
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

/// The job is sent after the run's changes are kept, once, and only when the person at the
/// keyboard approves it: Print from a control request or synthetic input is refused.
#[test]
fn a_scripts_print_job_is_sent_only_when_the_person_approves_it() {
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
    assert_eq!(open_kind(&app), Some(PRINT_KIND), "the question stays open");
    assert!(sent.borrow().is_empty());
    dialogs::confirm(&mut app).unwrap();
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
    let e = dialogs::confirm(&mut app).unwrap_err();
    assert_eq!(e, "The printer Lab is no longer installed: the script's print jobs were not sent");
    assert_eq!(app.ui.status, e);
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
