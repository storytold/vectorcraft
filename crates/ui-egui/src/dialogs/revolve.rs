//! Compact controls for the 3D and Materials effects: Extrude & Bevel, Revolve, Inflate and Rotate.
//! The object and the rotation and light gizmos live on the artboard. All document changes run
//! through the effect dialog's existing preview/commit commands.

use egui::{Align2, FontId, Id, Pos2, Sense, Stroke, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::Unit;
use vectorcraft_effects::{EXTRUDE, INFLATE, REVOLVE, ROTATION_PRESETS};

use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, scrub, widgets};

const LABEL_W: f32 = 102.0;
const VALUE_W: f32 = 74.0;
const SIDE_W: f32 = 354.0;

/// Sized to the screen, with scrolling inside the body so OK and Cancel stay reachable.
pub(super) fn body(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    // Values start in drag mode; clicking or Tab enters their text editor.
    ui.data_mut(|m| m.remove::<bool>(widgets::dialog_focus_flag()));
    let screen = ui.ctx().content_rect();
    let width = (screen.width() - 64.0).clamp(240.0, SIDE_W);
    // Tall enough for Extrude & Bevel's Object tab, the longest, where the screen has the room.
    let height = (screen.height() - 240.0).clamp(120.0, 450.0);
    ui.set_width(width);
    egui::ScrollArea::vertical().id_salt("revolve-body").max_height(height).auto_shrink([false, true]).show(ui, |ui| {
        controls(app, ui, d);
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let t = Tokens::get(ui.ctx());
        ui.allocate_ui_with_layout(vec2((width - 100.0).max(80.0), 32.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.add(egui::Label::new(egui::RichText::new(super::revolve_gizmo::hint(d)).size(11.0).color(t.text_dim)).wrap());
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let id = d.str("__effect");
            let defaults = vectorcraft_effects::default_params(&id).unwrap_or(json!({}));
            let changed = defaults.as_object().is_some_and(|m| {
                m.iter().any(|(k, v)| v.as_f64().map_or_else(|| d.fields.get(k) != Some(v), |default| (d.f64(k, default) - default).abs() > 1e-6))
            });
            let (r, _) = ui.allocate_exact_size(vec2(88.0, 24.0), Sense::hover());
            let response = ui.interact(r, Id::unique("revolve-reset-all"), Sense::click());
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tl!("Reset all")));
            if response.hovered() {
                ui.painter().rect_filled(r, 3.0, t.hover);
            }
            ui.painter().text(r.left_center() + vec2(4.0, 0.0), Align2::LEFT_CENTER, tl!("Reset all"), FontId::proportional(11.5), t.text);
            dot(ui, r.right_center() - vec2(10.0, 0.0), changed);
            let tip =
                if id == REVOLVE { tl!("Reset all Revolve settings to their defaults.") } else { tl!("Reset all 3D settings to their defaults.") };
            if response.on_hover_text(tip).clicked()
                && let Some(defaults) = defaults.as_object()
            {
                d.fields.extend(defaults.clone());
                d.fields.remove("__light");
                super::revolve_gizmo::clear(ui.ctx());
            }
        });
    });
}

