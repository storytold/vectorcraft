//! Links panel → Placement Options: how a file read again (Relink, Update Link) takes the place of
//! the selected images: what it preserves, where it aligns in the old bounds, and whether it is
//! clipped to them.
//!
//! Fields: `ids` (the images; null: the selected ones), `preserve` (transforms|bounds|fileDimensions|fit|fill), `align`
//! (topLeft … bottomRight) and `clip`. OK runs `links.placementOptions` with them: one undo step.

use serde_json::{Value, json};

use super::DialogSpec;
use super::form;
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

pub(super) const KIND: &str = "placementOptions";
const CMD: &str = "links.placementOptions";
/// (value, label) of each Preserve choice.
const PRESERVE: [(&str, &str); 5] = [
    ("transforms", "Transforms"),
    ("bounds", "Bounds"),
    ("fileDimensions", "File Dimensions"),
    ("fit", "Bounds (Fit Proportionally)"),
    ("fill", "Bounds (Fill Proportionally)"),
];
/// The alignment points, row by row from the top left (the reference point widget's order).
const ALIGN: [&str; 9] = ["topLeft", "top", "topRight", "left", "center", "right", "bottomLeft", "bottom", "bottomRight"];
const LABEL: f32 = 90.0;

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Placement Options").into(), body, confirm, min_width: 400.0, max_width: Some(420.0), ..DialogSpec::FORM };

/// Open the dialog on the placement options of images `ids` (default: the selected ones).
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let ids = p.get("ids").cloned().unwrap_or(Value::Null);
    let r = app.session.execute(CMD, &json!({ "ids": ids })).map_err(|e| e.to_string())?;
    let mut fields = r["placement"].clone();
    fields["ids"] = ids;
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let r = app.run(CMD, Value::Object(d.fields.clone()));
    if r.is_ok() {
        app.ui.dialog = None;
    }
    r
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    form::choice(ui, d, "preserve", tl!("Preserve:"), (LABEL, 240.0), &PRESERVE);
    ui.add_space(8.0);
    // Bounds stretches the art into the old bounds: nothing to align or clip.
    let free = d.str("preserve") != "bounds";
    ui.add_enabled_ui(free, |ui| {
        widgets::label_row(ui, tl!("Alignment:"), LABEL, |ui| {
            let current = ALIGN.iter().position(|a| *a == d.str("align")).unwrap_or(4);
            if let Some(i) = widgets::reference_point_named(ui, current, tl!("Alignment:"))
                && let Some(a) = ALIGN.get(i)
            {
                d.fields.insert("align".into(), json!(a));
            }
        });
        ui.add_space(8.0);
        widgets::label_row(ui, "", LABEL, |ui| {
            let clip = d.bool("clip");
            if widgets::check(ui, tl!("Clip to Bounding Box"), clip, free) {
                d.fields.insert("clip".into(), json!(!clip));
            }
        });
    });
    ui.add_space(6.0);
    widgets::dim_label(ui, tl!("Applies when the linked file is relinked or updated."));
    false
}
