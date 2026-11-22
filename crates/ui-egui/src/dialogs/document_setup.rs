//! File → Document Setup: General (units, bleed, Edit Artboards, outline images, the substituted
//! glyphs highlight), Transparency (grid size and colours, simulated paper, flattener preset, white
//! overprint), Type (language, quotes, superscript/subscript/small caps, text export) and Color
//! (document color mode and assigned profiles) tabs.
//!
//! Fields: what `document.setup` reports (and takes back), plus `tab`, `bleedLinked` and the Color
//! tab's `colorMode`, `convertColors`, `rgbProfile` and `cmykProfile` (null: the working space).
//! OK runs `document.setup` with them: one undo step. When the Color tab changed something, OK runs
//! `document.setup`, `edit.assignProfile` and `file.documentColorMode` in one `command.batch`,
//! and the document's changes are still one undo step. On an error the dialog stays open. When OK
//! raises the bleed on a side, the status bar counts the objects that reach an artboard's trim edge
//! on a side OK raised but stop short of the new bleed there.

use serde_json::{Map, Value, json};
use vectorcraft_color::cms::ProfileKind;
use vectorcraft_doc::setup::{GRID_COLOR_PRESETS, LANGUAGES, bleed_in_range};
use vectorcraft_doc::{ColorMode, GridSize, Quotes, Unit};
use vectorcraft_engine::cmd::colormgmt::doc_profiles;
use vectorcraft_engine::cmd::fileio::pdf;

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
        fields["gridColors"] = json!(st.doc.setup.grid_colors.map(|c| grid_color_entry(&c)));
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
    let before = app.session.active().map(|st| st.doc.setup.bleed);
    let r = if color.is_empty() {
        app.run("document.setup", Value::Object(params))
    } else {
        let commands: Vec<Value> = std::iter::once(json!({"command": "document.setup", "params": params})).chain(color).collect();
        app.run("command.batch", json!({"label": "Document Setup", "commands": commands}))
    };
    if r.is_ok() {
        app.ui.dialog = None;
        if let Some(note) = before.and_then(|b| short_of_bleed_note(app, b)) {
            app.status(note);
        }
    }
    r
}

/// The status note after OK raised the bleed on a side from `before`: how many objects reach an
/// artboard's trim edge on a side whose bleed grew but stop short of the new bleed there. `None`
/// when no side grew or no object stops short.
fn short_of_bleed_note(app: &VectorcraftApp, before: [f64; 4]) -> Option<String> {
    let doc = &app.session.active()?.doc;
    // In the range the PDF pages keep the document's bleed to.
    let (bleed, before) = (bleed_in_range(doc.setup.bleed), bleed_in_range(before));
    let raised: [f64; 4] = std::array::from_fn(|i| if bleed[i] > before[i] { bleed[i] } else { 0.0 });
    pdf::short_of_bleed_warning(pdf::short_of_bleed(doc, raised, None, false))
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

/// The id of the Transparency tab's box that shows grid color `index` (0 or 1).
fn grid_color_box_id(index: usize) -> egui::Id {
    egui::Id::unique(("grid-color-box", index))
}

/// The size of the Transparency tab's grid preview, in points.
const GRID_PREVIEW: egui::Vec2 = egui::vec2(112.0, 72.0);

/// The grid size and the two colors the grid preview shows for the dialog's fields `fields`, read
/// as `document.setup` reads them (a preset name or two colors): a field that is missing or not a
/// valid value shows as `setup`, the document's setting.
fn preview_grid(fields: &Map<String, Value>, setup: &vectorcraft_doc::DocSetup) -> (GridSize, [vectorcraft_color::Color; 2]) {
    let size = fields.get("gridSize").and_then(Value::as_str).and_then(GridSize::parse).unwrap_or(setup.grid_size);
    (size, grid_colors(fields, setup))
}

/// The two grid colors the dialog's fields `fields` hold, read as `document.setup` reads
/// `gridColors` (a preset name, in any letter case, or two colors): a field that is missing or not
/// a valid value gives `setup`'s colors, the document's setting.
fn grid_colors(fields: &Map<String, Value>, setup: &vectorcraft_doc::DocSetup) -> [vectorcraft_color::Color; 2] {
    let colors = match fields.get("gridColors") {
        Some(Value::String(name)) => GRID_COLOR_PRESETS.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, pair)| *pair),
        Some(Value::Array(a)) if a.len() == 2 => a.iter().map(vectorcraft_engine::cmd::color_value).collect::<Option<Vec<_>>>().map(|c| [c[0], c[1]]),
        _ => None,
    };
    colors.unwrap_or(setup.grid_colors)
}

/// The dialog's `gridColors` when it is a list of two colors `document.setup` reads, entries as
/// they are.
fn pending_grid_pair(fields: &Map<String, Value>) -> Option<Vec<Value>> {
    let entries = fields.get("gridColors").and_then(Value::as_array)?;
    (entries.len() == 2 && entries.iter().all(|c| vectorcraft_engine::cmd::color_value(c).is_some())).then(|| entries.clone())
}

