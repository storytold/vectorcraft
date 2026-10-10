//! A save format's options before the file is written; on the web, where there is no save panel,
//! also the Save As dialog that names the file and picks the format (SVG and PDF then go on to
//! their own dialogs). The rows come from the format's options in the engine's format table, so a
//! format gains rows as its encoder gains options.
//!
//! Fields: `__action` (the save command it finishes), `__pick` (file name and format
//! editable), `path`, `format`, and one field per option (agents set them with `ui.dialog.set`).
//! Native and `.ai` files show their save options (VectorCraft Options after Save As): each
//! artboard to a separate file, with `all` (true: every artboard) or `range`, Include Linked Files,
//! Embed ICC Profiles, Create PDF-Compatible File and Use Compression.

use std::sync::OnceLock;

use serde_json::{Map, Value, json};
use vectorcraft_engine::cmd::fileio::{self, ArtboardPick, Format, FormatOption, SAVE_FORMATS, SaveMode};

use super::{DialogResult, DialogSpec, form};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, io, widgets};

/// Dialog kind.
pub const KIND: &str = "saveOptions";

pub(super) const SPEC: DialogSpec = DialogSpec { heading, body, confirm, min_width: 380.0, ..DialogSpec::FORM };

fn heading(d: &Dialog) -> String {
    if d.bool("__pick") {
        return match SaveMode::of(&d.str("__action")) {
            Some(SaveMode::Copy) => tl!("Save a Copy"),
            Some(SaveMode::Template) => tl!("Save as Template"),
            _ => tl!("Save As"),
        }
        .into();
    }
    crate::i18n::fmt(tl!("{format} Options"), &[("format", fileio::format(&d.str("format")).map_or("", |f| f.label))])
}

/// `f`'s options with the values a save would use (`file.formatOptions`: as last saved in that
/// format, else the defaults). Text options start empty rather than null.
pub(crate) fn option_values(app: &mut VectorcraftApp, f: &Format) -> Map<String, Value> {
    let v = app.session.execute("file.formatOptions", &json!({ "format": f.id })).unwrap_or_default();
    f.options
        .iter()
        .map(|o| {
            let value = v["options"][o.name]["value"].clone();
            (o.name.to_string(), if o.ty == "string" && value.is_null() { json!("") } else { value })
        })
        .collect()
}

/// Formats whose options have a dialog of their own (SVG Options, Save PDF), which follows this
/// one.
fn own_dialog(f: &Format) -> bool {
    matches!(f.id, "svg" | "svgz" | "pdf")
}

/// Does `f` save each artboard to a separate file on request (native and `.ai` files)? Its
/// `range` then shows under that checkbox.
fn separates(f: &Format) -> bool {
    f.options.iter().any(|o| o.name == "separateArtboards")
}

/// The options the dialog shows: arrays and objects are for agents, and a page range replaces the
/// single artboard choice; none for a format with its own dialog.
fn shown(f: &Format) -> impl Iterator<Item = &'static FormatOption> {
    let range = f.options.iter().any(|o| o.name == "range");
    let separate = separates(f);
    let options = if own_dialog(f) { &[][..] } else { f.options };
    options.iter().filter(move |o| {
        !(matches!(o.ty, "array" | "object")
            || (range && o.name == "artboard")
            || (separate && o.name == "range")
            || (f.id == "ai" && o.name == "useArtboards"))
    })
}

/// An option's label: the save options as the reference app's options dialog names them, the
/// others from their names.
fn label(o: &FormatOption) -> String {
    match o.name {
        "separateArtboards" => tl!("Save each artboard to a separate file").into(),
        "includeLinked" => tl!("Include Linked Files").into(),
        "embedProfiles" => tl!("Embed ICC Profiles").into(),
        "pdfCompatible" => tl!("Create PDF-Compatible File").into(),
        "compress" => tl!("Use Compression").into(),
        _ => form::humanize(o.name),
    }
}

/// Every artboard (the All choice under Save each artboard to a separate file): `all` when set,
/// else no range given.
fn all_artboards(d: &Dialog) -> bool {
    d.fields.get("all").and_then(Value::as_bool).unwrap_or_else(|| d.str("range").trim().is_empty())
}

/// All or Range, under Save each artboard to a separate file (enabled while it is on).
fn artboard_range(ui: &mut egui::Ui, d: &mut Dialog) {
    ui.label("");
    ui.add_enabled_ui(d.bool("separateArtboards"), |ui| {
        ui.horizontal(|ui| {
            ui.add_space(22.0);
            let mut all = all_artboards(d);
            if ui.radio_value(&mut all, true, tl!("All")).changed() | ui.radio_value(&mut all, false, tl!("Range:")).changed() {
                d.fields.insert("all".into(), json!(all));
            }
            let mut range = d.str("range");
            let field = egui::TextEdit::singleline(&mut range).desired_width(90.0).hint_text("1-3, 5");
            if ui.add_enabled(!all, field).changed() {
                d.fields.insert("range".into(), json!(range));
            }
        });
    });
    ui.end_row();
}

