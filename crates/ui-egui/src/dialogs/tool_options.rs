//! What the Tool Options dialogs share: their title, setting a tool's options, the reference point row and reading a
//! length a dialog holds.

// PLAN: ui-egui reaches the framework through `craft::..`; the crate's Cargo.toml needs the `craft` dependency
// (hub-w1 owns the manifest).
use craft::behavior::Anchor9;
use craft::ui::contract::{Bind, Show as _};
use craft::ui::patterns::form_dialogs::{DialogFields, Param};
use craft::ui::pickers::{ReferencePoint, ReferencePointLook};
use craft::ui::runtime::env::Label;
use serde_json::{Map, Value, json};
use vectorcraft_doc::Unit;

use crate::VectorcraftApp;

/// The title of `tool`'s options dialog: "{tool} Options".
pub(crate) fn title(tool: &str) -> String {
    let label = vectorcraft_tools::tool_info(tool).map_or("", |t| t.label);
    crate::i18n::fmt(tl!("{tool} Options"), &[("tool", tl!(label))])
}

/// Sets `tool`'s options to `values` (`tool.setOption`), the options its next drag and click use.
pub(crate) fn set_options(app: &mut VectorcraftApp, tool: &str, values: Map<String, Value>) -> Result<(), String> {
    app.run("tool.setOption", json!({"tool": tool, "values": values})).map(|_| ())
}

/// The reference point `key` holds as an index 0-8 into [`Anchor9::ALL`] (top left first, row by row); an
/// index past 8 is 8 and a missing or negative one is 0.
pub(crate) fn reference_index(fields: &DialogFields, key: &str) -> usize {
    fields.int(key).map_or(0, |n| n.clamp(0, 8) as usize)
}

/// A row of the nine-point reference picker over the index `key` holds ([`reference_index`]).
pub(crate) fn reference_param(key: &str, label: impl Into<Label>) -> Param {
    let name = key.to_owned();
    Param::custom(key, label, move |ui, fields, _| {
        let shown = reference_index(fields, &name);
        let mut anchor = Some(Anchor9::ALL.get(shown).copied().unwrap_or(Anchor9::TopLeft));
        let edit = ReferencePoint::new(tl!("Reference Point"), Bind::bound(&mut anchor)).look(ReferencePointLook::Squares).show(ui);
        if let Some(picked) = Anchor9::ALL.iter().position(|point| Some(*point) == anchor).filter(|at| *at != shown) {
            fields.set(name.clone(), picked as i64);
        }
        edit.response
    })
    .opens_with(0_i64)
}

/// The length `key` holds in points: a number as it is, a text as a length with or without a unit
/// ("25.4 mm"), so automation that types a unit still reads. `default` when the key is missing or does not read.
pub(crate) fn length_of(fields: &DialogFields, key: &str, default: f64) -> f64 {
    fields.number(key).or_else(|| fields.text(key).and_then(|text| Unit::Points.parse(text))).unwrap_or(default)
}
