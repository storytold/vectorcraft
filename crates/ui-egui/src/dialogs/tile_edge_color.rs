//! Object → Pattern → Tile Edge Color: the colour pattern editing mode draws the tile edge and the
//! swatch bounds in, a preset (the layer colours) or a custom colour. OK sets the preference
//! `patternTileEdgeColor` (`prefs.set`).
//!
//! Fields: `color` (`#rrggbb`, or a preset's name such as "Light Blue").

use serde_json::{Value, json};
use vectorcraft_color::Color;
use vectorcraft_doc::LAYER_COLORS;

use super::swatch_options::{grid, label};
use super::{DialogSpec, run_and_close};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Tile Edge Color.
pub const KIND: &str = "tileEdgeColor";

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |_| tl!("Tile Edge Color").into(), body, confirm, min_width: 300.0, ..DialogSpec::FORM };

/// Open Tile Edge Color on the current preference.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    app.ui.dialog = Some(Dialog::new(KIND, json!({ "color": app.session.prefs.pattern_tile_edge_color })));
    Ok(Value::Null)
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

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let color = resolve(&d.str("color")).unwrap_or(Color::BLACK);
    let [r, g, b, _] = color.to_rgba8(1.0);
    let preset = LAYER_COLORS.iter().position(|(_, c)| *c == [r, g, b]);
    let names: Vec<&str> = LAYER_COLORS.iter().map(|(n, _)| *n).chain(["Custom"]).collect();
    grid(ui, |ui| {
        label(ui, tl!("Color:"));
        ui.horizontal(|ui| {
            if let Some((_, [r, g, b])) =
                widgets::dropdown(ui, "tile-edge", names[preset.unwrap_or(LAYER_COLORS.len())], &names, 140.0).and_then(|i| LAYER_COLORS.get(i))
            {
                d.fields.insert("color".into(), json!(Color::rgb8(*r, *g, *b).to_hex()));
            }
            // Any other colour (the preset list shows Custom then).
            let mut rgb = [r, g, b];
            if ui.color_edit_button_srgb(&mut rgb).changed() {
                d.fields.insert("color".into(), json!(Color::rgb8(rgb[0], rgb[1], rgb[2]).to_hex()));
            }
        });
        ui.end_row();
    });
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let color = d.str("color");
    let hex = checked(&color)?.to_hex();
    run_and_close(app, "prefs.set", json!({ "key": "patternTileEdgeColor", "value": hex }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    #[test]
    fn presets_or_custom_colours_set_the_preference() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("ui.tileEdgeColor", json!({})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("color"), app.session.prefs.pattern_tile_edge_color);
        // Drawn headlessly in the shared frame: the preset's name shows.
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(Default::default(), |ui| super::super::show(&mut app, ui.ctx()));
        out.textures_delta.clear();
        let set = |app: &mut VectorcraftApp, v: &str| {
            app.ui.dialog.as_mut().unwrap().fields.insert("color".into(), json!(v));
            super::super::confirm(app)
        };
        set(&mut app, "Red").unwrap();
        assert_eq!(app.session.prefs.pattern_tile_edge_color, "#ff4f4f");
        assert!(app.ui.dialog.is_none());
        app.run("ui.tileEdgeColor", json!({})).unwrap();
        set(&mut app, "#00FF00").unwrap();
        assert_eq!(app.session.prefs.pattern_tile_edge_color, "#00ff00");
        app.run("ui.tileEdgeColor", json!({})).unwrap();
        assert!(set(&mut app, "nope").is_err());
    }

    /// A hex color with an alpha part is refused and leaves the preference as it was; `#rrggbb`
    /// still sets it.
    #[test]
    fn a_color_with_an_alpha_part_is_refused() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("ui.tileEdgeColor", json!({})).unwrap();
        let before = app.session.prefs.pattern_tile_edge_color.clone();
        app.ui.dialog.as_mut().unwrap().fields.insert("color".into(), json!("#ff000080"));
        let err = super::super::confirm(&mut app).unwrap_err();
        assert_eq!(err, "`#ff000080` is not a colour (#rrggbb or a preset name)");
        assert_eq!(app.session.prefs.pattern_tile_edge_color, before);
        assert!(app.ui.dialog.is_some(), "the dialog stays open");
        app.ui.dialog.as_mut().unwrap().fields.insert("color".into(), json!("#ff0000"));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(app.session.prefs.pattern_tile_edge_color, "#ff0000");
    }

    /// The swatches the dialogs draw with `resolve` show an 8-digit color by its RGB, while
    /// `checked` refuses it; preset names and `#rrggbb` pass both.
    #[test]
    fn only_checked_refuses_a_color_with_an_alpha_part() {
        let red = Color::rgb8(0xff, 0, 0).to_rgba8(1.0);
        assert_eq!(resolve("#ff000080").map(|c| c.to_rgba8(1.0)), Some(red));
        assert_eq!(resolve("ff000080").map(|c| c.to_rgba8(1.0)), Some(red));
        assert!(checked("#ff000080").is_err() && checked(" ff000080 ").is_err());
        assert!(checked("#f008").is_err());
        assert_eq!(checked("#ff0000").map(|c| c.to_rgba8(1.0)), Ok(red));
        for (name, [r, g, b]) in LAYER_COLORS.iter() {
            assert_eq!(checked(name).map(|c| c.to_rgba8(1.0)), Ok([*r, *g, *b, 255]), "{name}");
        }
    }
}
