//! File → Document Setup: General (units, bleed, Edit Artboards, outline images, the substituted
//! glyphs highlight), Transparency (grid size and colours, simulated paper, flattener preset, white
//! overprint), Type (language, quotes, superscript/subscript/small caps, text export) and Color
//! (document color mode and assigned profiles) tabs.
//!
//! Fields: what `document.setup` reports (and takes back), plus `tab`, `bleedLinked` and the Color
//! tab's `colorMode`, `convertColors`, `rgbProfile` and `cmykProfile` (null: the working space).
//! OK runs `document.setup` with them: one undo step. When the Color tab changed something, OK runs
//! `document.setup`, `edit.assignProfile` and `file.documentColorMode` in one `command.batch`,
//! and the document's changes are still one undo step. On an error the dialog stays open.

use serde_json::{Map, Value, json};
use vectorcraft_color::cms::ProfileKind;
use vectorcraft_doc::setup::{GRID_COLOR_PRESETS, LANGUAGES};
use vectorcraft_doc::{ColorMode, GridSize, Quotes, Unit};
use vectorcraft_engine::cmd::colormgmt::doc_profiles;

use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

pub(super) const KIND: &str = "documentSetup";
const TABS: [&str; 4] = ["General", "Transparency", "Type", "Color"];
/// Dialog-only fields `document.setup` doesn't take.
const UI_ONLY: [&str; 6] = ["tab", "bleedLinked", "colorMode", "convertColors", "rgbProfile", "cmykProfile"];
pub(super) const LABEL: f32 = 130.0;
/// The profile rows' entry for no assigned profile: the document uses the working space.
const WORKING_RGB: &str = "Working RGB (don't tag)";
const WORKING_CMYK: &str = "Working CMYK (don't tag)";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Document Setup").into(), body, confirm, min_width: 540.0, max_width: Some(540.0), ..DialogSpec::FORM };

/// Open Document Setup on the active document's settings.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let mut fields = app.session.execute("document.setup", &json!({})).map_err(|e| e.to_string())?;
    let linked = fields["bleed"].as_array().is_some_and(|b| b.windows(2).all(|w| w[0] == w[1]));
    fields["bleedLinked"] = json!(linked);
    fields["tab"] = json!(TABS[0]);
    if let Some(st) = app.session.active() {
        let (rgb, cmyk) = doc_profiles(&st.doc);
        fields["colorMode"] = json!(mode_id(st.doc.color_mode));
        fields["rgbProfile"] = json!(rgb);
        fields["cmykProfile"] = json!(cmyk);
    }
    fields["convertColors"] = json!(true);
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

/// The `mode` param of `file.documentColorMode` for `m`.
fn mode_id(m: ColorMode) -> &'static str {
    match m {
        ColorMode::Rgb => "rgb",
        ColorMode::Cmyk => "cmyk",
    }
}

