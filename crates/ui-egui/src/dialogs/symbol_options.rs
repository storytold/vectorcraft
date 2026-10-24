//! Symbol Options: name, Export Type (Graphic or Movie Clip) and Registration for a new symbol
//! from the selection (`symbol.new`), or for the current symbol in the Symbols panel
//! (`symbol.options`).
//!
//! Fields: `name`, `nineSlice` (bool) with `sliceLeft` / `sliceTop` / `sliceRight` / `sliceBottom`
//! (percent of the symbol's size), `exportType` (`movieClip` | `graphic`), `registration` (0..8, the 9-point grid;
//! 4 = centre), `__edit` (true when editing an existing symbol). Help:
//! <https://helpx.adobe.com/illustrator/desktop/manage-objects/traces-mockups-symbols/create-and-place-symbols.html>.

use serde_json::{Map, Value, json};

use super::swatch_options::{grid, label};
use super::{DialogSpec, form};
use crate::VectorcraftApp;
use crate::state::Dialog;
use crate::widgets;

/// The dialog kind of Symbol Options.
pub const KIND: &str = "symbolOptions";

/// The 9-slice guide fields, in `[left, top, right, bottom]` order.
const SLICE_KEYS: [&str; 4] = ["sliceLeft", "sliceTop", "sliceRight", "sliceBottom"];

/// The 9-slice dialog fields for `guides` (percent of the symbol's size; a third each when off).
fn slice_fields(guides: Option<vectorcraft_doc::SymbolNineSlice>) -> Map<String, Value> {
    let g = guides.unwrap_or_default();
    let mut m = Map::new();
    m.insert("nineSlice".into(), json!(guides.is_some()));
    for (key, v) in SLICE_KEYS.iter().zip([g.left, g.top, g.right, g.bottom]) {
        m.insert((*key).to_string(), json!((v * 100.0).round()));
    }
    m
}

/// The guides the dialog's fields describe, as the `nineSlice` / `nineSliceGuides` params.
fn slice_params(d: &Dialog) -> Value {
    let [l, t, r, b] = SLICE_KEYS.map(|key| d.f64(key, 100.0 / 3.0) / 100.0);
    if d.bool("nineSlice") { json!({ "nineSlice": true, "nineSliceGuides": [l, t, r, b] }) } else { json!({ "nineSlice": false }) }
}

/// Export Type choices: (value, label). Illustrator's default is Movie Clip.
const EXPORT_TYPES: [(&str, &str); 2] = [("movieClip", "Movie Clip"), ("graphic", "Graphic")];

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |_| tl!("Symbol Options").into(), body, confirm, min_width: 320.0, ..DialogSpec::FORM };

/// Open Symbol Options to name a new symbol made from the selection.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document open")?;
    if st.selection.is_empty() {
        return Err("nothing selected".into());
    }
    let name = st.doc.unique_symbol_name("New Symbol");
    let mut fields = json!({ "name": name, "exportType": "movieClip", "registration": 4, "__edit": false });
    if let Some(o) = fields.as_object_mut() {
        o.extend(slice_fields(None));
    }
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

/// Open Symbol Options on an existing symbol (`name`, else the Symbols panel's current).
pub fn open_edit(app: &mut VectorcraftApp, name: Option<&str>) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document open")?;
    let name = match name.filter(|n| !n.is_empty()) {
        Some(n) if st.doc.symbols.iter().any(|s| s.name == n) => n.to_string(),
        Some(n) => return Err(format!("no symbol named `{n}`")),
        None => st
            .doc
            .unknown
            .get("currentSymbol")
            .and_then(|v| v.as_str())
            .filter(|n| st.doc.symbols.iter().any(|s| s.name == *n))
            .map(str::to_string)
            .or_else(|| st.doc.symbols.first().map(|s| s.name.clone()))
            .ok_or_else(|| "the document has no symbols".to_string())?,
    };
    let export = st.doc.symbol_export_type(&name).id();
    let registration = st.doc.symbol_registration(&name).0;
    let mut fields = json!({
        "name": name,
        "exportType": export,
        "registration": registration,
        "__edit": true,
        "__from": name,
    });
    if let Some(o) = fields.as_object_mut() {
        o.extend(slice_fields(st.doc.symbol_nine_slice(&name)));
    }
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    grid(ui, |ui| {
        label(ui, tl!("Name:"));
        form::text(ui, d, "name", 190.0);
        ui.end_row();
        label(ui, tl!("Export Type:"));
        let cur = d.str("exportType");
        let shown = EXPORT_TYPES.iter().find(|(v, _)| *v == cur).map_or(EXPORT_TYPES[0].1, |(_, l)| *l);
        let labels = EXPORT_TYPES.map(|(_, l)| l);
        if let Some((value, _)) = widgets::dropdown(ui, "symbol-export-type", shown, &labels, 190.0).and_then(|i| EXPORT_TYPES.get(i)) {
            d.fields.insert("exportType".into(), json!(value));
        }
        ui.end_row();
        label(ui, tl!("Registration:"));
        let cur = d.fields.get("registration").and_then(Value::as_u64).unwrap_or(4) as usize;
        if let Some(i) = widgets::reference_point(ui, cur) {
            d.fields.insert("registration".into(), json!(i));
        }
        ui.end_row();
        label(ui, "");
        form::check(ui, d, "nineSlice", tl!("Enable Guides for 9-Slice Scaling"));
        ui.end_row();
        if d.bool("nineSlice") {
            for (key, text) in SLICE_KEYS.into_iter().zip([tl!("Left:"), tl!("Top:"), tl!("Right:"), tl!("Bottom:")]) {
                label(ui, text);
                let v = d.f64(key, 100.0 / 3.0);
                if let Some(n) = widgets::range_field(ui, ("dlg-field", key), v, 0.0..=100.0, "%", 0, 64.0) {
                    d.fields.insert(key.into(), json!(n));
                }
                ui.end_row();
            }
        }
    });
    false
}

/// OK: make a new symbol, or apply name / Export Type / Registration on an existing one.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let name = d.str("name");
    let name = name.trim();
    if name.is_empty() {
        return Err("name is empty".into());
    }
    let export = d.str("exportType");
    let registration = d.fields.get("registration").and_then(Value::as_u64).unwrap_or(4);
    let edit = d.fields.get("__edit").and_then(Value::as_bool).unwrap_or(false);
    let r = if edit {
        let from = d.fields.get("__from").and_then(Value::as_str).unwrap_or(name);
        let mut params = json!({ "name": from, "exportType": export, "registration": registration });
        merge(&mut params, slice_params(d));
        if from != name {
            params["newName"] = json!(name);
        }
        app.run("symbol.options", params)
    } else {
        let mut params = json!({ "name": name, "exportType": export, "registration": registration });
        merge(&mut params, slice_params(d));
        app.run("symbol.new", params)
    };
    if r.is_ok() {
        app.ui.dialog = None;
    }
    r
}

/// Copy the keys of the object `extra` into the object `into`.
fn merge(into: &mut Value, extra: Value) {
    if let (Some(a), Value::Object(b)) = (into.as_object_mut(), extra) {
        a.extend(b);
    }
}