fn controls(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    ui.spacing_mut().item_spacing.y = 4.0;
    let id = d.str("__effect");
    if id != REVOLVE {
        return solid_controls(app, ui, d, &id);
    }
    let lighting = super::revolve_gizmo::lighting(d);
    if let Some(tab) = widgets::tab_bar(ui, &[tl!("Rotation"), tl!("Lighting")], usize::from(lighting)) {
        d.fields.insert("__gizmo".into(), json!(if tab == 1 { "light" } else { "rotation" }));
    }
    ui.add_space(8.0);
    if super::revolve_gizmo::lighting(d) {
        lighting_controls(ui, d);
        return;
    }
    let unit = app.session.general_unit();
    heading(ui, tl!("Shape"));
    number(ui, d, "angle", tl!("Revolve angle"), (0.0, 360.0), "°", 1, None);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        label(ui, tl!("Axis side"));
        let edge = d.str("edge");
        let sides = super::effect::own_choices(REVOLVE, "edge", false).unwrap_or_default();
        let current = sides.iter().find(|(_, v)| *v == edge).or(sides.first()).map_or("", |(l, _)| *l);
        let labels: Vec<&str> = sides.iter().map(|(l, _)| *l).collect();
        let width = VALUE_W + rail_width(ui) + 6.0;
        if let Some((_, v)) = widgets::dropdown(ui, "revolve-edge", current, &labels, width).and_then(|k| sides.get(k)) {
            d.fields.insert("edge".into(), json!(v));
        }
        if reset_dot(ui, "edge", edge != "left", tl!("Left")) {
            d.fields.insert("edge".into(), json!("left"));
        }
    });
    number(ui, d, "offset", tl!("Offset"), (0.0, 100000.0), "", 1, Some(unit));
    ui.horizontal(|ui| {
        let mut cap = d.bool("cap");
        let tip = tl!("Close the circular ends of open profiles. Partial sweeps keep their cut sides open.");
        if widgets::check_tip(ui, c3("Cap"), cap, true, tip) {
            cap = !cap;
            d.fields.insert("cap".into(), json!(cap));
        }
        ui.add_space((ui.available_width() - 24.0).max(0.0));
        if reset_dot(ui, "cap", !cap, "true") {
            d.fields.insert("cap".into(), json!(true));
        }
    });
    ui.add_space(10.0);
    heading(ui, tl!("View"));
    for (key, name) in [("rotationX", tl!("Rotation X")), ("rotationY", tl!("Rotation Y")), ("rotationZ", tl!("Rotation Z"))] {
        number(ui, d, key, name, (-360.0, 360.0), "°", 1, None);
    }
    number(ui, d, "perspectiveAngle", tl!("Perspective"), (0.0, 160.0), "°", 1, None);
    ui.add_space(6.0);
    egui::CollapsingHeader::new(tl!("Advanced")).id_salt("revolve-advanced").show(ui, |ui| {
        number(ui, d, "segments", tl!("Segments"), (8.0, 128.0), "", 0, None);
        super::form::check(ui, d, "__showAxis", tl!("Show axis"));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let visible = d.fields.get("expandVisibleOnly").and_then(Value::as_bool).unwrap_or(true);
            let tip = tl!("Trim covered geometry on Expand Appearance. Live preview keeps the full surface. Transparent paints and surfaces exceeding visibility limits retain all faces.");
            let toggled = ui.allocate_ui_with_layout(
                vec2((ui.available_width() - 30.0).max(100.0), 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    widgets::check_tip(ui, tl!("Keep visible surfaces only when expanding"), visible, true, tip)
                },
            ).inner;
            if toggled {
                d.fields.insert("expandVisibleOnly".into(), json!(!visible));
            }
            if reset_dot(ui, "expandVisibleOnly", !visible, "true") {
                d.fields.insert("expandVisibleOnly".into(), json!(true));
            }
        });
    });
}

