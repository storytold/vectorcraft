//! Printing through the platform. The host app installs a [`PrintService`] ([`crate::Services`]):
//! the desktop hands the job to the system's print spooler, the web to the browser's print dialog.
//! `file.print` with settings sends the engine's print-ready PDF (`file.print` in the engine) to
//! it, or saves the PDF when asked to (`toFile`, `path`) or when there is no print service; a
//! PostScript file (`format: "postscript"`, or a `.ps` path) is always saved. `print.printers`
//! lists the printers.

use serde::Serialize;
use serde_json::{Value, json};
use vectorcraft_engine::cmd::fileio;

use crate::{VectorcraftApp, io};

/// A printer the system knows.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Printer {
    pub name: String,
    /// The system's default printer.
    pub default: bool,
}

/// A print job: a print-ready PDF, every copy and page in it.
pub struct PrintJob<'a> {
    /// The printer; `None` = the system's default (the web: the browser's print dialog).
    pub printer: Option<&'a str>,
    /// The job's name in the print queue.
    pub title: &'a str,
    pub pdf: &'a [u8],
    /// The pages are gray already (the PDF was composed so); also ask the queue for monochrome.
    pub grayscale: bool,
    /// The person picked the paper: the queue must have it. `false`: the pages are the document's
    /// own size (an artboard, a custom size), so the job goes on the queue's own paper, shrunk to fit.
    pub paper_chosen: bool,
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
    /// Setup…: open the system's settings of `printer` (`None`: the printers).
    fn setup(&mut self, _printer: Option<&str>) -> Result<(), String> {
        Err("there are no printer settings to open here".into())
    }
}

