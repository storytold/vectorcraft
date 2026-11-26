//! Type Tools in the interface: its submenu (Object › Plug-ins › Type Tools) and Text Tweaker's
//! dialog, which picks an attribute, its start and end values and a shape, previews on the canvas
//! and runs `addon.type.textTweaker` on OK.

use craft::menu::Node;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamDialog, ParamKind};
use craft::ui::runtime::env::UnitKind;
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::addons::dialogs::{self, Spec};
use crate::menus::{c, cp};

/// The command the dialog runs.
pub const COMMAND: &str = "addon.type.textTweaker";
/// An attribute: (param, label, unit, decimals, values taken, default start and end), as the
/// command takes it.
type Attribute = (&'static str, &'static str, &'static str, usize, (f64, f64), (f64, f64));

const ATTRIBUTES: [Attribute; 8] = [
    ("size", "Font Size", "pt", 1, (0.1, 1296.0), (12.0, 36.0)),
    ("baselineShift", "Baseline Shift", "pt", 1, (-1296.0, 1296.0), (0.0, 12.0)),
    ("rotation", "Character Rotation", "°", 1, (-360.0, 360.0), (0.0, 30.0)),
    ("hScale", "Horizontal Scale", "%", 0, (1.0, 10000.0), (100.0, 150.0)),
    ("vScale", "Vertical Scale", "%", 0, (1.0, 10000.0), (100.0, 150.0)),
    ("tracking", "Tracking", "", 0, (-1000.0, 10000.0), (0.0, 200.0)),
    ("leading", "Leading", "pt", 1, (0.1, 5000.0), (12.0, 36.0)),
    ("strokeWidth", "Stroke Weight", "pt", 2, (0.0, 1000.0), (0.0, 2.0)),
];
const SHAPES: [(&str, &str); 5] = [("straight", "Straight"), ("ease", "Ease In Out"), ("bow", "Bow"), ("spike", "Spike"), ("random", "Random")];
/// The seeds the dialog offers.
const SEEDS: u64 = 1_000_000;

/// Release Type on a Path's command. Its menu item opens the generic command dialog
/// (`ui.paramDialog`) on these options, with a Preview (`dialogs::command`).
pub const RELEASE: &str = "addon.type.releasePath";
/// Release Type on a Path › Keep: what the command's `mode` takes, by name.
pub const RELEASE_MODES: &[(&str, &str)] = &[("Text and Path", "both"), ("Path Only", "path"), ("Text Only", "text")];

/// Object › Plug-ins › Type Tools.
pub fn menu() -> Vec<Node> {
    vec![c("Text Tweaker…", COMMAND), cp("Release Type on a Path…", RELEASE, json!({"mode": "both", "keepOriginal": false}))]
}

/// The attribute row of param `key` (the first, Font Size, for an unknown one).
fn attribute(key: &str) -> &'static Attribute {
    ATTRIBUTES.iter().find(|a| a.0 == key).unwrap_or(&ATTRIBUTES[0])
}

/// How the dialog's values reach the command.
pub(crate) static SPEC: Spec = Spec { command: COMMAND, params, found: None };

/// The command's parameters from the dialog's fields; an end of the ramp not set yet is the attribute's own.
pub(crate) fn params(fields: &DialogFields) -> Value {
    let &(key, _, _, _, _, (from, to)) = attribute(fields.text("attribute").unwrap_or("size"));
    json!({
        "attribute": key,
        "from": fields.number("from").unwrap_or(from),
        "to": fields.number("to").unwrap_or(to),
        "shape": fields.text("shape").unwrap_or("straight"),
        "seed": fields.int("seed").unwrap_or(0),
        "skipSpaces": fields.bool("skipSpaces").unwrap_or(true),
    })
}

/// The unit an attribute's values are shown in.
fn unit_kind(unit: &str) -> Option<UnitKind> {
    match unit {
        "pt" => Some(UnitKind::Points),
        "°" => Some(UnitKind::Degrees),
        "%" => Some(UnitKind::Percent),
        _ => None,
    }
}

/// The choice over `list`'s `(id, label)` pairs, on its first.
fn choice(list: &[(&str, &'static str)]) -> ParamKind {
    ParamKind::choice(list.iter().map(|&(id, label)| (id.to_string(), tl!(label).into())).collect(), list.first().map(|(id, _)| id.to_string()))
}

/// Opens Text Tweaker's dialog on Font Size from 12 to 36 pt in a straight ramp, and a new seed each time the
/// document has changed, so running it again varies a random result.
pub(crate) fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let seed = app.session.active().map_or(0, |st| st.revision % SEEDS);
    let fields = DialogFields::from_pairs([
        ("attribute", FieldValue::Text("size".into())),
        ("from", FieldValue::Number(12.0)),
        ("to", FieldValue::Number(36.0)),
        ("shape", FieldValue::Text("straight".into())),
        ("seed", FieldValue::Int(i64::try_from(seed).unwrap_or(0))),
        ("skipSpaces", FieldValue::Bool(true)),
    ]);
    dialogs::open(app, &SPEC, fields, move |fields| {
        let &(_, _, unit, decimals, (lo, hi), (from, to)) = attribute(fields.text("attribute").unwrap_or("size"));
        // Another attribute starts from its own values: changing the attribute removes the ends it resets.
        for (field, start) in [("from", from), ("to", to)] {
            if !fields.contains(field) {
                fields.set(field, FieldValue::Number(start));
            }
        }
        let places = u32::try_from(decimals).unwrap_or(0);
        let step = if places == 0 { 1.0 } else { 0.1 };
        let ends = [("from", from, tl!("From:")), ("to", to, tl!("To:"))].map(|(field, start, label)| {
            let row = Param::number(field, label, start, step, places).bounds(Some(lo), Some(hi));
            match unit_kind(unit) {
                Some(kind) => row.unit(kind),
                None => row,
            }
        });
        let attributes: Vec<(&str, &str)> = ATTRIBUTES.iter().map(|a| (a.0, a.1)).collect();
        let mut rows = vec![Param::new("attribute", tl!("Attribute:"), choice(&attributes)).resets(["from", "to"])];
        rows.extend(ends);
        rows.extend([
            Param::new("shape", tl!("Shape:"), choice(&SHAPES)),
            Param::new("seed", tl!("Seed:"), ParamKind::int(Some(0), Some(i64::try_from(SEEDS - 1).unwrap_or(i64::MAX)), 0))
                .enabled_when("shape", FieldValue::Text("random".into())),
            Param::new("skipSpaces", tl!("Skip Spaces"), ParamKind::bool(true)),
        ]);
        ParamDialog::new(COMMAND, tl!("Text Tweaker"), rows).previews(true)
    })
}

#[cfg(test)]
mod tests;
