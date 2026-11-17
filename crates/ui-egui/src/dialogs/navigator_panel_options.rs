//! Navigator panel › Panel Options…: the color the view box is drawn in, a preset or a custom
//! color, and Draw Dashed Lines as Solid Lines for the thumbnail.
//!
//! Fields: `color` (`#rrggbb`, or a preset's name such as "Light Red") and `solidDashes`. OK runs
//! `ui.navigatorPanelOptions` with them (kept with the UI state).

use serde_json::{Value, json};
use vectorcraft_color::Color;

use super::swatch_options::{grid, label};
use super::{DialogSpec, run_and_close};
use crate::panels::navigator::{VIEW_BOX_RED, resolve, view_box_colors};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of the Navigator Panel Options.
pub const KIND: &str = "navigatorPanelOptions";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Navigator Panel Options").into(), body, confirm, min_width: 300.0, ..DialogSpec::FORM };

/// `ui.navigatorPanelOptions` without options: open the dialog on the panel's options.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let o = &app.ui.navigator_panel;
    app.ui.dialog = Some(Dialog::new(KIND, json!({ "color": o.color, "solidDashes": o.solid_dashes })));
    Ok(Value::Null)
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let [r, g, b] = resolve(&d.str("color")).unwrap_or(VIEW_BOX_RED);
    let presets: Vec<(&str, [u8; 3])> = view_box_colors().collect();
    let preset = presets.iter().position(|(_, c)| *c == [r, g, b]);
    let names: Vec<&str> = presets.iter().map(|(n, _)| *n).chain(["Custom"]).collect();
    grid(ui, |ui| {
        label(ui, tl!("Color:"));
        ui.horizontal(|ui| {
            if let Some((_, [r, g, b])) =
                widgets::dropdown(ui, "navigator-view-box", names[preset.unwrap_or(presets.len())], &names, 140.0).and_then(|i| presets.get(i))
            {
                d.fields.insert("color".into(), json!(Color::rgb8(*r, *g, *b).to_hex()));
            }
            // Any other color (the preset list shows Custom then).
            let mut rgb = [r, g, b];
            if ui.color_edit_button_srgb(&mut rgb).changed() {
                d.fields.insert("color".into(), json!(Color::rgb8(rgb[0], rgb[1], rgb[2]).to_hex()));
            }
        });
        ui.end_row();
    });
    ui.add_space(8.0);
    let solid = d.bool("solidDashes");
    if widgets::check(ui, tl!("Draw Dashed Lines as Solid Lines"), solid, true) {
        d.fields.insert("solidDashes".into(), json!(!solid));
    }
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let params = json!({ "color": d.fields.get("color").cloned().unwrap_or(Value::Null), "solidDashes": d.bool("solidDashes") });
    run_and_close(app, "ui.navigatorPanelOptions", params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    /// The dialog opens on the panel's options (Light Red and solid dashes off by default), and OK
    /// sets a preset's or a custom color and Draw Dashed Lines as Solid Lines, kept with the UI
    /// state; a value that is not a color is refused and leaves the options as they were.
    #[test]
    fn panel_options_set_the_view_box_color_and_solid_dashes() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("ui.navigatorPanelOptions", json!({})).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.kind.as_str(), d.str("color"), d.bool("solidDashes")), (KIND, "#ff3030".to_string(), false));
        // Drawn headlessly in the shared frame: the preset's name shows.
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(Default::default(), |ui| super::super::show(&mut app, ui.ctx()));
        out.textures_delta.clear();
        let set = |app: &mut VectorcraftApp, color: &str, solid: bool| {
            let d = app.ui.dialog.as_mut().unwrap();
            d.fields.insert("color".into(), json!(color));
            d.fields.insert("solidDashes".into(), json!(solid));
            super::super::confirm(app)
        };
        assert_eq!(set(&mut app, "Green", true).unwrap(), json!({"color": "#4fff4f", "solidDashes": true}));
        assert!(app.ui.dialog.is_none());
        app.run("ui.navigatorPanelOptions", json!({})).unwrap();
        set(&mut app, "#0000FF", true).unwrap();
        assert_eq!(app.ui.navigator_panel.color, "#0000ff");
        app.run("ui.navigatorPanelOptions", json!({})).unwrap();
        assert!(set(&mut app, "nope", false).is_err());
        assert!(app.ui.navigator_panel.solid_dashes && app.ui.navigator_panel.color == "#0000ff", "a refused value changes nothing");
        let back: crate::UiState = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
        assert_eq!(back.navigator_panel, app.ui.navigator_panel, "kept with the UI state");
        let old: crate::UiState = serde_json::from_value(json!({})).unwrap();
        assert_eq!(old.navigator_panel, crate::panels::navigator::PanelOptions::default(), "UI state saved without them");
    }
}
