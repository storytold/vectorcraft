//! Color Effects' panel and submenu (Object › Plug-ins › Color Effects). The panel picks a colour
//! adjustment and its values, then adds it to the selection as a live effect or applies it as a
//! filter, into the colours themselves (`addon.color.filter`); below, under Inks, it opens Duotone's
//! dialog and Print Ink Fixes' (`addon.color.printFixes`). The submenu applies the add-on's live
//! effects, Duotone and Vector Halftone among them (Effect › Color Effects has them too), opens
//! Print Ink Fixes, Find Nearest Swatch ([`nearest`]) and Create Swatch Legend ([`legend`]) and
//! opens the panel.

pub mod legend;
pub mod nearest;

use craft::imaging::ui::adjust::{AdjustContext, AdjustField, AdjustSliders};
use craft::menu::Node;
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Entry, Show as _, ShowContainer as _};
use craft::ui::form::{Field, FieldGroup};
use craft::ui::pickers::Select;
use craft::ui::sections::Section;
use egui::Ui;
use serde_json::{Map, Value, json};

use crate::VectorcraftApp;
use crate::menus::{Sep, c, cp, sub};
use crate::panels::empty_state;

/// The panel's id (`window.panel {panel: "colorEffects"}`).
pub const ID: &str = "colorEffects";

/// A value of an adjustment: (parameter, label, range, decimals, default).
type AdjustValue = (&'static str, &'static str, (f64, f64), usize, f64);

/// The adjustments the panel offers: (effect, label, values). Curves and Shift to Color, which take
/// a curve or a colour, keep to their dialogs (Effect › Color Adjustments).
const ADJUSTMENTS: [(&str, &str, &[AdjustValue]); 7] = [
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
fn adjustment(i: usize) -> (&'static str, &'static str, &'static [AdjustValue]) {
    ADJUSTMENTS.get(i).copied().unwrap_or(ADJUSTMENTS[0])
}

/// Object › Plug-ins › Color Effects.
pub fn menu() -> Vec<Node> {
    let cmd = |(label, id, p): (&'static str, &'static str, Value)| cp(label, id, p);
    vec![
        cp("Color Effects Panel", "window.panel", json!({ "panel": ID })),
        Sep,
        cp("Exposure…", "effect.dialog", json!({ "effect": "addon.color.exposure" })),
        cp("Desaturate", "effect.apply", json!({ "effect": "addon.color.desaturate" })),
        cmd(duotone()),
        cp("Invert Colors", "effect.apply", json!({ "effect": "addon.color.invert" })),
        cp("Vector Halftone…", "effect.dialog", json!({ "effect": "addon.color.halftone" })),
        Sep,
        cmd(print_fixes()),
        c("Find Nearest Swatch…", nearest::COMMAND),
        c("Create Swatch Legend…", legend::COMMAND),
        Sep,
        // Each selected object's stroke takes its own fill, or the other way (#5530).
        cp("Apply Fill to Stroke", "addon.color.fillToStroke", json!({})),
        cp("Apply Stroke to Fill", "addon.color.strokeToFill", json!({})),
        Sep,
        sub(
            "Select by Color Kind",
            [
                ("Objects with Spot Colors", "spotColors"),
                ("Objects with Color Effects", "colorEffects"),
                ("RGB Images", "rgbImages"),
                ("CMYK Images", "cmykImages"),
                ("Grayscale Images", "grayImages"),
            ]
            .into_iter()
            .map(|(label, kind)| cp(label, "addon.color.select", json!({ "kind": kind })))
            .collect(),
        ),
    ]
}

/// The panel's choice and the values set for each adjustment, kept at `app.addon_panels.color`.
#[derive(Clone, Default)]
pub(crate) struct Picked {
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
    let mut st = std::mem::take(&mut app.addon_panels.color);
    let entries: Vec<Entry<usize>> = ADJUSTMENTS.iter().enumerate().map(|(i, a)| Entry::new(i, tl!(a.1).to_string())).collect();
    FieldGroup::new(tl!("Adjustment").to_string()).show(ui, |ui, _| {
        Field::new(tl!("Adjustment").to_string()).show(ui, |ui, _| {
            Select::new(tl!("Adjustment").to_string(), Bind::bound(&mut st.index), &entries).show(ui);
        });
    });
    let (id, _, fields) = adjustment(st.index);
    // An adjustment with no values (Desaturate, Invert Colors) has no table: an empty one cannot be shown.
    if !fields.is_empty() {
        let rows: Vec<AdjustField> = fields
            .iter()
            .map(|&(key, name, (min, max), decimals, default)| {
                AdjustField::new(key, tl!(name).to_string(), min..=max, default).step(10f64.powi(-(decimals as i32)))
            })
            .collect();
        let (_, params) = st.params();
        let mut values: Vec<f64> = fields.iter().map(|f| params[f.0].as_f64().unwrap_or(f.4)).collect();
        let before = values.clone();
        AdjustSliders::new(tl!("Values").to_string(), &rows, Bind::bound(&mut values)).context(AdjustContext::properties(true)).show(ui);
        if values != before
            && let Some(set) = st.values.entry(id).or_insert_with(|| json!({})).as_object_mut()
        {
            set.extend(fields.iter().zip(&values).map(|(f, v)| (f.0.to_string(), json!(v))));
        }
    }
    let (id, params) = st.params();
    let mut command = None;
    ui.horizontal(|ui| {
        if Button::new(tl!("Add as Effect").to_string()).show(ui).clicked() {
            command = Some("effect.apply");
        }
        if Button::new(tl!("Apply as Filter").to_string()).show(ui).clicked() {
            command = Some("addon.color.filter");
        }
        if Button::new(tl!("Reset").to_string()).show(ui).clicked() {
            st.values.remove(id);
        }
    });
    // Inks: Duotone's and Print Ink Fixes' dialogs, as their menu items open them.
    let mut ink = None;
    Section::fixed(tl!("Inks").to_string()).show(ui, |ui, _| {
        for item in [duotone(), print_fixes()] {
            if Button::new(tl!(item.0).to_string()).show(ui).clicked() {
                ink = Some(item);
            }
        }
    });
    app.addon_panels.color = st;
    if let Some(cmd) = command
        && let Err(e) = app.run(cmd, json!({ "effect": id, "params": params }))
    {
        app.status(e);
    }
    if let Some((label, id, p)) = ink {
        let (id, p) = crate::menus::click_target(label, id, &p);
        crate::menus::invoke(app, &id, p);
    }
}

#[cfg(test)]
mod tests;
