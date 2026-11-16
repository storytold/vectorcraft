//! Document Info panel: the document's setup and what it contains (objects by kind, graphic
//! styles, brushes, objects painted with spot colors, with patterns and with gradients, symbols, fonts and their files,
//! linked and embedded images, repeat objects), for the whole document or the selection only. The flyout picks one category (or all)
//! and saves the text report (`docInfo.save`, the report File → Package writes too).

use egui::Ui;
use serde_json::{Value, json};

use super::{pstate, set_pstate};
use crate::VectorcraftApp;
use crate::theme::Tokens;
use crate::widgets::{self, menu_item};

/// `document.info` and what it was fetched for: (document uid, revision, selection only).
#[derive(Clone, Default)]
struct Cache {
    key: Option<(u64, u64, bool)>,
    info: Value,
}

/// A `label: value` row. `label` is shown as given: translate an interface label first, a name
/// stays as it is.
pub(crate) fn row(ui: &mut Ui, label: &str, value: String) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).size(12.0).color(t.text_dim));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Long values (paths) are cut to the panel's width; hovering shows them whole.
            ui.add(egui::Label::new(egui::RichText::new(value).size(12.0).color(t.text)).truncate());
        });
    });
}

/// `document.info` with its categories, fetched again only when the document or the scope changes.
fn info(app: &mut VectorcraftApp, ctx: &egui::Context, sel_only: bool) -> Option<Value> {
    let st = app.session.active()?;
    let key = Some((st.uid, st.revision, sel_only));
    let mut c: Cache = pstate(ctx, "docinfo-cache");
    if c.key != key {
        c.info = app.session.execute("document.info", &json!({ "selectionOnly": sel_only })).ok()?;
        c.key = key;
        set_pstate(ctx, "docinfo-cache", c.clone());
    }
    Some(c.info)
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let sel_only: bool = pstate(ui.ctx(), "docinfo-sel");
    let category: String = pstate(ui.ctx(), "docinfo-category");
    let Some(i) = info(app, ui.ctx(), sel_only) else {
        widgets::dim_label(ui, tl!("No document"));
        return;
    };
    egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
        let sections = i["sections"].as_array().map_or(&[][..], Vec::as_slice);
        let shown = sections.iter().filter(|s| category.is_empty() || s["id"] == category.as_str());
        for (n, s) in shown.enumerate() {
            if n > 0 {
                widgets::divider(ui);
            }
            let title = s["title"].as_str().unwrap_or_default();
            let rows = s["rows"].as_array().map_or(&[][..], Vec::as_slice);
            // Lists show how many they hold; the selection scope shows on the first.
            let title = match s["id"].as_str() {
                Some("document" | "objects") => tl!(title).to_string(),
                _ => format!("{} ({})", tl!(title), rows.len()),
            };
            widgets::subheader(
                ui,
                &if sel_only && n == 0 && category.is_empty() { crate::i18n::fmt(tl!("{title} (selection)"), &[("title", &title)]) } else { title },
            );
            if rows.is_empty() {
                widgets::dim_label(ui, tl!("None"));
            }
            let id = s["id"].as_str().unwrap_or_default();
            let named = named_rows(id, rows);
            for (k, r) in rows.iter().enumerate() {
                let (label, value) = (r[0].as_str().unwrap_or_default(), r[1].as_str().unwrap_or_default());
                if value.is_empty() {
                    widgets::dim_name(ui, super::label_or_name(label, !named.contains(&k)));
                } else {
                    let shown = shown_value(&i, id, k, (label, value), named.contains(&k)).unwrap_or_else(|| value.to_string());
                    row(ui, super::label_or_name(label, !named.contains(&k)), shown);
                }
            }
        }
    });
}

/// A Brushes row's value in the interface language: the brush's type (`Art Brush`), or that the
/// Brushes panel does not hold the brush. `document.info` and the text report keep them in English.
fn brush_value(value: &str) -> &str {
    match value {
        "Calligraphic Brush" => tl!("Calligraphic Brush"),
        "Scatter Brush" => tl!("Scatter Brush"),
        "Art Brush" => tl!("Art Brush"),
        "Pattern Brush" => tl!("Pattern Brush"),
        "Bristle Brush" => tl!("Bristle Brush"),
        "missing from the Brushes panel" => tl!("missing from the Brushes panel"),
        _ => value,
    }
}

