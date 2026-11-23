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

/// The Pathfinder Options that `effect.apply` gives a new Pathfinder effect, as its params.
pub fn options_in_force(app: &VectorcraftApp) -> Value {
    vectorcraft_engine::cmd::pathfinder_effect_options(&app.session).to_params()
}

/// The params that an Effect › Pathfinder menu item with params `p` (`effect.apply`) runs with: its
/// `params` with the Pathfinder Options in force ([`options_in_force`]) added where they are
/// missing, as `effect.apply` journals them. Insert Menu Item stores the same params as the item's
/// step. Params of any other effect are returned unchanged.
pub fn menu_effect_params(app: &VectorcraftApp, mut p: Value) -> Value {
    if !p.get("effect").and_then(Value::as_str).is_some_and(vectorcraft_effects::takes_pathfinder_options) {
        return p;
    }
    let options = options_in_force(app);
    if let Some(o) = p.as_object_mut() {
        let params = o.entry("params").or_insert_with(|| json!({}));
        if let (Some(params), Some(options)) = (params.as_object_mut(), options.as_object()) {
            for (key, v) in options {
                params.entry(key.clone()).or_insert_with(|| v.clone());
            }
        }
    }
    p
}

fn check(ui: &mut egui::Ui, d: &mut Dialog, key: &str, text: &str) {
    let v = d.bool(key);
    if widgets::check(ui, text, v, true) {
        d.fields.insert(key.into(), json!(!v));
    }
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    fields(ui, d);
    false
}

/// Precision, Remove Redundant Points and Divide and Outline Will Remove Unpainted Artwork, editing
/// `precision`, `removeRedundantPoints` and `removeUnpainted` in `d` (this dialog's, and a
/// Pathfinder effect's in the effect dialog).
pub(super) fn fields(ui: &mut egui::Ui, d: &mut Dialog) {
    let range = vectorcraft_engine::cmd::PATHFINDER_PRECISION_RANGE;
    grid(ui, |ui| {
        label(ui, tl!("Precision:"));
        let v = d.f64("precision", vectorcraft_engine::PathfinderOptions::default().precision);
        if let Some(v) = widgets::range_field(ui, "pathfinder-precision", v, range, " pt", 3, 80.0) {
            d.fields.insert("precision".into(), json!(v));
            // An entered precision replaces a stored one out of range, the bound included.
            d.fields.remove(super::effect::OPENED_PRECISION);
        }
        ui.end_row();
    });
    ui.add_space(8.0);
    check(ui, d, "removeRedundantPoints", tl!("Remove Redundant Points"));
    check(ui, d, "removeUnpainted", tl!("Divide and Outline Will Remove Unpainted Artwork"));
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    super::run_and_close(app, "object.pathfinder.options", Value::Object(d.fields.clone()))
}