/// `s` as the 3D effects mean it (a solid's cap, not a stroke's; a bevelled edge, not a join).
fn c3(s: &'static str) -> &'static str {
    crate::i18n::tr_ctx(crate::i18n::current(), "3d", s)
}

/// Extrude & Bevel and Inflate: Object, Rotation and Lighting tabs (Rotate has no Object tab).
/// The rotation gizmo shows on the first two, the light gizmo on Lighting.
fn solid_controls(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog, id: &str) {
    let object = id == EXTRUDE || id == INFLATE;
    let mut tabs = vec![];
    if object {
        tabs.push(tl!("Object"));
    }
    tabs.extend([tl!("Rotation"), tl!("Lighting")]);
    let current = if super::revolve_gizmo::lighting(d) {
        tabs.len() - 1
    } else if object && d.str("__tab") != "rotation" {
        0
    } else {
        tabs.len() - 2
    };
    if let Some(tab) = widgets::tab_bar(ui, &tabs, current) {
        let light = tab + 1 == tabs.len();
        d.fields.insert("__gizmo".into(), json!(if light { "light" } else { "rotation" }));
        d.fields.insert("__tab".into(), json!(if object && tab == 0 { "object" } else { "rotation" }));
    }
    ui.add_space(8.0);
    if super::revolve_gizmo::lighting(d) {
        lighting_controls(ui, d);
    } else if object && d.str("__tab") != "rotation" {
        let unit = app.session.general_unit();
        if id == EXTRUDE {
            extrude_controls(ui, d, unit);
        } else {
            heading(ui, tl!("Object"));
            number(ui, d, "depth", tl!("Depth"), (0.0, 2000.0), "", 1, Some(unit));
            number(ui, d, "twist", tl!("Twist"), (0.0, 360.0), "°", 1, None);
            number(ui, d, "taper", tl!("Taper"), (1.0, 100.0), "%", 1, None);
            choice(ui, d, "cap", c3("Cap"), &[(json!(true), c3("Solid")), (json!(false), c3("Hollow"))]);
            number(ui, d, "volume", tl!("Volume"), (0.0, 100.0), "%", 1, None);
            toggle(ui, d, "bothSides", tl!("Both sides"), false);
        }
    } else {
        view_controls(ui, d);
    }
}

/// Extrude & Bevel's depth, twist, taper, cap and bevel.
fn extrude_controls(ui: &mut egui::Ui, d: &mut Dialog, unit: Unit) {
    heading(ui, tl!("Object"));
    number(ui, d, "depth", tl!("Depth"), (0.0, 2000.0), "", 1, Some(unit));
    number(ui, d, "twist", tl!("Twist"), (0.0, 360.0), "°", 1, None);
    number(ui, d, "taper", tl!("Taper"), (1.0, 100.0), "%", 1, None);
    choice(ui, d, "cap", c3("Cap"), &[(json!(true), c3("Solid")), (json!(false), c3("Hollow"))]);
    ui.add_space(10.0);
    heading(ui, c3("Bevel"));
    let shapes: Vec<(Value, &str)> =
        super::effect::own_choices(EXTRUDE, "bevel", false).unwrap_or_default().into_iter().map(|(label, value)| (json!(value), label)).collect();
    choice(ui, d, "bevel", tl!("Bevel Shape"), &shapes);
    ui.add_enabled_ui(d.str("bevel") != "none", |ui| {
        number(ui, d, "bevelWidth", tl!("Width"), (0.0, 1000.0), "", 1, Some(unit));
        number(ui, d, "bevelHeight", tl!("Height"), (0.0, 1000.0), "", 1, Some(unit));
        number(ui, d, "bevelRepeat", tl!("Repeat"), (1.0, 32.0), "", 0, None);
        number(ui, d, "bevelSpace", c3("Space"), (0.0, 90.0), "%", 1, None);
        toggle(ui, d, "bevelInside", tl!("Bevel Inside"), false);
        toggle(ui, d, "bevelBothSides", tl!("Bevel both sides"), false);
    });
}

/// Rotation presets, rotation and perspective for Extrude & Bevel, Inflate and Rotate.
fn view_controls(ui: &mut egui::Ui, d: &mut Dialog) {
    heading(ui, tl!("Rotation"));
    let names = [
        tl!("Front"),
        tl!("Back"),
        tl!("Left"),
        tl!("Right"),
        tl!("Top"),
        tl!("Bottom"),
        tl!("Off-Axis Front"),
        tl!("Off-Axis Back"),
        tl!("Off-Axis Left"),
        tl!("Off-Axis Right"),
        tl!("Off-Axis Top"),
        tl!("Off-Axis Bottom"),
        tl!("Isometric Left"),
        tl!("Isometric Right"),
        tl!("Isometric Top"),
        tl!("Isometric Bottom"),
    ];
    let angles = [d.f64("rotationX", 0.0), d.f64("rotationY", 0.0), d.f64("rotationZ", 0.0)];
    let current = ROTATION_PRESETS.iter().position(|(_, a)| a.iter().zip(angles).all(|(a, b)| (a - b).abs() < 0.005));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        label(ui, tl!("Presets"));
        let width = VALUE_W + rail_width(ui) + 6.0;
        let shown = current.and_then(|i| names.get(i).copied()).unwrap_or(tl!("Custom"));
        if let Some(k) = widgets::dropdown(ui, "three-d-position", shown, &names, width)
            && let Some((_, [x, y, z])) = ROTATION_PRESETS.get(k)
        {
            d.fields.extend([("rotationX".into(), json!(x)), ("rotationY".into(), json!(y)), ("rotationZ".into(), json!(z))]);
        }
    });
    for (key, name) in [("rotationX", tl!("Rotation X")), ("rotationY", tl!("Rotation Y")), ("rotationZ", tl!("Rotation Z"))] {
        number(ui, d, key, name, (-180.0, 180.0), "°", 2, None);
    }
    number(ui, d, "perspectiveAngle", tl!("Perspective"), (0.0, 160.0), "°", 1, None);
}