/// Row `k` of Document Info section `id` (`label` and `value` as `document.info` gives them, the
/// label a name when `named`) with its value in the interface language, built from what
/// `document.info` (`info`) gives as values: the
/// units, the raster effects resolution, each brush's type, each font's file, match and
/// embedding permission, each image's size and file, each Repeat object's kind and counts, and the
/// kinds of the gradients no swatch names. Names (fonts, files, faces, swatches) stay as they are.
/// None when the row's value is shown as `document.info` gives it, or the values do not match the
/// row.
fn shown_value(info: &Value, id: &str, k: usize, (label, value): (&str, &str), named: bool) -> Option<String> {
    let entry = |list: &str| info[list].get(k).filter(|e| e["name"] == label || e["font"] == label);
    match id {
        "document" if !named => match label {
            "Units" => {
                let unit = info["document"]["unitId"].as_str()?;
                vectorcraft_doc::Unit::EAST_ASIAN.into_iter().find(|u| u.key() == unit).map(|u| tl!(u.label()).to_string())
            }
            "Raster Effects" => {
                let ppi = info["document"]["rasterEffectsPpi"].as_f64()?;
                Some(fill(tl!("{n} ppi"), &[("n", &ppi.to_string())]))
            }
            _ => None,
        },
        "brushes" => Some(brush_value(value).to_string()),
        "fontDetails" => font_detail_value(entry("fontDetails")?),
        "linkedImages" | "embeddedImages" => {
            let linked = id == "linkedImages";
            let image = info["imageObjects"].as_array()?.iter().filter(|o| o["linked"] == linked).nth(k).filter(|o| o["name"] == label)?;
            image_value(image)
        }
        "repeats" => repeat_value(entry("repeatObjects")?),
        "gradients" => gradient_value(entry("gradientObjects")?),
        _ => None,
    }
}

/// [`crate::i18n::fill_once`]: a name that holds `{…}` text stays as it is.
fn fill(template: &str, args: &[(&str, &str)]) -> String {
    crate::i18n::fill_once(template, args)
}

/// A `fontDetails` entry as Font Details shows it: the font's file (or that it is built in) and
/// what its license allows, the face shown in place of a style the family lacks, the face shown in
/// place of a font that is not installed, or that it is missing.
fn font_detail_value(e: &Value) -> Option<String> {
    let found = || {
        let embedding = match e["embedding"].as_str()? {
            "forbidden" => tl!("embedding not allowed"),
            "editing" => tl!("embedding for editing"),
            "previewPrint" => tl!("embedding for preview and print"),
            "allowed" => tl!("embedding allowed"),
            _ => return None,
        };
        let file = e["file"].as_str().unwrap_or(tl!("built in"));
        Some(fill(tl!("{file}; {embedding}"), &[("file", file), ("embedding", embedding)]))
    };
    let face = || Some((e["family"].as_str()?, e["style"].as_str()?));
    match e["match"].as_str()? {
        "exact" => found(),
        "style" => {
            let ((family, style), detail) = (face()?, found()?);
            Some(fill(tl!("substituted: shown in {family} {style} ({detail})"), &[("family", family), ("style", style), ("detail", &detail)]))
        }
        "missingWithFallback" => {
            let (family, style) = face()?;
            Some(fill(tl!("missing: shown in {family} {style}"), &[("family", family), ("style", style)]))
        }
        "missing" => Some(tl!("missing").to_string()),
        _ => None,
    }
}

/// An `imageObjects` entry as Linked Images and Embedded Images show it: its size in pixels, and
/// for a linked image the file it shows.
fn image_value(o: &Value) -> Option<String> {
    let size = fill(tl!("{width} × {height} px"), &[("width", &o["width"].as_u64()?.to_string()), ("height", &o["height"].as_u64()?.to_string())]);
    if o["linked"] == true { Some(fill(tl!("{size} — {file}"), &[("size", &size), ("file", o["file"].as_str()?)])) } else { Some(size) }
}