/// The commands the Color tab's changes run, as `command.batch` steps, in the order OK runs them:
/// Edit › Assign Profile first, so that a mode change converts through the profiles it assigns,
/// then File › Document Color Mode. Empty when the tab matches the active document. A field the
/// dialog holds with a value of the wrong kind is an error, and then nothing runs; an omitted
/// field leaves its setting as it is (`convertColors`: converts).
fn color_commands(app: &VectorcraftApp, d: &Dialog) -> Result<Vec<Value>, String> {
    let Some(st) = app.session.active() else { return Ok(vec![]) };
    let mut out = vec![];
    let (rgb, cmyk) = doc_profiles(&st.doc);
    let mut assign = Map::new();
    for (key, param, current) in [("rgbProfile", "rgb", rgb), ("cmykProfile", "cmyk", cmyk)] {
        let chosen = match d.fields.get(key) {
            None => continue,
            Some(Value::Null) => None,
            Some(Value::String(n)) => Some(n.clone()),
            Some(_) => return Err(format!("`{key}` must be a profile name or null")),
        };
        if chosen != current {
            assign.insert(param.into(), json!(chosen));
        }
    }
    let mode = match d.fields.get("colorMode") {
        None => None,
        Some(Value::String(m)) if m.eq_ignore_ascii_case("rgb") => Some(ColorMode::Rgb),
        Some(Value::String(m)) if m.eq_ignore_ascii_case("cmyk") => Some(ColorMode::Cmyk),
        Some(_) => return Err("`colorMode` must be one of rgb, cmyk".into()),
    };
    let key = "convertColors";
    let convert = match d.fields.get(key) {
        None => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(format!("`{key}` must be true or false")),
    };
    if !assign.is_empty() {
        out.push(json!({"command": "edit.assignProfile", "params": assign}));
    }
    if let Some(mode) = mode.filter(|m| *m != st.doc.color_mode) {
        out.push(json!({"command": "file.documentColorMode", "params": {"mode": mode_id(mode), "convert": convert}}));
    }
    Ok(out)
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let params: Map<String, Value> = d.fields.iter().filter(|(k, _)| !UI_ONLY.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
    let color = color_commands(app, d)?;
    let r = if color.is_empty() {
        app.run("document.setup", Value::Object(params))
    } else {
        let commands: Vec<Value> = std::iter::once(json!({"command": "document.setup", "params": params})).chain(color).collect();
        app.run("command.batch", json!({"label": "Document Setup", "commands": commands}))
    };
    if r.is_ok() {
        app.ui.dialog = None;
    }
    r
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let tab = TABS.iter().position(|t| *t == d.str("tab")).unwrap_or(0);
    if let Some(i) = widgets::tab_bar(ui, &TABS, tab) {
        d.fields.insert("tab".into(), json!(TABS[i]));
    }
    ui.add_space(12.0);
    let mut close = false;
    ui.vertical(|ui| {
        ui.set_min_height(330.0);
        match tab {
            0 => close = general(app, ui, d),
            1 => transparency(app, ui, d),
            2 => typography(ui, d),
            _ => color(app, ui, d),
        }
    });
    close
}

/// A dropdown row bound to the string `d.fields[key]`; `options` are (value, label).
pub(super) fn choice(ui: &mut egui::Ui, d: &mut Dialog, key: &str, label: &str, options: &[(&str, &str)]) {
    form::choice(ui, d, key, label, (LABEL, 220.0), options);
}

/// A checkbox bound to `d.fields[key]`.
pub(super) fn check(ui: &mut egui::Ui, d: &mut Dialog, key: &str, label: &str) {
    let on = d.bool(key);
    if widgets::check(ui, label, on, true) {
        d.fields.insert(key.into(), json!(!on));
    }
}

/// The units the dialog shows lengths in.
fn unit(d: &Dialog) -> Unit {
    Unit::named(&d.str("units")).unwrap_or_default()
}

/// The General tab of Document Setup (units, bleed, artboard editing, options).
fn general(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let units: Vec<(&str, &str)> = Unit::ALL.iter().map(|u| (u.label(), u.label())).collect();
    choice(ui, d, "units", tl!("Units:"), &units);
    ui.add_space(6.0);
    let unit = unit(d);
    widgets::label_row(ui, tl!("Bleed:"), LABEL, |ui| form::bleed(ui, d, unit, 64.0));
    ui.add_space(6.0);
    let mut close = false;
    widgets::label_row(ui, "", LABEL, |ui| {
        // Applies the settings, then edits the artboards with the Artboard tool.
        if widgets::flat_button_fit(ui, tl!("Edit Artboards"), 120.0).clicked() && confirm(app, d).is_ok() {
            app.select_tool("artboard");
            // The shared frame restores its working copy after drawing the body. Tell it to
            // close too, otherwise it reopens the dialog that confirm just closed.
            close = true;
        }
    });
    ui.add_space(12.0);
    check(ui, d, "outlineImages", tl!("Show Images in Outline Mode"));
    check(ui, d, "highlightSubstitutedGlyphs", tl!("Highlight Substituted Glyphs"));
    close
}

fn transparency(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    let t = Tokens::get(ui.ctx());
    widgets::subheader(ui, tl!("Transparency Grid"));
    let sizes: Vec<(&str, &str)> = GridSize::ALL.iter().map(|g| (g.id(), g.label())).collect();
    choice(ui, d, "gridSize", tl!("Grid Size:"), &sizes);
    let colors: Vec<String> = d
        .fields
        .get("gridColors")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|c| c.as_str().unwrap_or("").to_string()).collect())
        .unwrap_or_default();
    let preset = GRID_COLOR_PRESETS
        .iter()
        .find(|(_, pair)| colors.len() == 2 && colors.iter().zip(pair).all(|(h, c)| h.eq_ignore_ascii_case(&c.to_hex())))
        .map_or("Custom", |(n, _)| n);
    let names: Vec<&str> = GRID_COLOR_PRESETS.iter().map(|(n, _)| *n).chain(["Custom"]).collect();
    widgets::label_row(ui, tl!("Grid Colors:"), LABEL, |ui| {
        if let Some((_, pair)) = widgets::dropdown(ui, "gridColors", preset, &names, 220.0).and_then(|i| GRID_COLOR_PRESETS.get(i)) {
            d.fields.insert("gridColors".into(), json!(pair.map(|c| c.to_hex())));
        }
    });
    // The two colours (the first is also the paper colour): editing either makes them Custom.
    widgets::label_row(ui, "", LABEL, |ui| {
        for i in 0..2 {
            let hex = colors.get(i).map_or("", |h| h.trim_start_matches('#'));
            let (chip, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
            if let Some(c) = vectorcraft_color::Color::from_hex(hex) {
                ui.painter().rect_filled(chip, 2.0, crate::panels::c32(&c));
            }
            ui.painter().rect_stroke(chip, 2.0, egui::Stroke::new(1.0, t.input_border), egui::StrokeKind::Inside);
            let name = format!("{} {}", tl!("Grid Colors:"), i + 1);
            if let Some(c) = widgets::hex_field_named(ui, ("grid-color", i), Some(&name), hex).and_then(|h| vectorcraft_color::Color::from_hex(&h)) {
                let mut v = colors.clone();
                v.resize(2, "#ffffff".into());
                v[i] = c.to_hex();
                d.fields.insert("gridColors".into(), json!(v));
            }
            ui.add_space(10.0);
        }
    });
    ui.add_space(4.0);
    check(ui, d, "simulatePaper", tl!("Simulate Colored Paper"));
    ui.add_space(14.0);
    widgets::subheader(ui, tl!("Export and Clipboard Transparency Flattener Settings"));
    let presets: Vec<String> = app.session.flattener_presets().into_iter().map(|p| p.name).collect();
    let options: Vec<(&str, &str)> = presets.iter().map(|p| (p.as_str(), p.as_str())).collect();
    choice(ui, d, "flattenerPreset", tl!("Preset:"), &options);
    check(ui, d, "discardWhiteOverprint", tl!("Discard White Overprint in Output"));
}

