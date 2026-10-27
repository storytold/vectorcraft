//! File → Save for Office Documents…: an artboard as a PNG for office documents and slides, at
//! 72, 150 or 300 ppi, on white or transparent. Fields are `document.exportForOffice` params.

use serde_json::{Value, json};

use super::png_options::{RESOLUTION_LABELS, RESOLUTIONS};
use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, io, widgets};

pub(crate) const KIND: &str = "saveForOffice";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Save for Office Documents").into(),
    body,
    confirm,
    ok: Some("Save…"),
    min_width: 340.0,
    inputs: |_| &["path"],
    ..DialogSpec::FORM
};

/// Open the dialog (150 ppi on white, the first artboard).
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    app.session.active().ok_or("no document")?;
    app.ui.dialog = Some(Dialog::new(KIND, json!({"ppi": RESOLUTIONS[1], "transparent": false, "artboard": 0})));
    Ok(Value::Null)
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    let label = |ui: &mut egui::Ui, text: &str| ui.label(egui::RichText::new(text).color(t.text_dim));
    let Some(doc) = app.session.active().map(|st| &st.doc) else { return true };
    let ppi = d.f64("ppi", RESOLUTIONS[1]);
    let board = (d.f64("artboard", 0.0).max(0.0) as usize).min(doc.artboards.len().saturating_sub(1));
    egui::Grid::new("office-export").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
        label(ui, tl!("Resolution:"));
        let at = RESOLUTIONS.iter().position(|r| *r == ppi).unwrap_or(1);
        if let Some(i) = widgets::dropdown(ui, "office-ppi", RESOLUTION_LABELS[at], &RESOLUTION_LABELS[..RESOLUTIONS.len()], 160.0)
            && let Some(r) = RESOLUTIONS.get(i)
        {
            d.fields.insert("ppi".into(), json!(r));
        }
        ui.end_row();
        if doc.artboards.len() > 1 {
            label(ui, tl!("Artboard:"));
            let names: Vec<&str> = doc.artboards.iter().map(|a| a.name.as_str()).collect();
            if let Some(i) = widgets::dropdown_names(ui, "office-artboard", names.get(board).copied().unwrap_or_default(), &names, 160.0) {
                d.fields.insert("artboard".into(), json!(i));
            }
            ui.end_row();
        }
        if let Some(a) = doc.artboards.get(board) {
            label(ui, tl!("Size:"));
            let (w, h) = vectorcraft_render::region_pixels(a.rect, ppi / 72.0);
            ui.label(egui::RichText::new(format!("{w} × {h} px")).color(t.text));
            ui.end_row();
        }
    });
    ui.add_space(8.0);
    form::check(ui, d, "transparent", tl!("Transparent Background"));
    false
}

/// Pick the file and write it (the command runs first: bad options keep the dialog open).
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let path = io::save_command_output(app, "document.exportForOffice", "png", form::params(d))?;
    app.ui.dialog = None;
    Ok(json!({ "path": path }))
}
