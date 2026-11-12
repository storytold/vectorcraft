//! The own controls of Extrude & Bevel (Classic) and Revolve (Classic) in the compact 3D dialog: the
//! extrude and bevel options, or the revolve options, on the Object tab, and the surface with its
//! lights, highlight, blend steps and shading color on the Lighting tab. Position (presets,
//! rotation, perspective) is the Rotation tab every 3D effect shares.

use serde_json::{Value, json};
use vectorcraft_doc::Unit;
use vectorcraft_effects::{CLASSIC_EXTRUDE, CLASSIC_REVOLVE};

use super::revolve::{c3, choice, heading, label, light_list, light_record, number, reset_dot, selected_light, store_selected, toggle};
use crate::state::Dialog;

/// Depth and cap, then the bevel: its shape, its height and whether it is added around the shape
/// or carved into it.
pub(super) fn object_controls(ui: &mut egui::Ui, d: &mut Dialog, unit: Unit) {
    heading(ui, tl!("Object"));
    number(ui, d, "depth", tl!("Depth"), (0.0, 2000.0), "", 1, Some(unit));
    choice(ui, d, "cap", c3("Cap"), &[(json!(true), c3("Solid")), (json!(false), c3("Hollow"))]);
    ui.add_space(10.0);
    heading(ui, c3("Bevel"));
    choice(ui, d, "bevel", tl!("Bevel Shape"), &options("bevel"));
    ui.add_enabled_ui(d.str("bevel") != "none", |ui| {
        number(ui, d, "bevelHeight", tl!("Height"), (1.0, 100.0), "", 1, Some(unit));
        toggle(ui, d, "bevelInside", tl!("Bevel Inside"), false);
    });
}

/// Revolve (Classic)'s angle, cap and offset, and the edge of the profile's bounds the axis is at.
pub(super) fn revolve_controls(ui: &mut egui::Ui, d: &mut Dialog, unit: Unit) {
    heading(ui, tl!("Object"));
    number(ui, d, "angle", tl!("Angle"), (0.0, 360.0), "°", 1, None);
    choice(ui, d, "cap", c3("Cap"), &[(json!(true), c3("Solid")), (json!(false), c3("Hollow"))]);
    number(ui, d, "offset", tl!("Offset"), (0.0, 1000.0), "", 1, Some(unit));
    let sides: Vec<(Value, &str)> = super::effect::own_choices(CLASSIC_REVOLVE, "edge", false)
        .unwrap_or_default()
        .into_iter()
        .map(|(label, value)| (json!(value), label))
        .collect();
    choice(ui, d, "edge", tl!("Axis side"), &sides);
}

/// The surface, then for Diffuse and Plastic the lights, Ambient Light, the highlight (Plastic
/// only), Blend Steps and Shading Color; Draw Hidden Faces last, for Extrude & Bevel (Classic)
/// (Revolve (Classic) draws every face).
pub(super) fn surface_controls(ui: &mut egui::Ui, d: &mut Dialog) {
    choice(ui, d, "surface", tl!("Surface"), &options("surface"));
    let surface = d.str("surface");
    ui.add_enabled_ui(surface == "diffuse" || surface == "plastic", |ui| {
        light_list(ui, d);
        let mut selected = selected_light(d);
        let before = light_record(&selected);
        number(ui, &mut selected, "lightIntensity", tl!("Light Intensity"), (0.0, 100.0), "%", 1, None);
        number(ui, &mut selected, "lightRotation", tl!("Rotation"), (-180.0, 180.0), "°", 1, None);
        number(ui, &mut selected, "lightHeight", tl!("Height"), (0.0, 90.0), "°", 1, None);
        toggle(ui, &mut selected, "lightBehind", tl!("Light behind the object"), false);
        if light_record(&selected) != before {
            store_selected(d, &selected);
        }
        number(ui, d, "ambient", tl!("Ambient Light"), (0.0, 100.0), "%", 1, None);
        ui.add_enabled_ui(surface == "plastic", |ui| {
            number(ui, d, "highlightIntensity", tl!("Highlight Intensity"), (0.0, 100.0), "%", 1, None);
            number(ui, d, "highlightSize", tl!("Highlight Size"), (0.0, 100.0), "%", 1, None);
        });
        number(ui, d, "blendSteps", tl!("Blend Steps"), (1.0, 256.0), "", 0, None);
        choice(ui, d, "shadingColor", tl!("Shading Color"), &options("shadingColor"));
        if d.str("shadingColor") == "custom" {
            shading_custom(ui, d);
        }
    });
    if d.str("__effect") == CLASSIC_EXTRUDE {
        toggle(ui, d, "drawHiddenFaces", tl!("Draw Hidden Faces"), false);
    }
}

/// The values of Extrude & Bevel (Classic)'s `key` as the dropdowns list them, (value, label).
fn options(key: &str) -> Vec<(Value, &'static str)> {
    super::effect::own_choices(CLASSIC_EXTRUDE, key, false).unwrap_or_default().into_iter().map(|(label, value)| (json!(value), label)).collect()
}

/// The custom shading color: a color button, its hex value and its reset dot.
fn shading_custom(ui: &mut egui::Ui, d: &mut Dialog) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        label(ui, tl!("Color"));
        let color = d
            .fields
            .get("shadingCustom")
            .and_then(Value::as_str)
            .filter(|s| s.len() == 7 && s.starts_with('#'))
            .and_then(vectorcraft_color::Color::from_hex)
            .unwrap_or(vectorcraft_color::Color::BLACK);
        let mut rgb = color.to_rgb_uncalibrated().map(|c| (c * 255.0).round() as u8);
        if ui.color_edit_button_srgb(&mut rgb).on_hover_text(tl!("Shading Color")).changed() {
            d.fields.insert("shadingCustom".into(), json!(format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])));
        }
        ui.label(format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]));
        ui.add_space((ui.available_width() - 24.0).max(0.0));
        if reset_dot(ui, "shadingCustom", rgb != [0; 3], "#000000") {
            d.fields.insert("shadingCustom".into(), json!("#000000"));
        }
    });
}
