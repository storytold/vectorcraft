//! File → Document Setup: General (units, bleed, Edit Artboards, outline images, the substituted
//! glyphs highlight), Transparency (grid size and colours, simulated paper, flattener preset, white
//! overprint), Type (language, quotes, superscript/subscript/small caps, text export) and Color
//! (document color mode and assigned profiles) tabs, as a hosted properties dialog.
//!
//! Fields: what `document.setup` reports (and takes back, with `quotes`, `superscript` and
//! `subscript` as `quotes.double`-style keys, one per row), plus `bleedLinked` and the Color tab's
//! `colorMode`, `convertColors`, `rgbProfile` and `cmykProfile` (the working-space entry: none
//! assigned). The shown tab is the dialog's own (`tab`). OK runs `document.setup` with them: one
//! undo step. When the Color tab changed something, OK runs `document.setup`, `edit.assignProfile`
//! and `file.documentColorMode` in one `command.batch`, and the document's changes are still one
//! undo step. On an error the dialog stays open with the reason. When OK raises the bleed on a
//! side, the status bar counts the objects that reach an artboard's trim edge on a side OK raised
//! but stop short of the new bleed there.

use std::sync::Arc;

use craft::behavior::Mixed;
use craft::color_ui::display::DisplayTransform;
use craft::theme::{Rgba, Srgba};
use craft::ui::buttons::Button;
use craft::ui::choice::Swatch;
use craft::ui::contract::{Bind, Entry, Show as _, ShowContainer as _, clicked};
use craft::ui::dialog::dialog_id;
use craft::ui::display::Picture;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::ThumbAspect;
use craft::ui::patterns::form_dialogs::{
    CustomContext, DialogFields, FieldValue, HostedPropertiesDialog, Param, ParamKind, PropertiesDialog, PropertiesTab, PropertyRow,
};
use craft::ui::pickers::Select;
use craft::ui::primitives::Checker;
use craft::ui::runtime::env::{LengthUnits, UnitKind};
use craft::ui::style::DataColor;
use craft::ui::text::{HexField, LinkMode, LinkedFields, LinkedLayout};
use serde_json::{Map, Value, json};
use vectorcraft_color::Color;
use vectorcraft_color::cms::ProfileKind;
use vectorcraft_doc::setup::{GRID_COLOR_PRESETS, LANGUAGES, MAX_BLEED, bleed_in_range, language_quotes};
use vectorcraft_doc::{ColorMode, DocSetup, GridSize, Quotes, Unit};
use vectorcraft_engine::cmd::colormgmt::doc_profiles;
use vectorcraft_engine::cmd::fileio::pdf;

use crate::VectorcraftApp;

pub(super) const KIND: &str = "documentSetup";
const TABS: [&str; 4] = ["General", "Transparency", "Type", "Color"];
/// Dialog-only fields `document.setup` doesn't take; `gridColors.N` (a picked colour waiting to be
/// folded into the pair) is dialog-only too.
const UI_ONLY: [&str; 7] =
    ["bleedLinked", "colorMode", "convertColors", "rgbProfile", "cmykProfile", "editArtboards", "numbersWithoutUnitsArePoints"];
/// The settings `document.setup` keeps as an object, which the rows edit as `key.part`.
const PARTS: [(&str, [&str; 2]); 3] = [("quotes", ["double", "single"]), ("superscript", ["size", "position"]), ("subscript", ["size", "position"])];
/// The keys a picked grid colour arrives under: the entry's index is the key's last character.
const GRID_KEYS: [&str; 2] = ["gridColors.0", "gridColors.1"];
/// The profile rows' entry for no assigned profile: the document uses the working space.
const WORKING_RGB: &str = "Working RGB (don't tag)";
const WORKING_CMYK: &str = "Working CMYK (don't tag)";

/// The transform the grid preview shows its colours through.
type Display = Option<Arc<DisplayTransform>>;

/// Open Document Setup on the active document's settings.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let mut fields = app.session.execute("document.setup", &json!({})).map_err(|e| e.to_string())?;
    let linked = fields["bleed"].as_array().is_some_and(|b| b.windows(2).all(|w| w[0] == w[1]));
    fields["bleedLinked"] = json!(linked);
    fields["numbersWithoutUnitsArePoints"] = json!(app.session.prefs.numbers_without_units_are_points);
    let Some(st) = app.session.active() else { return Err("no document".into()) };
    fields["gridColors"] = json!(st.doc.setup.grid_colors.map(|c| grid_color_entry(&c)));
    let (rgb, cmyk) = doc_profiles(&st.doc);
    fields["colorMode"] = json!(mode_id(st.doc.color_mode));
    fields["rgbProfile"] = json!(rgb.unwrap_or_else(|| WORKING_RGB.into()));
    fields["cmykProfile"] = json!(cmyk.unwrap_or_else(|| WORKING_CMYK.into()));
    fields["convertColors"] = json!(true);
    let (mode, setup) = (st.doc.color_mode, st.doc.setup.clone());
    let Value::Object(map) = fields else { return Err("document.setup reported no settings".into()) };
    let presets: Vec<String> = app.session.flattener_presets().into_iter().map(|p| p.name).collect();
    let display = app.display_profile.transform(app.session.prefs.monitor_profile_display);
    let tabs = tabs(mode, setup, presets, display);
    let dialog = HostedPropertiesDialog::new(KIND, dialog_fields(flatten(map)), move |values| {
        PropertiesDialog::new(KIND, tl!("Document Setup"), tabs.clone(), values)
    });
    super::host::open_held(app, dialog, |app, fields| confirm(app, fields).map(|_| ())).ok_or("no window to show the dialog in")?;
    Ok(Value::Null)
}

/// The `mode` param of `file.documentColorMode` for `m`.
fn mode_id(m: ColorMode) -> &'static str {
    match m {
        ColorMode::Rgb => "rgb",
        ColorMode::Cmyk => "cmyk",
    }
}

/// `fields` with each object of [`PARTS`] as one `key.part` entry per part.
fn flatten(mut fields: Map<String, Value>) -> Map<String, Value> {
    for (key, parts) in PARTS {
        if let Some(Value::Object(object)) = fields.remove(key) {
            for part in parts {
                if let Some(v) = object.get(part) {
                    fields.insert(format!("{key}.{part}"), v.clone());
                }
            }
        }
    }
    fields
}

/// The inverse of [`flatten`].
fn nest(mut fields: Map<String, Value>) -> Map<String, Value> {
    for (key, parts) in PARTS {
        let object: Map<String, Value> =
            parts.iter().filter_map(|part| fields.remove(&format!("{key}.{part}")).map(|v| ((*part).to_string(), v))).collect();
        if !object.is_empty() {
            fields.insert(key.into(), Value::Object(object));
        }
    }
    fields
}

/// The dialog value a JSON value is.
fn field_value(v: Value) -> FieldValue {
    serde_json::from_value(v.clone()).unwrap_or(FieldValue::Json(v))
}

fn dialog_fields(map: Map<String, Value>) -> DialogFields {
    DialogFields::from_pairs(map.into_iter().map(|(k, v)| (k, field_value(v))))
}