/// A `repeatObjects` entry as Repeat Objects shows it: its kind and the number of instances it
/// draws, or for a grid its rows and columns, followed by the number it draws when that is fewer.
fn repeat_value(e: &Value) -> Option<String> {
    let kind = match e["type"].as_str()? {
        "radial" => tl!("Radial Repeat"),
        "grid" => tl!("Grid Repeat"),
        "mirror" => tl!("Mirror Repeat"),
        _ => return None,
    };
    let n = e["instances"].as_u64()?;
    let instances = crate::i18n::tn(n, "{n} instance", "{n} instances");
    if e["type"] != "grid" {
        return Some(fill(tl!("{kind}, {instances}"), &[("kind", kind), ("instances", &instances)]));
    }
    let (rows, cols) = (e["rows"].as_u64()?, e["cols"].as_u64()?);
    let (shown_rows, shown_cols) = (crate::i18n::tn(rows, "{n} row", "{n} rows"), crate::i18n::tn(cols, "{n} column", "{n} columns"));
    let grid = fill(tl!("{kind}, {rows} × {cols}"), &[("kind", kind), ("rows", &shown_rows), ("cols", &shown_cols)]);
    Some(if n < rows.saturating_mul(cols) { fill(tl!("{grid}, {instances}"), &[("grid", &grid), ("instances", &instances)]) } else { grid })
}

/// A `gradientObjects` entry as Gradient Objects shows it: its gradients, a swatch's name as it
/// is and a gradient named by its kind in the interface language.
fn gradient_value(e: &Value) -> Option<String> {
    let (names, kinds) = (e["gradients"].as_array()?, e["kinds"].as_array()?);
    if names.len() != kinds.len() {
        return None;
    }
    let shown: Option<Vec<&str>> = names
        .iter()
        .zip(kinds)
        .map(|(name, kind)| match kind.as_str() {
            None => name.as_str(),
            Some("linear") => Some(tl!("Linear gradient")),
            Some("radial") => Some(tl!("Radial gradient")),
            Some("conic") => Some(tl!("Conic gradient")),
            Some("diamond") => Some(tl!("Diamond gradient")),
            Some("freeform") => Some(tl!("Freeform gradient")),
            Some(_) => name.as_str(),
        })
        .collect();
    Some(shown?.join(", "))
}

/// The rows of Document Info section `id` labelled by a name (an artboard, a style, a swatch, a
/// font, an image, a repeat object) rather than an interface label: every row of the lists, and in Document the
/// artboards, which follow the Artboards count. Names are shown as they are.
fn named_rows(id: &str, rows: &[Value]) -> std::ops::Range<usize> {
    match id {
        "objects" => 0..0,
        "document" => {
            let Some(at) = rows.iter().position(|r| r[0] == "Artboards") else { return 0..0 };
            let count = rows.get(at).and_then(|r| r[1].as_str()).and_then(|n| n.parse::<usize>().ok()).unwrap_or(0);
            at + 1..(at + 1).saturating_add(count).min(rows.len())
        }
        _ => 0..rows.len(),
    }
}

