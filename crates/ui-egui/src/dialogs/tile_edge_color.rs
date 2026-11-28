//! Object → Pattern → Tile Edge Color: the colour pattern editing mode draws the tile edge and the
//! swatch bounds in, a preset (the layer colours) or a custom colour. OK sets the preference
//! `patternTileEdgeColor` (`prefs.set`).
//!
//! Fields: `color` (`#rrggbb`, or a preset's name such as "Light Blue").

use serde_json::{Map, Value, json};
use vectorcraft_color::Color;
use vectorcraft_doc::LAYER_COLORS;

use super::option_form::{self, OptionSpec};
use crate::VectorcraftApp;

/// The dialog kind of Tile Edge Color.
pub const KIND: &str = "tileEdgeColor";

/// The dialog's description for the option form.
pub(super) static SPEC: OptionSpec = OptionSpec {
    key: KIND,
    title: || tl!("Tile Edge Color").to_owned(),
    keys: &["color"],
    colors: &["color"],
    preview: false,
    body: |ui, rows| rows.field(ui, tl!("Color"), true, |ui, rows, label| rows.color(ui, "color", label, true)),
    inactive: |_, _| false,
    run,
    cancelled: None,
};

/// Open Tile Edge Color on the current preference.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let values = Map::from_iter([("color".to_owned(), json!(app.session.prefs.pattern_tile_edge_color))]);
    option_form::open(app, &SPEC, values, Value::Null).ok_or("no window")?;
    Ok(Value::Null)
}

/// OK: the colour as `#rrggbb` is the preference.
fn run(app: &mut VectorcraftApp, values: &Map<String, Value>, _: &Value) -> Result<Value, String> {
    let hex = checked(values.get("color").and_then(Value::as_str).unwrap_or_default())?.to_hex();
    app.run("prefs.set", json!({ "key": "patternTileEdgeColor", "value": hex }))
}

/// `color` (a preset's name or a hex colour) as a colour.
pub(super) fn resolve(color: &str) -> Option<Color> {
    match LAYER_COLORS.iter().find(|(name, _)| name.eq_ignore_ascii_case(color.trim())) {
        Some((_, [r, g, b])) => Some(Color::rgb8(*r, *g, *b)),
        None => Color::from_hex(color),
    }
}

/// Whether `color` is an 8-digit hex color, `#rrggbbaa` (with or without `#`).
pub(super) fn has_alpha(color: &str) -> bool {
    let color = color.trim();
    let digits = color.strip_prefix('#').unwrap_or(color);
    digits.len() == 8 && digits.chars().all(|c| c.is_ascii_hexdigit())
}

/// `color` as [`resolve`] reads it, or an error naming the forms it takes. A hex color with an
/// alpha part (`#rrggbbaa` or `#rgba`) is an error.
pub(super) fn checked(color: &str) -> Result<Color, String> {
    let refused = || format!("`{color}` is not a colour (#rrggbb or a preset name)");
    if has_alpha(color) {
        return Err(refused());
    }
    resolve(color).ok_or_else(refused)
}

#[cfg(test)]
mod tests {
    use super::super::option_form::tests::{app, fields, held, is_open, press_ok, set};
    use super::*;
    use craft::ui::dialog::HeldOk;

    #[test]
    fn presets_or_custom_colours_set_the_preference() {
        let mut app = app();
        app.run("ui.tileEdgeColor", json!({})).unwrap();
        assert_eq!(fields(&app)["color"], json!(app.session.prefs.pattern_tile_edge_color));
        set(&app, "color", json!("Red")).unwrap();
        assert!(press_ok(&mut app));
        assert_eq!(app.session.prefs.pattern_tile_edge_color, "#ff4f4f");
        app.run("ui.tileEdgeColor", json!({})).unwrap();
        set(&app, "color", json!("#00FF00")).unwrap();
        assert!(press_ok(&mut app));
        assert_eq!(app.session.prefs.pattern_tile_edge_color, "#00ff00");
        // A word that is no colour is refused at OK: the dialog stays open and the preference is as it was.
        app.run("ui.tileEdgeColor", json!({})).unwrap();
        set(&app, "color", json!("nope")).unwrap();
        assert!(!press_ok(&mut app));
        assert!(is_open(&app));
        assert!(matches!(held(&app), Some(HeldOk::Refused(_))));
        assert_eq!(app.session.prefs.pattern_tile_edge_color, "#00ff00");
    }

    /// A hex color with an alpha part is refused and leaves the preference as it was; `#rrggbb` still sets it.
    #[test]
    fn a_color_with_an_alpha_part_is_refused() {
        let mut app = app();
        app.run("ui.tileEdgeColor", json!({})).unwrap();
        let before = app.session.prefs.pattern_tile_edge_color.clone();
        set(&app, "color", json!("#ff000080")).unwrap();
        assert!(!press_ok(&mut app));
        assert!(is_open(&app), "the dialog stays open");
        assert!(matches!(held(&app), Some(HeldOk::Refused(_))), "with the reason");
        assert_eq!(app.session.prefs.pattern_tile_edge_color, before);
        set(&app, "color", json!("#ff0000")).unwrap();
        assert!(press_ok(&mut app));
        assert_eq!(app.session.prefs.pattern_tile_edge_color, "#ff0000");
    }
}
