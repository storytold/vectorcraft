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

/// What the mock service was asked beside the PDF: (grayscale, paper chosen) per job.
type Asked = Rc<RefCell<Vec<(bool, bool)>>>;

/// What the app wrote: (path, bytes).
type Written = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

struct MockPrint(Jobs, Asked);

impl PrintService for MockPrint {
    fn printers(&mut self) -> Vec<Printer> {
        vec![Printer { name: "Office".into(), default: false }, Printer { name: "Proofer".into(), default: true }]
    }

    fn print(&mut self, job: &PrintJob) -> Result<String, String> {
        self.0.borrow_mut().push((job.printer.map(str::to_string), job.title.to_string(), job.pdf.to_vec()));
        self.1.borrow_mut().push((job.grayscale, job.paper_chosen));
        Ok(format!("Sent to {}", job.printer.unwrap_or("the default printer")))
    }
}

/// An app with three 120 × 90 artboards and a rectangle, printing through the mock.
fn app() -> (VectorcraftApp, Jobs) {
    let (app, jobs, _) = app_asked();
    (app, jobs)
}

/// The same, also giving what each job asked of the queue; the rectangle is red.
fn app_asked() -> (VectorcraftApp, Jobs, Asked) {
    let (jobs, asked) = (Jobs::default(), Asked::default());
    let services = Services { print: Some(Box::new(MockPrint(jobs.clone(), asked.clone()))), ..Default::default() };
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 120, "height": 90, "artboards": 3})).unwrap();
    let rect = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    app.run("paint.setFill", json!({"ids": [rect["id"].clone()], "color": "#dc1e28"})).unwrap();
    (app, jobs, asked)
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
fn dragging_the_preview_moves_the_placement_on_the_paper() {
    let (mut app, _) = app();
    // The page as the preview shows it, for each way the paper can be turned.
    for (settings, page_move) in [
        (json!({}), (10.0, 5.0)),
        (json!({"autoRotate": false, "orientation": "portraitFlipped"}), (10.0, 5.0)),
        (json!({"output": {"emulsion": "down"}, "transverse": true}), (-4.0, 7.0)),
        (json!({"scaling": "custom", "scale": {"width": 50, "height": 200}}), (6.0, -8.0)),
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

// ---------- one Printer list ----------

#[test]
fn the_printer_list_holds_every_system_printer_then_network_then_pdf() {
    use super::print::Destination::*;
    let (mut app, _) = app();
    app.run("file.print", json!({})).unwrap();
    let printers = dialog(&mut app).fields["__printers"].clone();
    let entries = super::print::destinations(Some(&printers));
    assert_eq!(
        entries,
        [Printer("Proofer".into(), true), Printer("Office".into(), false), Network, File],
        "the default queue first, the other queues, Network Printer…, PDF File"
    );
    let labels: Vec<String> = entries.iter().map(super::print::destination_label).collect();
    assert_eq!(labels, ["Proofer (default)", "Office", "Network Printer…", "PDF File"]);
    // A system with a print service and no queue lists its default printer.
    let none = json!({"printers": [], "service": true});
    assert_eq!(super::print::destinations(Some(&none)), [Default, Network, File]);
    frame(&mut app);
}

#[test]
fn choosing_the_network_entry_shows_the_network_section_and_a_queue_hides_it() {
    use super::print::{Destination, choose};
    let (mut app, _) = app();
    app.run("file.print", json!({})).unwrap();
    let mut d = app.ui.dialog.take().unwrap();
    choose(&mut app, &mut d, &Destination::Network);
    assert!(d.bool("network") && !d.bool("toFile"));
    choose(&mut app, &mut d, &Destination::Printer("Office".into(), false));
    assert_eq!((d.str("printer"), d.bool("network"), d.bool("toFile")), ("Office".to_string(), false, false));
    choose(&mut app, &mut d, &Destination::Network);
    app.ui.dialog = Some(d);
    // The section draws (address field, Query printer) in the same dialog as the queues.
    frame(&mut app);
    assert!(dialog(&mut app).bool("network"));
}

// ---------- Grayscale ----------

#[test]
fn the_colour_choice_says_grayscale_not_black_and_white() {
    let labels = crate::network_glue::labels();
    assert_eq!(labels.color_monochrome, "Grayscale");
    assert!(labels.unsupported_color.contains("grayscale"), "{}", labels.unsupported_color);
    assert!(!labels.unsupported_color.to_lowercase().contains("black"));
}

/// Whether the PDF's art has RGB colours and whether it has gray ones (read back through the importer).
fn paints(pdf: &[u8]) -> (bool, bool) {
    let layers = format!("{:?}", vectorcraft_pdf::import(pdf).unwrap().layers);
    (layers.contains("Rgb"), layers.contains("Gray"))
}

#[test]
fn grayscale_makes_the_printed_pdf_gray_and_asks_the_queue_for_monochrome_too() {
    let (mut app, jobs, asked) = app_asked();
    let print = |app: &mut VectorcraftApp, gray: bool| {
        app.run("file.print", json!({"settings": {"color": {"grayscale": gray}}, "printer": "Office"})).unwrap();
        let (pdf, ask) = (jobs.borrow().last().unwrap().2.clone(), *asked.borrow().last().unwrap());
        (pdf, ask)
    };
    let (colour, ask) = print(&mut app, false);
    assert!(!ask.0);
    assert!(paints(&colour).0, "the colour job paints RGB");
    let (gray, ask) = print(&mut app, true);
    assert!(ask.0, "the queue is asked for monochrome as well");
    assert_ne!(colour, gray, "the PDF itself changes");
    let (rgb, g) = paints(&gray);
    assert!(g && !rgb, "the gray PDF paints gray only (rgb {rgb}, gray {g})");
    // Through the dialog: the box is the setting `color.grayscale`, kept with the document.
    app.run("file.print", json!({})).unwrap();
    set(&mut app, "printer", json!("Office"));
    set(&mut app, "color", json!({"grayscale": true}));
    frame(&mut app);
    super::confirm(&mut app).unwrap();
    assert!(asked.borrow().last().unwrap().0);
    assert_eq!(app.session.active().unwrap().doc.print_setup.as_ref().unwrap()["color"]["grayscale"], true);
    assert!(!paints(&jobs.borrow().last().unwrap().2).0);
}

#[test]
fn grayscale_uses_rec_709_weights() {
    let (app, _) = app();
    let doc = app.session.active().unwrap().doc.clone();
    let gray = vectorcraft_engine::cmd::print::grayscale_document(&doc);
    let fills = |d: &vectorcraft_doc::Document| format!("{:?}", d.layers);
    assert!(fills(&doc).contains("Rgb"), "the rectangle is coloured");
    assert!(!fills(&gray).contains("Rgb"), "no colour is left");
    // Rec. 709: 0.2126 × 220/255 + 0.7152 × 30/255 + 0.0722 × 40/255 = 0.2789 light: 0.7211 ink.
    assert!(fills(&gray).contains("0.721"), "{}", fills(&gray));
}

#[test]
fn the_preview_art_is_gray_from_the_frame_the_box_is_ticked() {
    let (mut app, _) = app();
    app.run("file.print", json!({})).unwrap();
    let ctx = egui::Context::default();
    let area = vectorcraft_geom::Rect::new(0.0, 0.0, 120.0, 90.0);
    let layers = vectorcraft_pdf::PrintLayers::VisiblePrintable;
    let colour = super::print::art_texture(&mut app, &ctx, layers, false, area, 64.0).unwrap();
    let gray = super::print::art_texture(&mut app, &ctx, layers, true, area, 64.0).unwrap();
    assert_ne!(colour.id(), gray.id(), "toggling renders again: the cache key holds the gray flag");
    let again = super::print::art_texture(&mut app, &ctx, layers, false, area, 64.0).unwrap();
    assert_ne!(gray.id(), again.id(), "back to colour is a colour render, not the gray one");
    // What the gray preview draws: the document the print composes, rendered.
    let doc = app.session.active().unwrap().doc.clone();
    let img = app.canvas.renderer.render_region(&vectorcraft_engine::cmd::print::grayscale_document(&doc), area, 1.0, true);
    assert!(img.pixels.chunks(4).all(|p| p[0] == p[1] && p[1] == p[2]), "every pixel is gray");
    let img = app.canvas.renderer.render_region(&doc, area, 1.0, true);
    assert!(img.pixels.chunks(4).any(|p| p[0] != p[1] || p[1] != p[2]), "the colour render has colour");
    // Not offered for separations (inks are plates already).
    assert!(super::print::prints_gray(&json!({"color": {"grayscale": true}})));
    assert!(!super::print::prints_gray(&json!({"color": {"grayscale": true}, "output": {"mode": "separations"}})));
    assert!(!super::print::prints_gray(&json!({})));
}

// ---------- paper ----------

#[test]
fn a_document_sized_job_does_not_demand_a_paper_but_a_chosen_paper_is_demanded() {
    let (mut app, _, asked) = app_asked();
    // Untouched paper: the queue's default paper, shrunk to fit.
    app.run("file.print", json!({})).unwrap();
    set(&mut app, "printer", json!("Office"));
    super::confirm(&mut app).unwrap();
    assert!(!asked.borrow().last().unwrap().1);
    // A custom size is the document's own size too (a 13.33 x 7.50 in slide page).
    app.run("file.print", json!({})).unwrap();
    set(&mut app, "media", json!("custom"));
    set(&mut app, "width", json!(960));
    set(&mut app, "height", json!(540));
    super::confirm(&mut app).unwrap();
    assert!(!asked.borrow().last().unwrap().1);
    // The person picks A4: the queue has to have it.
    app.run("file.print", json!({})).unwrap();
    set(&mut app, "media", json!("a4"));
    super::confirm(&mut app).unwrap();
    assert!(asked.borrow().last().unwrap().1);
    // An agent naming a paper chose it; one naming none did not.
    app.run("file.print", json!({"settings": {"media": "legal"}})).unwrap();
    assert!(asked.borrow().last().unwrap().1);
    app.run("file.print", json!({"settings": {"copies": 2}})).unwrap();
    assert!(!asked.borrow().last().unwrap().1);
}

#[test]
fn a_network_printers_grayscale_makes_the_preview_and_the_job_gray() {
    use craft_ui::printing::network::ColorMode;
    let (mut app, ..) = app_asked();
    app.run("file.print", json!({})).unwrap();
    set(&mut app, "network", json!(true));
    let mut d = dialog(&mut app).clone();
    assert!(!super::print::prints_gray(&super::print::shown_settings(&app, &d)));
    app.print_net.state.draft.color = Some(ColorMode::Monochrome);
    assert!(super::print::prints_gray(&super::print::shown_settings(&app, &d)), "the Color setting is Grayscale");
    // The dialog's own box does not leak into a network job: the printer's Color setting decides.
    app.print_net.state.draft.color = Some(ColorMode::Color);
    d.fields.insert("color".into(), json!({"grayscale": true}));
    assert!(!super::print::prints_gray(&super::print::shown_settings(&app, &d)));
}

// ---------- the network printer destination ----------

mod network {
    use std::sync::Arc;
    use std::time::Duration;

    use craft_print::wsd::testing::{FakeConfig as Config, FakePrinter, MemTransport};

    use super::*;

    const ADDRESS: &str = "127.0.0.1:5357/printer";

    fn network_app() -> (VectorcraftApp, Jobs, Arc<FakePrinter>) {
        let (mut app, jobs) = app();
        let printer = FakePrinter::new(Config::default());
        app.set_network_print_transport(Arc::new(MemTransport(printer.clone())));
        app.print_net.state.draft = craft_ui::printing::network::Draft::for_address(ADDRESS);
        app.run("file.print", json!({})).unwrap();
        set(&mut app, "network", json!(true));
        set(&mut app, "toFile", json!(false));
        (app, jobs, printer)
    }

    #[test]
    fn the_destination_draws_and_is_not_a_print_setting() {
        let (mut app, ..) = network_app();
        frame(&mut app);
        assert!(app.ui.dialog.is_some());
        let settings = super::super::print::settings(app.ui.dialog.as_ref().unwrap());
        assert!(settings.get("network").is_none() && settings.get("printer").is_none(), "{settings}");
    }

    #[test]
    fn print_sends_nothing_until_the_printer_was_queried_and_the_box_ticked() {
        let (mut app, jobs, printer) = network_app();
        let err = super::super::confirm(&mut app).unwrap_err();
        assert!(err.contains("Query the printer first"), "{err}");
        assert!(app.ui.dialog.is_some(), "the dialog stays");
        assert_eq!((printer.jobs(), jobs.borrow().len()), (0, 0));
        assert!(printer.actions().is_empty(), "nothing reached the printer: {:?}", printer.actions());
    }

    #[test]
    fn a_confirmed_job_goes_to_the_network_printer_not_the_spooler_and_the_dialog_stays_open() {
        let (mut app, jobs, printer) = network_app();
        set(&mut app, "copies", json!(2));
        set(&mut app, "media", json!("a4"));
        // Query as the Query button does.
        let transport = app.print_net.transport();
        let worker = crate::network_glue::query_printer(transport, None, Default::default());
        assert!(app.print_net.state.start_query(&worker, true));
        let ctx = egui::Context::default();
        for _ in 0..200 {
            app.print_net.poll(&ctx, true);
            if !app.print_net.state.draft.querying() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        // The query went to a worker of its own: this app's "queried" memory is empty, the job reads again.
        app.print_net.state.draft.confirmed = true;
        super::super::confirm(&mut app).expect("the job starts");
        assert!(app.ui.dialog.is_some() && app.print_net.state.draft.running());
        for _ in 0..400 {
            app.print_net.poll(&ctx, true);
            if !app.print_net.state.draft.running() {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert_eq!(printer.jobs(), 1);
        assert!(jobs.borrow().is_empty(), "the spooler was not used");
        let pdf = printer.job(1).unwrap().document.unwrap();
        // Three artboards, one copy made by the printer (it lists a copies range): three sheets in the PDF.
        assert_eq!(pages(&pdf), 3);
    }
}