/// Color `c` as an entry of the dialog's `gridColors` that `document.setup` reads back as `c`: its
/// hex code when that reads back as `c`, else the color in its own model (a Lab or CMYK color, or
/// an RGB color whose hex code is converted to display RGB or rounded).
fn grid_color_entry(c: &vectorcraft_color::Color) -> Value {
    let hex = json!(c.to_hex());
    if vectorcraft_engine::cmd::color_value(&hex) == Some(*c) { hex } else { serde_json::to_value(c).unwrap_or(hex) }
}

/// The checkered preview of the grid the dialog's Grid Size and Grid Colors hold, drawn with the
/// canvas's transparency grid tile and cell size: the first color in the top-left cell. It goes
/// beside the rows that start at `top`, against the right edge, or below them when they leave no
/// room. It shows the grid whatever Simulate Colored Paper and View › Show Transparency Grid are,
/// and changes nothing in the document.
fn grid_preview(app: &VectorcraftApp, ui: &mut egui::Ui, d: &Dialog, top: f32) {
    let t = Tokens::get(ui.ctx());
    let fallback = vectorcraft_doc::DocSetup::default();
    let setup = app.session.active().map_or(&fallback, |st| &st.doc.setup);
    let (size, colors) = preview_grid(&d.fields, setup);
    let right = ui.max_rect().right();
    let rect = if ui.min_rect().right() + 16.0 + GRID_PREVIEW.x <= right {
        egui::Rect::from_min_size(egui::pos2(right - GRID_PREVIEW.x, top), GRID_PREVIEW)
    } else {
        ui.add_space(4.0);
        ui.allocate_exact_size(GRID_PREVIEW, egui::Sense::hover()).0
    };
    let resp = ui.interact(rect, grid_preview_id(), egui::Sense::hover());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, tl!("Transparency Grid")));
    let display = app.display_profile.transform(app.session.prefs.monitor_profile_display);
    let id = egui::Id::unique("document-setup-grid-preview").with(display.is_some());
    let tile = crate::canvas::checker_tile(ui.ctx(), colors, display.as_deref(), id);
    let uv = egui::Rect::from_min_max(egui::Pos2::ZERO, (rect.size() / (2.0 * size.cell())).to_pos2());
    ui.painter().image(tile.id(), rect, uv, egui::Color32::WHITE);
    ui.painter().rect_stroke(rect, 0.0, egui::Stroke::new(1.0, t.input_border), egui::StrokeKind::Outside);
}

/// The id of the Transparency tab's grid preview.
fn grid_preview_id() -> egui::Id {
    egui::Id::unique("grid-preview")
}