/// `docInfo.save {path?, selectionOnly?, category?}`: write the text report (only `category` when given,
/// else every category) to `path`, else a picked file (the web downloads it) → `{path}`.
pub(crate) fn save_report(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let sel_only = p.get("selectionOnly").and_then(Value::as_bool).unwrap_or(false);
    let mut q = json!({ "format": "text", "selectionOnly": sel_only });
    if let Some(c) = p.get("category").and_then(Value::as_str).filter(|c| !c.is_empty()) {
        q["category"] = c.into();
    }
    let r = app.session.execute("document.info", &q).map_err(|e| e.to_string())?;
    let text = r["text"].as_str().unwrap_or_default();
    let title = app.session.active().map(|d| d.title()).unwrap_or_default();
    let stem = std::path::Path::new(&title).file_stem().map_or_else(|| "Untitled".into(), |s| s.to_string_lossy().into_owned());
    let path = p.get("path").and_then(Value::as_str).map(str::to_string);
    let path = crate::io::write_named(app, path, &format!("{stem} Info.txt"), text.as_bytes())?;
    app.status(format!("Saved {path}"));
    Ok(json!({ "path": path }))
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let sel: bool = pstate(ui.ctx(), "docinfo-sel");
    if menu_item(ui, tl!("Selection Only"), true, sel) {
        set_pstate(ui.ctx(), "docinfo-sel", !sel);
    }
    ui.separator();
    let category: String = pstate(ui.ctx(), "docinfo-category");
    if menu_item(ui, tl!("All Categories"), true, category.is_empty()) {
        set_pstate(ui.ctx(), "docinfo-category", String::new());
    }
    let info = info(app, ui.ctx(), sel).unwrap_or_default();
    for s in info["sections"].as_array().into_iter().flatten() {
        let (id, title) = (s["id"].as_str().unwrap_or_default(), s["title"].as_str().unwrap_or_default());
        if menu_item(ui, title, true, category == id) {
            set_pstate(ui.ctx(), "docinfo-category", id.to_string());
        }
    }
    ui.separator();
    if menu_item(ui, tl!("Save…"), app.session.active().is_some(), false) {
        crate::menus::invoke(app, "docInfo.save", json!({ "selectionOnly": sel, "category": category }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_headless() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "Hi"})).unwrap();
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            show(&mut app, ui);
            menu(&mut app, ui);
        });
        out.textures_delta.clear();
    }

    /// Artboard, font and other names in the rows are shown as they are, the labels in the UI
    /// language: an artboard named like a label ("Units") is still a name.
    #[test]
    fn names_in_rows_are_told_from_labels() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("artboard.setProps", &json!({"index": 0, "name": "Units"})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "Hi"})).unwrap();
        let info = app.session.execute("document.info", &json!({})).unwrap();
        let rows = |id: &str| info["sections"].as_array().unwrap().iter().find(|s| s["id"] == id).unwrap()["rows"].as_array().unwrap().clone();
        let doc = rows("document");
        let named: Vec<&str> = named_rows("document", &doc).map(|k| doc[k][0].as_str().unwrap()).collect();
        assert_eq!(named, ["Units"], "{doc:?}");
        assert_eq!(doc.iter().filter(|r| r[0] == "Units").count(), 2, "the label and the artboard");
        assert_eq!(named_rows("objects", &rows("objects")), 0..0);
        let fonts = rows("fonts");
        assert!(!fonts.is_empty());
        assert_eq!(named_rows("fonts", &fonts), 0..fonts.len());
    }

    #[test]
    fn categories_filter_the_panel_and_save_writes_the_report() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "Hi"})).unwrap();
        let text = crate::tests_labels::painted_text(&mut app, show);
        assert!(text.contains("Color Mode") && text.contains("Artboard 1"), "{text}");
        let menu_text = crate::tests_labels::painted_text(&mut app, menu);
        assert!(
            menu_text.contains("Font Details")
                && menu_text.contains("Brushes")
                && menu_text.contains("Repeat Objects")
                && menu_text.contains("Gradient Objects")
                && menu_text.contains("Spot Color Objects")
                && menu_text.contains("Save…"),
            "{menu_text}"
        );
        // Gradient Objects lists the objects painted with a gradient by name.
        let r = app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap()["id"].clone();
        let ramp = json!({"kind": "radial", "stops": [{"offset": 0, "color": "#000"}, {"offset": 1, "color": "#fff"}]});
        app.session.execute("paint.setFill", &json!({"ids": [r], "gradient": ramp})).unwrap();
        app.session.execute("layer.setProps", &json!({"id": r, "name": "Glow"})).unwrap();
        let gradients = crate::tests_labels::painted_text(&mut app, |app, ui| {
            set_pstate(ui.ctx(), "docinfo-category", "gradients".to_string());
            show(app, ui);
        });
        assert!(gradients.contains("Gradient Objects (1)") && gradients.contains("Glow") && gradients.contains("Radial gradient"), "{gradients}");
        // Spot Color Objects lists the objects painted with a spot color by name.
        let spot = json!({"model": "cmyk", "c": 0.0, "m": 0.8, "y": 0.6, "k": 0.0});
        app.session.execute("swatch.new", &json!({"name": "Signal Red", "color": spot, "spot": true})).unwrap();
        let badge = app.session.execute("shape.ellipse", &json!({"x": 50, "y": 50, "width": 20, "height": 20})).unwrap()["id"].clone();
        app.session.execute("paint.setFill", &json!({"ids": [badge], "swatch": "Signal Red"})).unwrap();
        app.session.execute("layer.setProps", &json!({"id": badge, "name": "Badge"})).unwrap();
        let spots = crate::tests_labels::painted_text(&mut app, |app, ui| {
            set_pstate(ui.ctx(), "docinfo-category", "spotColors".to_string());
            show(app, ui);
        });
        assert!(spots.contains("Spot Color Objects (1)") && spots.contains("Badge") && spots.contains("Signal Red"), "{spots}");
        let filtered = crate::tests_labels::painted_text(&mut app, |app, ui| {
            set_pstate(ui.ctx(), "docinfo-category", "fonts".to_string());
            show(app, ui);
        });
        assert!(filtered.contains("Source Sans 3 Regular") && !filtered.contains("Color Mode"), "{filtered}");
        // Save…: written through the host.
        let written: std::rc::Rc<std::cell::RefCell<Vec<u8>>> = Default::default();
        let w = written.clone();
        app.services.write = Some(Box::new(move |_: &str, b: &[u8]| {
            w.borrow_mut().extend_from_slice(b);
            Ok(())
        }));
        let r = app.run("docInfo.save", json!({"path": "/tmp/info.txt"})).unwrap();
        assert_eq!(r["path"], "/tmp/info.txt");
        let report = String::from_utf8(written.borrow().clone()).unwrap();
        assert!(report.starts_with("Document Info: ") && report.contains("\nFONT DETAILS\n") && report.contains("\nREPEAT OBJECTS\n"), "{report}");
        assert!(report.contains("\nGRADIENT OBJECTS\nGlow: Radial gradient\n"), "{report}");
        assert!(report.contains("\nSPOT COLOR OBJECTS\nBadge: Signal Red\n"), "{report}");
    }

    /// The report the panel menu's Save… writes with category `category` shown ("" for All
    /// Categories) and Selection Only set to `sel_only`.
    fn saved_from_menu(app: &mut VectorcraftApp, category: &str, sel_only: bool) -> String {
        let written: std::rc::Rc<std::cell::RefCell<Vec<u8>>> = Default::default();
        let w = written.clone();
        app.services.write = Some(Box::new(move |_: &str, b: &[u8]| {
            w.borrow_mut().extend_from_slice(b);
            Ok(())
        }));
        app.services.pick_save = Some(Box::new(|pick: &crate::FilePick| Some(format!("/tmp/{}", pick.name))));
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        set_pstate(&ctx, "docinfo-category", category.to_string());
        set_pstate(&ctx, "docinfo-sel", sel_only);
        let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(236.0, 800.0));
            let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| menu(app, ui));
            out.textures_delta.clear();
            crate::tests_removeanchors::shapes_text(&out.shapes.iter().map(|c| c.shape.clone()).collect::<Vec<_>>())
        };
        let labels = frame(app, vec![]);
        let at = labels.iter().find(|(t, _)| t.trim() == "Save…").map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no Save… in {labels:?}"));
        let button =
            |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        frame(app, vec![egui::Event::PointerMoved(at), button(true)]);
        frame(app, vec![button(false)]);
        let report = String::from_utf8(written.borrow().clone()).unwrap();
        assert!(report.starts_with("Document Info: "), "nothing saved: {report:?}");
        report
    }

    /// Save… writes the category the panel shows, with Selection Only applied; All Categories
    /// writes every category.
    #[test]
    fn save_writes_the_category_the_panel_shows() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
        let text = app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "Hi"})).unwrap()["id"].clone();
        let objects = saved_from_menu(&mut app, "objects", false);
        assert!(objects.contains("\nOBJECTS\n") && objects.contains("Paths: 1") && objects.contains("Text Objects: 1"), "{objects}");
        assert!(!objects.contains("\nDOCUMENT\n") && !objects.contains("\nFONTS\n"), "only Objects: {objects}");
        let fonts = saved_from_menu(&mut app, "fonts", false);
        assert!(fonts.contains("\nFONTS\n") && fonts.contains("Source Sans 3"), "{fonts}");
        assert!(!fonts.contains("\nOBJECTS\n") && !fonts.contains("\nFONT DETAILS\n"), "only Fonts: {fonts}");
        let all = saved_from_menu(&mut app, "", false);
        assert!(
            all.contains("\nDOCUMENT\n") && all.contains("\nOBJECTS\n") && all.contains("\nFONTS\n") && all.contains("\nREPEAT OBJECTS\n"),
            "{all}"
        );
        app.session.execute("select.set", &json!({"ids": [text]})).unwrap();
        let selected = saved_from_menu(&mut app, "objects", true);
        assert!(selected.contains("(the selection only)") && selected.contains("Text Objects: 1"), "{selected}");
        assert!(!selected.contains("Paths: 1") && !selected.contains("\nFONTS\n"), "the selection's objects only: {selected}");
        // The command takes the category too; an unknown one is an error and writes nothing.
        let err = app.run("docInfo.save", json!({"path": "/tmp/info.txt", "category": "colors"})).unwrap_err();
        assert!(err.contains("category `colors`"), "{err}");
    }

    /// Brushes shows each brush's type, and a brush the Brushes panel does not hold, in the
    /// interface language; the saved report keeps them in English.
    #[test]
    fn brush_types_show_in_the_interface_language() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        let r = app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap()["id"].clone();
        app.session.execute("brush.apply", &json!({"ids": [r], "name": "Charcoal"})).unwrap();
        let brushes = |app: &mut VectorcraftApp| {
            crate::tests_labels::painted_text(app, |app, ui| {
                set_pstate(ui.ctx(), "docinfo-category", "brushes".to_string());
                show(app, ui);
            })
        };
        let english = brushes(&mut app);
        assert!(english.contains("Charcoal") && english.contains("Art Brush"), "{english}");
        let Some(de) = crate::i18n::Lang::from_code("de") else { panic!("no German catalog") };
        crate::i18n::set_current(de);
        let (german, missing) = (brushes(&mut app), brush_value("missing from the Brushes panel").to_string());
        crate::i18n::set_current(crate::i18n::Lang::EN);
        assert!(german.contains("Charcoal") && german.contains("Bildpinsel") && !german.contains("Art Brush"), "{german}");
        assert_eq!(missing, "fehlt im Bedienfeld „Pinsel“");
        let report = app.session.execute("document.info", &json!({"category": "brushes", "format": "text"})).unwrap()["text"].to_string();
        assert!(report.contains("Charcoal: Art Brush"), "{report}");
    }

    /// Section `id`'s rows as the panel shows them for `info`: each label and its shown value.
    fn shown_rows(info: &Value, id: &str) -> Vec<(String, String)> {
        let section = info["sections"].as_array().unwrap().iter().find(|s| s["id"] == id).unwrap_or_else(|| panic!("no {id}"));
        let rows = section["rows"].as_array().unwrap();
        let named = named_rows(id, rows);
        rows.iter()
            .enumerate()
            .map(|(k, r)| {
                let (label, value) = (r[0].as_str().unwrap(), r[1].as_str().unwrap());
                (label.to_string(), shown_value(info, id, k, (label, value), named.contains(&k)).unwrap_or_else(|| value.to_string()))
            })
            .collect()
    }

    /// The values Document Info makes (the units, the raster effects resolution, each font's file
    /// and state, each image's size, each Repeat object's kind and counts, and gradients named by
    /// their kind) show in the interface language, from the same `document.info` after a language
    /// change; font, face, file and swatch names stay as they are, also with Selection Only, and
    /// `document.info` and the saved report stay in English.
    #[test]
    fn values_follow_interface_language_and_keep_names() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 400, "units": "Millimeters"})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "Hi"})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 80, "text": "Heavy", "font": "Source Sans 3", "style": "Black Wide"})).unwrap();
        app.session.execute("text.create", &json!({"x": 10, "y": 120, "text": "Gone", "font": "Qx Absent Family"})).unwrap();
        let rect = |app: &mut VectorcraftApp, x: f64| {
            app.session.execute("shape.rectangle", &json!({"x": x, "y": 200, "width": 20, "height": 10})).unwrap()["id"].clone()
        };
        let ramp = |kind: &str| json!({"kind": kind, "stops": [{"offset": 0, "color": "#000"}, {"offset": 1, "color": "#fff"}]});
        app.session.execute("swatch.new", &json!({"name": "Linear gradient", "gradient": ramp("radial")})).unwrap();
        let glow = rect(&mut app, 0.0);
        app.session.execute("paint.setFill", &json!({"ids": [glow], "swatch": "Linear gradient"})).unwrap();
        app.session.execute("paint.setStroke", &json!({"ids": [glow], "gradient": ramp("radial")})).unwrap();
        app.session.execute("layer.setProps", &json!({"id": glow, "name": "Glow"})).unwrap();
        let star = rect(&mut app, 50.0);
        app.session.execute("object.repeat.radial", &json!({"ids": [star], "instances": 6})).unwrap();
        let tile = rect(&mut app, 100.0);
        app.session.execute("object.repeat.grid", &json!({"ids": [tile], "rows": 2, "cols": 1})).unwrap();
        let pic = rect(&mut app, 150.0);
        app.session.execute("select.set", &json!({"ids": [pic]})).unwrap();
        app.session.execute("object.rasterize", &json!({"ppi": 72})).unwrap();
        app.session.execute("select.set", &json!({"ids": []})).unwrap();
        let info = app.session.execute("document.info", &json!({})).unwrap();
        let value = |rows: &[(String, String)], label: &str| {
            rows.iter().find(|(l, _)| l == label).unwrap_or_else(|| panic!("no {label} in {rows:?}")).1.clone()
        };
        let english = |id: &str| shown_rows(&info, id);
        assert_eq!((value(&english("document"), "Units"), value(&english("document"), "Raster Effects")), ("Millimeters".into(), "72 ppi".into()));
        assert_eq!(value(&english("fontDetails"), "Source Sans 3 Regular"), "built in; embedding allowed");
        assert_eq!(value(&english("gradients"), "Glow"), "Linear gradient, Radial gradient");
        let repeats = english("repeats");
        assert_eq!((repeats[0].1.as_str(), repeats[1].1.as_str()), ("Radial Repeat, 6 instances", "Grid Repeat, 2 rows × 1 column"));
        // Every value the panel builds reads as `document.info`'s English row.
        for s in info["sections"].as_array().unwrap() {
            let id = s["id"].as_str().unwrap();
            let rows: Vec<(String, String)> =
                s["rows"].as_array().unwrap().iter().map(|r| (r[0].as_str().unwrap().into(), r[1].as_str().unwrap().into())).collect();
            assert_eq!(shown_rows(&info, id), rows, "{id}");
        }
        let Some(de) = crate::i18n::Lang::from_code("de") else { panic!("no German catalog") };
        crate::i18n::set_current(de);
        let german = |id: &str| shown_rows(&info, id);
        let (document, fonts, images, gradients, repeats) =
            (german("document"), german("fontDetails"), german("embeddedImages"), german("gradients"), german("repeats"));
        let painted = crate::tests_labels::painted_text(&mut app, show);
        app.session.execute("select.set", &json!({"ids": [glow]})).unwrap();
        let selected = shown_rows(&app.session.execute("document.info", &json!({"selectionOnly": true})).unwrap(), "gradients");
        let written: std::rc::Rc<std::cell::RefCell<Vec<u8>>> = Default::default();
        let w = written.clone();
        app.services.write = Some(Box::new(move |_: &str, b: &[u8]| {
            w.borrow_mut().extend_from_slice(b);
            Ok(())
        }));
        app.run("docInfo.save", json!({"path": "/tmp/info.txt", "category": "gradients", "selectionOnly": true})).unwrap();
        crate::i18n::set_current(crate::i18n::Lang::EN);
        let saved = String::from_utf8(written.borrow().clone()).unwrap();
        assert_eq!((value(&document, "Units"), value(&document, "Raster Effects")), ("Millimeter".into(), "72 ppi".into()));
        assert!(painted.contains("Millimeter") && !painted.contains("Millimeters"), "{painted}");
        assert_eq!(value(&fonts, "Source Sans 3 Regular"), "integriert; Einbetten erlaubt");
        let wide = value(&fonts, "Source Sans 3 Black Wide");
        assert!(wide.starts_with("ersetzt: angezeigt in Source Sans 3 ") && wide.ends_with("Einbetten erlaubt)"), "{wide}");
        let gone = fonts.iter().find(|(l, _)| l.starts_with("Qx Absent Family")).unwrap_or_else(|| panic!("{fonts:?}")).1.clone();
        assert!(gone == "fehlt" || gone.starts_with("fehlt: angezeigt in "), "{gone}");
        assert_eq!(images.len(), 1);
        assert!(images[0].1.ends_with(" px") && images[0].1.contains(" × "), "{images:?}");
        // A swatch named like a kind keeps its name; the gradient named by its kind is translated.
        assert_eq!(value(&gradients, "Glow"), "Linear gradient, Radialer Verlauf");
        assert_eq!(value(&selected, "Glow"), "Linear gradient, Radialer Verlauf");
        assert_eq!((repeats[0].1.as_str(), repeats[1].1.as_str()), ("Radiale Wiederholung, 6 Instanzen", "Rasterwiederholung, 2 Zeilen × 1 Spalte"));
        assert!(saved.contains("(the selection only)") && saved.contains("\nGRADIENT OBJECTS\nGlow: Linear gradient, Radial gradient\n"), "{saved}");
        let report = app.session.execute("document.info", &json!({"format": "text"})).unwrap()["text"].as_str().unwrap().to_string();
        assert!(report.contains("Units: Millimeters\n") && report.contains("Radial Repeat, 6 instances"), "{report}");
    }

    /// A Repeat object's counts take each language's plural forms, and a grid that draws fewer
    /// instances than its rows and columns give is followed by the number it draws.
    #[test]
    fn repeat_value_uses_locale_plural_forms_and_drawn_count() {
        let radial = |n: u64| json!({"name": "R", "type": "radial", "instances": n});
        let grid = json!({"name": "G", "type": "grid", "rows": 500, "cols": 500, "instances": 125_000});
        let in_lang = |code: &str, e: &Value| {
            let Some(lang) = crate::i18n::Lang::from_code(code) else { panic!("no {code} catalog") };
            crate::i18n::set_current(lang);
            let shown = repeat_value(e);
            crate::i18n::set_current(crate::i18n::Lang::EN);
            shown.unwrap_or_else(|| panic!("{e}"))
        };
        assert_eq!(in_lang("en", &radial(1)), "Radial Repeat, 1 instance");
        assert_eq!(in_lang("en", &grid), "Grid Repeat, 500 rows × 500 columns, 125000 instances");
        assert_eq!(
            [1, 3, 5, 21].map(|n| in_lang("ru", &radial(n))),
            [
                "Радиальный повтор, 1 экземпляр",
                "Радиальный повтор, 3 экземпляра",
                "Радиальный повтор, 5 экземпляров",
                "Радиальный повтор, 21 экземпляр"
            ]
        );
        assert_eq!(in_lang("ru", &grid), "Повтор сеткой, 500 строк × 500 столбцов, 125000 экземпляров");
        assert_eq!([1, 2].map(|n| in_lang("ja", &radial(n))), ["ラジアルリピート、インスタンス 1 個", "ラジアルリピート、インスタンス 2 個"]);
        // A full grid shows its rows and columns alone; an unknown kind is shown as `document.info` gives it.
        assert_eq!(in_lang("de", &json!({"type": "grid", "rows": 1, "cols": 3, "instances": 3})), "Rasterwiederholung, 1 Zeile × 3 Spalten");
        assert_eq!(repeat_value(&json!({"type": "spiral", "instances": 2})), None);
    }
}