/// The dialog's values as the JSON object `document.setup` and the helpers below read.
fn json_map(fields: &DialogFields) -> Map<String, Value> {
    match serde_json::to_value(fields) {
        Ok(Value::Object(map)) => map,
        _ => Map::new(),
    }
}

/// Whether `document.setup` does not take the field `key`.
fn is_ui_only(key: &str) -> bool {
    UI_ONLY.contains(&key) || GRID_KEYS.contains(&key)
}

/// The commands the Color tab's changes run, as `command.batch` steps, in the order OK runs them:
/// Edit › Assign Profile first, so that a mode change converts through the profiles it assigns,
/// then File › Document Color Mode. Empty when the tab matches the active document. A field the
/// dialog holds with a value of the wrong kind is an error, and then nothing runs; an omitted
/// field leaves its setting as it is (`convertColors`: converts).
fn color_commands(app: &VectorcraftApp, fields: &Map<String, Value>) -> Result<Vec<Value>, String> {
    let Some(st) = app.session.active() else { return Ok(vec![]) };
    let mut out = vec![];
    let (rgb, cmyk) = doc_profiles(&st.doc);
    let mut assign = Map::new();
    for (key, param, current, working) in [("rgbProfile", "rgb", rgb, WORKING_RGB), ("cmykProfile", "cmyk", cmyk, WORKING_CMYK)] {
        let chosen = match fields.get(key) {
            None => continue,
            Some(Value::Null) => None,
            Some(Value::String(n)) if n == working => None,
            Some(Value::String(n)) => Some(n.clone()),
            Some(_) => return Err(format!("`{key}` must be a profile name or null")),
        };
        if chosen != current {
            assign.insert(param.into(), json!(chosen));
        }
    }
    let mode = match fields.get("colorMode") {
        None => None,
        Some(Value::String(m)) if m.eq_ignore_ascii_case("rgb") => Some(ColorMode::Rgb),
        Some(Value::String(m)) if m.eq_ignore_ascii_case("cmyk") => Some(ColorMode::Cmyk),
        Some(_) => return Err("`colorMode` must be one of rgb, cmyk".into()),
    };
    let key = "convertColors";
    let convert = match fields.get(key) {
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

fn confirm(app: &mut VectorcraftApp, fields: &DialogFields) -> Result<Value, String> {
    let map = json_map(fields);
    let params = nest(map.iter().filter(|(k, _)| !is_ui_only(k)).map(|(k, v)| (k.clone(), v.clone())).collect());
    let color = color_commands(app, &map)?;
    let before = app.session.active().map(|st| st.doc.setup.bleed);
    let r = if color.is_empty() {
        app.run("document.setup", Value::Object(params))
    } else {
        let commands: Vec<Value> = std::iter::once(json!({"command": "document.setup", "params": params})).chain(color).collect();
        app.run("command.batch", json!({"label": "Document Setup", "commands": commands}))
    };
    if r.is_ok()
        && let Some(note) = before.and_then(|b| short_of_bleed_note(app, b))
    {
        app.status(note);
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

/// Runs Edit Artboards when the open dialog asked for it this frame: it applies the settings, then
/// edits the artboards with the Artboard tool and closes the dialog; a refusal keeps the dialog
/// open. OK is the held OK [`open`] gives the dialog.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(dialogs) = app.dialogs() else { return };
    let id = dialog_id(KIND);
    let edit = dialogs.with(|host| {
        host.get_by_id::<HostedPropertiesDialog>(id).filter(|d| d.fields().bool("editArtboards") == Some(true)).map(|d| d.fields().clone())
    });
    if let Some(fields) = edit {
        match confirm(app, &fields) {
            Ok(_) => {
                app.select_tool("artboard");
                // The held dialog's record takes the result as a cancel: the settings are applied already.
                dialogs.with(|host| host.cancel_by_id(ctx, id));
            }
            Err(e) => {
                app.status(e);
                // Fails only when the dialog is gone, and then there is no flag left to clear.
                let _ = dialogs.with(|host| host.set_json_field(ctx, id, "editArtboards", json!(false)));
            }
        }
    }
}

/// The four tabs. `setup` is the document's setting, which a grid field that holds no valid value
/// falls back to; `mode` is the document's color mode.
fn tabs(mode: ColorMode, setup: DocSetup, presets: Vec<String>, display: Display) -> Vec<PropertiesTab> {
    vec![
        PropertiesTab::new(tl!("General"), general()).key(TABS[0]),
        PropertiesTab::new(tl!("Transparency"), transparency(setup, presets, display)).key(TABS[1]),
        PropertiesTab::new(tl!("Type"), typography()).key(TABS[2]),
        PropertiesTab::new(tl!("Color"), color(mode)).key(TABS[3]),
    ]
}

/// A dropdown row over `options`, (value, label).
fn choice(key: &str, label: &'static str, options: impl IntoIterator<Item = (String, String)>) -> PropertyRow {
    let options = options.into_iter().map(|(id, text)| (id, text.into())).collect();
    PropertyRow::field(Param::new(key, label, ParamKind::choice(options, None)))
}

fn check(key: &str, label: &'static str) -> PropertyRow {
    PropertyRow::field(Param::new(key, label, ParamKind::bool(false)))
}

fn section(label: &'static str) -> PropertyRow {
    PropertyRow::field(Param::section(label))
}

/// A percent field.
fn percent(key: &str, label: &'static str, default: f64) -> PropertyRow {
    PropertyRow::field(Param::number(key, label, default, 1.0, 1).unit(UnitKind::Percent))
}

/// The framework's unit for the document unit `unit`.
fn unit_kind(unit: Unit) -> UnitKind {
    match unit {
        Unit::Points => UnitKind::Points,
        Unit::Picas => UnitKind::Picas,
        Unit::Inches => UnitKind::Inches,
        Unit::Millimeters => UnitKind::Millimetres,
        Unit::Centimeters => UnitKind::Centimetres,
        Unit::Pixels => UnitKind::Pixels,
        Unit::FeetInches | Unit::Feet => UnitKind::Feet,
        Unit::Meters => UnitKind::Metres,
        Unit::Yards => UnitKind::Yards,
        Unit::Q => UnitKind::Q,
        Unit::Ha => UnitKind::Ha,
    }
}

/// The General tab (units, bleed, artboard editing, options).
fn general() -> Vec<PropertyRow> {
    let units = Unit::ALL.iter().map(|u| (u.label().to_string(), u.label().to_string()));
    vec![
        choice("units", tl!("Units:"), units),
        PropertyRow::field(Param::custom("bleed", tl!("Bleed:"), bleed_row).companions(["bleedLinked"])),
        PropertyRow::field(Param::custom("editArtboards", "", edit_artboards_row)),
        check("outlineImages", tl!("Show Images in Outline Mode")),
        check("highlightSubstitutedGlyphs", tl!("Highlight Substituted Glyphs")),
    ]
}

/// The four bleed sides in the document's unit, linked to one value while `bleedLinked`.
fn bleed_row(ui: &mut egui::Ui, fields: &mut DialogFields, _: &mut CustomContext) -> egui::Response {
    let mut linked = fields.bool("bleedLinked").unwrap_or(true);
    let unit = Unit::named(fields.text("units").unwrap_or_default()).unwrap_or_default();
    let bare_points = fields.bool("numbersWithoutUnitsArePoints").unwrap_or(true);
    let lengths = crate::fields::bare_points(LengthUnits::new(UnitKind::Points, unit_kind(unit), 72.0), bare_points);
    let parts = bleed_of(fields).map(|b| Mixed::Same(Some(b)));
    // PLAN: assumed the linked fields' output reads as `out.edit.value()`, as the shape dialog uses it.
    let out = LinkedFields::<4>::new(tl!("Bleed:"), Bind::given(parts), Bind::bound(&mut linked))
        .mode(LinkMode::Equal)
        .layout(LinkedLayout::Columns)
        .captions([tl!("Top"), tl!("Bottom"), tl!("Left"), tl!("Right")])
        .range(0.0..=MAX_BLEED)
        .lengths(lengths)
        .show(ui);
    if let Some([Mixed::Same(Some(t)), Mixed::Same(Some(b)), Mixed::Same(Some(l)), Mixed::Same(Some(r))]) = out.edit.value().copied() {
        fields.set("bleed", FieldValue::List([t, b, l, r].map(FieldValue::Number).to_vec()));
    }
    if fields.bool("bleedLinked") != Some(linked) {
        fields.set("bleedLinked", linked);
    }
    ui.response()
}

/// The four bleed sides the dialog holds; a field that is not four numbers shows as none.
fn bleed_of(fields: &DialogFields) -> [f64; 4] {
    let numbers: Vec<f64> = fields
        .list("bleed")
        .iter()
        .flat_map(|l| l.iter())
        .filter_map(|v| match v {
            FieldValue::Number(n) => Some(*n),
            FieldValue::Int(i) => Some(*i as f64),
            _ => None,
        })
        .collect();
    <[f64; 4]>::try_from(numbers).unwrap_or([0.0; 4])
}

/// Edit Artboards: asks the pump to apply the settings and edit the artboards with the Artboard tool.
fn edit_artboards_row(ui: &mut egui::Ui, fields: &mut DialogFields, _: &mut CustomContext) -> egui::Response {
    let button = Button::new(tl!("Edit Artboards")).show(ui).response;
    if clicked(&button) {
        fields.set("editArtboards", true);
    }
    button
}

/// The Transparency tab: the grid, its preview beside it, paper simulation and the flattener.
fn transparency(setup: DocSetup, presets: Vec<String>, display: Display) -> Vec<PropertyRow> {
    let sizes = GridSize::ALL.iter().map(|g| (g.id().to_string(), g.label().to_string()));
    vec![
        section(tl!("Transparency Grid")),
        choice("gridSize", tl!("Grid Size:"), sizes),
        PropertyRow::field(grid_colors_row(setup.clone()).companions(GRID_KEYS)),
        PropertyRow::field(grid_preview_row(setup, display)),
        check("simulatePaper", tl!("Simulate Colored Paper")),
        section(tl!("Export and Clipboard Transparency Flattener Settings")),
        choice("flattenerPreset", tl!("Preset:"), presets.into_iter().map(|p| (p.clone(), p))),
        check("discardWhiteOverprint", tl!("Discard White Overprint in Output")),
    ]
}

/// The name Grid Colors shows for `pair`: the preset whose colors the pair is, as `document.setup`
/// names it, else Custom.
fn grid_preset_name(pair: [Color; 2]) -> &'static str {
    GRID_COLOR_PRESETS.iter().find(|(_, p)| *p == pair).map_or("Custom", |(n, _)| n)
}

/// The Grid Colors menu and the two colors' swatches and hex fields. A preset name in the fields
/// (any case, as `document.setup` takes it) becomes the preset's two colors, which the hex fields
/// and the Color Picker edit one at a time; editing either makes them Custom.
fn grid_colors_row(setup: DocSetup) -> Param {
    Param::custom("gridColors", tl!("Grid Colors:"), move |ui, fields, cx| {
        fold_picked(fields, &setup);
        let preset = json_map(fields)
            .get("gridColors")
            .and_then(Value::as_str)
            .and_then(|n| GRID_COLOR_PRESETS.iter().find(|(p, _)| p.eq_ignore_ascii_case(n)).map(|(_, pair)| *pair));
        if let Some(pair) = preset {
            fields.set("gridColors", entries_value(pair));
        }
        let resolved = grid_colors(&json_map(fields), &setup);
        let menu: Vec<Entry<String>> =
            GRID_COLOR_PRESETS.iter().map(|(n, _)| *n).chain(["Custom"]).map(|n| Entry::new(n.to_string(), n.to_string())).collect();
        let mut picked = None;
        let (mut wells, mut edits) = ([None; 2], [None; 2]);
        let shown = Stack::new().show(ui, |ui, _| {
            picked = Select::new(tl!("Grid Colors:"), Bind::given(grid_preset_name(resolved).to_string()), &menu).show(ui).value().cloned();
            Row::new().show(ui, |ui, _| {
                for (i, color) in resolved.iter().enumerate() {
                    let name = format!("{} {}", tl!("Grid Colors:"), i + 1);
                    let [r, g, b, _] = color.to_rgba8(1.0);
                    let srgb = box_color(color);
                    wells[i] = Swatch::new(name.clone(), srgb).well().show(ui).clicked.then_some(srgb);
                    edits[i] = HexField::hex(name, Bind::given(Rgba::rgb(r, g, b))).show(ui).value().copied();
                }
            });
        });
        if let Some((_, pair)) = picked.and_then(|n| GRID_COLOR_PRESETS.iter().find(|(p, _)| *p == n)) {
            fields.set("gridColors", entries_value(*pair));
        }
        for (i, (well, edit)) in wells.into_iter().zip(edits).enumerate() {
            if let Some(srgb) = well {
                cx.pick_color(GRID_KEYS[i], srgb);
            }
            if let Some(Rgba([r, g, b, _])) = edit {
                set_grid_entry(fields, &setup, i, &Color::rgb8(r, g, b));
            }
        }
        shown.output
    })
}

/// Takes the colors the Color Picker answered into their entries of the pair. When the dialog holds
/// two colors and the answer is the color the entry's box showed (the picker's OK left it
/// unchanged), the entry stays as it is: a Lab or CMYK color keeps its own model, and a hex code
/// stays that hex code.
fn fold_picked(fields: &mut DialogFields, setup: &DocSetup) {
    for (i, key) in GRID_KEYS.iter().enumerate() {
        if let Some([r, g, b]) = fields.color(key) {
            fields.remove(key);
            let map = json_map(fields);
            let shown = grid_colors(&map, setup).get(i).map(box_color).map(|s| [s.r, s.g, s.b]);
            if pending_grid_pair(&map).is_none() || shown != Some([r, g, b]) {
                set_grid_entry(fields, setup, i, &Color::rgb(r, g, b));
            }
        }
    }
}

/// The color grid color `c`'s box shows, which the Color Picker it opens starts on.
fn box_color(c: &Color) -> Srgba {
    let [r, g, b, _] = c.to_rgba8(1.0);
    Srgba::rgb(f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0)
}

/// Sets entry `i` of the dialog's `gridColors` to `c`, the other entry staying as it is. A field
/// that is not two colors becomes the pair the boxes show first.
fn set_grid_entry(fields: &mut DialogFields, setup: &DocSetup, i: usize, c: &Color) {
    let map = json_map(fields);
    let mut pair = pending_grid_pair(&map).unwrap_or_else(|| grid_colors(&map, setup).iter().map(grid_color_entry).collect());
    if let Some(entry) = pair.get_mut(i) {
        *entry = grid_color_entry(c);
    }
    fields.set("gridColors", field_value(Value::Array(pair)));
}

/// `pair` as the dialog's `gridColors` value.
fn entries_value(pair: [Color; 2]) -> FieldValue {
    field_value(Value::Array(pair.iter().map(grid_color_entry).collect()))
}

/// The checkered preview of the grid the dialog's Grid Size and Grid Colors hold, drawn with the
/// canvas's transparency grid cell size: the first color in the top-left cell. It shows the grid
/// whatever Simulate Colored Paper and View › Show Transparency Grid are, and changes nothing in
/// the document.
fn grid_preview_row(setup: DocSetup, display: Display) -> Param {
    Param::custom("gridPreview", "", move |ui, fields, _| {
        let (size, colors) = preview_grid(&json_map(fields), &setup);
        let [light, dark] = colors.map(|c| DataColor(crate::display_profile::color32(display.as_deref(), crate::panels::c32(&c))));
        let checker = Checker::new().size(size.cell()).colors(light, dark);
        // PLAN: the picture takes the default extent; the 112 × 72 pt size wants a sample extent in the framework.
        Picture::painter(tl!("Transparency Grid"), ThumbAspect::new(14, 9), move |cx, rect| cx.checker_with(&checker, rect, egui::CornerRadius::ZERO))
            .show(ui)
    })
    .beside()
}

/// The grid size and the two colors the grid preview shows for the dialog's fields `fields`, read
/// as `document.setup` reads them (a preset name or two colors): a field that is missing or not a
/// valid value shows as `setup`, the document's setting.
fn preview_grid(fields: &Map<String, Value>, setup: &DocSetup) -> (GridSize, [Color; 2]) {
    let size = fields.get("gridSize").and_then(Value::as_str).and_then(GridSize::parse).unwrap_or(setup.grid_size);
    (size, grid_colors(fields, setup))
}

/// The two grid colors the dialog's fields `fields` hold, read as `document.setup` reads
/// `gridColors` (a preset name, in any letter case, or two colors): a field that is missing or not
/// a valid value gives `setup`'s colors, the document's setting.
fn grid_colors(fields: &Map<String, Value>, setup: &DocSetup) -> [Color; 2] {
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
fn grid_color_entry(c: &Color) -> Value {
    let hex = json!(c.to_hex());
    if vectorcraft_engine::cmd::color_value(&hex) == Some(*c) { hex } else { serde_json::to_value(c).unwrap_or(hex) }
}

/// The Type tab: language and quotes, the script sizes and the text export.
fn typography() -> Vec<PropertyRow> {
    let quotes =
        |choices: &[[char; 2]]| -> Vec<(String, String)> { choices.iter().map(|p| p.iter().collect::<String>()).map(|s| (s.clone(), s)).collect() };
    vec![
        PropertyRow::field(Param::custom("language", tl!("Language:"), language_row).companions(["quotes.double", "quotes.single"])),
        choice("quotes.double", tl!("Double Quotes:"), quotes(Quotes::DOUBLE_CHOICES)),
        choice("quotes.single", tl!("Single Quotes:"), quotes(Quotes::SINGLE_CHOICES)),
        check("typographersQuotes", tl!("Use Typographer's Quotes")),
        section(tl!("Options")),
        percent("superscript.size", tl!("Superscript Size:"), 0.0),
        percent("superscript.position", tl!("Superscript Position:"), 0.0),
        percent("subscript.size", tl!("Subscript Size:"), 0.0),
        percent("subscript.position", tl!("Subscript Position:"), 0.0),
        percent("smallCapsSize", tl!("Small Caps:"), 70.0),
        choice(
            "exportText",
            tl!("Export:"),
            [("editable", tl!("Preserve Text Editability")), ("appearance", tl!("Preserve Text Appearance"))]
                .map(|(id, l)| (id.to_string(), l.to_string())),
        ),
    ]
}

/// The language menu: a language brings its quotes, in the pass the menu reports the pick.
fn language_row(ui: &mut egui::Ui, fields: &mut DialogFields, _: &mut CustomContext) -> egui::Response {
    let languages: Vec<Entry<String>> = LANGUAGES.iter().map(|(l, _, _)| Entry::new(l.to_string(), l.to_string())).collect();
    let current = fields.text("language").unwrap_or_default().to_string();
    let picked = Select::new(tl!("Language:"), Bind::given(current), &languages).show(ui).value().cloned();
    if let Some(language) = picked {
        if let Some(q) = language_quotes(&language) {
            fields.set("quotes.double", q.double.iter().collect::<String>());
            fields.set("quotes.single", q.single.iter().collect::<String>());
        }
        fields.set("language", language);
    }
    ui.response()
}

/// The Color tab: the document color mode, whether a mode change converts the colors (unchecked:
/// they keep their numbers), and the RGB and CMYK profiles assigned to the document.
fn color(mode: ColorMode) -> Vec<PropertyRow> {
    let others: Vec<String> = ["rgb", "cmyk"].into_iter().filter(|m| *m != mode_id(mode)).map(String::from).collect();
    let convert = if mode == ColorMode::Rgb { tl!("Convert to CMYK") } else { tl!("Convert to RGB") };
    let mut rows = vec![
        section(tl!("Document Color Mode")),
        choice("colorMode", tl!("Color Mode:"), [("rgb", "RGB Color"), ("cmyk", "CMYK Color")].map(|(a, b)| (a.to_string(), b.to_string()))),
        PropertyRow::field(Param::new("convertColors", convert, ParamKind::bool(true)).enabled_when_in("colorMode", others)),
        section(tl!("Assign Profile")),
    ];
    for (key, label, kind, working) in
        [("rgbProfile", tl!("RGB:"), ProfileKind::Rgb, WORKING_RGB), ("cmykProfile", tl!("CMYK:"), ProfileKind::Cmyk, WORKING_CMYK)]
    {
        rows.push(choice(key, label, crate::panels::separations::profile_entries(kind, Some(working))));
    }
    rows
}

#[cfg(test)]
mod tests {
    use craft::theme::Srgba;
    use craft::ui::dialog::{DialogId, HeldOk, dialog_id};
    use craft::ui::patterns::form_dialogs::{DialogFields, HostedPropertiesDialog};
    use egui::{Pos2, Rect, vec2};
    use serde_json::{Value, json};
    use vectorcraft_color::{Color, Paint, cms};
    use vectorcraft_doc::{ColorMode, NodeId};
    use vectorcraft_engine::Session;

    use crate::VectorcraftApp;

    /// The dialog's id.
    fn id() -> DialogId {
        dialog_id(super::KIND)
    }

    /// An app with a 200 × 200 document and a window to show dialogs in.
    fn new_app() -> (VectorcraftApp, egui::Context) {
        let ctx = crate::testing::ui_context();
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.ui_ctx = Some(ctx.clone());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        (app, ctx)
    }

    /// An app with Document Setup open.
    fn setup_open() -> (VectorcraftApp, egui::Context) {
        let (mut app, ctx) = new_app();
        app.run("file.documentSetup", json!({})).unwrap();
        (app, ctx)
    }

    /// Sets the field `key` as the automation channel does.
    fn set(app: &VectorcraftApp, ctx: &egui::Context, key: &str, value: Value) {
        app.dialogs().unwrap().with(|h| h.set_json_field(ctx, id(), key, value)).unwrap();
    }

    fn fields(app: &VectorcraftApp) -> DialogFields {
        app.dialogs().unwrap().with(|h| h.get_by_id::<HostedPropertiesDialog>(id()).map(|d| d.fields().clone())).unwrap()
    }

    fn is_open(app: &VectorcraftApp) -> bool {
        app.dialogs().unwrap().with(|h| h.get_by_id::<HostedPropertiesDialog>(id()).is_some())
    }

    /// Runs one frame of the dialog host.
    fn frame(app: &VectorcraftApp, ctx: &egui::Context) {
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 900.0))), ..Default::default() };
        let dialogs = app.dialogs().unwrap();
        let mut out = ctx.run_ui(raw, |ui| dialogs.with(|h| h.run(ui)));
        out.textures_delta.clear();
    }

    /// Frames enough for the dialog to show its tab and apply answers.
    fn settle(app: &VectorcraftApp, ctx: &egui::Context) {
        for _ in 0..4 {
            frame(app, ctx);
        }
    }

    /// Presses OK: whether the app accepted it (the dialog then closes).
    fn press_ok(app: &mut VectorcraftApp, ctx: &egui::Context) -> bool {
        let dialogs = app.dialogs().unwrap();
        dialogs.with(|h| h.confirm_by_id(ctx, id()));
        crate::dialogs::host::pump(app, ctx);
        let accepted = dialogs.with(|h| matches!(h.held_state(id()), Some(HeldOk::Accepted)));
        if accepted {
            dialogs.with(|h| h.confirm_by_id(ctx, id()));
        }
        accepted
    }

    fn undo_steps(app: &VectorcraftApp) -> usize {
        app.session.active().unwrap().history.undo.len()
    }

    #[test]
    fn edit_artboards_applies_settings_and_closes_the_dialog() {
        let (mut app, ctx) = setup_open();
        set(&app, &ctx, "bleed", json!([9, 9, 9, 9]));
        set(&app, &ctx, "editArtboards", json!(true));
        super::pump(&mut app, &ctx);
        assert!(!is_open(&app), "Edit Artboards must leave the canvas accessible");
        assert_eq!(app.session.tool_id(), "artboard");
        let st = app.session.active().unwrap();
        assert_eq!(st.doc.setup.bleed, [9.0; 4]);
        assert_eq!(st.history.undo.len(), 1);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.setup.bleed, [0.0; 4]);
    }

    #[test]
    fn edit_artboards_with_invalid_settings_keeps_the_dialog_and_tool() {
        let (mut app, ctx) = setup_open();
        set(&app, &ctx, "bleed", json!(500));
        set(&app, &ctx, "editArtboards", json!(true));
        super::pump(&mut app, &ctx);
        assert!(is_open(&app));
        assert_eq!(app.session.tool_id(), "selection");
        let st = app.session.active().unwrap();
        assert_eq!(st.doc.setup.bleed, [0.0; 4]);
        assert!(st.history.undo.is_empty());
    }

    #[test]
    fn every_tab_draws_and_ok_is_one_undo_step() {
        let (mut app, ctx) = setup_open();
        for tab in ["General", "Transparency", "Type", "Color"] {
            set(&app, &ctx, "tab", json!(tab));
            settle(&app, &ctx);
        }
        let f = fields(&app);
        assert_eq!((f.text("flattenerPreset"), f.bool("bleedLinked")), (Some("Medium Resolution"), Some(true)));
        for (k, v) in [
            ("bleed", json!([9, 9, 9, 9])),
            ("gridColors", json!("Blue")),
            ("language", json!("German")),
            ("quotes.double", json!("„“")),
            ("units", json!("Millimeters")),
        ] {
            set(&app, &ctx, k, v);
        }
        assert!(press_ok(&mut app, &ctx));
        assert!(!is_open(&app));
        let st = app.session.active().unwrap();
        assert_eq!((st.doc.setup.bleed, st.doc.setup.grid_colors_name()), ([9.0; 4], "Blue"));
        assert_eq!((st.doc.setup.quotes.double, st.doc.units), (['„', '“'], vectorcraft_doc::Unit::Millimeters));
        assert_eq!(st.history.undo.len(), 1);
        // A bad value keeps the dialog open.
        app.run("file.documentSetup", json!({})).unwrap();
        set(&app, &ctx, "bleed", json!(500));
        assert!(!press_ok(&mut app, &ctx));
        assert!(is_open(&app));
    }

    /// OK with a larger bleed shows how many objects reach the trim edge but stop short of it; a
    /// smaller bleed, or a larger one that no object falls short of, shows no such note.
    #[test]
    fn ok_with_a_larger_bleed_counts_the_art_short_of_it() {
        let short = |app: &VectorcraftApp| app.ui.status.contains("short of the bleed");
        let ok_with_sides = |app: &mut VectorcraftApp, ctx: &egui::Context, bleed: [f64; 4]| {
            app.status("");
            app.run("file.documentSetup", json!({})).unwrap();
            set(app, ctx, "bleed", json!(bleed));
            assert!(press_ok(app, ctx));
        };
        let (mut app, ctx) = new_app();
        // A background that fills a 4 pt bleed on the top, left and right.
        let id = app.run("shape.rectangle", json!({"x": -4, "y": -4, "width": 208, "height": 50})).unwrap()["id"].clone();
        app.run("paint.setStroke", json!({"ids": [id], "none": true})).unwrap();
        ok_with_sides(&mut app, &ctx, [4.0; 4]);
        assert!(!short(&app), "{}", app.ui.status);
        ok_with_sides(&mut app, &ctx, [9.0; 4]);
        assert!(short(&app), "{}", app.ui.status);
        ok_with_sides(&mut app, &ctx, [6.0; 4]);
        assert!(!short(&app), "{}", app.ui.status);
        // Only the sides whose bleed grows count: a larger bottom bleed alone shows no note, and a
        // larger top bleed does.
        ok_with_sides(&mut app, &ctx, [6.0, 12.0, 6.0, 6.0]);
        assert!(!short(&app), "{}", app.ui.status);
        ok_with_sides(&mut app, &ctx, [12.0, 12.0, 6.0, 6.0]);
        assert!(short(&app), "{}", app.ui.status);
        // Art inside the trim falls short of no bleed.
        let (mut app, ctx) = new_app();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
        ok_with_sides(&mut app, &ctx, [9.0; 4]);
        assert!(!short(&app), "{}", app.ui.status);
        assert_eq!(app.session.active().unwrap().doc.setup.bleed, [9.0; 4]);
        // A bleed that isn't a number, as a file can hold, counts as none before OK, as in the PDF.
        let (mut app, ctx) = new_app();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 200, "height": 50})).unwrap();
        std::sync::Arc::make_mut(&mut app.session.doc_mut().unwrap().doc).setup.bleed = [f64::NAN; 4];
        ok_with_sides(&mut app, &ctx, [9.0; 4]);
        assert!(short(&app), "{}", app.ui.status);
    }

    /// A new document with a red square → the app, its window and the square.
    fn red_square() -> (VectorcraftApp, egui::Context, NodeId) {
        let (mut app, ctx) = new_app();
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
        app.run("paint.setFill", json!({"ids": [id], "color": "#ff0000"})).unwrap();
        (app, ctx, NodeId(id))
    }

    fn fill(app: &VectorcraftApp, id: NodeId) -> Paint {
        app.session.active().unwrap().doc.node(id).unwrap().appearance.fill().unwrap().paint.clone()
    }

    /// Opens Document Setup on the Color tab and sets `values` in it.
    fn color_tab(app: &mut VectorcraftApp, ctx: &egui::Context, values: Value) {
        app.run("file.documentSetup", json!({})).unwrap();
        set(app, ctx, "tab", json!("Color"));
        for (k, v) in values.as_object().unwrap() {
            set(app, ctx, k, v.clone());
        }
    }

    /// OK with a new mode converts the colors as File › Document Color Mode does, together with the
    /// dialog's other changes, in one undo step.
    #[test]
    fn ok_converts_the_color_mode_with_the_other_changes_in_one_undo_step() {
        let (mut app, ctx, id) = red_square();
        let (mut menu, _, menu_id) = red_square();
        menu.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
        let steps = undo_steps(&app);
        color_tab(&mut app, &ctx, json!({"colorMode": "cmyk", "bleed": [9, 9, 9, 9]}));
        assert!(press_ok(&mut app, &ctx));
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
        let (mut app, ctx, id) = red_square();
        let before = fill(&app, id);
        let steps = undo_steps(&app);
        color_tab(&mut app, &ctx, json!({"colorMode": "cmyk", "convertColors": false}));
        assert!(press_ok(&mut app, &ctx));
        assert_eq!(app.session.active().unwrap().doc.color_mode, ColorMode::Cmyk);
        assert_eq!(fill(&app, id), before);
        assert_eq!(undo_steps(&app), steps + 1);
    }

    /// OK assigns the profiles picked, with the dialog's other changes, in one undo step; the
    /// working-space entry assigns none.
    #[test]
    fn ok_assigns_the_profiles_with_the_other_changes_in_one_undo_step() {
        let (mut app, ctx, _) = red_square();
        let working = cms::active_settings().cmyk;
        let steps = undo_steps(&app);
        color_tab(&mut app, &ctx, json!({"cmykProfile": working, "bleed": [9, 9, 9, 9]}));
        assert!(press_ok(&mut app, &ctx));
        let st = app.session.active().unwrap();
        assert_eq!((st.doc.color_profiles.cmyk.clone(), st.doc.color_profiles.rgb.clone()), (Some(working.clone()), None));
        assert_eq!((st.doc.setup.bleed, st.doc.color_mode), ([9.0; 4], ColorMode::Rgb));
        assert_eq!(undo_steps(&app), steps + 1);
        // Back to the working space: no profile assigned.
        color_tab(&mut app, &ctx, json!({"cmykProfile": super::WORKING_CMYK}));
        assert!(press_ok(&mut app, &ctx));
        assert_eq!(app.session.active().unwrap().doc.color_profiles.cmyk, None);
        app.run("edit.undo", json!({})).unwrap();
        app.run("edit.undo", json!({})).unwrap();
        let st = app.session.active().unwrap();
        assert_eq!((st.doc.color_profiles.cmyk.clone(), st.doc.setup.bleed), (None, [0.0; 4]));
    }

    /// OK assigns the RGB profile picked, in one undo step.
    #[test]
    fn ok_assigns_the_rgb_profile() {
        let (mut app, ctx, _) = red_square();
        let working = cms::active_settings().rgb;
        let steps = undo_steps(&app);
        color_tab(&mut app, &ctx, json!({"rgbProfile": working}));
        assert!(press_ok(&mut app, &ctx));
        let st = app.session.active().unwrap();
        assert_eq!((st.doc.color_profiles.rgb.clone(), st.doc.color_profiles.cmyk.clone()), (Some(working), None));
        assert_eq!(undo_steps(&app), steps + 1);
    }

    /// A profile and a mode changed in one OK: Assign Profile runs before Document Color Mode, so
    /// the conversion goes through the profile picked.
    #[test]
    fn a_profile_and_a_mode_change_assign_the_profile_first() {
        let (mut app, ctx, _) = red_square();
        color_tab(&mut app, &ctx, json!({"colorMode": "cmyk", "cmykProfile": cms::DEVICE_CMYK}));
        let commands = super::color_commands(&app, &super::json_map(&fields(&app))).unwrap();
        assert_eq!(
            commands,
            vec![
                json!({"command": "edit.assignProfile", "params": {"cmyk": cms::DEVICE_CMYK}}),
                json!({"command": "file.documentColorMode", "params": {"mode": "cmyk", "convert": true}}),
            ]
        );
    }

    /// A Color tab field with a value of the wrong kind is an error and nothing changes, the
    /// profile picked with it included.
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
            let (mut app, ctx, id) = red_square();
            let before = fill(&app, id);
            let steps = undo_steps(&app);
            color_tab(&mut app, &ctx, json!({"colorMode": "cmyk", "cmykProfile": cms::active_settings().cmyk}));
            let mut map = super::json_map(&fields(&app));
            map.insert(key.into(), value.clone());
            assert!(super::color_commands(&app, &map).is_err(), "{key}: {value}");
            assert!(super::confirm(&mut app, &super::dialog_fields(map)).is_err(), "{key}: {value}");
            let st = app.session.active().unwrap();
            assert_eq!((st.doc.color_mode, st.doc.color_profiles.cmyk.clone()), (ColorMode::Rgb, None), "{key}: {value}");
            assert_eq!(fill(&app, id), before);
            assert_eq!(undo_steps(&app), steps);
        }
    }

    /// The Light pair a new document's transparency grid starts with.
    const LIGHT: [&str; 2] = ["#ffffff", "#cccccc"];

    fn hex(h: &str) -> Color {
        Color::from_hex(h).unwrap_or_else(|| panic!("bad hex {h}"))
    }

    /// The grid colors the dialog holds.
    fn dialog_grid(app: &VectorcraftApp) -> Vec<Color> {
        let map = super::json_map(&fields(app));
        map["gridColors"].as_array().unwrap().iter().map(|c| vectorcraft_engine::cmd::color_value(c).unwrap()).collect()
    }

    /// The pair and name the Grid Colors row shows for the dialog's `gridColors`.
    fn shown_grid(app: &VectorcraftApp) -> ([Color; 2], &'static str) {
        let setup = &app.session.active().unwrap().doc.setup;
        let pair = super::grid_colors(&super::json_map(&fields(app)), setup);
        (pair, super::grid_preset_name(pair))
    }

    /// Document Setup open on its Transparency tab, drawn.
    fn transparency_tab() -> (VectorcraftApp, egui::Context) {
        let (app, ctx) = setup_open();
        set(&app, &ctx, "tab", json!("Transparency"));
        settle(&app, &ctx);
        (app, ctx)
    }

    /// The Color Picker answers `color` for grid entry `index`, and the dialog applies it.
    fn answer(app: &VectorcraftApp, ctx: &egui::Context, index: usize, color: &str) {
        let c = Srgba::from_hex(color).unwrap();
        app.dialogs().unwrap().with(|h| h.answer_color(id(), super::GRID_KEYS[index], c));
        settle(app, ctx);
    }

    /// A picked color sets only its entry, the Grid Colors menu names the pair (Custom, or the
    /// preset it matches), and Document Setup's OK applies the pair in one undo step.
    #[test]
    fn a_grid_color_box_opens_the_color_picker_and_its_ok_sets_that_color() {
        for (index, picked, pair, name) in [(0, "#00ff00", ["#00ff00", "#cccccc"], "Custom"), (1, "#c4dcff", ["#ffffff", "#c4dcff"], "Blue")] {
            let (mut app, ctx) = transparency_tab();
            answer(&app, &ctx, index, picked);
            assert_eq!(dialog_grid(&app), pair.map(hex).to_vec(), "{name}");
            assert_eq!(shown_grid(&app).1, name);
            assert_eq!(undo_steps(&app), 0, "{name}: the picker's answer leaves the document as it is");
            assert!(press_ok(&mut app, &ctx));
            let st = app.session.active().unwrap();
            assert_eq!((st.doc.setup.grid_colors, st.doc.setup.grid_colors_name()), (pair.map(hex), name));
            assert_eq!(undo_steps(&app), 1, "{name}");
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(app.session.active().unwrap().doc.setup.grid_colors, LIGHT.map(hex), "{name}");
        }
    }

    /// When Grid Colors holds a preset name (in any letter case) or a list that is not two colors,
    /// the row shows the preset's pair or the document's, and a picked color sets only its entry.
    #[test]
    fn a_grid_color_box_opens_on_a_preset_name_or_a_short_list() {
        for (field, index, kept, name) in
            [(json!("Blue"), 1, "#ffffff", "Blue"), (json!("purple"), 0, "#e2c9ff", "Purple"), (json!(["#ff0000"]), 1, "#ffffff", "Light")]
        {
            let (mut app, ctx) = transparency_tab();
            set(&app, &ctx, "gridColors", field.clone());
            settle(&app, &ctx);
            assert_eq!(shown_grid(&app).1, name, "{field}");
            answer(&app, &ctx, index, "#00ff00");
            let mut pair = [hex(kept); 2];
            pair[index] = hex("#00ff00");
            assert_eq!(dialog_grid(&app), pair.to_vec(), "{field}");
            assert!(press_ok(&mut app, &ctx));
            assert_eq!(app.session.active().unwrap().doc.setup.grid_colors, pair, "{field}");
        }
    }

    /// With a lower-case preset name in `gridColors`, a picked color sets its entry and keeps the
    /// preset's other one.
    #[test]
    fn a_grid_color_box_opens_the_picker_when_the_field_names_a_preset() {
        let (app, ctx) = transparency_tab();
        set(&app, &ctx, "gridColors", json!("blue"));
        settle(&app, &ctx);
        answer(&app, &ctx, 1, "#00ff00");
        assert_eq!(dialog_grid(&app), ["#ffffff", "#00ff00"].map(hex).to_vec());
    }

    /// A color picked after a preset is chosen replaces only its own color in the preset's pair:
    /// the other color is the preset's.
    #[test]
    fn a_hex_edit_committed_with_a_preset_choice_keeps_the_presets_other_color() {
        let (app, ctx) = transparency_tab();
        set(&app, &ctx, "gridColors", json!("Blue"));
        settle(&app, &ctx);
        answer(&app, &ctx, 0, "#123456");
        assert_eq!(dialog_grid(&app), ["#123456", "#c4dcff"].map(hex).to_vec());
    }

    /// An answer for a key the dialog has no color for, or for a closed dialog, changes nothing.
    #[test]
    fn grid_color_picker_without_its_color_or_its_dialog_is_an_error() {
        let (app, ctx) = transparency_tab();
        let red = Srgba::from_hex("#ff0000").unwrap();
        app.dialogs().unwrap().with(|h| h.answer_color(id(), "gridColors.9", red));
        settle(&app, &ctx);
        assert_eq!(dialog_grid(&app), LIGHT.map(hex).to_vec());
        app.dialogs().unwrap().with(|h| {
            h.cancel_by_id(&ctx, id());
            h.answer_color(id(), super::GRID_KEYS[0], red);
        });
        assert_eq!(app.session.active().unwrap().doc.setup.grid_colors, LIGHT.map(hex));
        assert_eq!(undo_steps(&app), 0);
    }

    /// Document Setup opens on the document's own grid colors, a Lab color included: the grid
    /// preview reads them as the document holds them, and an unchanged OK keeps them.
    #[test]
    fn document_setup_opens_on_the_documents_own_grid_colors() {
        let (mut app, ctx) = new_app();
        app.run("document.setup", json!({"gridColors": ["lab 55 60 40", "#cccccc"]})).unwrap();
        app.run("file.documentSetup", json!({})).unwrap();
        let grid = app.session.active().unwrap().doc.setup.grid_colors;
        assert!(matches!(grid[0], Color::Lab { .. }), "{grid:?}");
        let map = super::json_map(&fields(&app));
        assert_eq!(super::preview_grid(&map, &app.session.active().unwrap().doc.setup).1, grid);
        assert!(press_ok(&mut app, &ctx));
        assert_eq!(app.session.active().unwrap().doc.setup.grid_colors, grid);
    }

    /// A picked color replaces only its own entry of the pair the dialog holds: the other entry
    /// stays as it is, a Lab color in its own model.
    #[test]
    fn a_grid_color_hex_edit_keeps_the_other_entry_as_it_is() {
        let (app, ctx) = transparency_tab();
        set(&app, &ctx, "gridColors", json!(["lab 55 60 40", "#cccccc"]));
        settle(&app, &ctx);
        answer(&app, &ctx, 1, "#123456");
        assert_eq!(super::json_map(&fields(&app))["gridColors"], json!(["lab 55 60 40", "#123456"]));
    }

    /// The Color Picker's OK on a grid color it opened on, unchanged, keeps that color in its own
    /// model: a Lab or CMYK color stays Lab or CMYK through Document Setup's OK, and a hex color
    /// stays that hex color. The other color stays as it was.
    #[test]
    fn an_unchanged_grid_color_picker_ok_keeps_the_colors_own_model() {
        for (index, grid) in
            [(0, json!(["lab 55 60 40", "#cccccc"])), (1, json!(["#ffffff", "cmyk 0 50 100 0"])), (1, json!(["lab 55 60 40", "#123456"]))]
        {
            let (mut app, ctx) = new_app();
            app.run("document.setup", json!({ "gridColors": grid })).unwrap();
            app.run("file.documentSetup", json!({})).unwrap();
            set(&app, &ctx, "tab", json!("Transparency"));
            settle(&app, &ctx);
            let before = app.session.active().unwrap().doc.setup.grid_colors;
            let entries = super::json_map(&fields(&app))["gridColors"].clone();
            // The picker opens on the color the box shows, and an unchanged OK answers that color.
            let shown = super::box_color(&before[index]);
            app.dialogs().unwrap().with(|h| h.answer_color(id(), super::GRID_KEYS[index], shown));
            settle(&app, &ctx);
            assert_eq!(super::json_map(&fields(&app))["gridColors"], entries, "{grid}");
            assert!(press_ok(&mut app, &ctx));
            assert_eq!(app.session.active().unwrap().doc.setup.grid_colors, before, "{grid}");
        }
    }

    /// The Grid Colors menu names a preset only when the pair is that preset's colors, as
    /// `document.setup` names it: a color that rounds to the preset's hex code makes it Custom.
    #[test]
    fn the_grid_colors_menu_names_a_preset_only_for_its_own_colors() {
        let (mut app, ctx) = transparency_tab();
        set(&app, &ctx, "gridColors", json!(["#ffffff", [0.801, 0.8, 0.8]]));
        settle(&app, &ctx);
        assert_eq!(shown_grid(&app).1, "Custom");
        assert!(press_ok(&mut app, &ctx));
        assert_eq!(app.session.active().unwrap().doc.setup.grid_colors_name(), "Custom");
        let (app, ctx) = transparency_tab();
        set(&app, &ctx, "gridColors", json!("Light"));
        settle(&app, &ctx);
        assert_eq!(shown_grid(&app).1, "Light");
    }

    /// With a list in Grid Colors that is not two colors, a cancelled picker leaves that list as it
    /// was, and Document Setup's OK still refuses it.
    #[test]
    fn grid_color_picker_cancel_keeps_a_short_list() {
        let (mut app, ctx) = transparency_tab();
        set(&app, &ctx, "gridColors", json!(["#ff0000"]));
        settle(&app, &ctx);
        // The Color Picker's Cancel: the request goes unanswered.
        app.dialogs().unwrap().with(|h| h.take_color_request());
        settle(&app, &ctx);
        assert_eq!(super::json_map(&fields(&app))["gridColors"], json!(["#ff0000"]));
        assert!(!press_ok(&mut app, &ctx));
        assert_eq!(app.session.active().unwrap().doc.setup.grid_colors, LIGHT.map(hex));
    }

    /// The preview reads the pending grid size and colors, not the document's, and neither it nor
    /// Cancel changes the document.
    #[test]
    fn the_transparency_tab_previews_the_pending_grid_size_and_colors() {
        let (app, ctx) = transparency_tab();
        let setup = app.session.active().unwrap().doc.setup.clone();
        assert_eq!(super::preview_grid(&super::json_map(&fields(&app)), &setup), (vectorcraft_doc::GridSize::Medium, LIGHT.map(hex)));
        set(&app, &ctx, "gridSize", json!("small"));
        set(&app, &ctx, "gridColors", json!(["#ff0000", "#0000ff"]));
        set(&app, &ctx, "simulatePaper", json!(true));
        assert_eq!(
            super::preview_grid(&super::json_map(&fields(&app)), &setup),
            (vectorcraft_doc::GridSize::Small, [hex("#ff0000"), hex("#0000ff")])
        );
        let st = app.session.active().unwrap();
        assert_eq!(
            (st.doc.setup.grid_size, st.doc.setup.grid_colors, st.doc.setup.simulate_paper),
            (vectorcraft_doc::GridSize::Medium, LIGHT.map(hex), false)
        );
        assert_eq!(undo_steps(&app), 0);
        app.dialogs().unwrap().with(|h| h.cancel_by_id(&ctx, id()));
        assert!(!is_open(&app));
        assert_eq!(app.session.active().unwrap().doc.setup.grid_colors, LIGHT.map(hex));
        assert_eq!(undo_steps(&app), 0);
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

    /// The grid preview the open dialog draws in `ctx`: its rect, the span of its texture
    /// coordinates (two cells per unit) and the four pixels of the texture it draws. `tiles` holds
    /// the pixels of each 2 × 2 texture uploaded so far in `ctx`, so every frame drawn in `ctx`
    /// goes through this function. The preview is the largest rect drawn with such a texture.
    fn drawn_preview(
        app: &VectorcraftApp,
        ctx: &egui::Context,
        tiles: &mut std::collections::HashMap<egui::TextureId, Vec<egui::Color32>>,
    ) -> (Rect, egui::Vec2, Vec<egui::Color32>) {
        let mut preview = None;
        for _ in 0..4 {
            let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 900.0))), ..Default::default() };
            let dialogs = app.dialogs().unwrap();
            let mut out = ctx.run_ui(raw, |ui| dialogs.with(|h| h.run(ui)));
            for (id, deltas) in &out.textures_delta.set {
                if let Some(d) = deltas.last() {
                    let egui::ImageData::Color(img) = &d.image;
                    if img.size == [2, 2] {
                        tiles.insert(*id, img.pixels.clone());
                    }
                }
            }
            out.textures_delta.clear();
            preview = out
                .shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    egui::Shape::Rect(r) => {
                        r.brush.as_ref().filter(|b| tiles.contains_key(&b.fill_texture_id)).map(|b| (r.rect, b.uv, b.fill_texture_id))
                    }
                    _ => None,
                })
                .max_by(|a, b| a.0.area().total_cmp(&b.0.area()));
        }
        let (rect, uv, texture) = preview.unwrap_or_else(|| panic!("no grid preview"));
        let pixels = tiles.get(&texture).cloned().unwrap_or_else(|| panic!("no tile for {texture:?}"));
        (rect, uv.size(), pixels)
    }

    /// The tile's pixels for the grid colors `a` and `b`: `a` in the top-left and bottom-right cells.
    fn tile(a: &str, b: &str) -> Vec<egui::Color32> {
        let c = |h| crate::panels::c32(&hex(h));
        vec![c(a), c(b), c(b), c(a)]
    }

    /// In an open dialog, the preview follows each edit of Grid Colors and Grid Size: after a
    /// color edit it draws a tile in the new colors, and after a size edit it draws the new cells.
    #[test]
    fn the_grid_preview_follows_edits_in_the_open_dialog() {
        let (app, ctx) = setup_open();
        let mut tiles = std::collections::HashMap::new();
        set(&app, &ctx, "tab", json!("Transparency"));
        let (rect, uv, pixels) = drawn_preview(&app, &ctx, &mut tiles);
        assert_eq!((uv, pixels), (rect.size() / 16.0, tile(LIGHT[0], LIGHT[1])), "Medium, Light");
        set(&app, &ctx, "gridColors", json!(["#ff0000", "#0000ff"]));
        let (rect, uv, pixels) = drawn_preview(&app, &ctx, &mut tiles);
        assert_eq!((uv, pixels), (rect.size() / 16.0, tile("#ff0000", "#0000ff")), "color edit");
        set(&app, &ctx, "gridSize", json!("large"));
        let (rect, uv, pixels) = drawn_preview(&app, &ctx, &mut tiles);
        assert_eq!((uv, pixels), (rect.size() / 32.0, tile("#ff0000", "#0000ff")), "size edit");
        set(&app, &ctx, "gridColors", json!("Dark"));
        let (_, _, pixels) = drawn_preview(&app, &ctx, &mut tiles);
        assert_eq!(pixels, tile("#666666", "#4d4d4d"), "preset edit");
        let st = app.session.active().unwrap();
        assert_eq!((st.doc.setup.grid_size, st.doc.setup.grid_colors), (vectorcraft_doc::GridSize::Medium, LIGHT.map(hex)));
        assert_eq!(undo_steps(&app), 0);
    }
}
