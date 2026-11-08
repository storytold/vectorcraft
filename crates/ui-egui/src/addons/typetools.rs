//! Type Tools in the interface: its submenu (Object › Plug-ins › Type Tools) and Text Tweaker's
//! dialog, which picks an attribute, its start and end values and a shape, previews on the canvas
//! and runs `addon.type.textTweaker` on OK.

use egui::Ui;
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::dialogs::{DialogSpec, form};
use crate::menus::Item;
use crate::state::Dialog;
use crate::widgets;

/// The command the dialog runs, and its kind.
pub const COMMAND: &str = "addon.type.textTweaker";
pub const KIND: &str = "textTweaker";
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
const LABEL_W: f32 = 90.0;
const FIELD_W: f32 = 90.0;

/// Release Type on a Path's command. Its menu item opens the generic command dialog
/// (`ui.paramDialog`) on these options, with a Preview (`dialogs::command`).
pub const RELEASE: &str = "addon.type.releasePath";
/// Release Type on a Path › Keep: what the command's `mode` takes, by name.
pub const RELEASE_MODES: &[(&str, &str)] = &[("Text and Path", "both"), ("Path Only", "path"), ("Text Only", "text")];

/// Object › Plug-ins › Type Tools.
pub fn menu() -> Vec<Item> {
    vec![
        Item::Cmd("Text Tweaker…", COMMAND, Value::Null),
        Item::Cmd("Release Type on a Path…", RELEASE, json!({"mode": "both", "keepOriginal": false})),
    ]
}

/// The attribute row of param `key` (the first, Font Size, for an unknown one).
fn attribute(key: &str) -> &'static Attribute {
    ATTRIBUTES.iter().find(|a| a.0 == key).unwrap_or(&ATTRIBUTES[0])
}

/// The dialog's fields to start with: Font Size from 12 to 36 pt in a straight ramp, and a new
/// seed each time the document has changed, so running it again varies a random result.
pub(crate) fn fields(app: &VectorcraftApp) -> Value {
    let seed = app.session.active().map_or(0, |st| st.revision % SEEDS);
    json!({
        "attribute": "size", "__shown": "size", "from": 12, "to": 36, "shape": "straight", "seed": seed, "skipSpaces": true,
    })
}

pub(crate) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Text Tweaker").to_string(), body, confirm, preview: true, min_width: 320.0, ..DialogSpec::FORM };

fn body(app: &mut VectorcraftApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let widths = (LABEL_W, 170.0);
    let options: Vec<(&str, &str)> = ATTRIBUTES.iter().map(|a| (a.0, a.1)).collect();
    form::choice(ui, d, "attribute", tl!("Attribute:"), widths, &options);
    let &(key, _, unit, decimals, (lo, hi), (from, to)) = attribute(&d.str("attribute"));
    // Another attribute starts from its own values.
    if d.str("__shown") != key {
        d.fields.insert("__shown".into(), json!(key));
        d.fields.insert("from".into(), json!(from));
        d.fields.insert("to".into(), json!(to));
    }
    for (field, label) in [("from", tl!("From:")), ("to", tl!("To:"))] {
        widgets::label_row(ui, label, LABEL_W, |ui| {
            if let Some(v) = widgets::range_field(ui, ("text-tweaker", field), d.f64(field, lo), lo..=hi, unit, decimals, FIELD_W) {
                d.fields.insert(field.into(), json!(v));
            }
        });
    }
    form::choice(ui, d, "shape", tl!("Shape:"), widths, &SHAPES);
    ui.add_enabled_ui(d.str("shape") == "random", |ui| {
        widgets::label_row(ui, tl!("Seed:"), LABEL_W, |ui| {
            if let Some(v) = widgets::range_field(ui, ("text-tweaker", "seed"), d.f64("seed", 0.0), 0.0..=(SEEDS - 1) as f64, "", 0, FIELD_W) {
                d.fields.insert("seed".into(), json!(v as u64));
            }
        });
    });
    form::check(ui, d, "skipSpaces", tl!("Skip Spaces"));
    form::preview(app, ui, d, "Text Tweaker", COMMAND, form::params(d));
    false
}

/// OK: run the command (keep its preview) and close.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    form::commit_preview(app, COMMAND, form::params(d))
}

#[cfg(test)]
mod tests;
