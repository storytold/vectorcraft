//! Color Effects' panel and submenu (Object › Plug-ins › Color Effects). The panel picks a colour
//! adjustment and its values, then adds it to the selection as a live effect or applies it as a
//! filter, into the colours themselves (`addon.color.filter`); below, under Inks, it opens Duotone's
//! dialog and Print Ink Fixes' (`addon.color.printFixes`). The submenu applies the add-on's live
//! effects, Duotone and Vector Halftone among them (Effect › Color Effects has them too), opens
//! Print Ink Fixes and opens the panel.

use egui::Ui;
use serde_json::{Map, Value, json};

use crate::VectorcraftApp;
use crate::menus::Item;
use crate::panels::{empty_state, pstate, set_pstate};
use crate::widgets;

/// The panel's id (`window.panel {panel: "colorEffects"}`).
pub const ID: &str = "colorEffects";

/// Where the panel keeps its choice and values (panel state).
const STATE: &str = "color-effects";

/// A value of an adjustment: (parameter, label, range, decimals, default).
type Field = (&'static str, &'static str, (f64, f64), usize, f64);

/// The adjustments the panel offers: (effect, label, values). Curves and Shift to Color, which take
/// a curve or a colour, keep to their dialogs (Effect › Color Adjustments).
const ADJUSTMENTS: [(&str, &str, &[Field]); 7] = [
    (
        "adjust.brightnessContrast",
        "Brightness/Contrast",
        &[("brightness", "Brightness", (-100.0, 100.0), 0, 0.0), ("contrast", "Contrast", (-100.0, 100.0), 0, 0.0)],
    ),
    (
        "adjust.hueSaturation",
        "Hue/Saturation",
        &[
            ("hue", "Hue", (-180.0, 180.0), 0, 0.0),
            ("saturation", "Saturation", (-100.0, 100.0), 0, 0.0),
            ("lightness", "Lightness", (-100.0, 100.0), 0, 0.0),
        ],
    ),
    (
        "adjust.levels",
        "Levels",
        &[
            ("inputBlack", "Input Black", (0.0, 255.0), 0, 0.0),
            ("inputWhite", "Input White", (0.0, 255.0), 0, 255.0),
            ("gamma", "Gamma", (0.1, 10.0), 2, 1.0),
            ("outputBlack", "Output Black", (0.0, 255.0), 0, 0.0),
            ("outputWhite", "Output White", (0.0, 255.0), 0, 255.0),
        ],
    ),
    (
        "adjust.temperatureTint",
        "Temperature/Tint",
        &[("temperature", "Temperature", (-100.0, 100.0), 0, 0.0), ("tint", "Tint", (-100.0, 100.0), 0, 0.0)],
    ),
    (
        "addon.color.exposure",
        "Exposure",
        &[("exposure", "Exposure", (-20.0, 20.0), 2, 0.0), ("offset", "Offset", (-0.5, 0.5), 3, 0.0), ("gamma", "Gamma", (0.1, 10.0), 2, 1.0)],
    ),
    ("addon.color.desaturate", "Desaturate", &[]),
    ("addon.color.invert", "Invert Colors", &[]),
];

/// Print Ink Fixes' menu item: its label, command and the options its dialog opens with (the
/// selection, or the document while nothing is selected).
fn print_fixes() -> (&'static str, &'static str, Value) {
    let options = json!({
        "inkLimit": 300, "excludePureBlack": true, "removeWhiteOverprint": true, "overprintBlack": 100,
        "textOnly": false, "fill": true, "stroke": true,
    });
    ("Print Ink Fixes…", "addon.color.printFixes", options)
}

/// Duotone's menu item: its label, and its effect's dialog.
fn duotone() -> (&'static str, &'static str, Value) {
    ("Duotone…", "effect.dialog", json!({ "effect": vectorcraft_effects::addons::duotone::DUOTONE }))
}

/// Adjustment `i` of [`ADJUSTMENTS`] (the first past the end).
fn adjustment(i: usize) -> (&'static str, &'static str, &'static [Field]) {
    ADJUSTMENTS.get(i).copied().unwrap_or(ADJUSTMENTS[0])
}

