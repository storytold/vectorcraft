//! The layer-colour row of the Live Paint, Shape Builder, Tile Edge Color and Gap Options dialogs: the
//! layer colours as named rows, then the custom colour and Select Custom Color…. A dialog keeps the
//! colour as a layer colour's name or `#rrggbb`; a named pick writes `#rrggbb`.

use craft::color_ui::ui::{ChosenColor, ColorChoice, NamedColor};
use craft::theme::{Rgba, Srgba};
use craft::ui::runtime::env::Label;
use craft::ui::runtime::requests::{ColorRequest, HostAnswer};
use egui::Ui;
use serde_json::{Map, Value, json};
use vectorcraft_color::Color;
use vectorcraft_doc::LAYER_COLORS;

use super::tile_edge_color::resolve;

/// `rgb` as the framework's colour.
fn srgba([r, g, b]: [u8; 3]) -> Srgba {
    Srgba::from(Rgba::rgb(r, g, b))
}

/// The layer colours as the named rows of a colour choice; the name is the row's id and label.
pub(super) fn named() -> Vec<NamedColor> {
    LAYER_COLORS.iter().map(|(name, rgb)| NamedColor::new(*name, Label::from(name.to_string()), Some(srgba(*rgb)))).collect()
}

/// The choice over `values[key]`, labelled `label`. The row shows the layer colour the value has, else Custom; a
/// named pick writes its `#rrggbb`. Returns the request "Select Custom Color…" files, for the host to answer
/// ([`apply_answers`]).
pub(super) fn row(ui: &mut Ui, values: &mut Map<String, Value>, key: &str, label: Label, enabled: bool) -> Option<ColorRequest> {
    let color = resolve(values.get(key).and_then(Value::as_str).unwrap_or_default()).unwrap_or(Color::BLACK);
    let [r, g, b, _] = color.to_rgba8(1.0);
    let chosen = match LAYER_COLORS.iter().find(|(_, rgb)| *rgb == [r, g, b]) {
        Some((name, _)) => ChosenColor::Named((*name).to_owned()),
        None => ChosenColor::Custom,
    };
    let custom = matches!(chosen, ChosenColor::Custom).then(|| srgba([r, g, b]));
    let out = ColorChoice::new(label, &chosen).named(named()).custom(custom).enabled(enabled).show(ui);
    if let Some(ChosenColor::Named(name)) = out.chosen
        && let Some((_, [r, g, b])) = LAYER_COLORS.iter().find(|(layer, _)| *layer == name)
    {
        values.insert(key.to_owned(), json!(Color::rgb8(*r, *g, *b).to_hex()));
    }
    out.pick_color
}

/// Writes the colour the host picked for the request [`row`] filed into `values[key]`. A dialog has one colour row, so
/// the request's fixed key ([`ColorChoice::CUSTOM_KEY`]) names it.
pub(super) fn apply_answers(values: &mut Map<String, Value>, key: &str, answers: &[HostAnswer]) {
    for answer in answers {
        if let HostAnswer::Color { key: asked, color } = answer
            && asked == ColorChoice::CUSTOM_KEY
        {
            let [r, g, b, _] = color.to_rgba().0;
            values.insert(key.to_owned(), json!(Color::rgb8(r, g, b).to_hex()));
        }
    }
}