/// A dropdown of `options` (value, label) for `key`, with its reset dot.
fn choice(ui: &mut egui::Ui, d: &mut Dialog, key: &str, name: &str, options: &[(Value, &str)]) {
    let id = d.str("__effect");
    let default = vectorcraft_effects::default_params(&id).and_then(|p| p.get(key).cloned()).unwrap_or(Value::Null);
    let value = d.fields.get(key).cloned().unwrap_or_else(|| default.clone());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        label(ui, name);
        let current = options.iter().find(|(v, _)| *v == value).map_or("", |(_, l)| *l);
        let labels: Vec<&str> = options.iter().map(|(_, l)| *l).collect();
        let width = VALUE_W + rail_width(ui) + 6.0;
        if let Some(k) = widgets::dropdown(ui, ("three-d-choice", key), current, &labels, width)
            && let Some((v, _)) = options.get(k)
        {
            d.fields.insert(key.into(), v.clone());
        }
        let default_label = options.iter().find(|(v, _)| *v == default).map_or("", |(_, l)| *l);
        if reset_dot(ui, key, value != default, default_label) {
            d.fields.insert(key.into(), default.clone());
        }
    });
}

/// A checkbox for `key`, with its reset dot.
fn toggle(ui: &mut egui::Ui, d: &mut Dialog, key: &str, name: &str, default: bool) {
    ui.horizontal(|ui| {
        let on = d.bool(key);
        if widgets::check(ui, name, on, ui.is_enabled()) {
            d.fields.insert(key.into(), json!(!on));
        }
        ui.add_space((ui.available_width() - 24.0).max(0.0));
        if reset_dot(ui, key, on != default, if default { "true" } else { "false" }) {
            d.fields.insert(key.into(), json!(default));
        }
    });
}

fn lighting_controls(ui: &mut egui::Ui, d: &mut Dialog) {
    ui.horizontal(|ui| {
        let shade = d.bool("shade");
        if widgets::check(ui, tl!("Shading"), shade, true) {
            d.fields.insert("shade".into(), json!(!shade));
        }
        ui.add_space((ui.available_width() - 24.0).max(0.0));
        if reset_dot(ui, "shade", !shade, "true") {
            d.fields.insert("shade".into(), json!(true));
        }
    });
    ui.add_enabled_ui(d.bool("shade"), |ui| {
        lighting_presets(ui, d);
        light_list(ui, d);
        let mut selected = selected_light(d);
        let before = light_record(&selected);
        // The documented Light Settings, in their order.
        light_color(ui, &mut selected);
        number(ui, &mut selected, "lightIntensity", tl!("Intensity"), (0.0, 100.0), "%", 1, None);
        number(ui, &mut selected, "lightRotation", tl!("Rotation"), (-180.0, 180.0), "°", 1, None);
        number(ui, &mut selected, "lightHeight", tl!("Height"), (0.0, 90.0), "°", 1, None);
        number(ui, &mut selected, "lightSoftness", tl!("Softness"), (0.0, 100.0), "%", 1, None);
        number(ui, d, "ambient", tl!("Ambient Light"), (0.0, 200.0), "%", 1, None);
        toggle(ui, &mut selected, "lightBehind", tl!("Light behind the object"), false);
        if light_record(&selected) != before {
            store_selected(d, &selected);
        }
    });
}

