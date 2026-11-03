//! Create Swatch Legend in the interface (Object › Plug-ins › Color Effects › Create Swatch
//! Legend…, #4919): a dialog for the columns, the chip size, the gap and which colour values the
//! labels show, and whether only the swatches selected in the Swatches panel are drawn. It
//! previews on the canvas and runs `addon.color.swatchLegend` on OK.

use egui::Ui;
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::dialogs::{DialogSpec, form};
use crate::state::Dialog;
use crate::widgets;

/// The command, and its dialog's kind.
pub const COMMAND: &str = "addon.color.swatchLegend";
pub const KIND: &str = "swatchLegend";
const LABEL_W: f32 = 80.0;
const FIELD_W: f32 = 90.0;
/// The values a label can show: (the command's name, the dialog's label).
pub const VALUES: [(&str, &str); 5] = [("hex", "HEX"), ("rgb", "RGB"), ("cmyk", "CMYK"), ("lab", "Lab"), ("gray", "Grayscale")];

/// The dialog's fields to start with: the command's defaults, each swatch's own values.
pub(crate) fn fields() -> Value {
    json!({ "columns": 4, "size": 72, "gap": 12, "selectedOnly": false })
}

pub(crate) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Create Swatch Legend").to_string(), body, confirm, preview: true, min_width: 300.0, ..DialogSpec::FORM };

/// The colour swatches selected in the Swatches panel.
fn selected_swatches(app: &VectorcraftApp, ctx: &egui::Context) -> Vec<String> {
    let picked: Vec<String> = crate::panels::pstate(ctx, "swatch-selected");
    app.session
        .active()
        .map(|st| st.doc.swatches_iter().filter(|w| w.paint.color().is_some() && picked.contains(&w.name)).map(|w| w.name.clone()).collect())
        .unwrap_or_default()
}

/// The command's params from the dialog's fields.
pub(crate) fn params(d: &Dialog) -> Value {
    let mut p = json!({ "columns": d.f64("columns", 4.0).round(), "size": d.f64("size", 72.0), "gap": d.f64("gap", 12.0) });
    let values: Vec<&str> = VALUES.iter().filter(|(v, _)| d.bool(v)).map(|(v, _)| *v).collect();
    if !values.is_empty() {
        p["values"] = json!(values);
    }
    if d.bool("selectedOnly")
        && let Some(names) = d.fields.get("names").filter(|n| n.as_array().is_some_and(|a| !a.is_empty()))
    {
        p["names"] = names.clone();
    }
    p
}

fn body(app: &mut VectorcraftApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let unit = app.session.general_unit();
    widgets::label_row(ui, tl!("Columns:"), LABEL_W, |ui| {
        if let Some(v) = widgets::range_field(ui, ("swatch-legend", "columns"), d.f64("columns", 4.0), 1.0..=50.0, "", 0, FIELD_W) {
            d.fields.insert("columns".into(), json!(v.round()));
        }
    });
    widgets::label_row(ui, tl!("Size:"), LABEL_W, |ui| {
        form::length_within(ui, d, "size", unit, FIELD_W, Some(8.0..=1000.0));
    });
    widgets::label_row(ui, tl!("Gap:"), LABEL_W, |ui| {
        form::length_within(ui, d, "gap", unit, FIELD_W, Some(0.0..=1000.0));
    });
    ui.add_space(4.0);
    // None ticked: each swatch shows the values of its own colour model.
    ui.label(tl!("Show:"));
    ui.horizontal_wrapped(|ui| {
        for (v, label) in VALUES {
            form::check(ui, d, v, tl!(label));
        }
    });
    // Only the swatches selected in the Swatches panel, when some are.
    let names = selected_swatches(app, ui.ctx());
    if names.is_empty() {
        d.fields.insert("selectedOnly".into(), json!(false));
    } else {
        form::check(ui, d, "selectedOnly", tl!("Selected Swatches Only"));
    }
    d.fields.insert("names".into(), json!(names));
    form::preview(app, ui, d, "Create Swatch Legend", COMMAND, params(d));
    false
}

/// OK: run the command (keep its preview) and close.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    form::commit_preview(app, COMMAND, params(d))
}

#[cfg(test)]
mod tests;