fn transparency(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    let t = Tokens::get(ui.ctx());
    widgets::subheader(ui, tl!("Transparency Grid"));
    let top = ui.cursor().top();
    let sizes: Vec<(&str, &str)> = GridSize::ALL.iter().map(|g| (g.id(), g.label())).collect();
    choice(ui, d, "gridSize", tl!("Grid Size:"), &sizes);
    // A preset name in the fields (any case, as `document.setup` takes it) becomes the preset's two
    // colors, which the hex fields and the Color Picker edit one at a time.
    if let Some((_, pair)) =
        d.fields.get("gridColors").and_then(Value::as_str).and_then(|n| GRID_COLOR_PRESETS.iter().find(|(p, _)| p.eq_ignore_ascii_case(n)))
    {
        d.fields.insert("gridColors".into(), json!(pair.map(|c| grid_color_entry(&c))));
    }
    // The pair the field holds, or the document's pair when it holds no two colors, so each box and
    // hex field shows a color and the picker and hex fields set an entry of a two-color list. The
    // menu names the preset whose colors the pair is, as `document.setup` names it.
    let fallback = vectorcraft_doc::DocSetup::default();
    let setup = app.session.active().map_or(&fallback, |st| &st.doc.setup);
    let resolved = grid_colors(&d.fields, setup);
    let colors: Vec<String> = resolved.iter().map(vectorcraft_color::Color::to_hex).collect();
    let preset = GRID_COLOR_PRESETS.iter().find(|(_, pair)| *pair == resolved).map_or("Custom", |(n, _)| n);
    let names: Vec<&str> = GRID_COLOR_PRESETS.iter().map(|(n, _)| *n).chain(["Custom"]).collect();
    widgets::label_row(ui, tl!("Grid Colors:"), LABEL, |ui| {
        if let Some((_, pair)) = widgets::dropdown(ui, "gridColors", preset, &names, 220.0).and_then(|i| GRID_COLOR_PRESETS.get(i)) {
            d.fields.insert("gridColors".into(), json!(pair.map(|c| grid_color_entry(&c))));
        }
    });
    // The two colours (the first is also the paper colour): editing either makes them Custom. A
    // click on a color's box, or Space or Enter with the keyboard on it, opens the Color Picker on
    // that color, and the picker's OK sets that color. The picker opens after the row, on the color
    // Document Setup holds then: a press on a box ends a hex field's editing in the same frame, and
    // that field's edit is the color the picker opens on. A hex field's edit replaces its own color
    // in the pair Document Setup holds when it commits, and the other entry stays as it is.
    let mut pick = None;
    widgets::label_row(ui, "", LABEL, |ui| {
        for i in 0..2 {
            let hex = colors.get(i).map_or("", |h| h.trim_start_matches('#'));
            let colour = vectorcraft_color::Color::from_hex(hex);
            let name = format!("{} {}", tl!("Grid Colors:"), i + 1);
            let (chip, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
            let resp = ui.interact(chip, grid_color_box_id(i), egui::Sense::click());
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
            let keyed = resp.has_focus()
                && ui.input_mut(|inp| {
                    inp.consume_key(egui::Modifiers::NONE, egui::Key::Space) | inp.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                });
            if resp.clicked() || keyed {
                pick = Some(i);
            }
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if let Some(c) = colour {
                ui.painter().rect_filled(chip, 2.0, crate::panels::c32(&c));
            }
            ui.painter().rect_stroke(chip, 2.0, egui::Stroke::new(1.0, t.input_border), egui::StrokeKind::Inside);
            if resp.has_focus() {
                ui.painter().rect_stroke(chip.expand(2.0), 3.0, ui.visuals().selection.stroke, egui::StrokeKind::Outside);
            }
            if let Some(c) = widgets::hex_field_named(ui, ("grid-color", i), Some(&name), hex).and_then(|h| vectorcraft_color::Color::from_hex(&h)) {
                let mut v = pending_grid_pair(&d.fields).unwrap_or_else(|| grid_colors(&d.fields, setup).iter().map(grid_color_entry).collect());
                v[i] = grid_color_entry(&c);
                d.fields.insert("gridColors".into(), json!(v));
            }
            ui.add_space(10.0);
        }
    });
    grid_preview(app, ui, d, top);
    if let Some(i) = pick {
        // A field that is not two colors becomes the pair the boxes show when the picker's OK sets
        // its entry; its Cancel goes back to the field as it was.
        let pair = grid_colors(&d.fields, setup);
        let fill = pending_grid_pair(&d.fields).is_none().then(|| json!(pair.map(|c| grid_color_entry(&c))));
        let c = pair.get(i).copied().unwrap_or(vectorcraft_color::Color::WHITE);
        *d = super::color_picker::over_list_entry(d, "gridColors", i, c, fill);
    }
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
    use serde_json::{Value, json};
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

    /// OK with a larger bleed shows how many objects reach the trim edge but stop short of it; a
    /// smaller bleed, or a larger one that no object falls short of, shows no such note.
    #[test]
    fn ok_with_a_larger_bleed_counts_the_art_short_of_it() {
        let short = |app: &VectorcraftApp| app.ui.status.contains("short of the bleed");
        let ok_with_sides = |app: &mut VectorcraftApp, bleed: [f64; 4]| {
            app.status("");
            app.run("file.documentSetup", json!({})).unwrap();
            app.ui.dialog.as_mut().unwrap().fields.insert("bleed".into(), json!(bleed));
            crate::dialogs::confirm(app).unwrap();
        };
        let ok_with_bleed = |app: &mut VectorcraftApp, bleed: f64| ok_with_sides(app, [bleed; 4]);
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        // A background that fills a 4 pt bleed on the top, left and right.
        let id = app.run("shape.rectangle", json!({"x": -4, "y": -4, "width": 208, "height": 50})).unwrap()["id"].clone();
        app.run("paint.setStroke", json!({"ids": [id], "none": true})).unwrap();
        ok_with_bleed(&mut app, 4.0);
        assert!(!short(&app), "{}", app.ui.status);
        ok_with_bleed(&mut app, 9.0);
        assert_eq!(app.ui.status, "1 object reaches the trim edge but stops short of the bleed");
        ok_with_bleed(&mut app, 6.0);
        assert!(!short(&app), "{}", app.ui.status);
        // Only the sides whose bleed grows count: the background stops short at the top, left and
        // right, so a larger bottom bleed alone shows no note, and a larger top bleed does.
        ok_with_sides(&mut app, [6.0, 12.0, 6.0, 6.0]);
        assert!(!short(&app), "{}", app.ui.status);
        ok_with_sides(&mut app, [12.0, 12.0, 6.0, 6.0]);
        assert!(short(&app), "{}", app.ui.status);
        // Art inside the trim falls short of no bleed.
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
        ok_with_bleed(&mut app, 9.0);
        assert!(!short(&app), "{}", app.ui.status);
        assert_eq!(app.session.active().unwrap().doc.setup.bleed, [9.0; 4]);
        // A bleed that isn't a number, as a file can hold, counts as none before OK, as in the PDF.
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 200, "height": 50})).unwrap();
        std::sync::Arc::make_mut(&mut app.session.doc_mut().unwrap().doc).setup.bleed = [f64::NAN; 4];
        ok_with_bleed(&mut app, 9.0);
        assert_eq!(app.ui.status, "1 object reaches the trim edge but stops short of the bleed");
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

    /// The Light pair a new document's transparency grid starts with.
    const LIGHT: [&str; 2] = ["#ffffff", "#cccccc"];

    fn hex(h: &str) -> Color {
        Color::from_hex(h).unwrap_or_else(|| panic!("bad hex {h}"))
    }

    /// The grid colors Document Setup `d` holds.
    fn dialog_grid(d: &crate::state::Dialog) -> Vec<Color> {
        d.fields["gridColors"].as_array().unwrap().iter().map(|c| vectorcraft_engine::cmd::color_value(c).unwrap()).collect()
    }

    /// A new document with Document Setup open on its Transparency tab, drawn in the context returned.
    fn transparency_tab() -> (VectorcraftApp, egui::Context) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("file.documentSetup", json!({})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("tab".into(), json!("Transparency"));
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, Default::default());
        for _ in 0..4 {
            dialog_frame(&mut app, &ctx, vec![]);
        }
        (app, ctx)
    }

    /// Clicks the box of grid color `index`.
    fn click_grid_box(app: &mut VectorcraftApp, ctx: &egui::Context, index: usize) {
        let at = ctx.read_response(super::grid_color_box_id(index)).unwrap_or_else(|| panic!("no box {index}")).rect.center();
        let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        dialog_frame(app, ctx, vec![Event::PointerMoved(at), button(true)]);
        dialog_frame(app, ctx, vec![button(false)]);
    }

    /// The Color Picker is open over Document Setup, on `color`, and Document Setup's OK has not run.
    fn assert_picker_over_setup(app: &VectorcraftApp, color: &str, case: &str) {
        let d = app.ui.dialog.as_ref().unwrap_or_else(|| panic!("{case}: no dialog"));
        assert_eq!(d.kind, "colorPicker", "{case}");
        assert_eq!(d.fields.get("color").and_then(vectorcraft_engine::cmd::color_value), Some(hex(color)), "{case}");
        assert_eq!(crate::dialogs::parent(d).map(|p| p.kind), Some(super::KIND.to_string()), "{case}");
        assert_eq!(undo_steps(app), 0, "{case}: Document Setup's OK has not run");
    }

    /// A click on either grid color's box opens the Color Picker over Document Setup on that
    /// color. Its OK sets only that color, the Grid Colors menu names the pair (Custom, or the
    /// preset it matches), and Document Setup's OK applies the pair in one undo step.
    #[test]
    fn a_grid_color_box_opens_the_color_picker_and_its_ok_sets_that_color() {
        for (index, picked, pair, name) in [(0, "#00ff00", ["#00ff00", "#cccccc"], "Custom"), (1, "#c4dcff", ["#ffffff", "#c4dcff"], "Blue")] {
            let (mut app, ctx) = transparency_tab();
            click_grid_box(&mut app, &ctx, index);
            assert_picker_over_setup(&app, LIGHT[index], name);
            app.ui.dialog.as_mut().unwrap().fields.insert("color".into(), json!(picked));
            crate::dialogs::confirm(&mut app).unwrap();
            let d = app.ui.dialog.as_ref().unwrap();
            assert_eq!((d.kind.as_str(), dialog_grid(d)), (super::KIND, pair.map(hex).to_vec()), "{name}");
            assert_eq!(undo_steps(&app), 0, "{name}: the picker's OK leaves the document as it is");
            dialog_frame(&mut app, &ctx, vec![]);
            let texts: Vec<String> = dialog_frame(&mut app, &ctx, vec![]).into_iter().map(|(t, _)| t).collect();
            assert!(texts.iter().any(|t| t == name), "{name}: {texts:?}");
            crate::dialogs::confirm(&mut app).unwrap();
            assert!(app.ui.dialog.is_none());
            let st = app.session.active().unwrap();
            assert_eq!((st.doc.setup.grid_colors, st.doc.setup.grid_colors_name()), (pair.map(hex), name));
            assert_eq!(undo_steps(&app), 1, "{name}");
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(app.session.active().unwrap().doc.setup.grid_colors, LIGHT.map(hex), "{name}");
        }
    }

    /// With the keyboard on a grid color's box, Enter or Space opens the Color Picker on that
    /// color instead of pressing Document Setup's OK.
    #[test]
    fn enter_or_space_on_a_grid_color_box_opens_the_color_picker() {
        for (index, key) in [(0, egui::Key::Enter), (1, egui::Key::Space)] {
            let (mut app, ctx) = transparency_tab();
            ctx.memory_mut(|m| m.request_focus(super::grid_color_box_id(index)));
            dialog_frame(&mut app, &ctx, vec![]);
            let press = Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
            dialog_frame(&mut app, &ctx, vec![press]);
            assert_picker_over_setup(&app, LIGHT[index], &format!("{key:?}"));
        }
    }

    /// Cancel in the Color Picker goes back to Document Setup with the grid colors as they were,
    /// and so does its OK on the color it opened on; Document Setup's OK then records no undo step.
    #[test]
    fn grid_color_picker_cancel_or_an_unchanged_ok_changes_nothing() {
        for cancel in [true, false] {
            let (mut app, ctx) = transparency_tab();
            click_grid_box(&mut app, &ctx, 1);
            assert_picker_over_setup(&app, LIGHT[1], &format!("cancel {cancel}"));
            if cancel {
                app.ui.dialog.as_mut().unwrap().fields.insert("color".into(), json!("#ff0000"));
                crate::dialogs::cancel(&mut app);
            } else {
                crate::dialogs::confirm(&mut app).unwrap();
            }
            let d = app.ui.dialog.as_ref().unwrap();
            assert_eq!((d.kind.as_str(), dialog_grid(d)), (super::KIND, LIGHT.map(hex).to_vec()), "cancel {cancel}");
            crate::dialogs::confirm(&mut app).unwrap();
            assert!(app.ui.dialog.is_none());
            assert_eq!(app.session.active().unwrap().doc.setup.grid_colors_name(), "Light");
            assert_eq!(undo_steps(&app), 0, "cancel {cancel}");
        }
    }

    /// When Grid Colors holds a preset name (in any letter case) or a list that is not two colors,
    /// the boxes and hex fields show the preset's pair or the document's, a box opens the Color
    /// Picker on its color, and the picker's OK sets only that entry of a two-color list.
    #[test]
    fn a_grid_color_box_opens_on_a_preset_name_or_a_short_list() {
        for (field, index, opens, kept, name) in [
            (json!("Blue"), 1, "#c4dcff", "#ffffff", "Blue"),
            (json!("purple"), 0, "#ffffff", "#e2c9ff", "Purple"),
            (json!(["#ff0000"]), 1, "#cccccc", "#ffffff", "Light"),
        ] {
            let (mut app, ctx) = transparency_tab();
            app.ui.dialog.as_mut().unwrap().fields.insert("gridColors".into(), field.clone());
            dialog_frame(&mut app, &ctx, vec![]);
            let texts: Vec<String> = dialog_frame(&mut app, &ctx, vec![]).into_iter().map(|(t, _)| t).collect();
            for shown in [name, opens.trim_start_matches('#'), kept.trim_start_matches('#')] {
                assert!(texts.iter().any(|t| t.eq_ignore_ascii_case(shown)), "{field}: no {shown} in {texts:?}");
            }
            click_grid_box(&mut app, &ctx, index);
            assert_picker_over_setup(&app, opens, &field.to_string());
            app.ui.dialog.as_mut().unwrap().fields.insert("color".into(), json!("#00ff00"));
            crate::dialogs::confirm(&mut app).unwrap();
            let d = app.ui.dialog.as_ref().unwrap();
            let mut pair = [hex(kept); 2];
            pair[index] = hex("#00ff00");
            assert_eq!((d.kind.as_str(), dialog_grid(d)), (super::KIND, pair.to_vec()), "{field}");
            crate::dialogs::confirm(&mut app).unwrap();
            assert_eq!(app.session.active().unwrap().doc.setup.grid_colors, pair, "{field}");
        }
    }

    /// With a preset name in Document Setup's `gridColors` field, the grid color boxes show the
    /// preset's colors, and the Color Picker opens on the clicked one; its OK sets that color and
    /// keeps the preset's other one.
    #[test]
    fn a_grid_color_box_opens_the_picker_when_the_field_names_a_preset() {
        let (mut app, ctx) = transparency_tab();
        app.ui.dialog.as_mut().unwrap().fields.insert("gridColors".into(), json!("blue"));
        dialog_frame(&mut app, &ctx, vec![]);
        click_grid_box(&mut app, &ctx, 1);
        assert_picker_over_setup(&app, "#c4dcff", "blue");
        app.ui.dialog.as_mut().unwrap().fields.insert("color".into(), json!("#00ff00"));
        crate::dialogs::confirm(&mut app).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.kind.as_str(), dialog_grid(d)), (super::KIND, ["#ffffff", "#00ff00"].map(hex).to_vec()));
    }

    /// The screen color sampler works in the Color Picker of a grid color: the color sampled goes
    /// to that picker, and its OK sets the grid color.
    #[test]
    fn the_grid_color_picker_takes_a_screen_sampled_color() {
        let (mut app, ctx) = transparency_tab();
        let (tx, rx) = std::sync::mpsc::channel::<crate::screen_color::Answer>();
        let mut rx = Some(rx);
        app.services.screen_color = Some(Box::new(move || rx.take().ok_or_else(|| "shown twice".to_string())));
        click_grid_box(&mut app, &ctx, 0);
        assert_picker_over_setup(&app, LIGHT[0], "before sampling");
        app.run("ui.sampleScreenColor", json!({"picker": true})).unwrap();
        tx.send(Ok(Some([0.0, 0.5, 1.0]))).unwrap();
        crate::screen_color::poll(&mut app, &ctx);
        let sampled = app.ui.dialog.as_ref().unwrap().fields.get("color").and_then(vectorcraft_engine::cmd::color_value).unwrap();
        assert_ne!(sampled.to_hex(), LIGHT[0]);
        crate::dialogs::confirm(&mut app).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        let grid: Vec<String> = dialog_grid(d).iter().map(Color::to_hex).collect();
        assert_eq!((d.kind.as_str(), grid), (super::KIND, vec![sampled.to_hex(), LIGHT[1].to_string()]));
    }

    /// Clicks the hex field that shows `digits` and types `typed` over its text, leaving the edit
    /// pending; returns the field's id.
    fn type_into_hex_field(app: &mut VectorcraftApp, ctx: &egui::Context, digits: &str, typed: &str) -> egui::Id {
        let texts = dialog_frame(app, ctx, vec![]);
        let at = texts.iter().find(|(t, _)| t == digits).unwrap_or_else(|| panic!("no field {digits}: {texts:?}")).1.center();
        let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        dialog_frame(app, ctx, vec![Event::PointerMoved(at), button(true)]);
        dialog_frame(app, ctx, vec![button(false)]);
        let field = ctx.memory(|m| m.focused()).unwrap_or_else(|| panic!("field {digits} has no focus"));
        dialog_frame(app, ctx, vec![Event::Text(typed.into())]);
        assert_eq!(ctx.memory(|m| m.focused()), Some(field), "the edit in {digits} is pending");
        field
    }

    /// A click on a grid color's box whose press and release come in one frame, while that
    /// color's hex field holds an edit, opens the Color Picker on the edited color: the press
    /// commits the edit before the picker opens. The picker's unchanged OK keeps the edit, and the
    /// other color stays as it was.
    #[test]
    fn a_grid_color_box_clicked_during_a_hex_edit_opens_the_picker_on_the_edit() {
        for index in [0, 1] {
            let (mut app, ctx) = transparency_tab();
            type_into_hex_field(&mut app, &ctx, LIGHT[index].trim_start_matches('#'), "123456");
            let at = ctx.read_response(super::grid_color_box_id(index)).unwrap_or_else(|| panic!("no box {index}")).rect.center();
            let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
            dialog_frame(&mut app, &ctx, vec![Event::PointerMoved(at), button(true), button(false)]);
            let case = format!("box {index}");
            assert_picker_over_setup(&app, "#123456", &case);
            let mut pair = LIGHT;
            pair[index] = "#123456";
            assert_eq!(dialog_grid(&crate::dialogs::parent(app.ui.dialog.as_ref().unwrap()).unwrap()), pair.map(hex).to_vec(), "{case}");
            crate::dialogs::confirm(&mut app).unwrap();
            let d = app.ui.dialog.as_ref().unwrap();
            assert_eq!((d.kind.as_str(), dialog_grid(d)), (super::KIND, pair.map(hex).to_vec()), "{case}");
        }
    }

    /// A hex field's edit that commits in the frame a Grid Colors preset is chosen replaces only
    /// its own color in the preset's pair: the other color is the preset's. The test gives the
    /// field the focus while the Grid Colors menu is open, then chooses a preset with a click whose
    /// press and release come in one frame.
    #[test]
    fn a_hex_edit_committed_with_a_preset_choice_keeps_the_presets_other_color() {
        let field = {
            let (mut app, ctx) = transparency_tab();
            type_into_hex_field(&mut app, &ctx, "ffffff", "123456")
        };
        let (mut app, ctx) = transparency_tab();
        let click = |app: &mut VectorcraftApp, label: &str, frames: usize| {
            let texts = dialog_frame(app, &ctx, vec![]);
            let at = texts.iter().find(|(t, _)| t == label).unwrap_or_else(|| panic!("no {label}: {texts:?}")).1.center();
            let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
            if frames == 1 {
                dialog_frame(app, &ctx, vec![Event::PointerMoved(at), button(true), button(false)]);
            } else {
                dialog_frame(app, &ctx, vec![Event::PointerMoved(at), button(true)]);
                dialog_frame(app, &ctx, vec![button(false)]);
            }
        };
        click(&mut app, "Light", 2);
        ctx.memory_mut(|m| m.request_focus(field));
        dialog_frame(&mut app, &ctx, vec![]);
        let select_all = Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND };
        dialog_frame(&mut app, &ctx, vec![select_all, Event::Text("123456".into())]);
        assert_eq!(ctx.memory(|m| m.focused()), Some(field), "the edit is pending");
        let texts: Vec<String> = dialog_frame(&mut app, &ctx, vec![]).into_iter().map(|(t, _)| t).collect();
        assert!(texts.iter().any(|t| t == "123456") && texts.iter().any(|t| t == "Blue"), "{texts:?}");
        click(&mut app, "Blue", 1);
        assert_eq!(dialog_grid(app.ui.dialog.as_ref().unwrap()), ["#123456", "#c4dcff"].map(hex).to_vec());
    }

    /// The picker's OK sets nothing when the dialog under it has no such color or is gone: it is
    /// an error, and the picker stays open.
    #[test]
    fn grid_color_picker_without_its_color_or_its_dialog_is_an_error() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("file.documentSetup", json!({})).unwrap();
        let setup = app.ui.dialog.clone().unwrap();
        let mut short = setup.clone();
        short.fields.insert("gridColors".into(), json!(["#ffffff"]));
        let mut orphan = crate::dialogs::color_picker::over_list_entry(&setup, "gridColors", 0, Color::WHITE, None);
        orphan.fields.remove(crate::dialogs::PARENT);
        for picker in [crate::dialogs::color_picker::over_list_entry(&short, "gridColors", 1, Color::WHITE, None), orphan] {
            app.ui.dialog = Some(picker);
            app.ui.dialog.as_mut().unwrap().fields.insert("color".into(), json!("#ff0000"));
            assert!(crate::dialogs::confirm(&mut app).is_err());
            assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some("colorPicker"));
        }
        assert_eq!(app.session.active().unwrap().doc.setup.grid_colors, LIGHT.map(hex));
        assert_eq!(undo_steps(&app), 0);
    }

    /// [`transparency_tab`] on a document whose grid colors `document.setup` set to `grid` first.
    fn transparency_tab_on(grid: Value) -> (VectorcraftApp, egui::Context) {
        let (mut app, ctx) = transparency_tab();
        app.ui.dialog = None;
        app.run("document.setup", json!({"gridColors": grid})).unwrap();
        app.run("file.documentSetup", json!({})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("tab".into(), json!("Transparency"));
        for _ in 0..4 {
            dialog_frame(&mut app, &ctx, vec![]);
        }
        (app, ctx)
    }

    /// Document Setup opens on the document's own grid colors, a Lab color included: the grid
    /// preview reads them as the document holds them, and an unchanged OK keeps them.
    #[test]
    fn document_setup_opens_on_the_documents_own_grid_colors() {
        let (mut app, _ctx) = transparency_tab_on(json!(["lab 55 60 40", "#cccccc"]));
        let grid = app.session.active().unwrap().doc.setup.grid_colors;
        assert!(matches!(grid[0], Color::Lab { .. }), "{grid:?}");
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!(super::preview_grid(&d.fields, &app.session.active().unwrap().doc.setup).1, grid);
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(app.session.active().unwrap().doc.setup.grid_colors, grid);
    }

    /// A hex field's edit replaces only its own entry of the pair Document Setup holds: the other
    /// entry stays as it is, a Lab color in its own model.
    #[test]
    fn a_grid_color_hex_edit_keeps_the_other_entry_as_it_is() {
        let (mut app, ctx) = transparency_tab();
        app.ui.dialog.as_mut().unwrap().fields.insert("gridColors".into(), json!(["lab 55 60 40", "#cccccc"]));
        type_into_hex_field(&mut app, &ctx, "cccccc", "123456");
        // A click on the grid preview ends the edit.
        let at = ctx.read_response(super::grid_preview_id()).expect("no grid preview").rect.center();
        let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        dialog_frame(&mut app, &ctx, vec![Event::PointerMoved(at), button(true)]);
        dialog_frame(&mut app, &ctx, vec![button(false)]);
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields["gridColors"], json!(["lab 55 60 40", "#123456"]));
    }

    /// The Grid Colors menu names a preset only when the pair is that preset's colors, as
    /// `document.setup` names it: a color that rounds to the preset's hex code makes it Custom.
    #[test]
    fn the_grid_colors_menu_names_a_preset_only_for_its_own_colors() {
        let (mut app, ctx) = transparency_tab();
        app.ui.dialog.as_mut().unwrap().fields.insert("gridColors".into(), json!(["#ffffff", [0.801, 0.8, 0.8]]));
        dialog_frame(&mut app, &ctx, vec![]);
        let texts: Vec<String> = dialog_frame(&mut app, &ctx, vec![]).into_iter().map(|(t, _)| t).collect();
        assert!(texts.iter().any(|t| t == "Custom") && !texts.iter().any(|t| t == "Light"), "{texts:?}");
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(app.session.active().unwrap().doc.setup.grid_colors_name(), "Custom");
    }

    /// With a list in Grid Colors that is not two colors, Cancel in a grid color's Color Picker
    /// goes back to that list as it was, and Document Setup's OK still refuses it.
    #[test]
    fn grid_color_picker_cancel_keeps_a_short_list() {
        let (mut app, ctx) = transparency_tab();
        app.ui.dialog.as_mut().unwrap().fields.insert("gridColors".into(), json!(["#ff0000"]));
        dialog_frame(&mut app, &ctx, vec![]);
        click_grid_box(&mut app, &ctx, 1);
        assert_picker_over_setup(&app, LIGHT[1], "short list");
        crate::dialogs::cancel(&mut app);
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.kind.as_str(), &d.fields["gridColors"]), (super::KIND, &json!(["#ff0000"])));
        assert!(crate::dialogs::confirm(&mut app).is_err());
        assert_eq!(app.session.active().unwrap().doc.setup.grid_colors, LIGHT.map(hex));
    }

    /// The grid preview Document Setup draws, in a fresh context: its rect, the texture
    /// coordinates of its bottom-right corner (two cells per unit) and the tile's four pixels.
    fn grid_preview(app: &mut VectorcraftApp) -> (Rect, egui::Vec2, Vec<egui::Color32>) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, Default::default());
        let (mut mesh, mut pixels) = (None, None);
        for _ in 0..3 {
            let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 900.0))), ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| crate::dialogs::show(app, ui.ctx()));
            let tiles: Vec<egui::TextureId> =
                ctx.tex_manager().read().allocated().filter(|(_, m)| m.name == crate::canvas::TRANSPARENCY_GRID).map(|(id, _)| *id).collect();
            for (id, deltas) in &out.textures_delta.set {
                if tiles.contains(id)
                    && let Some(d) = deltas.last()
                {
                    let egui::ImageData::Color(img) = &d.image;
                    pixels = Some(img.pixels.clone());
                }
            }
            out.textures_delta.clear();
            mesh = out.shapes.iter().find_map(|c| match &c.shape {
                egui::Shape::Mesh(m) if tiles.contains(&m.texture_id) => Some(m.clone()),
                _ => None,
            });
        }
        let mesh = mesh.unwrap_or_else(|| panic!("no grid preview"));
        let uv = mesh.vertices.iter().fold(vec2(0.0, 0.0), |a, v| a.max(v.uv.to_vec2()));
        (mesh.calc_bounds(), uv, pixels.unwrap_or_else(|| panic!("no tile")))
    }

    /// The tile's pixels for the grid colors `a` and `b`: `a` in the top-left and bottom-right cells.
    fn tile(a: &str, b: &str) -> Vec<egui::Color32> {
        let c = |h| crate::panels::c32(&hex(h));
        vec![c(a), c(b), c(b), c(a)]
    }

    /// The Transparency tab shows a checkered preview of the grid its Grid Size and Grid Colors
    /// hold before OK: the canvas's cells, the first color in the top-left cell. Simulate Colored
    /// Paper leaves it checkered, and neither it nor Cancel changes the document.
    #[test]
    fn the_transparency_tab_previews_the_pending_grid_size_and_colors() {
        let (mut app, _) = transparency_tab();
        let (rect, uv, pixels) = grid_preview(&mut app);
        assert_eq!(rect.size(), super::GRID_PREVIEW);
        assert_eq!((uv, pixels), (rect.size() / 16.0, tile(LIGHT[0], LIGHT[1])), "Medium, Light");
        for (size, cell) in [("small", 4.0), ("large", 16.0)] {
            let d = app.ui.dialog.as_mut().unwrap();
            d.fields.insert("gridSize".into(), json!(size));
            d.fields.insert("gridColors".into(), json!(["#ff0000", "#0000ff"]));
            let (rect, uv, pixels) = grid_preview(&mut app);
            assert_eq!((uv, pixels), (rect.size() / (2.0 * cell), tile("#ff0000", "#0000ff")), "{size}");
        }
        app.ui.dialog.as_mut().unwrap().fields.insert("simulatePaper".into(), json!(true));
        let (rect, uv, pixels) = grid_preview(&mut app);
        assert_eq!((uv, pixels), (rect.size() / 32.0, tile("#ff0000", "#0000ff")), "Simulate Colored Paper");
        let st = app.session.active().unwrap();
        assert_eq!((st.doc.setup.grid_size, st.doc.setup.grid_colors), (vectorcraft_doc::GridSize::Medium, LIGHT.map(hex)));
        assert_eq!(undo_steps(&app), 0);
        crate::dialogs::cancel(&mut app);
        assert!(app.ui.dialog.is_none());
        let st = app.session.active().unwrap();
        assert_eq!(
            (st.doc.setup.grid_size, st.doc.setup.grid_colors, st.doc.setup.simulate_paper),
            (vectorcraft_doc::GridSize::Medium, LIGHT.map(hex), false)
        );
        assert_eq!(undo_steps(&app), 0);
    }

    /// The grid preview sits beside Grid Size, Grid Colors and the color boxes, inside the dialog,
    /// and covers none of the tab's text or color boxes.
    #[test]
    fn the_grid_preview_sits_beside_the_grid_rows() {
        let (mut app, ctx) = transparency_tab();
        let texts = dialog_frame(&mut app, &ctx, vec![]);
        let (rect, _, _) = grid_preview(&mut app);
        let label = texts.iter().find(|(t, _)| t == "Grid Size:").unwrap_or_else(|| panic!("no Grid Size: {texts:?}")).1;
        assert!(rect.top() <= label.bottom() && rect.left() > label.right(), "{rect:?} beside {label:?}");
        for (text, r) in &texts {
            assert!(!r.intersects(rect), "{text:?} at {r:?} under the preview {rect:?}");
        }
        for i in 0..2 {
            let r = ctx.read_response(super::grid_color_box_id(i)).unwrap().rect;
            assert!(!r.intersects(rect), "box {i} at {r:?} under the preview {rect:?}");
        }
        // The dialog's content is 540 points wide from its heading's left edge.
        let left = texts.iter().find(|(t, _)| t == "Document Setup").unwrap_or_else(|| panic!("no heading: {texts:?}")).1.left();
        assert!(rect.right() <= left + 540.0 + 0.5, "{rect:?} inside the dialog from {left}");
    }

    /// A grid size or color pair the dialog holds that is not a valid value shows as the
    /// document's setting.
    #[test]
    fn the_grid_preview_shows_the_document_setting_for_a_bad_field() {
        let setup = vectorcraft_doc::DocSetup {
            grid_size: vectorcraft_doc::GridSize::Large,
            grid_colors: [hex("#111111"), hex("#222222")],
            ..Default::default()
        };
        let fields = |v: serde_json::Value| v.as_object().unwrap().clone();
        for bad in [
            json!({}),
            json!({"gridSize": "huge", "gridColors": ["#ff0000"]}),
            json!({"gridSize": 3, "gridColors": ["#ff0000", "zz"]}),
            json!({"gridColors": "Plaid"}),
            json!({"gridColors": ["#ff0000", "#00ff00", "#0000ff"]}),
        ] {
            assert_eq!(super::preview_grid(&fields(bad.clone()), &setup), (setup.grid_size, setup.grid_colors), "{bad}");
        }
        let good = fields(json!({"gridSize": "small", "gridColors": ["#ff0000", "#0000ff"]}));
        assert_eq!(super::preview_grid(&good, &setup), (vectorcraft_doc::GridSize::Small, [hex("#ff0000"), hex("#0000ff")]));
        let preset = fields(json!({"gridColors": "blue"}));
        assert_eq!(super::preview_grid(&preset, &setup), (vectorcraft_doc::GridSize::Large, [hex("#ffffff"), hex("#c4dcff")]));
    }
}