/// Object › Plug-ins › Color Effects.
pub fn menu() -> Vec<Item> {
    let cmd = |(label, id, p): (&'static str, &'static str, Value)| Item::Cmd(label, id, p);
    vec![
        Item::Cmd("Color Effects Panel", "window.panel", json!({ "panel": ID })),
        Item::Sep,
        Item::Cmd("Exposure…", "effect.dialog", json!({ "effect": "addon.color.exposure" })),
        Item::Cmd("Desaturate", "effect.apply", json!({ "effect": "addon.color.desaturate" })),
        cmd(duotone()),
        Item::Cmd("Invert Colors", "effect.apply", json!({ "effect": "addon.color.invert" })),
        Item::Cmd("Vector Halftone…", "effect.dialog", json!({ "effect": "addon.color.halftone" })),
        Item::Sep,
        cmd(print_fixes()),
        Item::Sep,
        Item::Sub(
            "Select by Color Kind",
            [
                ("Objects with Spot Colors", "spotColors"),
                ("Objects with Color Effects", "colorEffects"),
                ("RGB Images", "rgbImages"),
                ("CMYK Images", "cmykImages"),
                ("Grayscale Images", "grayImages"),
            ]
            .into_iter()
            .map(|(label, kind)| Item::Cmd(label, "addon.color.select", json!({ "kind": kind })))
            .collect(),
        ),
    ]
}

/// The panel's choice and the values set for each adjustment.
#[derive(Clone, Default)]
struct Picked {
    /// Index in [`ADJUSTMENTS`].
    index: usize,
    /// Values set, by effect id then parameter.
    values: Map<String, Value>,
}

impl Picked {
    /// The params of the adjustment picked: its values, the defaults where none is set.
    fn params(&self) -> (&'static str, Value) {
        let (id, _, fields) = adjustment(self.index);
        let set = self.values.get(id).and_then(Value::as_object);
        let p: Map<String, Value> =
            fields.iter().map(|(k, _, _, _, default)| (k.to_string(), set.and_then(|m| m.get(*k)).cloned().unwrap_or(json!(default)))).collect();
        (id, Value::Object(p))
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    if app.session.active().is_none_or(|d| d.selection.is_empty()) {
        empty_state(ui, "sun", tl!("No selection"), tl!("Select objects to adjust their colors."));
        return;
    }
    let mut st: Picked = pstate(ui.ctx(), STATE);
    let labels: Vec<&str> = ADJUSTMENTS.iter().map(|a| a.1).collect();
    let (id, label, fields) = adjustment(st.index);
    const LABEL_W: f32 = 84.0;
    widgets::label_row(ui, tl!("Adjustment"), LABEL_W, |ui| {
        if let Some(i) = widgets::dropdown(ui, "color-effects-pick", label, &labels, 150.0) {
            st.index = i;
        }
    });
    let (_, params) = st.params();
    for (key, name, (min, max), decimals, _) in fields {
        let v = params.get(*key).and_then(Value::as_f64);
        widgets::label_row(ui, tl!(name), LABEL_W, |ui| {
            if let Some(n) = widgets::range_field(ui, ("color-effects", id, key), v, *min..=*max, "", *decimals, 70.0) {
                let set = st.values.entry(id).or_insert_with(|| json!({}));
                if let Some(m) = set.as_object_mut() {
                    m.insert(key.to_string(), json!(n));
                }
            }
        });
    }
    ui.add_space(4.0);
    let (id, params) = st.params();
    ui.horizontal(|ui| {
        let run = |app: &mut VectorcraftApp, cmd: &str| {
            if let Err(e) = app.run(cmd, json!({ "effect": id, "params": params })) {
                app.status(e);
            }
        };
        if widgets::flat_button(ui, tl!("Add as Effect"), 100.0).clicked() {
            run(app, "effect.apply");
        }
        if widgets::flat_button(ui, tl!("Apply as Filter"), 100.0).clicked() {
            run(app, "addon.color.filter");
        }
        if widgets::flat_button(ui, tl!("Reset"), 56.0).clicked() {
            st.values.remove(id);
        }
    });
    set_pstate(ui.ctx(), STATE, st);
    // Inks: Duotone's and Print Ink Fixes' dialogs, as their menu items open them.
    ui.add_space(8.0);
    widgets::subheader(ui, tl!("Inks"));
    let width = ui.available_width();
    for (label, id, p) in [duotone(), print_fixes()] {
        if widgets::flat_button(ui, tl!(label), width).clicked() {
            let (id, p) = crate::menus::click_target(label, id, &p);
            crate::menus::invoke(app, &id, p);
        }
    }
}

#[cfg(test)]
mod tests;
