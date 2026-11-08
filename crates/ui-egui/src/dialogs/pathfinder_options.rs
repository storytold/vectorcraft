//! Pathfinder panel › Pathfinder Options…: Precision, Remove Redundant Points and Divide and Outline
//! Will Remove Unpainted Artwork. OK runs `object.pathfinder.options` (the options live in the
//! preferences).
//!
//! Fields: `precision` (pt, 0.001–10), `removeRedundantPoints` and `removeUnpainted`.

use serde_json::{Value, json};

use super::DialogSpec;
use super::swatch_options::{grid, label};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Pathfinder Options.
pub const KIND: &str = "pathfinderOptions";

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |_| tl!("Pathfinder Options").into(), body, confirm, min_width: 360.0, ..DialogSpec::FORM };

/// Open the dialog with the current options.
pub fn open(app: &mut VectorcraftApp) {
    app.ui.dialog = Some(Dialog::new(KIND, json!(app.session.prefs.pathfinder)));
}

fn check(ui: &mut egui::Ui, d: &mut Dialog, key: &str, text: &str) {
    let v = d.bool(key);
    if widgets::check(ui, text, v, true) {
        d.fields.insert(key.into(), json!(!v));
    }
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let range = vectorcraft_engine::cmd::PATHFINDER_PRECISION_RANGE;
    grid(ui, |ui| {
        label(ui, tl!("Precision:"));
        let v = d.f64("precision", vectorcraft_engine::PathfinderOptions::default().precision);
        if let Some(v) = widgets::range_field(ui, "pathfinder-precision", v, range, " pt", 3, 80.0) {
            d.fields.insert("precision".into(), json!(v));
        }
        ui.end_row();
    });
    ui.add_space(8.0);
    check(ui, d, "removeRedundantPoints", tl!("Remove Redundant Points"));
    check(ui, d, "removeUnpainted", tl!("Divide and Outline Will Remove Unpainted Artwork"));
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    super::run_and_close(app, "object.pathfinder.options", Value::Object(d.fields.clone()))
}