fn lighting_presets(ui: &mut egui::Ui, d: &mut Dialog) {
    let names = [tl!("Standard"), tl!("Diffuse"), tl!("Top Left"), tl!("Right")];
    let current = vectorcraft_effects::matching_lighting_preset(&super::form::params(d));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        label(ui, tl!("Presets"));
        let shown = current.and_then(|i| names.get(i).copied()).unwrap_or(tl!("Custom"));
        if let Some(preset) = widgets::dropdown(ui, "three-d-lighting-presets", shown, &names, VALUE_W + rail_width(ui) + 6.0)
            .and_then(|i| vectorcraft_effects::LIGHTING_PRESETS.get(i))
            && let Some(patch) = preset.params().as_object()
        {
            d.fields.extend(patch.clone());
            d.fields.insert("__light".into(), json!(0));
            ui.memory_mut(|m| m.stop_text_input());
            super::revolve_gizmo::clear(ui.ctx());
        }
    });
    ui.add_space(4.0);
}

const LIGHT_KEYS: [(&str, &str); 6] = [
    ("color", "lightColor"),
    ("intensity", "lightIntensity"),
    ("rotation", "lightRotation"),
    ("height", "lightHeight"),
    ("softness", "lightSoftness"),
    ("behind", "lightBehind"),
];

fn explicit_lights(d: &Dialog) -> Option<&Vec<Value>> {
    d.fields.get("lights").and_then(Value::as_array).filter(|v| !v.is_empty() && v.len() <= vectorcraft_three_d::MAX_LIGHTS)
}

fn light_index(d: &Dialog, count: usize) -> usize {
    d.fields.get("__light").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()).unwrap_or(0).min(count.saturating_sub(1))
}

/// Only the selected light's six settings are overlaid. Selection is UI state and never
/// enters effect parameters, so selecting another light creates no document history.
pub(super) fn selected_light(d: &Dialog) -> Dialog {
    let mut selected = d.clone();
    selected.fields.remove("lights");
    if let Some(list) = explicit_lights(d)
        && let Some(light) = list.get(light_index(d, list.len()))
    {
        let defaults = vectorcraft_effects::default_params(&d.str("__effect")).unwrap_or(json!({}));
        for (key, field) in LIGHT_KEYS {
            if let Some(value) = light.get(key).or_else(|| defaults.get(field)) {
                selected.fields.insert(field.into(), value.clone());
            }
        }
    }
    selected
}

pub(super) fn light_options(d: &Dialog) -> vectorcraft_three_d::Revolve {
    vectorcraft_effects::revolve_options(&super::form::params(&selected_light(d)))
}

fn light_record(d: &Dialog) -> Value {
    Value::Object(LIGHT_KEYS.iter().filter_map(|(key, field)| d.fields.get(*field).map(|v| ((*key).into(), v.clone()))).collect())
}

fn store_selected(d: &mut Dialog, selected: &Dialog) {
    if let Some(list) = explicit_lights(d) {
        let index = light_index(d, list.len());
        let mut list = list.clone();
        if let Some(light) = list.get_mut(index) {
            *light = light_record(selected);
        }
        d.fields.insert("lights".into(), json!(list));
    } else {
        for (_, key) in LIGHT_KEYS {
            if let Some(value) = selected.fields.get(key) {
                d.fields.insert(key.into(), value.clone());
            }
        }
    }
}

pub(super) fn set_light_value(d: &mut Dialog, key: &str, value: Value) {
    let mut selected = selected_light(d);
    selected.fields.insert(key.into(), value);
    store_selected(d, &selected);
}