fn typography(ui: &mut egui::Ui, d: &mut Dialog) {
    let t = Tokens::get(ui.ctx());
    let langs: Vec<(&str, &str)> = LANGUAGES.iter().map(|(l, _, _)| (*l, *l)).collect();
    let before = d.str("language");
    choice(ui, d, "language", tl!("Language:"), &langs);
    // A language brings its quotes.
    if d.str("language") != before
        && let Some(q) = vectorcraft_doc::setup::language_quotes(&d.str("language"))
    {
        d.fields.insert("quotes".into(), json!({"double": q.double.iter().collect::<String>(), "single": q.single.iter().collect::<String>()}));
    }
    for (key, label, choices) in
        [("double", tl!("Double Quotes:"), Quotes::DOUBLE_CHOICES), ("single", tl!("Single Quotes:"), Quotes::SINGLE_CHOICES)]
    {
        let pairs: Vec<String> = choices.iter().map(|p| p.iter().collect()).collect();
        let labels: Vec<&str> = pairs.iter().map(String::as_str).collect();
        let cur = d.fields.get("quotes").and_then(|q| q.get(key)).and_then(Value::as_str).unwrap_or("").to_string();
        widgets::label_row(ui, label, LABEL, |ui| {
            if let Some(pair) = widgets::dropdown(ui, ("quotes", key), &cur, &labels, 90.0).and_then(|i| pairs.get(i)) {
                let mut q = d.fields.get("quotes").cloned().unwrap_or_else(|| json!({}));
                q[key] = json!(pair);
                d.fields.insert("quotes".into(), q);
            }
        });
    }
    check(ui, d, "typographersQuotes", tl!("Use Typographer's Quotes"));
    ui.add_space(14.0);
    widgets::subheader(ui, tl!("Options"));
    egui::Grid::new("docsetup-scripts").num_columns(3).spacing([16.0, 6.0]).show(ui, |ui| {
        ui.label("");
        for h in [tl!("Size"), tl!("Position")] {
            ui.label(egui::RichText::new(h).size(11.0).color(t.text_dim));
        }
        ui.end_row();
        for (key, label) in [("superscript", tl!("Superscript:")), ("subscript", tl!("Subscript:"))] {
            widgets::field_label(ui, egui::RichText::new(label).color(t.text));
            for part in ["size", "position"] {
                let v = d.fields.get(key).and_then(|m| m.get(part)).and_then(Value::as_f64).unwrap_or(0.0);
                if let Some(n) = widgets::plain_field(ui, (key, part), v, "%", 1, 70.0) {
                    let mut m = d.fields.get(key).cloned().unwrap_or_else(|| json!({}));
                    m[part] = json!(n);
                    d.fields.insert(key.into(), m);
                }
            }
            ui.end_row();
        }
        widgets::field_label(ui, egui::RichText::new(tl!("Small Caps:")).color(t.text));
        if let Some(n) = widgets::plain_field(ui, "smallCaps", d.f64("smallCapsSize", 70.0), "%", 1, 70.0) {
            d.fields.insert("smallCapsSize".into(), json!(n));
        }
        ui.end_row();
    });
    ui.add_space(6.0);
    choice(ui, d, "exportText", tl!("Export:"), &[("editable", tl!("Preserve Text Editability")), ("appearance", tl!("Preserve Text Appearance"))]);
}

