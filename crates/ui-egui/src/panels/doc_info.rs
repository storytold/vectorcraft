//! Document Info panel: the document's setup and what it contains (objects by kind, graphic
//! styles, brushes, spot colors, patterns, gradients, symbols, fonts and their files, linked and embedded
//! images, repeat objects), for the whole document or the selection only. The flyout picks one category (or all)
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
            let named = named_rows(s["id"].as_str().unwrap_or_default(), rows);
            for (k, r) in rows.iter().enumerate() {
                let (label, value) = (r[0].as_str().unwrap_or_default(), r[1].as_str().unwrap_or_default());
                let label = super::label_or_name(label, !named.contains(&k));
                if value.is_empty() {
                    widgets::dim_name(ui, label);
                } else {
                    row(ui, label, value.into());
                }
            }
        }
    });
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

/// `docInfo.save {path?, selectionOnly?}`: write the text report to `path`, else a picked file
/// (the web downloads it) → `{path}`.
pub(crate) fn save_report(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let sel_only = p.get("selectionOnly").and_then(Value::as_bool).unwrap_or(false);
    let r = app.session.execute("document.info", &json!({ "format": "text", "selectionOnly": sel_only })).map_err(|e| e.to_string())?;
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
        crate::menus::invoke(app, "docInfo.save", json!({ "selectionOnly": sel }));
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
                && menu_text.contains("Save…"),
            "{menu_text}"
        );
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
    }
}