fn light_list(ui: &mut egui::Ui, d: &mut Dialog) {
    let mut list = explicit_lights(d).cloned().unwrap_or_else(|| vec![light_record(d)]);
    let mut index = light_index(d, list.len());
    let names: Vec<_> = (1..=list.len()).map(|i| format!("{} {i}", c3("Light"))).collect();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        label(ui, tl!("Lights"));
        let labels: Vec<_> = names.iter().map(String::as_str).collect();
        if let Some(next) =
            widgets::dropdown(ui, "three-d-lights", names.get(index).map_or("", String::as_str), &labels, VALUE_W + rail_width(ui) + 6.0)
        {
            index = next;
            ui.memory_mut(|m| m.stop_text_input());
            super::revolve_gizmo::clear(ui.ctx());
        }
    });
    ui.horizontal(|ui| {
        if ui.add_enabled(list.len() < vectorcraft_three_d::MAX_LIGHTS, egui::Button::new(tl!("Add Light"))).clicked() {
            let mut added = selected_light(d);
            let defaults = vectorcraft_effects::default_params(&d.str("__effect")).unwrap_or(json!({}));
            for (_, key) in LIGHT_KEYS {
                if let Some(value) = defaults.get(key) {
                    added.fields.insert(key.into(), value.clone());
                }
            }
            added.fields.insert("lightRotation".into(), json!(0.0));
            added.fields.insert("lightHeight".into(), json!(90.0));
            list.push(light_record(&added));
            index = list.len() - 1;
            d.fields.insert("lights".into(), json!(list));
            super::revolve_gizmo::clear(ui.ctx());
        }
        if ui.add_enabled(list.len() > 1, egui::Button::new(tl!("Delete Light"))).clicked() && index < list.len() {
            list.remove(index);
            index = index.min(list.len().saturating_sub(1));
            d.fields.insert("lights".into(), json!(list));
            super::revolve_gizmo::clear(ui.ctx());
        }
    });
    d.fields.insert("__light".into(), json!(index));
    ui.add_space(4.0);
}

fn light_color(ui: &mut egui::Ui, d: &mut Dialog) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        label(ui, tl!("Color"));
        let color = d
            .fields
            .get("lightColor")
            .and_then(Value::as_str)
            .filter(|s| s.len() == 7 && s.starts_with('#'))
            .and_then(vectorcraft_color::Color::from_hex)
            .unwrap_or(vectorcraft_color::Color::WHITE);
        // Light colors are display RGB, independent of the document's working profile.
        let mut rgb = color.to_rgb_uncalibrated().map(|c| (c * 255.0).round() as u8);
        if ui.color_edit_button_srgb(&mut rgb).on_hover_text(tl!("Color")).changed() {
            d.fields.insert("lightColor".into(), json!(format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])));
        }
        ui.label(format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]));
        ui.add_space((ui.available_width() - 24.0).max(0.0));
        if reset_dot(ui, "lightColor", rgb != [255; 3], "#FFFFFF") {
            d.fields.insert("lightColor".into(), json!("#ffffff"));
        }
    });
}

fn heading(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(text).font(crate::theme::semibold(13.0)).color(t.text));
}

fn label(ui: &mut egui::Ui, name: &str) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(LABEL_W, 24.0), Sense::hover());
    ui.put(r, egui::Label::new(egui::RichText::new(name).size(12.5).color(t.text)).halign(egui::Align::Min).truncate()).on_hover_text(name);
    scrub::note_label(ui, r);
}

fn rail_width(ui: &egui::Ui) -> f32 {
    (ui.available_width() - VALUE_W - 24.0 - 18.0).clamp(40.0, 134.0)
}

