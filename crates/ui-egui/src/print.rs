//! Printing through the platform. The host app installs a [`PrintService`] ([`crate::Services`]):
//! the desktop hands the job to the system's print spooler, the web to the browser's print dialog.
//! `file.print` with settings sends the engine's print-ready PDF (`file.print` in the engine) to
//! it, or saves the PDF when asked to (`toFile`, `path`) or when there is no print service; a
//! PostScript file (`format: "postscript"`, or a `.ps` path) is always saved. `print.printers`
//! lists the printers.
//!
//! A script run started in the app gets the printers of a service that lets scripts print
//! ([`PrintService::scripts_may_print`], the desktop's), as names only. The jobs a script prepares
//! for them are sent after the run's changes are kept and the person approves them
//! ([`crate::dialogs::script_run::PRINT_KIND`], [`send_script_prints`]).

use serde_json::{Value, json};
use vectorcraft_engine::cmd::fileio;
use vectorcraft_engine::script::printing::{PreparedPrint, Printers};

use crate::{VectorcraftApp, io};

pub use craft::print::ui::Printer;

/// A print job: a print-ready PDF, every copy and page in it.
pub struct PrintJob<'a> {
    /// The printer; `None` = the system's default (the web: the browser's print dialog).
    pub printer: Option<&'a str>,
    /// The job's name in the print queue.
    pub title: &'a str,
    pub pdf: &'a [u8],
    /// Send the PDF to the printer's own PDF interpreter over its RAW TCP port instead of
    /// spooling it through the platform driver (Windows).
    pub direct_pdf: bool,
}

