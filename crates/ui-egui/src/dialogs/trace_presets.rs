//! Image Trace panel › Manage Presets: Save as New Preset… keeps the panel's settings as a saved
//! tracing preset under the name entered, and Rename… gives a saved preset another name. OK runs
//! `imageTrace.presets.save`.
//!
//! Fields: `name`, `rename` (the saved preset to rename; empty to save a new one) and `params` (the
//! settings a new preset keeps).

use serde_json::{Value, json};

use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Save as New Preset… and Rename… (Image Trace).
pub const KIND: &str = "imageTracePreset";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |d| if d.str("rename").is_empty() { tl!("Save Preset") } else { tl!("Rename Preset") }.into(),
    body,
    confirm,
    ok_enabled: Some(|_, d| !d.str("name").trim().is_empty()),
    min_width: 320.0,
    ..DialogSpec::FORM
};

/// Open Save as New Preset… for `params` (default: the Default preset's settings), or, with
/// `rename`, Rename… for that saved preset (OK reports a name that isn't one).
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let rename = p.get("rename").and_then(Value::as_str).map(str::trim).unwrap_or_default();
    let fields = if rename.is_empty() {
        json!({"name": app.session.new_trace_preset_name(), "rename": "", "params": p.get("params").cloned().unwrap_or(Value::Null)})
    } else {
        let name = app.session.trace_preset(rename).map_or(rename, |saved| saved.name.as_str());
        json!({"name": name, "rename": name, "params": null})
    };
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Name:"));
        form::text(ui, d, "name", 220.0);
    });
    let name = d.str("name");
    let rename = d.str("rename");
    if rename.is_empty() && app.session.trace_preset(&name).is_some() {
        widgets::dim_label(ui, tl!("Replaces the saved preset of that name."));
    }
    false
}

/// `imageTrace.presets.save` parameters from the fields.
fn params(d: &Dialog) -> Value {
    let rename = d.str("rename");
    if rename.is_empty() {
        json!({"name": d.str("name"), "params": d.fields.get("params").cloned().unwrap_or(Value::Null)})
    } else {
        json!({"name": rename, "newName": d.str("name")})
    }
}

/// A name the save refuses (a taken one when renaming, a built-in preset's) keeps the dialog open,
/// to be changed.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let r = app.run("imageTrace.presets.save", params(d));
    if r.is_ok() {
        app.ui.dialog = None;
    }
    r
}
