use std::cell::RefCell;
use std::rc::Rc;

use vectorcraft_engine::Session;

use super::*;
use crate::{FilePick, Services, theme};

type Written = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

/// A desktop app whose save panel answers `picked` and records what it was shown.
fn desktop(picked: &'static str) -> (VectorcraftApp, Written, Rc<RefCell<Vec<FilePick>>>) {
    let (written, picks) = (Written::default(), Rc::new(RefCell::new(vec![])));
    let (w, p) = (written.clone(), picks.clone());
    let services = Services {
        pick_save: Some(Box::new(move |f: &FilePick| {
            p.borrow_mut().push(f.clone());
            Some(picked.to_string())
        })),
        write: Some(Box::new(move |path: &str, b: &[u8]| {
            w.borrow_mut().push((path.to_string(), b.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    (with_art(services), written, picks)
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
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
    app
}

/// One headless frame of the dialog layer; `enter` presses OK.
fn frame(app: &mut VectorcraftApp, enter: bool) {
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    let events = if enter {
        vec![egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]
    } else {
        vec![]
    };
    let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| super::super::show(app, ui.ctx()));
    out.textures_delta.clear();
}

fn dialog(app: &VectorcraftApp) -> &Dialog {
    app.ui.dialog.as_ref().expect("a dialog is open")
}

#[test]
fn save_as_svg_opens_svg_options_then_writes_svg() {
    let (mut app, written, picks) = desktop("art.svg");
    let r = app.run("file.saveAs", json!({})).unwrap();
    assert_eq!((r["pending"].as_str(), r["path"].as_str()), (Some("svgOptions"), Some("art.svg")));
    // The save panel offers every save format, the document's own first.
    let pick = &picks.borrow()[0];
    let labels: Vec<&str> = pick.filters.iter().map(|f| f.0.as_str()).collect();
    assert_eq!(labels, ["VectorCraft", "VectorCraft Template", "PDF", "SVG", "SVG Compressed", "PDF-compatible .ai"]);
    assert_eq!(pick.name, "Untitled-1.vectorcraft");
    let d = dialog(&app);
    assert_eq!((d.kind.as_str(), d.str("path").as_str(), d.str("mode").as_str()), ("svgOptions", "art.svg", "save"));
    assert!(written.borrow().is_empty(), "nothing is written before OK");
    frame(&mut app, false);
    assert!(app.ui.dialog.is_some(), "drawing keeps it open");
    app.ui.dialog.as_mut().unwrap().fields.insert("outlineText".into(), json!(true));
    frame(&mut app, true);
    assert!(app.ui.dialog.is_none());
    let (path, bytes) = written.borrow()[0].clone();
    assert_eq!(path, "art.svg");
    assert!(bytes.starts_with(b"<?xml"));
    let st = app.session.active().unwrap();
    assert_eq!((st.path.as_deref(), st.format, st.is_dirty()), (Some("art.svg"), "svg", false));
    assert_eq!((&st.save_options["outlineText"], &st.save_options["styling"]), (&json!(true), &json!("css")), "the SVG Options are remembered");
    assert!(app.ui.status.contains("SVG keeps the artwork"), "{}", app.ui.status);
    // Save writes it back as SVG, with the same options and no questions.
    assert!(!crate::menus::enabled(&app, "file.save"), "nothing to save");
    app.run("shape.ellipse", json!({"x": 50, "y": 10, "width": 20, "height": 20})).unwrap();
    assert!(crate::menus::enabled(&app, "file.save"));
    app.run("file.save", json!({})).unwrap();
    assert!(app.ui.dialog.is_none() && picks.borrow().len() == 1);
    assert!(written.borrow()[1].1.starts_with(b"<?xml"));
}

#[test]
fn save_as_ai_version_7_writes_a_file_per_artboard() {
    let (mut app, written, _) = desktop("art.ai");
    app.run("artboard.new", json!({"x": 200, "y": 0, "width": 100, "height": 80, "name": "Back"})).unwrap();
    assert_eq!(app.run("file.saveAs", json!({})).unwrap()["pending"], KIND);
    assert_eq!(dialog(&app).str("aiVersion"), "pdf", "PDF-compatible unless asked");
    assert_eq!(dialog(&app).str("transparentImages"), "flatten");
    assert_eq!(dialog(&app).str("hiddenObjects"), "layer");
    assert!(!dialog(&app).bool("outlineText"));
    frame(&mut app, false);
    app.ui.dialog.as_mut().unwrap().fields.insert("aiVersion".into(), json!("7"));
    app.ui.dialog.as_mut().unwrap().fields.insert("transparentImages".into(), json!("omit"));
    app.ui.dialog.as_mut().unwrap().fields.insert("hiddenObjects".into(), json!("visible"));
    app.ui.dialog.as_mut().unwrap().fields.insert("outlineText".into(), json!(true));
    frame(&mut app, false);
    frame(&mut app, true);
    assert!(app.ui.dialog.is_none());
    let names: Vec<String> = written.borrow().iter().map(|(p, _)| p.clone()).collect();
    assert_eq!(names, ["art-Artboard-1.ai", "art-Back.ai"]);
    assert!(written.borrow().iter().all(|(_, b)| b.starts_with(b"%!PS")));
    let st = app.session.active().unwrap();
    assert_eq!((st.format, st.save_options["aiVersion"].clone()), ("ai", json!("7")), "Save remembers the version");
    assert_eq!(st.save_options["transparentImages"], json!("omit"));
    assert_eq!((st.save_options["hiddenObjects"].clone(), st.save_options["outlineText"].clone()), (json!("visible"), json!(true)));
}

/// An Illustrator file opened, then saved as `.ai`: the options dialog stays open through the
/// Enter that closed the save panel, and only an Enter after it is let go confirms it.
#[test]
fn legacy_ai_saves_the_artboards_checked_in_its_picker() {
    let (mut app, written, _) = desktop("art.ai");
    for (i, name) in ["Back", "Side", "Top"].iter().enumerate() {
        let x = 200.0 * (i + 1) as f64;
        app.run("artboard.new", json!({"x": x, "y": 0, "width": 100, "height": 80 + 20 * i, "name": name})).unwrap();
        app.run("shape.ellipse", json!({"x": x + 10.0, "y": 10, "width": 40, "height": 40})).unwrap();
    }
    app.run("file.saveAs", json!({})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("aiVersion".into(), json!("7"));
    frame(&mut app, false);
    assert_eq!(dialog(&app).fields["boards"], json!([true, true, true, true]), "every artboard to start with");
    if let Some(dir) = std::env::var_os("CRAFT_UI_SAVE_OPTIONS_SHOT") {
        let mut ready = false;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 760.0)).wgpu().build_ui_state(
            move |ui, app: &mut VectorcraftApp| {
                if !ready {
                    theme::install_fonts(ui.ctx());
                    theme::apply(ui.ctx(), theme::Brightness::MediumDark);
                    ready = true;
                    return;
                }
                super::super::show(app, ui.ctx());
            },
            app,
        );
        h.run_steps(6);
        h.render().unwrap().save(std::path::Path::new(&dir).join("legacy-ai-picker.png")).unwrap();
        let fields = &mut h.state_mut().ui.dialog.as_mut().unwrap().fields;
        fields.insert("path".into(), json!("desktop-ideas1-vc5.ai"));
        fields.insert("boards".into(), json!([false, false, false, false]));
        h.run_steps(2);
        h.state_mut().ui.dialog.as_mut().unwrap().fields.insert("boards".into(), json!([true, true, false, true]));
        h.run_steps(2);
        h.render().unwrap().save(std::path::Path::new(&dir).join("legacy-ai-picker-long.png")).unwrap();
        h.state_mut().ui.dialog.as_mut().unwrap().fields.insert("boards".into(), json!([false, false, false, false]));
        h.run_steps(2);
        h.render().unwrap().save(std::path::Path::new(&dir).join("legacy-ai-picker-none.png")).unwrap();
        app = h.into_state();
        app.ui.dialog.as_mut().unwrap().fields.insert("path".into(), json!("art.ai"));
    }
    // Clearing them all keeps the dialog open.
    app.ui.dialog.as_mut().unwrap().fields.insert("boards".into(), json!([false, false, false, false]));
    let d = dialog(&app).clone();
    assert!(!can_save(&app, &d), "OK is greyed out");
    frame(&mut app, true);
    assert!(app.ui.dialog.is_some(), "Enter does nothing");
    assert!(confirm(&mut app, &d).is_err());
    assert!(written.borrow().is_empty());
    app.ui.dialog.as_mut().unwrap().fields.insert("boards".into(), json!([true, false, true, false]));
    frame(&mut app, true);
    assert!(app.ui.dialog.is_none());
    let names: Vec<String> = written.borrow().iter().map(|(p, _)| p.clone()).collect();
    assert_eq!(names, ["art-Artboard-1.ai", "art-Side.ai"]);
    assert_eq!(board_files_of(&app, [true, false, true, false]), [Some("art-Artboard-1.ai".into()), None, Some("art-Side.ai".into()), None]);
    assert_eq!(board_files_of(&app, [false, true, false, false]), [None, Some("art.ai".into()), None, None], "one: the file itself");
    // Save As again starts from the artboards saved.
    assert_eq!(app.session.active().unwrap().save_options["range"], json!("1, 3"));
    app.run("file.saveAs", json!({})).unwrap();
    frame(&mut app, false);
    assert_eq!(dialog(&app).fields["boards"], json!([true, false, true, false]));
}

fn board_files_of(app: &VectorcraftApp, boards: [bool; 4]) -> Vec<Option<String>> {
    let d = Dialog { kind: KIND.into(), fields: [("path".to_string(), json!("art.ai"))].into_iter().collect() };
    board_files(app, &d, &boards)
}

#[test]
fn the_enter_that_closed_the_save_panel_does_not_confirm_the_options() {
    let (mut app, written, _) = desktop("again.ai");
    let dir = vectorcraft_testkit::temp_dir("ai-options-enter");
    let orig = dir.join("orig.ai");
    std::fs::write(&orig, vectorcraft_testkit::ai::ai(&vectorcraft_testkit::ai::sample_data(), vectorcraft_testkit::ai::page_pdf())).unwrap();
    app.run("document.open", json!({"path": orig.to_string_lossy()})).unwrap();
    assert_eq!(app.run("file.saveAs", json!({})).unwrap()["pending"], KIND);
    frame(&mut app, true);
    assert!(app.ui.dialog.is_some(), "the panel's Enter doesn't confirm it");
    assert!(written.borrow().is_empty());
    // Let go, then pressed again: OK.
    frame(&mut app, false);
    frame(&mut app, true);
    assert!(app.ui.dialog.is_none());
    assert_eq!(written.borrow()[0].0, "again.ai");
}

#[test]
fn save_a_copy_and_template_suggest_their_names() {
    let (mut app, written, picks) = desktop("copy.vectorcraft");
    let folder = vectorcraft_testkit::temp_dir("save-template-dialog");
    app.session.prefs.templates_folder = folder.to_string_lossy().into_owned();
    app.run("file.saveCopy", json!({})).unwrap();
    assert_eq!(picks.borrow()[0].name, "Untitled-1 copy.vectorcraft");
    assert!(app.ui.dialog.is_none(), "the native format has no options");
    assert!(vectorcraft_format::sniff(&written.borrow()[0].1));
    assert!(app.session.active().unwrap().path.is_none() && app.session.active().unwrap().is_dirty());
    app.run("file.saveAsTemplate", json!({})).unwrap();
    let pick = &picks.borrow()[1];
    assert_eq!(pick.name, "Untitled-1 template.vctemplate");
    assert_eq!(pick.folder.as_deref().map(std::path::Path::new), Some(folder.as_path()));
    assert_eq!(
        pick.filters,
        crate::FilePick::filters([("VectorCraft Template", &["vctemplate"][..]), ("PDF-compatible .ait template", &["ait"][..])])
    );
    assert_eq!(written.borrow()[1].0, "copy.vctemplate", "the picked name, its extension replaced with the template's");
}

#[test]
fn save_as_template_writes_ait_when_it_is_picked() {
    let (mut app, written, _) = desktop("flyer.ait");
    app.session.prefs.templates_folder = vectorcraft_testkit::temp_dir("save-template-ait").to_string_lossy().into_owned();
    app.run("file.saveAsTemplate", json!({})).unwrap();
    assert!(app.ui.dialog.is_none(), "a template asks no options");
    let (path, bytes) = written.borrow()[0].clone();
    assert_eq!(path, "flyer.ait", "the picked name is kept");
    assert!(bytes.starts_with(b"%PDF") && vectorcraft_pdf::editing(&bytes).is_some(), "a PDF carrying the document");
    assert!(app.session.active().unwrap().path.is_none() && app.session.active().unwrap().is_dirty());
    // The web's dialog picks the format: .vctemplate first, .ait beside it.
    let (mut app, written) = web();
    app.run("file.saveAsTemplate", json!({})).unwrap();
    assert_eq!((heading(dialog(&app)).as_str(), dialog(&app).str("format").as_str()), ("Save as Template", "template"));
    assert_eq!(template_labels(), ["VectorCraft Template", "PDF-compatible .ait template"]);
    frame(&mut app, false);
    let mut d = app.ui.dialog.take().unwrap();
    switch_format(&mut app, &mut d, "ait");
    assert_eq!((d.str("path").as_str(), d.fields["pdfCompatible"].clone()), ("Untitled-1 template.ait", json!(true)));
    app.ui.dialog = Some(d);
    frame(&mut app, true);
    let (name, bytes) = written.borrow()[0].clone();
    assert_eq!(name, "Untitled-1 template.ait");
    assert!(bytes.starts_with(b"%PDF"));
}

/// The `.ait` template's options have no Version row: Save as Template writes it
/// PDF-compatible only. A `.ai` file's options keep it.
#[test]
fn the_ait_template_options_have_no_version_row() {
    let names = |id: &str| shown(fileio::format(id).unwrap()).map(|o| o.name).collect::<Vec<_>>();
    assert!(names("ai").contains(&"aiVersion"), "{:?}", names("ai"));
    assert!(!names("ait").contains(&"aiVersion"), "{:?}", names("ait"));
}

/// A name typed in the Save as Template panel with a Save As format's extension has it
/// replaced with the template's, as Save As replaces the extension of a format it doesn't write.
#[test]
fn save_as_template_replaces_a_save_formats_extension() {
    for (picked, saved) in [("card.pdf", "card.vctemplate"), ("card.vectorcraft", "card.vctemplate"), ("card.svg", "card.vctemplate")] {
        let (mut app, written, _) = desktop(picked);
        app.session.prefs.templates_folder = vectorcraft_testkit::temp_dir("save-template-ext").to_string_lossy().into_owned();
        app.run("file.saveAsTemplate", json!({})).unwrap();
        let (path, bytes) = written.borrow()[0].clone();
        assert_eq!(path, saved, "typed {picked}");
        assert!(vectorcraft_format::load(&bytes).is_ok_and(|d| d.template), "typed {picked}: a native template");
    }
}

#[test]
fn web_save_as_names_the_file_and_picks_the_format() {
    let (mut app, written) = web();
    app.run("file.saveAs", json!({})).unwrap();
    let d = dialog(&app);
    assert_eq!((heading(d).as_str(), d.str("path").as_str(), d.str("format").as_str()), ("Save As", "Untitled-1.vectorcraft", "vectorcraft"));
    frame(&mut app, false);
    let mut d = app.ui.dialog.take().unwrap();
    switch_format(&mut app, &mut d, "pdf");
    assert_eq!((d.str("path").as_str(), d.str("format").as_str()), ("Untitled-1.pdf", "pdf"));
    assert_eq!(shown(fileio::format("pdf").unwrap()).count(), 0, "PDF options have their own dialog");
    app.ui.dialog = Some(d);
    frame(&mut app, true);
    // OK goes on to the Save PDF dialog, whose OK saves.
    assert_eq!((dialog(&app).kind.as_str(), dialog(&app).str("path").as_str()), ("savePdf", "Untitled-1.pdf"));
    assert!(written.borrow().is_empty());
    super::super::confirm(&mut app).unwrap();
    let (name, bytes) = written.borrow()[0].clone();
    assert_eq!(name, "Untitled-1.pdf");
    assert!(bytes.starts_with(b"%PDF"));
    assert_eq!(app.session.active().unwrap().format, "pdf");
    // A blank name isn't saved.
    app.run("file.saveCopy", json!({})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("path".into(), json!(" "));
    assert!(super::super::confirm(&mut app).is_err());
}

#[test]
fn save_as_pdf_asks_with_the_save_pdf_dialog_and_remembers_its_options() {
    let (mut app, written, _) = desktop("art.pdf");
    assert_eq!(app.run("file.saveAs", json!({})).unwrap()["pending"], "savePdf");
    let d = app.ui.dialog.as_mut().unwrap();
    assert_eq!((d.str("path").as_str(), d.str("__save").as_str()), ("art.pdf", "file.saveAs"));
    d.fields.insert("compatibility".into(), json!("1.5"));
    d.fields.insert("createLayers".into(), json!(true));
    super::super::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert!(written.borrow()[0].1.starts_with(b"%PDF-1.5"));
    let st = app.session.active().unwrap();
    assert_eq!((st.path.as_deref(), st.format, st.is_dirty()), (Some("art.pdf"), "pdf", false));
    assert_eq!((st.save_options["compatibility"].clone(), st.save_options["createLayers"].clone()), (json!("1.5"), json!(true)));
    // Save writes the PDF again with those options, without asking.
    app.run("shape.ellipse", json!({"x": 50, "y": 10, "width": 20, "height": 20})).unwrap();
    app.run("file.save", json!({})).unwrap();
    assert!(app.ui.dialog.is_none() && written.borrow()[1].1.starts_with(b"%PDF-1.5"));
    // Save a Copy as PDF starts from them.
    app.run("file.saveCopy", json!({"format": "pdf"})).unwrap();
    assert_eq!(dialog(&app).str("compatibility"), "1.5");
    assert_eq!(dialog(&app).str("__save"), "file.saveCopy");
}
