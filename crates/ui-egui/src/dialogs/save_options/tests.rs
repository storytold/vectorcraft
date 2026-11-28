use std::cell::RefCell;
use std::rc::Rc;

use vectorcraft_engine::Session;

use craft::host::PickRequest;
use craft::ui::dialog::DialogHost;

use super::super::tests_export::{answer_saves, save_panel};
use super::*;
use crate::Services;
use crate::tests_folder::Folder;

type Written = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

/// A desktop app whose save panel answers `picked` and records what it was shown.
fn desktop(picked: &'static str) -> (VectorcraftApp, Written, Rc<RefCell<Vec<PickRequest>>>) {
    let (written, picks) = (Written::default(), Rc::new(RefCell::new(vec![])));
    let (w, p) = (written.clone(), picks.clone());
    let services = Services {
        write: Some(Box::new(move |path: &str, b: &[u8]| {
            w.borrow_mut().push((path.to_string(), b.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = with_art(services);
    save_panel(&mut app, move |request| {
        p.borrow_mut().push(request.clone());
        Some(picked.to_string())
    });
    (app, written, picks)
}

/// A browser app: saves are downloads.
fn web() -> (VectorcraftApp, Written) {
    let written = Written::default();
    let w = written.clone();
    let services = Services {
        download: Some(Box::new(move |name: &str, b: &[u8]| {
            w.borrow_mut().push((name.to_string(), b.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    (with_art(services), written)
}

fn with_art(services: Services) -> VectorcraftApp {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), services));
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
    app
}

/// One headless frame: the app's pass, the dialog layer's pump, then the save panels answer.
fn frame(app: &mut VectorcraftApp) {
    let ctx = app.ui_ctx.clone().unwrap();
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| app.ui(ui));
    out.textures_delta.clear();
    super::super::host::pump(app, &ctx);
    answer_saves(app);
}

/// Runs `f` on the window's dialog host.
fn host<R>(app: &VectorcraftApp, f: impl FnOnce(&mut DialogHost) -> R) -> R {
    app.dialogs().expect("the test app has a window").with(f)
}

fn top(app: &mut VectorcraftApp) -> craft::ui::dialog::DialogId {
    host(app, |h| h.open_dialogs().last().map(|d| d.id)).expect("a dialog is open")
}

/// The open dialog's values.
fn dialog(app: &mut VectorcraftApp) -> DialogFields {
    let id = top(app);
    host(app, |h| h.get_by_id::<HostedParamDialog>(id).map(|d| d.fields().clone())).expect("a parameter dialog")
}

/// Sets a value as a person editing that row would.
fn set(app: &mut VectorcraftApp, key: &str, value: Value) {
    let (ctx, id) = (app.ui_ctx.clone().unwrap(), top(app));
    host(app, |h| h.set_json_field(&ctx, id, key, value)).unwrap_or_else(|e| panic!("{key}: {e}"));
}

/// Presses OK; whether the dialog took it.
fn ok(app: &mut VectorcraftApp) -> bool {
    let (ctx, id) = (app.ui_ctx.clone().unwrap(), top(app));
    let took = host(app, |h| h.confirm_by_id(&ctx, id));
    frame(app);
    frame(app);
    // A held OK reports `false` at the press; the app's answer then closes the dialog.
    took || host(app, |h| h.open_dialogs().iter().all(|d| d.id != id))
}

/// Runs the save command `cmd`: its save panel answers in the next frame, and the key of the dialog that follows.
fn save_as(app: &mut VectorcraftApp, cmd: &str) -> Option<String> {
    app.run(cmd, json!({})).unwrap();
    frame(app);
    host(app, |h| h.open_dialogs().last().and_then(|d| d.key.clone()))
}

fn open_dialogs(app: &mut VectorcraftApp) -> usize {
    host(app, |h| h.open_dialogs().len())
}

fn keys(f: &Format, legacy: bool) -> Vec<String> {
    let menus = Arc::new(Menus::default());
    params(f, &menus, legacy).iter().map(|p| p.key.clone()).collect()
}

/// Save As routes by format, and each format shows the rows its encoder reads.
#[test]
fn a_formats_rows_and_routing() {
    let (mut app, written, picks) = desktop("art.svg");
    assert_eq!((save_as(&mut app, "file.saveAs").as_deref(), dialog(&mut app).text("path")), (Some("svgOptions"), Some("art.svg")));
    // The save panel offers every save format, the document's own first.
    let labels: Vec<String> = picks.borrow()[0].filters.iter().map(|f| f.name.clone()).collect();
    assert_eq!(labels, ["VectorCraft", "VectorCraft Template", "PDF", "SVG", "SVG Compressed", "PDF-compatible .ai", "EPS"]);
    assert_eq!(picks.borrow()[0].file_name.as_deref(), Some("Untitled-1.vectorcraft"));
    assert!(written.borrow().is_empty(), "nothing is written before OK");
    // The Font menu holds its entries' ids.
    set(&mut app, "outlineText", json!("true"));
    assert!(ok(&mut app));
    assert_eq!(open_dialogs(&mut app), 0);
    let (path, bytes) = written.borrow()[0].clone();
    assert_eq!(path, "art.svg");
    assert!(bytes.starts_with(b"<?xml"));
    let st = app.session.active().unwrap();
    assert_eq!((st.path.as_deref(), st.format, st.is_dirty()), (Some("art.svg"), "svg", false));
    assert_eq!((&st.save_options["outlineText"], &st.save_options["styling"]), (&json!(true), &json!("css")), "the SVG Options are remembered");
    // Save writes it back as SVG, with the same options and no questions.
    app.run("shape.ellipse", json!({"x": 50, "y": 10, "width": 20, "height": 20})).unwrap();
    app.run("file.save", json!({})).unwrap();
    assert!(open_dialogs(&mut app) == 0 && picks.borrow().len() == 1);
    assert!(written.borrow()[1].1.starts_with(b"<?xml"));
    // The rows each format shows: what must be there, and what stays an agent's option.
    let rows: [(&str, bool, &[&str], &[&str]); 6] = [
        ("ai", false, &["aiVersion", "separateArtboards"], &["precision"]),
        ("ai", true, &["aiVersion", "transparentImages", "hiddenObjects", "outlineText"], &["editingData"]),
        ("ait", false, &["pdfCompatible"], &["aiVersion"]),
        ("vectorcraft", false, &["version", "compress"], &["precision"]),
        ("template", false, &["version", "compress"], &["precision"]),
        ("eps", false, &["useArtboards", "all", "range", "previewFormat", "level"], &["selectedOnly", "precision"]),
    ];
    for (id, legacy, wanted, absent) in rows {
        let names = keys(fileio::format(id).unwrap(), legacy);
        for key in wanted {
            assert!(names.iter().any(|n| n == key), "{id}: {key} in {names:?}");
        }
        for key in absent {
            assert!(!names.iter().any(|n| n == key), "{id}: no {key} in {names:?}");
        }
    }
    // EPS alone gains Use Artboards.
    for id in SAVE_FORMATS.iter().chain(TEMPLATE_FORMATS).filter(|id| **id != "eps") {
        let f = fileio::format(id).unwrap();
        assert!(!shown(f).any(|o| o.name == "useArtboards") && board_switch(f) != Some("useArtboards"), "{id}");
    }
}

#[test]
fn save_as_ai_version_7_writes_a_file_per_artboard() {
    let (mut app, written, _) = desktop("art.ai");
    app.run("artboard.new", json!({"x": 200, "y": 0, "width": 100, "height": 80, "name": "Back"})).unwrap();
    assert_eq!(save_as(&mut app, "file.saveAs").as_deref(), Some(KIND));
    let d = dialog(&mut app);
    assert_eq!((d.text("aiVersion"), d.text("transparentImages"), d.text("hiddenObjects")), (Some("pdf"), Some("flatten"), Some("layer")));
    assert_eq!(d.bool("outlineText"), Some(false));
    frame(&mut app);
    set(&mut app, "aiVersion", json!("7"));
    set(&mut app, "transparentImages", json!("omit"));
    set(&mut app, "hiddenObjects", json!("visible"));
    set(&mut app, "outlineText", json!("true"));
    frame(&mut app);
    assert!(ok(&mut app));
    assert_eq!(open_dialogs(&mut app), 0);
    let names: Vec<String> = written.borrow().iter().map(|(p, _)| p.clone()).collect();
    assert_eq!(names, ["art-Artboard-1.ai", "art-Back.ai"]);
    assert!(written.borrow().iter().all(|(_, b)| b.starts_with(b"%!PS")));
    let st = app.session.active().unwrap();
    assert_eq!((st.format, st.save_options["aiVersion"].clone()), ("ai", json!("7")), "Save remembers the version");
    assert_eq!(st.save_options["transparentImages"], json!("omit"));
    assert_eq!((st.save_options["hiddenObjects"].clone(), st.save_options["outlineText"].clone()), (json!("visible"), json!(true)));
}

/// Save Options sends a `.ai` save only the options its version reads, so a person's save gets no
/// note about an option the file doesn't read, and the other version's choices stay as the last
/// save left them.
#[test]
fn ai_save_options_send_only_the_options_the_version_reads() {
    let (mut app, _, _) = desktop("art.ai");
    for (round, legacy) in [true, false, true].into_iter().enumerate() {
        assert_eq!(save_as(&mut app, "file.saveAs").as_deref(), Some(KIND));
        frame(&mut app);
        match round {
            1 => assert_eq!(dialog(&mut app).text("aiVersion"), Some("7"), "the legacy choice is shown as last saved"),
            2 => assert_eq!(dialog(&mut app).bool("editingData"), Some(false), "the PDF-compatible choice is shown as last saved"),
            _ => {}
        }
        set(&mut app, "aiVersion", json!(if legacy { "7" } else { "pdf" }));
        if legacy {
            set(&mut app, "outlineText", json!("true"));
        } else {
            set(&mut app, "editingData", json!(false));
        }
        assert!(ok(&mut app), "round {round}");
        assert!(!app.ui.status.contains("was not used"), "round {round}: {}", app.ui.status);
    }
    let st = app.session.active().unwrap();
    assert_eq!((st.save_options["outlineText"].clone(), st.save_options["editingData"].clone()), (json!(true), json!(false)));
}

#[test]
fn legacy_ai_saves_the_artboards_checked_in_its_picker() {
    let (mut app, written, _) = desktop("art.ai");
    for (i, name) in ["Back", "Side", "Top"].iter().enumerate() {
        let x = 200.0 * (i + 1) as f64;
        app.run("artboard.new", json!({"x": x, "y": 0, "width": 100, "height": 80 + 20 * i, "name": name})).unwrap();
        app.run("shape.ellipse", json!({"x": x + 10.0, "y": 10, "width": 40, "height": 40})).unwrap();
    }
    save_as(&mut app, "file.saveAs");
    set(&mut app, "aiVersion", json!("7"));
    frame(&mut app);
    assert_eq!(boards_in(&dialog(&mut app)), Some(vec![true; 4]), "every artboard to start with");
    // Clearing them all keeps the dialog open: OK is greyed out.
    set(&mut app, "boards", json!([false, false, false, false]));
    frame(&mut app);
    assert!(!ok(&mut app), "OK is greyed out");
    assert!(open_dialogs(&mut app) == 1 && written.borrow().is_empty());
    set(&mut app, "boards", json!([true, false, true, false]));
    frame(&mut app);
    assert!(ok(&mut app));
    assert_eq!(open_dialogs(&mut app), 0);
    let names: Vec<String> = written.borrow().iter().map(|(p, _)| p.clone()).collect();
    assert_eq!(names, ["art-Artboard-1.ai", "art-Side.ai"]);
    let doc = app.session.active().unwrap().doc.clone();
    assert_eq!(
        board_files(Some(&doc), "art.ai", &[true, false, true, false]),
        [Some("art-Artboard-1.ai".into()), None, Some("art-Side.ai".into()), None]
    );
    assert_eq!(board_files(Some(&doc), "art.ai", &[false, true, false, false]), [None, Some("art.ai".into()), None, None], "one: the file itself");
    // Save As again starts from the artboards saved.
    assert_eq!(app.session.active().unwrap().save_options["range"], json!("1, 3"));
    save_as(&mut app, "file.saveAs");
    frame(&mut app);
    assert_eq!(boards_in(&dialog(&mut app)), Some(vec![true, false, true, false]));
}

/// The legacy picker's note, grid name and count are announced. The count follows the checks and
/// the grid name follows the interface language.
#[test]
fn legacy_ai_announces_its_note_grid_name_and_artboard_count() {
    let (mut app, _, _) = desktop("art.ai");
    app.run("artboard.new", json!({"x": 200, "y": 0, "width": 100, "height": 80})).unwrap();
    save_as(&mut app, "file.saveAs");
    set(&mut app, "aiVersion", json!("7"));
    let labels = |app: &mut VectorcraftApp, role: Option<egui::accesskit::Role>| -> Vec<String> {
        frame(app);
        let ctx = app.ui_ctx.clone().unwrap();
        let out = crate::testing::app_frame(app, &ctx, egui::RawInput::default());
        let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
        nodes.iter().filter(|(_, node)| role.is_none_or(|role| node.role() == role)).filter_map(|(_, node)| node.label()).map(str::to_owned).collect()
    };
    let shown = labels(&mut app, None);
    assert!(shown.iter().any(|l| l.starts_with("A legacy .ai file holds one artboard")), "{shown:?}");
    assert!(shown.iter().any(|l| l == "2 of 2 artboards"), "{shown:?}");
    set(&mut app, "boards", json!([true, false]));
    let shown = labels(&mut app, None);
    assert!(shown.iter().any(|l| l == "1 of 2 artboards") && !shown.iter().any(|l| l == "2 of 2 artboards"), "{shown:?}");

    app.session.prefs.interface_language = "fr".into();
    let shown = labels(&mut app, Some(egui::accesskit::Role::ListBox));
    assert!(shown.iter().any(|label| label == "Plans de travail"), "{shown:?}");
}

#[test]
fn save_a_copy_and_template_suggest_their_names() {
    let (mut app, written, picks) = desktop("copy.vectorcraft");
    let folder = Folder::adopt(vectorcraft_testkit::temp_dir("save-template-dialog"));
    app.session.prefs.templates_folder = folder.to_string_lossy().into_owned();
    save_as(&mut app, "file.saveCopy");
    assert_eq!(picks.borrow()[0].file_name.as_deref(), Some("Untitled-1 copy.vectorcraft"));
    assert_eq!(open_dialogs(&mut app), 0, "the native format has no options");
    assert!(vectorcraft_format::sniff(&written.borrow()[0].1));
    assert!(vectorcraft_format::pdf_content(&written.borrow()[0].1).is_some());
    assert!(app.session.active().unwrap().path.is_none() && app.session.active().unwrap().is_dirty());
    save_as(&mut app, "file.saveAsTemplate");
    let pick = &picks.borrow()[1];
    assert_eq!(pick.file_name.as_deref(), Some("Untitled-1 template.vctemplate"));
    assert_eq!(pick.directory.as_deref(), Some(folder.as_path()));
    let filters =
        PickRequest::save("template").filter("VectorCraft Template", &["vctemplate"]).filter("PDF-compatible .ait template", &["ait"]).filters;
    assert_eq!(pick.filters, filters);
    assert_eq!(written.borrow()[1].0, "copy.vctemplate", "the picked name, its extension replaced with the template's");
}

#[test]
fn save_as_template_writes_ait_when_it_is_picked() {
    let (mut app, written, _) = desktop("flyer.ait");
    let folder = Folder::adopt(vectorcraft_testkit::temp_dir("save-template-ait"));
    app.session.prefs.templates_folder = folder.to_string_lossy().into_owned();
    save_as(&mut app, "file.saveAsTemplate");
    assert_eq!(open_dialogs(&mut app), 0, "a template asks no options");
    let (path, bytes) = written.borrow()[0].clone();
    assert_eq!(path, "flyer.ait", "the picked name is kept");
    assert!(bytes.starts_with(b"%PDF") && vectorcraft_pdf::editing(&bytes).is_some(), "a PDF carrying the document");
    assert!(app.session.active().unwrap().path.is_none() && app.session.active().unwrap().is_dirty());
    // The web's dialog picks the format: .vctemplate first, .ait beside it.
    let (mut app, written) = web();
    save_as(&mut app, "file.saveAsTemplate");
    let d = dialog(&mut app);
    assert_eq!(d.text("format"), Some("template"));
    assert_eq!(heading(true, "file.saveAsTemplate", false, "template"), "Save as Template");
    frame(&mut app);
    set(&mut app, "format", json!("ait"));
    frame(&mut app);
    let d = dialog(&mut app);
    assert_eq!((d.text("path"), flag(&d, "pdfCompatible")), (Some("Untitled-1 template.ait"), Some(true)));
    assert!(ok(&mut app));
    let (name, bytes) = written.borrow()[0].clone();
    assert_eq!(name, "Untitled-1 template.ait");
    assert!(bytes.starts_with(b"%PDF"));
}

/// A name typed in the Save as Template panel with a Save As format's extension has it
/// replaced with the template's, as Save As replaces the extension of a format it doesn't write.
#[test]
fn save_as_template_replaces_a_save_formats_extension() {
    let folder = Folder::adopt(vectorcraft_testkit::temp_dir("save-template-ext"));
    for (picked, saved) in [("card.pdf", "card.vctemplate"), ("card.vectorcraft", "card.vctemplate"), ("card.svg", "card.vctemplate")] {
        let (mut app, written, _) = desktop(picked);
        app.session.prefs.templates_folder = folder.to_string_lossy().into_owned();
        save_as(&mut app, "file.saveAsTemplate");
        let (path, bytes) = written.borrow()[0].clone();
        assert_eq!(path, saved, "typed {picked}");
        assert!(vectorcraft_format::load(&bytes).is_ok_and(|d| d.template), "typed {picked}: a native template");
    }
}

/// Picking another format on the web removes the old format's options and brings the new one's.
#[test]
fn web_save_as_names_the_file_and_picks_the_format() {
    let (mut app, written) = web();
    save_as(&mut app, "file.saveAs");
    let d = dialog(&mut app);
    assert_eq!((d.text("path"), d.text("format")), (Some("Untitled-1.vectorcraft"), Some("vectorcraft")));
    assert_eq!(heading(true, "file.saveAs", false, "vectorcraft"), "Save As");
    assert!(d.contains("version"));
    frame(&mut app);
    set(&mut app, "format", json!("pdf"));
    frame(&mut app);
    let d = dialog(&mut app);
    assert_eq!((d.text("path"), d.text("format")), (Some("Untitled-1.pdf"), Some("pdf")));
    assert!(!d.contains("version"), "the old format's options are gone");
    assert_eq!(shown(fileio::format("pdf").unwrap()).count(), 0, "PDF options have their own dialog");
    // OK goes on to the Save PDF dialog, whose OK saves.
    assert!(ok(&mut app));
    assert!(written.borrow().is_empty());
    assert_eq!(host(&app, |h| h.open_dialogs().last().and_then(|d| d.key.clone())).as_deref(), Some("pdf-export"));
    let job = app.save_pdf.as_ref().and_then(|o| o.job.as_ref()).and_then(|j| j.path.clone());
    assert_eq!(job.as_deref(), Some("Untitled-1.pdf"), "Save PDF writes under the name Save As gave");
    assert!(ok(&mut app));
    let (name, bytes) = written.borrow()[0].clone();
    assert_eq!(name, "Untitled-1.pdf");
    assert!(bytes.starts_with(b"%PDF"));
    assert_eq!(app.session.active().unwrap().format, "pdf");
    // A blank name isn't saved.
    save_as(&mut app, "file.saveCopy");
    set(&mut app, "path", json!(" "));
    assert!(!ok(&mut app) || open_dialogs(&mut app) == 1);
}

#[test]
fn save_as_pdf_asks_with_the_save_pdf_dialog_and_remembers_its_options() {
    let (mut app, written, _) = desktop("art.pdf");
    assert_eq!(save_as(&mut app, "file.saveAs").as_deref(), Some("pdf-export"));
    set(&mut app, "compatibility", json!("1.5"));
    set(&mut app, "createLayers", json!(true));
    assert!(ok(&mut app));
    assert_eq!(open_dialogs(&mut app), 0);
    assert_eq!(written.borrow()[0].0, "art.pdf");
    assert!(written.borrow()[0].1.starts_with(b"%PDF-1.5"));
    let st = app.session.active().unwrap();
    assert_eq!((st.path.as_deref(), st.format, st.is_dirty()), (Some("art.pdf"), "pdf", false));
    assert_eq!((st.save_options["compatibility"].clone(), st.save_options["createLayers"].clone()), (json!("1.5"), json!(true)));
    // Save writes the PDF again with those options, without asking.
    app.run("shape.ellipse", json!({"x": 50, "y": 10, "width": 20, "height": 20})).unwrap();
    app.run("file.save", json!({})).unwrap();
    assert!(open_dialogs(&mut app) == 0 && written.borrow()[1].1.starts_with(b"%PDF-1.5"));
    // Save a Copy as PDF starts from them.
    app.run("file.saveCopy", json!({"format": "pdf"})).unwrap();
    // The save panel answers after the command returns; the dialog opens in the frame after.
    frame(&mut app);
    let compatibility = host(&app, |h| h.open_dialogs().pop().and_then(|d| d.fields)).map(|f| f["compatibility"].clone());
    assert_eq!(compatibility, Some(json!("1.5")));
}

/// The `%%BoundingBox` line of an EPS file.
fn bounding_box(eps: &[u8]) -> String {
    let ps = std::str::from_utf8(vectorcraft_eps::sections(eps).unwrap().0).unwrap();
    ps.lines().find(|l| l.starts_with("%%BoundingBox:")).expect("a bounding box").to_string()
}

#[test]
fn eps_save_as_shows_use_artboards_off_and_writes_the_art_or_the_artboard() {
    let (mut app, written, _) = desktop("art.eps");
    save_as(&mut app, "file.saveAs");
    assert_eq!(dialog(&mut app).bool("useArtboards"), Some(false), "Use Artboards starts off");
    assert!(ok(&mut app));
    assert_eq!(open_dialogs(&mut app), 0);
    {
        let saved = written.borrow();
        assert_eq!(saved[0].0, "art.eps");
        assert!(vectorcraft_eps::native(&saved[0].1).is_some());
        // The box Export writes of the visible art, not the artboard.
        let exported = app.session.execute("document.export", &json!({"format": "eps"})).unwrap();
        let exported = vectorcraft_format::base64_decode(exported["dataBase64"].as_str().unwrap()).unwrap();
        assert_eq!(bounding_box(&saved[0].1), bounding_box(&exported));
        assert_ne!(bounding_box(&saved[0].1), "%%BoundingBox: 0 0 100 80");
    }
    assert_eq!(app.session.active().unwrap().format, "eps");
    // Checked, the artboard is the page, and an artboard remembered from an earlier save doesn't
    // narrow All.
    save_as(&mut app, "file.saveAs");
    set(&mut app, "useArtboards", json!(true));
    assert!(ok(&mut app));
    {
        let saved = written.borrow();
        assert_eq!(saved.len(), 2, "one artboard: one file");
        assert_eq!(bounding_box(&saved[1].1), "%%BoundingBox: 0 0 100 80");
    }
    // A save that named an artboard without Use Artboards (as document.save {artboard} does) had
    // it on: the dialog shows it on, on that artboard, and OK writes the artboard again.
    let st = app.session.active_mut().unwrap();
    st.save_options.remove("useArtboards");
    st.save_options.insert("artboard".into(), json!(0));
    save_as(&mut app, "file.saveAs");
    let d = dialog(&mut app);
    assert!(d.bool("useArtboards") == Some(true) && d.text("range") == Some("1"));
    assert!(ok(&mut app));
    let saved = written.borrow();
    assert_eq!(saved.len(), 3);
    assert_eq!(bounding_box(&saved[2].1), "%%BoundingBox: 0 0 100 80");
}

/// Every option the save options dialog shows, in every format it serves, has a written label, so
/// none is shown as its name split into words.
#[test]
fn save_options_have_written_labels() {
    for id in SAVE_FORMATS.iter().chain(TEMPLATE_FORMATS) {
        let f = fileio::format(id).unwrap();
        for o in shown(f) {
            assert!(written_label(o).is_some(), "{id}: {} has no written label", o.name);
        }
    }
}

/// A pick from an EPS menu saves the option's value: the id, the level as a number, a saved
/// flattener preset by its name. The file is written with them.
#[test]
fn picking_eps_menu_items_saves_their_values() {
    let (mut app, written, _) = desktop("art.eps");
    app.session.prefs.flattener_presets.push(vectorcraft_engine::cmd::FlattenerPreset {
        name: "Mine".into(),
        options: vectorcraft_engine::cmd::FlattenOptions::preset("low").unwrap(),
    });
    save_as(&mut app, "file.saveAs");
    // The built-in presets come first, by id; a saved one follows by name.
    let f = fileio::format("eps").unwrap();
    let o = f.options.iter().find(|o| o.name == "flattenerPreset").unwrap();
    let c = choices(&app, f, o).unwrap();
    assert_eq!(c.values, [json!("high"), json!("medium"), json!("low"), json!("Mine")]);
    assert_eq!(c.builtins, 3);
    for (name, pick, value) in [
        ("previewFormat", "none", json!("none")),
        ("overprints", "discard", json!("discard")),
        ("flattenerPreset", "Mine", json!("Mine")),
        ("level", "2", json!(2)),
    ] {
        set(&mut app, name, json!(pick));
        assert_eq!(dialog(&mut app).text(name), Some(pick), "{name}");
        let _ = value;
    }
    assert!(ok(&mut app), "saved: {}", app.ui.status);
    assert_eq!(open_dialogs(&mut app), 0);
    let saved = &app.session.active().unwrap().save_options;
    for (name, value) in [("previewFormat", json!("none")), ("overprints", json!("discard")), ("flattenerPreset", json!("Mine")), ("level", json!(2))]
    {
        assert_eq!(saved[name], value, "{name}");
    }
    let bytes = written.borrow()[0].1.clone();
    let (ps, preview) = vectorcraft_eps::sections(&bytes).unwrap();
    assert!(preview.is_none(), "no preview");
    assert!(String::from_utf8_lossy(ps).contains("%%LanguageLevel: 2"));
}

/// Save As › EPS: Transparent and Opaque apply to the color TIFF preview only.
#[test]
fn eps_save_transparency_follows_the_color_preview() {
    let (mut app, _, _) = desktop("art.eps");
    save_as(&mut app, "file.saveAs");
    let menus = Arc::new(Menus::default());
    let f = fileio::format("eps").unwrap();
    let rows = params(f, &menus, false);
    let row = rows.iter().find(|p| p.key == "transparentPreview").expect("the Transparent row");
    for (preview, enabled) in [("tiffColor", true), ("none", false), ("tiffBw", false)] {
        set(&mut app, "previewFormat", json!(preview));
        assert_eq!(row.enabled(&dialog(&mut app)), enabled, "{preview}");
    }
    let fonts = rows.iter().find(|p| p.key == "embedFonts").expect("the Embed Fonts row");
    assert!(!fonts.enabled(&dialog(&mut app)), "Embed Fonts is fixed on");
    assert_eq!(dialog(&mut app).bool("embedFonts"), Some(true));
}

/// #4931: a native file's Version is a menu of the versions VectorCraft writes, the current one
/// first and chosen, each named by the releases that read it. An older version without
/// compression saves plain, whatever Use Compression held.
#[test]
fn native_version_is_a_menu_of_the_versions_vectorcraft_writes() {
    let listed: Vec<u32> = NATIVE_VERSIONS.iter().map(|(n, _)| *n).collect();
    let writable: Vec<u32> = (vectorcraft_format::MIN_VERSION..=vectorcraft_format::VERSION).rev().collect();
    assert_eq!(listed, writable, "every version VectorCraft writes, newest first");
    let (mut app, written, _) = desktop("art.vectorcraft");
    assert_eq!(save_as(&mut app, "file.saveAs").as_deref(), Some(KIND));
    assert_eq!(native_version_of(&dialog(&mut app)), vectorcraft_format::VERSION);
    // Use Compression takes input only while the version can be compressed: set it first.
    set(&mut app, "compress", json!(true));
    set(&mut app, "version", json!("2"));
    assert!(uncompressed(&dialog(&mut app)));
    assert!(ok(&mut app), "saved: {}", app.ui.status);
    assert_eq!(open_dialogs(&mut app), 0);
    let (_, bytes) = written.borrow()[0].clone();
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(200)]).to_string();
    assert!(head.contains("\"version\": 2") || head.contains("\"version\":2"), "{head}");
}

/// #4933: a save refused before anything is written (here a version agents can still set) says
/// so in the "Can't save" alert, in the reason's words without the command's, and writes nothing.
#[test]
fn a_refused_save_says_so_in_an_alert() {
    let (mut app, written, _) = desktop("art.vectorcraft");
    assert_eq!(save_as(&mut app, "file.saveAs").as_deref(), Some(KIND));
    // The Version row lists only the versions there are: automation's "55" is refused as it is set. The save's own
    // refusal is reached with the value put in the dialog directly, as a remembered version gone since would be.
    let (ctx, id) = (app.ui_ctx.clone().unwrap(), top(&mut app));
    assert!(host(&app, |h| h.set_json_field(&ctx, id, "version", json!("55"))).is_err());
    host(&app, |h| h.get_mut_by_id::<HostedParamDialog>(id).map(|d| d.set_field("version", FieldValue::from("55")))).expect("a parameter dialog");
    frame(&mut app);
    ok(&mut app);
    // The alert opens over the options and names the reason.
    let alert = host(&app, |h| h.open_dialogs().pop()).expect("the alert is on the host");
    assert_ne!(alert.key.as_deref(), Some(KIND));
    let shown = alert.fields.map(|f| f.to_string()).unwrap_or_default();
    assert!(shown.contains("can't save version 55"), "{shown}");
    assert!(written.borrow().is_empty());
    let st = app.session.active().unwrap();
    assert!(st.is_dirty() && st.path.is_none(), "not saved");
}
