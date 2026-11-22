//! Edit → Edit Colors → Recolor with Preset: before Recolor Artwork opens on a preset (an n-color
//! job or Color Library), this dialog asks whether to limit the new colors to a swatch library
//! (`ui.recolorDialog {preset: true, colors?, library?}`). OK opens Recolor Artwork with the
//! preset's colors and the library chosen (None: no limit); Cancel opens nothing.
//!
//! Fields: `colors` (the preset's count, null for Color Library) and `limitTo` (a library id,
//! `"document"` for the document's swatches, `""` for None), as Recolor Artwork's.

use serde_json::{Value, json};

use super::DialogSpec;
use super::recolor::{library_choices, library_param};
use super::swatch_options::{grid, label};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Recolor with Preset.
pub const KIND: &str = "recolorPreset";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Recolor with Preset").into(), body, confirm, min_width: 340.0, ..DialogSpec::FORM };

/// Open the dialog for the preset of `p` (`colors`: a count; `library`: the library it starts on,
/// "" the first library) on the selected art.
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document open")?;
    if st.selection.is_empty() {
        return Err("select artwork to recolor".into());
    }
    let colors = count(p.get("colors"))?;
    let limit = library_param(app, p)?;
    // A dialog this one replaces ends as Cancel ends it, its preview rolled back.
    if app.ui.dialog.is_some() {
        super::cancel(app);
    }
    app.ui.dialog = Some(Dialog::new(KIND, json!({"colors": colors, "limitTo": limit})));
    Ok(Value::Null)
}

/// The preset's count of new colors: none for Color Library, else at least 1.
fn count(v: Option<&Value>) -> Result<Value, String> {
    match v {
        None | Some(Value::Null) => Ok(Value::Null),
        Some(n) => Ok(json!(n.as_u64().filter(|n| *n >= 1).ok_or("`colors` must be a count of at least 1")?)),
    }
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let (keys, shown) = library_choices(app);
    let shown: Vec<&str> = shown.iter().map(String::as_str).collect();
    let cur = d.str("limitTo");
    let at = keys.iter().position(|k| *k == cur).unwrap_or(0);
    let current = shown.get(at).copied().unwrap_or_default();
    grid(ui, |ui| {
        label(ui, tl!("Limit to Library:"));
        let library = ui.name_inputs(tl!("Limit to Library:"), |ui| widgets::dropdown_names(ui, "recolor-preset-library", current, &shown, 200.0));
        if let Some(k) = library.and_then(|i| keys.get(i)) {
            d.fields.insert("limitTo".into(), json!(k));
        }
        ui.end_row();
    });
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut p = serde_json::Map::new();
    let n = count(d.fields.get("colors"))?;
    if !n.is_null() {
        p.insert("colors".into(), n);
    }
    let limit = d.str("limitTo");
    if !limit.is_empty() {
        p.insert("library".into(), json!(limit));
    }
    // Recolor Artwork takes the dialog's place; when it cannot open, this dialog stays.
    super::recolor::open(app, &Value::Object(p))
}