/// A value can be typed, dragged, scrubbed by its label, or moved on its slider.
#[allow(clippy::too_many_arguments)]
fn number(ui: &mut egui::Ui, d: &mut Dialog, key: &str, name: &str, range: (f64, f64), suffix: &str, decimals: usize, unit: Option<Unit>) {
    let default = vectorcraft_effects::default_params(&d.str("__effect")).and_then(|p| p.get(key).and_then(Value::as_f64)).unwrap_or(0.0);
    let from = |n| unit.map_or(n, |u| u.from_pt(n));
    let to = |n| unit.map_or(n, |u| u.to_pt(n));
    let typed = unit.map(|u| widgets::typed_unit(ui.ctx(), u));
    let mut value = from(d.f64(key, default)).clamp(from(range.0), from(range.1));
    let suffix = unit.map_or_else(|| suffix.to_string(), |u| format!(" {}", u.suffix()));
    let speed = if decimals == 0 { 1.0 } else { 0.25 };
    let mut next = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        label(ui, name);
        let t = Tokens::get(ui.ctx());
        let response = ui
            .scope(|ui| {
                ui.visuals_mut().widgets.inactive.weak_bg_fill = t.input;
                ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::new(1.0, t.input_border);
                ui.add_sized(
                    [VALUE_W, 24.0],
                    egui::DragValue::new(&mut value)
                        .range(from(range.0)..=from(range.1))
                        .speed(speed)
                        .max_decimals(decimals)
                        .suffix(&suffix)
                        .update_while_editing(false)
                        .custom_parser(|s| {
                            unit.map_or_else(
                                || vectorcraft_doc::parse_number(&s.replace(['°', '%'], "")),
                                |u| typed.unwrap_or(u).parse(s).map(|v| u.from_pt(v)),
                            )
                        }),
                )
            })
            .inner;
        response.clone().on_hover_text(tl!("Drag values or labels. Double-click a value to type."));
        if response.changed() {
            next = Some(value);
        }
        if let Some(n) =
            scrub::field(ui, response.id, response.rect, &value.to_string(), Some(value), decimals as i32, &(from(range.0)..=from(range.1)))
        {
            next = Some(n);
        }
        // Lengths' rails cover a useful working range; larger values can still be typed.
        let rail_max = match key {
            "offset" | "depth" => from(500.0),
            "bevelWidth" | "bevelHeight" => from(100.0),
            _ => from(range.1),
        };
        let rail_min = from(range.0);
        let rail = rail_width(ui) + VALUE_W;
        // The text the DragValue shows: at least the places its speed gives (one for 0.25 at 1x to
        // 3x scaling, as egui computes them), at most `decimals`.
        let places = ((f64::from(ui.input(|i| i.aim_radius())) / speed).log10().ceil().clamp(0.0, 15.0) as usize).min(decimals);
        let number = ui.style().number_formatter.format(value, places..=decimals);
        let shown = widgets::SliderValue::text(number.parse().unwrap_or(value), rail_min..=rail_max, format!("{number}{suffix}"));
        let slider = widgets::color_slider_stepped(
            ui,
            ("revolve-slider", key),
            name,
            Some(shown),
            ((value - rail_min) / (rail_max - rail_min)).clamp(0.0, 1.0) as f32,
            rail,
            &|_| t.input_border,
        );
        if let Some(n) = slider.0.map(|n| rail_min + f64::from(n) * (rail_max - rail_min)).or(slider.2) {
            next = Some(n);
        }
        if reset_dot(ui, key, (to(value) - default).abs() > 1e-6, &format!("{}{}", from(default), suffix)) {
            next = Some(from(default));
        }
    });
    if let Some(n) = next.filter(|n| n.is_finite()) {
        let factor = 10f64.powi(decimals as i32);
        let n = (n * factor).round() / factor;
        d.fields.insert(key.into(), json!(to(n.clamp(from(range.0), from(range.1)))));
    }
}

fn dot(ui: &egui::Ui, p: Pos2, changed: bool) {
    let t = Tokens::get(ui.ctx());
    if changed {
        ui.painter().circle_filled(p, 3.0, if ui.is_enabled() { t.accent } else { t.text_disabled });
    } else {
        ui.painter().circle_stroke(p, 3.0, Stroke::new(1.0, t.text_disabled));
    }
}

fn reset_dot(ui: &mut egui::Ui, key: &str, changed: bool, default: &str) -> bool {
    let (r, _) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::hover());
    let response = ui.interact(r, Id::unique(("revolve-reset", key)), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tl!("Reset to default")));
    if response.hovered() {
        ui.painter().rect_filled(r, 3.0, Tokens::get(ui.ctx()).hover);
    }
    dot(ui, r.center(), changed);
    response.on_hover_text(format!("{}: {default}", tl!("Reset to default"))).clicked()
}

#[cfg(test)]
mod tests;
