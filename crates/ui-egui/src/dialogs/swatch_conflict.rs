//! Swatch Conflict: pasting objects whose global or spot swatch has a name the document gives
//! another colour asks, swatch by swatch, whether to merge (the objects take the document's
//! swatch) or add (the pasted swatch comes in under a new name); Apply to All answers the rest the
//! same way. OK on the last one runs the paste with `swatchConflict`; Cancel pastes nothing.
//!
//! Fields: `conflicts` (as `clipboard.conflicts` lists them: `[{name, document, clipboard, spot}]`),
//! `index` (the one asked about), `choice` (`merge` or `add`), `applyToAll`, `choices` (the
//! answers so far, by swatch name), `__command` and `__params` (the paste to run).

use egui::vec2;
use serde_json::{Map, Value, json};
use vectorcraft_color::{Color, Paint};

use super::swatch_options::{grid, label};
use super::{DialogResult, DialogSpec};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Swatch Conflict.
pub const KIND: &str = "swatchConflict";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Swatch Conflict").into(), body, confirm, min_width: 380.0, max_width: Some(440.0), ..DialogSpec::FORM };

/// The choices: (`choice` value, label, what it does).
const CHOICES: [(&str, &str, &str); 2] = [
    ("merge", "Merge Swatches", "Use the document's swatch: pasted colors take its color."),
    ("add", "Add Swatches", "Keep the pasted color as a new swatch with a numbered name."),
];

/// Before running paste `command` with `params`: when the pasted objects' swatches conflict with
/// the document's and `params` don't say how to resolve them (`swatchConflict`), open the dialog
/// instead and answer `{dialog}`.
pub fn ask(app: &mut VectorcraftApp, command: &str, params: &Value) -> Option<DialogResult> {
    if params.get("swatchConflict").is_some() {
        return None;
    }
    let found = app.session.execute("clipboard.conflicts", &json!({})).ok()?;
    let conflicts = found["swatches"].as_array().filter(|a| !a.is_empty())?.clone();
    let fields = json!({
        "conflicts": conflicts, "index": 0, "choice": "merge", "applyToAll": false, "choices": {},
        "__command": command, "__params": params,
    });
    if app.ui.dialog.as_ref().is_some_and(|d| d.kind == KIND) {
        crate::placing::cancel_swatch_paste(app);
    }
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Some(Ok(json!({ "dialog": KIND })))
}

fn conflicts(d: &Dialog) -> &[Value] {
    d.fields.get("conflicts").and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

fn index(d: &Dialog) -> usize {
    d.fields.get("index").and_then(Value::as_u64).unwrap_or(0) as usize
}

/// A chip of a `#rrggbb` colour with a caption under it.
fn chip(ui: &mut egui::Ui, hex: Option<&str>, caption: &str) {
    ui.vertical(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(56.0, 28.0), egui::Sense::hover());
        let paint = hex.and_then(Color::from_hex).map_or(Paint::None, Paint::solid);
        widgets::paint_chip(ui, r, &paint);
        label(ui, caption);
    });
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let i = index(d);
    let (count, Some(c)) = (conflicts(d).len(), conflicts(d).get(i).cloned()) else { return true };
    let name = c["name"].as_str().unwrap_or_default();
    ui.label(crate::i18n::fmt(tl!("This document already has a swatch named “{name}” with another color."), &[("name", name)]));
    if count > 1 {
        label(ui, &crate::i18n::fmt(tl!("Conflict {n} of {count}"), &[("n", &(i + 1).to_string()), ("count", &count.to_string())]));
    }
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        chip(ui, c["document"].as_str(), tl!("Document"));
        ui.add_space(12.0);
        chip(ui, c["clipboard"].as_str(), tl!("Pasted"));
    });
    ui.add_space(10.0);
    let choice = d.str("choice");
    grid(ui, |ui| {
        for (k, (value, text, help)) in CHOICES.into_iter().enumerate() {
            label(ui, if k == 0 { tl!("Options:") } else { "" });
            ui.vertical(|ui| {
                if widgets::radio(ui, tl!(text), choice == value, true) {
                    d.fields.insert("choice".into(), json!(value));
                }
                ui.indent(text, |ui| label(ui, tl!(help)));
            });
            ui.end_row();
        }
    });
    ui.add_space(6.0);
    let all = d.bool("applyToAll");
    if widgets::check(ui, tl!("Apply to All"), all, count > 1) {
        d.fields.insert("applyToAll".into(), json!(!all));
    }
    false
}

/// Record the answer; ask about the next conflict, or paste once every one is answered.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> DialogResult {
    let choice = d.str("choice");
    if !matches!(choice.as_str(), "merge" | "add") {
        return Err(format!("choice must be \"merge\" or \"add\", not `{choice}`"));
    }
    let list = conflicts(d);
    let i = index(d);
    let all = d.bool("applyToAll");
    let mut choices: Map<String, Value> = d.fields.get("choices").and_then(Value::as_object).cloned().unwrap_or_default();
    let answered = if all { list.get(i..).unwrap_or_default() } else { list.get(i..=i).unwrap_or_default() };
    for c in answered {
        if let Some(n) = c["name"].as_str() {
            choices.insert(n.to_string(), json!(choice));
        }
    }
    if !all && i + 1 < list.len() {
        let mut next = d.clone();
        next.fields.insert("index".into(), json!(i + 1));
        next.fields.insert("choices".into(), Value::Object(choices));
        app.ui.dialog = Some(next);
        return Ok(Value::Null);
    }
    let mut params = d.fields.get("__params").cloned().filter(Value::is_object).unwrap_or_else(|| json!({}));
    params["swatchConflict"] = Value::Object(choices);
    app.ui.dialog = None;
    crate::placing::confirm_swatch_paste(app, &d.str("__command"), params, d.fields.get("__deferredPaste").and_then(Value::as_u64))
}