/// The Color tab: the document color mode, whether a mode change converts the colors (unchecked:
/// they keep their numbers), and the RGB and CMYK profiles assigned to the document.
fn color(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    widgets::subheader(ui, tl!("Document Color Mode"));
    choice(ui, d, "colorMode", tl!("Color Mode:"), &[("rgb", "RGB Color"), ("cmyk", "CMYK Color")]);
    let mode = d.str("colorMode");
    let changes = app.session.active().is_some_and(|st| mode_id(st.doc.color_mode) != mode);
    let label = if mode == "cmyk" { tl!("Convert to CMYK") } else { tl!("Convert to RGB") };
    widgets::label_row(ui, "", LABEL, |ui| {
        let on = d.bool("convertColors");
        if widgets::check(ui, label, on, changes) {
            d.fields.insert("convertColors".into(), json!(!on));
        }
    });
    ui.add_space(14.0);
    widgets::subheader(ui, tl!("Assign Profile"));
    for (key, label, kind, working) in
        [("rgbProfile", tl!("RGB:"), ProfileKind::Rgb, WORKING_RGB), ("cmykProfile", tl!("CMYK:"), ProfileKind::Cmyk, WORKING_CMYK)]
    {
        let current = d.fields.get(key).and_then(Value::as_str).unwrap_or(working).to_string();
        widgets::label_row(ui, label, LABEL, |ui| {
            if let Some(n) = crate::panels::separations::profile_dropdown(ui, key, &current, kind, Some(working)) {
                d.fields.insert(key.into(), if n == working { Value::Null } else { json!(n) });
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use egui::{Event, PointerButton, Pos2, Rect, vec2};
    use serde_json::json;
    use vectorcraft_color::{Color, Paint, cms};
    use vectorcraft_doc::{ColorMode, NodeId};
    use vectorcraft_engine::Session;

    use crate::VectorcraftApp;

    fn dialog_frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>) -> Vec<(String, Rect)> {
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 900.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| crate::dialogs::show(app, ui.ctx()));
        out.textures_delta.clear();
        out.shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some((t.galley.text().to_string(), Rect::from_min_size(t.pos, t.galley.size()))),
                _ => None,
            })
            .collect()
    }

    fn click_dialog_button(app: &mut VectorcraftApp, label: &str) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, Default::default());
        dialog_frame(app, &ctx, vec![]);
        let texts = dialog_frame(app, &ctx, vec![]);
        let at = texts.iter().find(|(text, _)| text == label).unwrap_or_else(|| panic!("missing {label}: {texts:?}")).1.center();
        let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        dialog_frame(app, &ctx, vec![Event::PointerMoved(at), button(true)]);
        dialog_frame(app, &ctx, vec![button(false)]);
        dialog_frame(app, &ctx, vec![]);
    }

    #[test]
    fn edit_artboards_applies_settings_and_closes_the_dialog() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("file.documentSetup", json!({})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("bleed".into(), json!([9, 9, 9, 9]));
        click_dialog_button(&mut app, "Edit Artboards");
        assert!(app.ui.dialog.is_none(), "Edit Artboards must leave the canvas accessible");
        assert_eq!(app.session.tool_id(), "artboard");
        let st = app.session.active().unwrap();
        assert_eq!(st.doc.setup.bleed, [9.0; 4]);
        assert_eq!(st.history.undo.len(), 1);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.setup.bleed, [0.0; 4]);
    }

    #[test]
    fn edit_artboards_with_invalid_settings_keeps_the_dialog_and_tool() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("file.documentSetup", json!({})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("bleed".into(), json!(500));
        click_dialog_button(&mut app, "Edit Artboards");
        assert!(app.ui.dialog.is_some());
        assert_eq!(app.session.tool_id(), "selection");
        let st = app.session.active().unwrap();
        assert_eq!(st.doc.setup.bleed, [0.0; 4]);
        assert!(st.history.undo.is_empty());
    }

    #[test]
    fn ok_and_cancel_close_without_switching_tools() {
        for (button, expected_bleed, undo_steps) in [("OK", [9.0; 4], 1), ("Cancel", [0.0; 4], 0)] {
            let mut app = VectorcraftApp::new(Session::new(), Default::default());
            app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
            app.run("file.documentSetup", json!({})).unwrap();
            app.ui.dialog.as_mut().unwrap().fields.insert("bleed".into(), json!([9, 9, 9, 9]));
            click_dialog_button(&mut app, button);
            assert!(app.ui.dialog.is_none(), "{button}");
            assert_eq!(app.session.tool_id(), "selection");
            let st = app.session.active().unwrap();
            assert_eq!(st.doc.setup.bleed, expected_bleed);
            assert_eq!(st.history.undo.len(), undo_steps);
        }
    }

    fn frame(app: &mut VectorcraftApp) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, Default::default());
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 900.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| crate::dialogs::show(app, ui.ctx()));
        out.textures_delta.clear();
    }

    #[test]
    fn every_tab_draws_and_ok_is_one_undo_step() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("file.documentSetup", json!({})).unwrap();
        for tab in ["General", "Transparency", "Type", "Color"] {
            app.ui.dialog.as_mut().unwrap().fields.insert("tab".into(), json!(tab));
            frame(&mut app);
        }
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!((d.fields["flattenerPreset"].as_str(), d.fields["bleedLinked"].as_bool()), (Some("Medium Resolution"), Some(true)));
        for (k, v) in [("bleed", json!([9, 9, 9, 9])), ("gridColors", json!("Blue")), ("language", json!("German")), ("units", json!("Millimeters"))]
        {
            d.fields.insert(k.into(), v);
        }
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let st = app.session.active().unwrap();
        assert_eq!((st.doc.setup.bleed, st.doc.setup.grid_colors_name()), ([9.0; 4], "Blue"));
        assert_eq!((st.doc.setup.quotes.double, st.doc.units), (['„', '“'], vectorcraft_doc::Unit::Millimeters));
        assert_eq!(st.history.undo.len(), 1);
        // A bad value keeps the dialog open.
        app.run("file.documentSetup", json!({})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("bleed".into(), json!(500));
        assert!(crate::dialogs::confirm(&mut app).is_err());
        assert!(app.ui.dialog.is_some());
    }

    /// Highlight Substituted Fonts is in Preferences › Type, so the General tab shows only the
    /// substituted glyphs highlight, and OK still applies.
    #[test]
    fn general_tab_has_no_substituted_fonts_checkbox() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("file.documentSetup", json!({})).unwrap();
        assert!(app.ui.dialog.as_ref().unwrap().fields.get("highlightSubstitutedFonts").is_none());
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, Default::default());
        dialog_frame(&mut app, &ctx, vec![]);
        let texts: Vec<String> = dialog_frame(&mut app, &ctx, vec![]).into_iter().map(|(t, _)| t).collect();
        assert!(texts.iter().any(|t| t == "Highlight Substituted Glyphs"), "{texts:?}");
        assert!(!texts.iter().any(|t| t == "Highlight Substituted Fonts"), "{texts:?}");
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
    }

    /// A new document with a red square → the app and the square.
    fn red_square() -> (VectorcraftApp, NodeId) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
        app.run("paint.setFill", json!({"ids": [id], "color": "#ff0000"})).unwrap();
        (app, NodeId(id))
    }

    fn fill(app: &VectorcraftApp, id: NodeId) -> Paint {
        app.session.active().unwrap().doc.node(id).unwrap().appearance.fill().unwrap().paint.clone()
    }

    fn undo_steps(app: &VectorcraftApp) -> usize {
        app.session.active().unwrap().history.undo.len()
    }

    /// Opens Document Setup on the Color tab and sets `fields` in it.
    fn color_tab(app: &mut VectorcraftApp, fields: serde_json::Value) {
        app.run("file.documentSetup", json!({})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("tab".into(), json!("Color"));
        for (k, v) in fields.as_object().unwrap() {
            d.fields.insert(k.clone(), v.clone());
        }
    }

    /// The Color tab opens on the document's color mode and assigned profiles, and shows them.
    /// The CMYK profile assigned is the working one, so the process-wide color settings stay as
    /// they are for the other tests.
    #[test]
    fn color_tab_shows_the_color_mode_and_profiles() {
        let (mut app, _) = red_square();
        let working = cms::active_settings().cmyk;
        app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
        app.run("edit.assignProfile", json!({"cmyk": working})).unwrap();
        let steps = undo_steps(&app);
        color_tab(&mut app, json!({}));
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.str("colorMode"), d.str("cmykProfile"), d.fields["rgbProfile"].clone()), ("cmyk".into(), working.clone(), json!(null)));
        assert!(d.bool("convertColors"));
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, Default::default());
        dialog_frame(&mut app, &ctx, vec![]);
        let texts: Vec<String> = dialog_frame(&mut app, &ctx, vec![]).into_iter().map(|(t, _)| t).collect();
        for shown in ["CMYK Color", "Convert to CMYK", "Working RGB (don't tag)", working.as_str()] {
            assert!(texts.iter().any(|t| t == shown), "no {shown} in {texts:?}");
        }
        // Picking the other mode names the conversion OK makes.
        app.ui.dialog.as_mut().unwrap().fields.insert("colorMode".into(), json!("rgb"));
        let texts: Vec<String> = dialog_frame(&mut app, &ctx, vec![]).into_iter().map(|(t, _)| t).collect();
        assert!(texts.iter().any(|t| t == "Convert to RGB"), "{texts:?}");
        app.ui.dialog.as_mut().unwrap().fields.insert("colorMode".into(), json!("cmyk"));
        // OK with nothing changed records nothing.
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(undo_steps(&app), steps);
    }

    /// OK with a new mode converts the colors as File › Document Color Mode does, together with the
    /// dialog's other changes, in one undo step.
    #[test]
    fn ok_converts_the_color_mode_with_the_other_changes_in_one_undo_step() {
        let (mut app, id) = red_square();
        let (mut menu, menu_id) = red_square();
        menu.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
        let steps = undo_steps(&app);
        color_tab(&mut app, json!({"colorMode": "cmyk", "bleed": [9, 9, 9, 9]}));
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let st = app.session.active().unwrap();
        assert_eq!((st.doc.color_mode, st.doc.setup.bleed), (ColorMode::Cmyk, [9.0; 4]));
        assert_eq!(fill(&app, id), fill(&menu, menu_id));
        assert!(matches!(fill(&app, id), Paint::Solid { color: Color::Cmyk { .. }, .. }), "{:?}", fill(&app, id));
        assert_eq!(undo_steps(&app), steps + 1);
        app.run("edit.undo", json!({})).unwrap();
        let st = app.session.active().unwrap();
        assert_eq!((st.doc.color_mode, st.doc.setup.bleed), (ColorMode::Rgb, [0.0; 4]));
        assert_eq!(fill(&app, id), Paint::solid(Color::from_hex("ff0000").unwrap()));
    }

    /// With Convert unchecked the mode changes and the colors keep their numbers.
    #[test]
    fn ok_without_convert_keeps_the_color_numbers() {
        let (mut app, id) = red_square();
        let before = fill(&app, id);
        let steps = undo_steps(&app);
        color_tab(&mut app, json!({"colorMode": "cmyk", "convertColors": false}));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(app.session.active().unwrap().doc.color_mode, ColorMode::Cmyk);
        assert_eq!(fill(&app, id), before);
        assert_eq!(undo_steps(&app), steps + 1);
    }

    /// OK assigns the profiles picked, with the dialog's other changes, in one undo step; the
    /// working-space entry assigns none.
    #[test]
    fn ok_assigns_the_profiles_with_the_other_changes_in_one_undo_step() {
        let (mut app, _) = red_square();
        let working = cms::active_settings().cmyk;
        let steps = undo_steps(&app);
        color_tab(&mut app, json!({"cmykProfile": working, "bleed": [9, 9, 9, 9]}));
        crate::dialogs::confirm(&mut app).unwrap();
        let st = app.session.active().unwrap();
        assert_eq!((st.doc.color_profiles.cmyk.clone(), st.doc.color_profiles.rgb.clone()), (Some(working.clone()), None));
        assert_eq!((st.doc.setup.bleed, st.doc.color_mode), ([9.0; 4], ColorMode::Rgb));
        assert_eq!(undo_steps(&app), steps + 1);
        // Back to the working space: no profile assigned.
        color_tab(&mut app, json!({"cmykProfile": null}));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(app.session.active().unwrap().doc.color_profiles.cmyk, None);
        app.run("edit.undo", json!({})).unwrap();
        app.run("edit.undo", json!({})).unwrap();
        let st = app.session.active().unwrap();
        assert_eq!((st.doc.color_profiles.cmyk.clone(), st.doc.setup.bleed), (None, [0.0; 4]));
    }

    /// OK assigns the RGB profile picked, in one undo step.
    #[test]
    fn ok_assigns_the_rgb_profile() {
        let (mut app, _) = red_square();
        let working = cms::active_settings().rgb;
        let steps = undo_steps(&app);
        color_tab(&mut app, json!({"rgbProfile": working}));
        crate::dialogs::confirm(&mut app).unwrap();
        let st = app.session.active().unwrap();
        assert_eq!((st.doc.color_profiles.rgb.clone(), st.doc.color_profiles.cmyk.clone()), (Some(working), None));
        assert_eq!(undo_steps(&app), steps + 1);
    }

    /// A profile and a mode changed in one OK: Assign Profile runs before Document Color Mode, so
    /// the conversion goes through the profile picked.
    #[test]
    fn a_profile_and_a_mode_change_assign_the_profile_first() {
        let (mut app, _) = red_square();
        color_tab(&mut app, json!({"colorMode": "cmyk", "cmykProfile": cms::DEVICE_CMYK}));
        let commands = super::color_commands(&app, app.ui.dialog.as_ref().unwrap()).unwrap();
        assert_eq!(
            commands,
            vec![
                json!({"command": "edit.assignProfile", "params": {"cmyk": cms::DEVICE_CMYK}}),
                json!({"command": "file.documentColorMode", "params": {"mode": "cmyk", "convert": true}}),
            ]
        );
    }

    /// A Color tab field with a value of the wrong kind is an error: the dialog stays open and
    /// nothing changes, the profile picked with it included.
    #[test]
    fn a_bad_color_field_is_an_error_and_changes_nothing() {
        let bad = [
            ("colorMode", json!("lab")),
            ("colorMode", json!(null)),
            ("rgbProfile", json!(5)),
            ("cmykProfile", json!(true)),
            ("convertColors", json!("no")),
        ];
        for (key, value) in bad {
            let (mut app, id) = red_square();
            let before = fill(&app, id);
            let steps = undo_steps(&app);
            color_tab(&mut app, json!({"colorMode": "cmyk", "cmykProfile": cms::active_settings().cmyk}));
            app.ui.dialog.as_mut().unwrap().fields.insert(key.into(), value.clone());
            assert!(super::color_commands(&app, app.ui.dialog.as_ref().unwrap()).is_err(), "{key}: {value}");
            assert!(crate::dialogs::confirm(&mut app).is_err(), "{key}: {value}");
            assert!(app.ui.dialog.is_some());
            let st = app.session.active().unwrap();
            assert_eq!((st.doc.color_mode, st.doc.color_profiles.cmyk.clone()), (ColorMode::Rgb, None), "{key}: {value}");
            assert_eq!(fill(&app, id), before);
            assert_eq!(undo_steps(&app), steps);
        }
    }

    /// Cancel leaves the color mode, the colors and the history as they were.
    #[test]
    fn cancel_changes_no_color_setting() {
        let (mut app, id) = red_square();
        let before = fill(&app, id);
        let steps = undo_steps(&app);
        color_tab(&mut app, json!({"colorMode": "cmyk", "cmykProfile": cms::active_settings().cmyk}));
        click_dialog_button(&mut app, "Cancel");
        assert!(app.ui.dialog.is_none());
        let st = app.session.active().unwrap();
        assert_eq!((st.doc.color_mode, st.doc.color_profiles.cmyk.clone()), (ColorMode::Rgb, None));
        assert_eq!(fill(&app, id), before);
        assert_eq!(undo_steps(&app), steps);
    }
}