/// The Save As format menu (labels in [`SAVE_FORMATS`] order).
fn save_labels() -> &'static [&'static str] {
    static LABELS: OnceLock<Vec<&'static str>> = OnceLock::new();
    LABELS.get_or_init(|| SAVE_FORMATS.iter().filter_map(|id| fileio::format(id)).map(|f| f.label).collect())
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    let Some(f) = fileio::format(&d.str("format")) else { return false };
    let mut switch = None;
    egui::Grid::new("save-options").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
        if d.bool("__pick") {
            form::field_w(ui, d, "path", tl!("File Name:"), 240.0);
            if SaveMode::of(&d.str("__action")) != Some(SaveMode::Template) {
                ui.label(egui::RichText::new(tl!("Format:")).color(t.text_dim));
                switch = widgets::dropdown(ui, "save-format", f.label, save_labels(), 254.0).and_then(|i| SAVE_FORMATS.get(i).copied());
                ui.end_row();
            }
        }
        for o in shown(f) {
            option_row(app, ui, d, o);
        }
    });
    if let Some(id) = switch.filter(|id| *id != f.id) {
        switch_format(app, d, id);
    }
    false
}

/// One option: a checkbox, an artboard menu, a number or a text field.
fn option_row(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog, o: &FormatOption) {
    let t = Tokens::get(ui.ctx());
    let label = label(o);
    let value = d.fields.get(o.name).cloned().unwrap_or(Value::Null);
    if o.ty == "boolean" {
        ui.label("");
        let mut on = value.as_bool().unwrap_or(false);
        if ui.checkbox(&mut on, label.trim_end_matches(':')).on_hover_text(o.description).changed() {
            d.fields.insert(o.name.into(), json!(on));
        }
        ui.end_row();
        if o.name == "separateArtboards" {
            artboard_range(ui, d);
        }
        return;
    }
    if o.ty == "string" {
        form::field_w(ui, d, o.name, &label, 240.0);
        return;
    }
    ui.label(egui::RichText::new(&label).color(t.text_dim)).on_hover_text(o.description);
    let artboards = app.session.active().filter(|_| o.name == "artboard").map(|st| &st.doc.artboards);
    if let Some(boards) = artboards {
        let i = value.as_u64().unwrap_or(0) as usize;
        let names: Vec<&str> = boards.iter().map(|a| a.name.as_str()).collect();
        if let Some(i) = widgets::dropdown_names(ui, ("save-option", o.name), names.get(i).copied().unwrap_or(""), &names, 254.0) {
            d.fields.insert(o.name.into(), json!(i));
        }
    } else {
        let integer = o.ty == "integer";
        let x = value.as_f64().unwrap_or(0.0);
        let field = ui.scope(|ui| widgets::range_field(ui, ("save-option", o.name), x, 0.0..=f64::MAX, "", if integer { 0 } else { 2 }, 80.0));
        field.response.on_hover_text(o.description);
        if let Some(x) = field.inner {
            d.fields.insert(o.name.into(), if integer { json!(x as i64) } else { json!(x) });
        }
    }
    ui.end_row();
}

/// Web Save As: another format brings its own options and file extension.
fn switch_format(app: &mut VectorcraftApp, d: &mut Dialog, id: &str) {
    let (Some(old), Some(new)) = (fileio::format(&d.str("format")), fileio::format(id)) else { return };
    for o in old.options {
        d.fields.remove(o.name);
    }
    d.fields.extend(option_values(app, new));
    d.fields.insert("format".into(), json!(new.id));
    d.fields.insert("path".into(), json!(with_extension(&d.str("path"), new)));
}

/// `path` ending in `f`'s extension (kept when it already names `f`).
fn with_extension(path: &str, f: &Format) -> String {
    if fileio::format_for_name(path).is_some_and(|g| g.id == f.id) {
        return path.to_string();
    }
    std::path::Path::new(path).with_extension(f.extensions[0]).to_string_lossy().to_string()
}

/// Write the file with the chosen options (blank ones fall back to the format's defaults).
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> DialogResult {
    let f = fileio::format(&d.str("format")).ok_or("unknown format")?;
    let path = d.str("path");
    if path.trim().is_empty() {
        return Err("name the file".into());
    }
    let path = if d.bool("__pick") { with_extension(path.trim(), f) } else { path };
    let mut options: Map<String, Value> = f
        .options
        .iter()
        .filter_map(|o| d.fields.get(o.name).filter(|v| !v.is_null() && v.as_str() != Some("")).map(|v| (o.name.to_string(), v.clone())))
        .collect();
    if separates(f) {
        if !d.bool("separateArtboards") || all_artboards(d) {
            options.remove("range");
        } else {
            // A bad range keeps the dialog open.
            let n = app.session.active().map_or(0, |s| s.doc.artboards.len());
            ArtboardPick { range: Some(d.str("range")), ..Default::default() }.resolve(n)?;
        }
    }
    app.ui.dialog = None;
    match SaveMode::of(&d.str("__action")) {
        Some(mode) if own_dialog(f) => io::ask_format_options(app, mode, f, &path),
        Some(mode) => io::save(app, mode, &json!({ "path": path, "format": f.id, "options": options }), false),
        None => Err(format!("`{}` is no save command", d.str("__action"))),
    }
}

#[cfg(test)]
mod tests {
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
            download: Some(Box::new(move |name: &str, b: &[u8]| w.borrow_mut().push((name.to_string(), b.to_vec())))),
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
        let labels: Vec<&str> = pick.filters.iter().map(|f| f.0).collect();
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
        assert_eq!(pick.filters, [("VectorCraft Template", &["vctemplate"][..])]);
        assert_eq!(written.borrow()[1].0, "copy.vectorcraft", "the picked name is kept");
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
}
