//! Object → Step and Repeat: copies of the selection on a grid or filling a sheet, with the gaps
//! between the cut paths, every other column or row turned by 180° (`object.stepAndRepeat`),
//! previewed live on the canvas.
//!
//! Fields: `rows`, `columns`, `gapX`, `gapY` (pt), `margin` (pt), `fill` (as many as fit the
//! artboard), `rotate` (`none`, `columns` or `rows`) and `preview`.

use serde_json::{Value, json};

use super::{DialogSpec, form};
use crate::VectorcraftApp;
use crate::state::Dialog;

pub const KIND: &str = "stepAndRepeat";

const CMD: &str = "object.stepAndRepeat";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Step and Repeat").into(),
    body,
    confirm,
    preview: true,
    min_width: 320.0,
    footer_left: Some(form::preview_check),
    ..DialogSpec::FORM
};

/// The fields a fresh dialog starts with.
pub fn fields() -> Value {
    json!({"rows": 2, "columns": 2, "gapX": 12.0, "gapY": 12.0, "margin": 0.0, "fill": false, "rotate": "none"})
}

fn params(d: &Dialog) -> Value {
    json!({
        "rows": d.f64("rows", 1.0).max(1.0) as u64,
        "columns": d.f64("columns", 1.0).max(1.0) as u64,
        "gapX": d.f64("gapX", 0.0),
        "gapY": d.f64("gapY", 0.0),
        "margin": d.f64("margin", 0.0),
        "fill": d.bool("fill"),
        "rotate": d.str("rotate"),
    })
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    const WIDTH: f32 = 96.0;
    let unit = app.session.general_unit();
    let fill = d.bool("fill");
    egui::Grid::new("dlg").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
        for (key, label) in [("rows", tl!("Rows:")), ("columns", tl!("Columns:"))] {
            let row = form::row_label(ui, label);
            ui.label_inputs_by(row.id, |ui| {
                ui.add_enabled_ui(!fill, |ui| {
                    if let Some(v) = crate::widgets::range_field(ui, ("dlg-num", key), d.f64(key, 1.0), 1.0..=500.0, "", 0, WIDTH) {
                        d.fields.insert(key.into(), json!(v));
                    }
                });
            });
            ui.end_row();
        }
        form::length_field(ui, d, "gapX", tl!("Horizontal Gap:"), unit);
        form::length_field(ui, d, "gapY", tl!("Vertical Gap:"), unit);
        let row = form::row_label(ui, tl!("Margin:"));
        ui.label_inputs_by(row.id, |ui| {
            ui.add_enabled_ui(fill, |ui| form::length(ui, d, "margin", unit, form::FIELD_W));
        });
        ui.end_row();
    });
    ui.add_space(8.0);
    form::check(ui, d, "fill", tl!("Fill the Artboard"));
    ui.add_space(4.0);
    form::choice(
        ui,
        d,
        "rotate",
        tl!("Rotate 180°:"),
        (96.0, 150.0),
        &[("none", "None"), ("columns", "Every Other Column"), ("rows", "Every Other Row")],
    );
    form::preview(app, ui, d, "Step and Repeat", CMD, params(d));
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    form::commit_preview(app, CMD, params(d))
}
