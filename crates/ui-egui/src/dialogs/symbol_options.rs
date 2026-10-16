//! Symbol Options: name, Export Type (Graphic or Movie Clip) and Registration for a new symbol
//! from the selection (`symbol.new`), or for the current symbol in the Symbols panel
//! (`symbol.options`).
//!
//! Fields: `name`, `exportType` (`movieClip` | `graphic`), `registration` (0..8, the 9-point grid;
//! 4 = centre), `__edit` (true when editing an existing symbol). Help:
//! <https://helpx.adobe.com/illustrator/desktop/manage-objects/traces-mockups-symbols/create-and-place-symbols.html>.

use serde_json::{Value, json};

use super::swatch_options::{grid, label};
use super::{DialogSpec, form};
use crate::VectorcraftApp;
use crate::state::Dialog;
use crate::widgets;

/// The dialog kind of Symbol Options.
pub const KIND: &str = "symbolOptions";

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
    app.ui.dialog = Some(Dialog::new(KIND, json!({ "name": name, "exportType": "movieClip", "registration": 4, "__edit": false })));
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
    app.ui.dialog = Some(Dialog::new(
        KIND,
        json!({
            "name": name,
            "exportType": export,
            "registration": registration,
            "__edit": true,
            "__from": name,
        }),
    ));
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
        if from != name {
            params["newName"] = json!(name);
        }
        app.run("symbol.options", params)
    } else {
        app.run("symbol.new", json!({ "name": name, "exportType": export, "registration": registration }))
    };
    if r.is_ok() {
        app.ui.dialog = None;
    }
    r
}