/// `print.printers`: the system's printers, and what the print service offers.
pub fn printers(app: &mut VectorcraftApp) -> Value {
    let (printers, setup) = app.services.print.as_mut().map(|s| (s.printers(), s.has_setup())).unwrap_or_default();
    json!({ "printers": printers, "service": app.services.print.is_some(), "setup": setup })
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
        let (path, mut v) = io::run_to_file(app, "file.print", if postscript { "ps" } else { "pdf" }, q)?;
        io::report_saved(app, &path, &v);
        if !to_file {
            app.ui.status.push_str(" (printing isn't available here: the job was saved as a PDF)");
        }
        v["path"] = json!(path);
        v["printed"] = json!(false);
        return Ok(v);
    }
    if app.services.print.as_ref().is_some_and(|s| !s.turns_landscape()) {
        q["forPrinter"] = json!(true);
    }
    let mut v = app.session.execute("file.print", &q).map_err(|e| e.to_string())?;
    let data = v.as_object_mut().and_then(|o| o.remove("dataBase64"));
    let pdf = data.as_ref().and_then(Value::as_str).and_then(vectorcraft_format::base64_decode).ok_or("the print job has no data")?;
    let title = app.session.active().map(|d| d.title()).unwrap_or_default();
    let printer = s("printer").filter(|n| !n.is_empty());
    let (grayscale, paper_chosen) = job_hints(app, p);
    let service = app.services.print.as_mut().ok_or("no print service")?;
    if let Some(submit) = service.shared_print() {
        let pending_printer = printer.clone();
        let mut pending = crate::background::run(
            app,
            true,
            format!("Printing {title}"),
            None,
            move |_| {
                submit(&PrintJob { printer: printer.as_deref(), title: &title, pdf: &pdf, grayscale, paper_chosen })
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
    let done = service.print(&PrintJob { printer: printer.as_deref(), title: &title, pdf: &pdf, grayscale, paper_chosen })?;
    app.status(done);
    v["printer"] = json!(printer);
    v["printed"] = json!(true);
    Ok(v)
}

/// What a local-queue job asks of the driver, beside the PDF: `(grayscale, paper chosen)`. Gray is
/// the Grayscale of the settings `file.print` resolves (the print preset `preset` names, else the
/// document's, with `settings` over them; composite output); the paper is chosen when the caller says so
/// (`paperChosen`, the dialog) or names a standard paper in `settings.media`. A Custom size, or no
/// paper named, is the document's own page size, which the queue may not list.
fn job_hints(app: &mut VectorcraftApp, p: &Value) -> (bool, bool) {
    let settings = p.get("settings").filter(|v| v.is_object()).cloned().unwrap_or_else(|| json!({}));
    // The same `preset` and `settings` as `file.print` gets, so the hint is about the job printed.
    let mut query = json!({ "settings": settings });
    if let Some(preset) = p.get("preset").filter(|v| !v.is_null()) {
        query["preset"] = preset.clone();
    }
    let merged = app.session.execute("print.preview", &query).ok().map(|v| v["settings"].clone());
    let gray = merged.as_ref().is_some_and(|m| m["color"]["grayscale"] == true && m["output"]["mode"] != "separations");
    let named = |v: &Value| v["media"].as_str().is_some_and(|m| m != "custom");
    let chosen = p.get("paperChosen").and_then(Value::as_bool).unwrap_or_else(|| named(&settings));
    (gray, chosen)
}

/// `settings` with Grayscale `gray` (what a network job prints: [`crate::network_glue::NetTasks::network_grayscale`]).
#[cfg(all(not(target_arch = "wasm32"), not(target_vendor = "win7")))]
pub(crate) fn with_grayscale(settings: &Value, gray: bool) -> Value {
    let mut out = settings.clone();
    if let Some(o) = out.as_object_mut() {
        let color = o.entry("color").or_insert_with(|| json!({}));
        if let Some(c) = color.as_object_mut() {
            c.insert("grayscale".into(), json!(gray));
        }
    }
    out
}

/// `file.print` for the Print dialog's Network printer destination: compose the print-ready PDF
/// from `settings` (over the document's) and hand it to the queried printer, off the UI thread. The
/// person's confirmation, the query and the address are craft-ui's rules (`Draft::block`), and a job
/// that cannot start says why. The dialog stays open to show the job; the result of the job is
/// shown there, or as a notice when the dialog is closed.
///
/// Copies are the network section's: a printer whose listed copies range covers them makes them
/// itself (the PDF is one copy); any other (no range listed, or a smaller one) gets them in the PDF
/// and is asked for one copy, so the sheets are what was asked for.
///
/// Grayscale: the network section's Color setting when one is picked, else the dialog's (or the
/// preset's) Grayscale; a gray job is a gray PDF and asks the printer for monochrome.
///
/// `paper_chosen`: the person picked the paper (the dialog says so), so it is sent as the media.
#[cfg(all(not(target_arch = "wasm32"), not(target_vendor = "win7")))]
pub fn run_network(app: &mut VectorcraftApp, settings: &Value, paper_chosen: bool) -> Result<Value, String> {
    use craft_ui::printing::network::Block;
    let sheets_ok =
        app.session.execute("print.preview", &json!({ "settings": settings })).is_ok_and(|v| v["sheets"].as_array().is_some_and(|s| !s.is_empty()));
    let draft = &app.print_net.state.draft;
    let address_ok = crate::network_glue::validate(&draft.normalized_url()).is_ok();
    if let Some(block) = draft.block(address_ok, sheets_ok, crate::network_glue::FORMATS) {
        let labels = crate::network_glue::labels();
        return Err(match block {
            Block::Address => labels.address_note,
            Block::NotQueried => labels.query_first,
            Block::Format => labels.no_common_format,
            Block::NoPages => tl!("Nothing to print"),
            // The box's own label is no reason: say what to do.
            Block::Unconfirmed => tl!("Tick the confirmation box to send this document to this printer."),
            Block::Busy => labels.sending,
        }
        .replace("{printer}", "…"));
    }
    let by_printer = app.print_net.printer_makes_copies();
    let copies = if by_printer { 1 } else { app.print_net.state.draft.copies.max(1) };
    // A printer told to print Grayscale may still print colour, so the PDF itself is gray too.
    let gray = app.print_net.network_grayscale(crate::dialogs::print::prints_gray(settings));
    if gray && app.print_net.state.draft.color.is_none() {
        // The dialog's (or the preset's) Grayscale, with no Color picked: the printer is asked for it too.
        app.print_net.state.draft.color = Some(craft_ui::printing::network::ColorMode::Monochrome);
    }
    let mut q = json!({ "settings": with_grayscale(settings, gray) });
    q["settings"]["copies"] = json!(copies);
    let mut v = app.session.execute("file.print", &q).map_err(|e| e.to_string())?;
    let data = v.as_object_mut().and_then(|o| o.remove("dataBase64"));
    let pdf = data.as_ref().and_then(Value::as_str).and_then(vectorcraft_format::base64_decode).ok_or("the print job has no data")?;
    let title = app.session.active().map(|d| d.title()).unwrap_or_default();
    let pages = v["pages"].as_u64().map_or(0, |n| n as usize);
    // The paper goes to the printer only when the person chose it; otherwise `media` is the document's
    // own default (Letter) and the printer's own paper stands, as for a local queue.
    let paper = network_media(settings, paper_chosen);
    if !app.print_net.send(None, &title, pdf, pages, paper, by_printer) {
        return Err(tl!("Couldn't reach the printer").into());
    }
    v["printed"] = json!(false);
    v["network"] = json!(true);
    Ok(v)
}

/// The media name a network printer is asked for: the dialog's paper, only when the person chose it.
#[cfg(all(not(target_arch = "wasm32"), not(target_vendor = "win7")))]
fn network_media(settings: &Value, paper_chosen: bool) -> Option<&'static str> {
    if !paper_chosen {
        return None;
    }
    settings["media"].as_str().and_then(crate::network_glue::media_for_paper)
}

#[cfg(test)]
mod tests {
    use super::job_hints;
    use crate::VectorcraftApp;
    use serde_json::json;
    use vectorcraft_engine::Session;

    fn app_with_gray_preset() -> VectorcraftApp {
        let mut session = Session::new();
        session.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
        session.execute("print.presets.save", &json!({"name": "Mono", "settings": {"color": {"grayscale": true}}})).unwrap();
        VectorcraftApp::new(session, Default::default())
    }

    /// The hint reads the preset `file.print` will resolve, not only the document's settings.
    #[test]
    fn a_grayscale_preset_makes_the_job_gray() {
        let mut app = app_with_gray_preset();
        assert_eq!(job_hints(&mut app, &json!({"preset": "Mono"})), (true, false));
        assert!(job_hints(&mut app, &json!({"preset": "mono"})).0, "any case, as the engine takes it");
        assert!(!job_hints(&mut app, &json!({})).0, "no preset: the document's settings, which are in colour");
        assert!(!job_hints(&mut app, &json!({"preset": null})).0);
        assert!(job_hints(&mut app, &json!({"settings": {"color": {"grayscale": true}}})).0, "explicit settings still apply");
        assert!(!job_hints(&mut app, &json!({"preset": "Mono", "settings": {"color": {"grayscale": false}}})).0, "settings stand over the preset's");
    }

    /// Without a preset or explicit media the queue's own paper stands, though the merged media is Letter.
    #[test]
    fn no_preset_and_no_media_does_not_choose_the_paper() {
        let mut app = app_with_gray_preset();
        assert_eq!(job_hints(&mut app, &json!({})), (false, false));
        assert!(!job_hints(&mut app, &json!({"preset": "Mono"})).1, "a preset's saved Letter is not a choice");
        assert!(!job_hints(&mut app, &json!({"settings": {"media": "custom"}})).1);
        assert!(job_hints(&mut app, &json!({"settings": {"media": "a4"}})).1, "named media is a choice");
        assert!(job_hints(&mut app, &json!({"paperChosen": true})).1, "so is the dialog saying so");
        assert!(!job_hints(&mut app, &json!({"paperChosen": false, "settings": {"media": "a4"}})).1);
    }

    /// An untouched dialog leaves the document's Letter as `media`; an A4-only printer must not get it.
    #[cfg(all(not(target_arch = "wasm32"), not(target_vendor = "win7")))]
    #[test]
    fn the_network_printer_gets_the_media_only_when_the_paper_was_chosen() {
        let s = json!({"media": "a4"});
        assert_eq!(super::network_media(&s, true), Some("iso_a4_210x297mm"));
        assert_eq!(super::network_media(&s, false), None, "the document's default paper is not a choice");
        assert_eq!(super::network_media(&json!({"media": "custom"}), true), None, "a custom size has no media name");
    }
}
