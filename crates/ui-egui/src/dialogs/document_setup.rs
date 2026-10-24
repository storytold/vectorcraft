//! File → Document Setup: General (units, bleed, Edit Artboards, outline images, substitution
//! highlights), Transparency (grid size and colours, simulated paper, flattener preset, white
//! overprint) and Type (language, quotes, superscript/subscript/small caps, text export) tabs.
//!
//! Fields: what `document.setup` reports (and takes back), plus `tab` and `bleedLinked`. OK runs
//! `document.setup` with them: one undo step. On an error the dialog stays open.

use serde_json::{Value, json};
use vectorcraft_doc::setup::{GRID_COLOR_PRESETS, LANGUAGES};
use vectorcraft_doc::{GridSize, Quotes, Unit};

use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

pub(super) const KIND: &str = "documentSetup";
const TABS: [&str; 3] = ["General", "Transparency", "Type"];
/// Dialog-only fields `document.setup` doesn't take.
const UI_ONLY: [&str; 2] = ["tab", "bleedLinked"];
pub(super) const LABEL: f32 = 130.0;

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Document Setup").into(), body, confirm, min_width: 540.0, max_width: Some(540.0), ..DialogSpec::FORM };

/// Open Document Setup on the active document's settings.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let mut fields = app.session.execute("document.setup", &json!({})).map_err(|e| e.to_string())?;
    let linked = fields["bleed"].as_array().is_some_and(|b| b.windows(2).all(|w| w[0] == w[1]));
    fields["bleedLinked"] = json!(linked);
    fields["tab"] = json!(TABS[0]);
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let params = d.fields.iter().filter(|(k, _)| !UI_ONLY.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
    let r = app.run("document.setup", Value::Object(params));
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
            _ => typography(ui, d),
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
    check(ui, d, "highlightSubstitutedFonts", tl!("Highlight Substituted Fonts"));
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
            if let Some(c) = widgets::hex_field(ui, ("grid-color", i), hex).and_then(|h| vectorcraft_color::Color::from_hex(&h)) {
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
    let langs: Vec<(&str, &str)> = LANGUAGES.iter().map(|(l, _)| (*l, *l)).collect();
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

#[cfg(test)]
mod tests {
    use egui::{Event, PointerButton, Pos2, Rect, vec2};
    use serde_json::json;
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
        for tab in ["General", "Transparency", "Type"] {
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
}