/// Submission callable from the desktop background worker.
pub type SharedPrintFn = std::sync::Arc<dyn Fn(&PrintJob<'_>) -> Result<String, String> + Send + Sync>;

/// The platform's printing ([`crate::Services::print`]).
pub trait PrintService {
    /// The printers the system knows (none: only the default one, or the browser's dialog).
    fn printers(&mut self) -> Vec<Printer>;
    /// Send `job` to its printer → what happened, for the status bar.
    fn print(&mut self, job: &PrintJob) -> Result<String, String>;
    /// Desktop services can submit on the existing background worker; browser services stay
    /// on the UI thread to invoke their print dialog.
    fn shared_print(&self) -> Option<SharedPrintFn> {
        None
    }
    /// Whether the system turns a landscape page to fit the paper. macOS's print queues put
    /// each page on the paper as it is, so their jobs are written upright (`file.print`'s
    /// `forPrinter`); Windows and the browser's print dialog turn them themselves.
    fn turns_landscape(&self) -> bool {
        true
    }
    /// Whether Setup… can open the system's printer settings.
    fn has_setup(&self) -> bool {
        false
    }
    /// Whether scripts may list this service's printers and send jobs to them once the person
    /// approves them. The browser's print dialog picks its own printer, so the web's does not.
    fn scripts_may_print(&self) -> bool {
        false
    }
    /// Setup…: open the system's settings of `printer` (`None`: the printers).
    fn setup(&mut self, _printer: Option<&str>) -> Result<(), String> {
        Err("there are no printer settings to open here".into())
    }
}

/// `print.printers`: the system's printers, and what the print service offers.
pub fn printers(app: &mut VectorcraftApp) -> Value {
    let (printers, setup) = app.services.print.as_mut().map(|s| (s.printers(), s.has_setup())).unwrap_or_default();
    let printers: Vec<Value> = printers.iter().map(|p| json!({ "name": p.name, "default": p.default })).collect();
    json!({ "printers": printers, "service": app.services.print.is_some(), "setup": setup })
}

/// The printers a script run may print to: the print service's, by name, when it lets scripts
/// print; `None` otherwise.
pub fn script_printers(app: &mut VectorcraftApp) -> Option<Printers> {
    let service = app.services.print.as_mut().filter(|s| s.scripts_may_print())?;
    let for_printer = !service.turns_landscape();
    Some(Printers::new(service.printers().into_iter().map(|p| (p.name, p.default)), for_printer))
}

/// Runs the engine command `id`. A `script.run` started here lists the printers it may print to
/// ([`script_printers`]), and only that run lists them.
pub(crate) fn execute_listing_printers(app: &mut VectorcraftApp, id: &str, params: &Value) -> Result<Value, String> {
    if id == "script.run" {
        app.session.script_printers = script_printers(app);
    }
    let r = app.session.execute(id, params).map_err(|e| e.to_string());
    app.session.script_printers = None;
    r
}

/// The error for a script's jobs when the print service does not let scripts print.
const NO_SCRIPT_PRINTING: &str = "Printing from scripts is not available here: the script's print jobs were not sent";

/// Sends the jobs a script prepared, which the person approved, each to its printer. Every printer
/// must still be one the service lists, by its exact name, or no job is sent. → `{sent}`, or
/// `{background: true}` while the desktop's worker sends them.
pub(crate) fn send_script_prints(app: &mut VectorcraftApp, jobs: Vec<PreparedPrint>) -> Result<Value, String> {
    let service = app.services.print.as_mut().filter(|s| s.scripts_may_print()).ok_or(NO_SCRIPT_PRINTING)?;
    let listed: Vec<String> = service.printers().into_iter().map(|p| p.name).collect();
    if let Some(gone) = jobs.iter().find(|j| !listed.contains(&j.printer)) {
        return Err(format!("The printer {} is no longer installed: the script's print jobs were not sent", gone.printer));
    }
    let count = jobs.len();
    if let Some(submit) = service.shared_print() {
        return crate::background::run(
            app,
            true,
            "Printing from a script".into(),
            None,
            move |_| send_each(&mut |job| submit(job), &jobs).map(|notes| json!({ "notes": notes })),
            move |app, result| {
                let notes = result?;
                app.status(notes["notes"].as_array().and_then(|n| n.last()).and_then(Value::as_str).unwrap_or_default());
                Ok(json!({ "sent": count }))
            },
        );
    }
    let notes = send_each(&mut |job| service.print(job), &jobs)?;
    app.status(notes.last().cloned().unwrap_or_default());
    Ok(json!({ "sent": count }))
}

/// Gives each of `jobs` to `submit`, in order, stopping at the first that fails. → what each one
/// gave, or the error, which says how many were sent before it.
fn send_each(submit: &mut dyn FnMut(&PrintJob<'_>) -> Result<String, String>, jobs: &[PreparedPrint]) -> Result<Vec<String>, String> {
    let mut notes = Vec::new();
    for (sent, job) in jobs.iter().enumerate() {
        match submit(&PrintJob { printer: Some(&job.printer), title: &job.title, pdf: &job.pdf, direct_pdf: false }) {
            Ok(note) => notes.push(note),
            Err(e) if sent == 0 => return Err(e),
            Err(e) => return Err(format!("{e} ({sent} of {} print jobs were sent)", jobs.len())),
        }
    }
    Ok(notes)
}

/// `file.print` with settings: print the active document with `settings` (over the document's, or
/// over those of the print preset `preset` names) on `printer` (default: the system's default), or
/// save the job as a PDF at `path` (else a picked file, the web downloads it) with `toFile`, a
/// `path` or no print service. A PostScript file
/// (`format: "postscript"` or a `.ps` path, at `level`, flattened with `flattenerPreset`) is saved
/// the same way.
pub fn run(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let s = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string);
    let postscript = s("format").as_deref() == Some("postscript") || s("path").is_some_and(|path| fileio::extension(&path) == "ps");
    let to_file = p.get("toFile").and_then(Value::as_bool).unwrap_or(false) || s("path").is_some() || postscript;
    let mut q = json!({});
    for k in ["preset", "settings", "format", "level", "flattenerPreset"] {
        if let Some(v) = p.get(k).filter(|v| !v.is_null()) {
            q[k] = v.clone();
        }
    }
    if postscript {
        // The path is taken off before the engine runs: it can't tell the format from it.
        q["format"] = json!("postscript");
    }
    if to_file || app.services.print.is_none() {
        if let Some(path) = s("path") {
            q["path"] = json!(path);
        }
        return io::run_to_file(app, "file.print", if postscript { "ps" } else { "pdf" }, q, move |app, path, mut v| {
            io::report_saved(app, &path, &v);
            if !to_file {
                app.ui.status.push_str(" (printing isn't available here: the job was saved as a PDF)");
            }
            v["path"] = json!(path);
            v["printed"] = json!(false);
            Ok(v)
        });
    }
    if app.services.print.as_ref().is_some_and(|s| !s.turns_landscape()) {
        q["forPrinter"] = json!(true);
    }
    let mut v = app.session.execute("file.print", &q).map_err(|e| e.to_string())?;
    let data = v.as_object_mut().and_then(|o| o.remove("dataBase64"));
    let pdf = data.as_ref().and_then(Value::as_str).and_then(vectorcraft_format::base64_decode).ok_or("the print job has no data")?;
    let title = app.session.active().map(|d| d.title()).unwrap_or_default();
    let printer = s("printer").filter(|n| !n.is_empty());
    let direct_pdf = p.get("directPdf").and_then(Value::as_bool).unwrap_or(false);
    let service = app.services.print.as_mut().ok_or("no print service")?;
    if let Some(submit) = service.shared_print() {
        let pending_printer = printer.clone();
        let mut pending = crate::background::run(
            app,
            true,
            format!("Printing {title}"),
            None,
            move |_| {
                submit(&PrintJob { printer: printer.as_deref(), title: &title, pdf: &pdf, direct_pdf })
                    .map(|note| json!({ "note": note, "printer": printer }))
            },
            move |app, result| {
                let receipt = result?;
                app.status(receipt.get("note").and_then(Value::as_str).unwrap_or_default());
                Ok(json!({ "printer": receipt["printer"], "printed": true }))
            },
        )?;
        if pending.get("background").and_then(Value::as_bool) == Some(true) {
            // Acceptance by the worker is not acceptance by the spooler.
            pending["printer"] = json!(pending_printer);
            pending["printed"] = json!(false);
        }
        // Preserve the engine's pages, byte count, format and warnings in automation replies.
        if let Some(fields) = pending.as_object() {
            for (key, value) in fields {
                v[key] = value.clone();
            }
        }
        return Ok(v);
    }
    let done = service.print(&PrintJob { printer: printer.as_deref(), title: &title, pdf: &pdf, direct_pdf })?;
    app.status(done);
    v["printer"] = json!(printer);
    v["printed"] = json!(true);
    Ok(v)
}
