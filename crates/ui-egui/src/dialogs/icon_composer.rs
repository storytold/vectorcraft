//! File › Export › Export for Icon Composer…: the format (SVG or PNG), which artboards (all, or a
//! range when there are several) and the folder the export makes its own folder in (the
//! document's folder once it is saved, else the export folder; the web downloads one ZIP instead).
//! Fields are `document.exportForIconComposer` params, but `__boards` (`all` | `range`).

use serde_json::{Value, json};

use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, io, widgets};

pub(crate) const KIND: &str = "exportForIconComposer";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Export for Icon Composer").into(),
    body,
    confirm,
    ok: Some("Export"),
    ok_label: Some(|app| if app.services.download.is_some() { "Download" } else { "Export" }),
    min_width: 380.0,
    inputs: |_| &["format", "range", "folder"],
    ..DialogSpec::FORM
};

/// Open the dialog: SVG, every artboard, into the document's folder.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document")?;
    let folder = st
        .path
        .as_deref()
        .and_then(|p| std::path::Path::new(p).parent())
        .map(|p| p.to_string_lossy().into_owned())
        .filter(|p| !p.is_empty())
        .or_else(vectorcraft_engine::cmd::fileio::export_folder)
        .unwrap_or_default();
    app.ui.dialog = Some(Dialog::new(KIND, json!({"format": "svg", "__boards": "all", "range": "", "folder": folder})));
    Ok(Value::Null)
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    let Some(boards) = app.session.active().map(|st| st.doc.artboards.len()) else { return true };
    form::choice(ui, d, "format", "Format:", (90.0, 160.0), &[("svg", "SVG"), ("png", "PNG")]);
    if boards > 1 {
        ui.add_space(6.0);
        form::choice(ui, d, "__boards", "Artboards:", (90.0, 160.0), &[("all", "All"), ("range", "Range")]);
        if d.str("__boards") == "range" {
            widgets::label_row(ui, "", 90.0, |ui| {
                form::text(ui, d, "range", 160.0);
            });
        }
    }
    ui.add_space(6.0);
    if app.services.download.is_some() {
        ui.label(egui::RichText::new(tl!("The layers download as one ZIP file.")).color(t.text_dim));
    } else {
        let pick = app.services.pick_folder.is_some();
        widgets::label_row(ui, "Export to:", 90.0, |ui| {
            form::text(ui, d, "folder", if pick { 220.0 } else { 254.0 });
            if pick && widgets::icon_button(ui, "folder-open", tl!("Choose a folder"), false, 26.0).clicked() {
                crate::picks::folder_field(app, d, "folder");
            }
        });
    }
    ui.add_space(8.0);
    ui.label(egui::RichText::new(tl!("Each visible layer becomes a file the size of its artboard, numbered from the back.")).color(t.text_dim));
    false
}

/// The `document.exportForIconComposer` params the dialog describes.
fn params(app: &VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut p = json!({ "format": d.str("format") });
    if d.str("__boards") == "range" {
        let range = d.str("range");
        if range.trim().is_empty() {
            return Err(tl!("type the artboards to export, as 1-3, 5").into());
        }
        p["range"] = json!(range.trim());
    }
    if app.services.download.is_none() {
        let folder = d.str("folder");
        if folder.trim().is_empty() {
            return Err(tl!("choose a folder to export to").into());
        }
        p["folder"] = json!(folder.trim());
    }
    Ok(p)
}

/// Export (bad params keep the dialog open, with the error in the status bar).
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let p = params(app, d)?;
    let r = io::export_for_icon_composer(app, p)?;
    app.ui.dialog = None;
    Ok(r)
}
