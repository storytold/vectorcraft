//! The format rows of Export for Screens and the Asset Export panel: the iOS and Android buttons,
//! the format presets (the ☰ menu), the Format Settings (the gear) and the rows (scale, suffix,
//! format) with Add Scale. They read and write the dialog's or the panel's fields as a JSON map.
//!
//! Keys: `formats` (rows `{format, scale, suffix, quality?, marks?}`), `preset` (""|mobile|density),
//! `subfolders`, `subfolderBy` (scale|format), `__itemFolders` (removed by an iOS or Android press,
//! set by a preset), `__settings` (the format whose Format Settings show, "" when closed),
//! `__savePreset` (the name typed; absent when not saving), `exportPreset` (the preset applied
//! last), `settings` (`{format id: {..}}`), `prefix`, `includeBleed`, `includeArtboardFill`.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::LazyLock;

use craft::behavior::{ColumnWidth, IconId};
use craft::egui::{Response, Ui};
use craft::host::PickRequest;
use craft::ui::buttons::{Button, IconButton};
use craft::ui::contract::overlay::Trigger;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::{Shade, Text};
use craft::ui::layout::{RowAlign, empty_cell};
use craft::ui::layout_components::{Grid, Row, Stack};
use craft::ui::menu::Menu;
use craft::ui::patterns::form_dialogs::ParamRows;
use craft::ui::pickers::{ComboBox, Select};
use craft::ui::runtime::env::Label;
use craft::ui::text::TextField;
use serde_json::{Map, Value, json};
use vectorcraft_engine::cmd::fileio::{self, ScreenSize};

use super::export_for_screens::{drop_unread_screen_settings, editable_row, folders};
use super::{form, png_options, svg_options};
use crate::file_asks::{self, Owner};
use crate::{VectorcraftApp, io};

/// The format choices of a row: (label, format, JPEG quality, with printer's marks).
pub(crate) const FORMATS: [(&str, &str, Option<u8>, bool); 14] = [
    ("PNG", "png", None, false),
    ("PNG 8", "png8", None, false),
    ("JPG 100", "jpg", Some(100), false),
    ("JPG 80", "jpg", Some(80), false),
    ("JPG 50", "jpg", Some(50), false),
    ("WebP", "webp", None, false),
    ("GIF", "gif", None, false),
    ("TIFF", "tiff", None, false),
    ("SVG", "svg", None, false),
    ("PDF", "pdf", None, false),
    ("PDF Print", "pdf", None, true),
    ("EPS", "eps", None, false),
    ("AI", "ai", None, false),
    ("VectorCraft", "vectorcraft", None, false),
];
/// The labels of [`FORMATS`], in order.
pub(crate) const FORMAT_LABELS: [&str; 14] =
    ["PNG", "PNG 8", "JPG 100", "JPG 80", "JPG 50", "WebP", "GIF", "TIFF", "SVG", "PDF", "PDF Print", "EPS", "AI", "VectorCraft"];

/// The printer's marks a PDF Print row writes: trim marks, registration targets, colour bars and
/// page information.
pub(crate) fn print_marks() -> Value {
    json!({"trim": true, "registration": true, "colorBars": true, "pageInfo": true})
}

/// The formats Format Settings has options for: (format, label).
pub(crate) const SETTINGS: [(&str, &str); 8] =
    [("png", "PNG"), ("png8", "PNG 8"), ("jpg", "JPG"), ("webp", "WebP"), ("gif", "GIF"), ("tiff", "TIFF"), ("svg", "SVG"), ("pdf", "PDF")];
/// The labels of [`SETTINGS`], in order.
pub(crate) const SETTINGS_LABELS: [&str; 8] = ["PNG", "PNG 8", "JPG", "WebP", "GIF", "TIFF", "SVG", "PDF"];

/// The sizes a row's scale list offers (any can be typed).
pub(crate) const SIZES: [&str; 12] = ["0.5x", "0.75x", "1x", "1.5x", "2x", "3x", "4x", "512w", "512h", "72ppi", "150ppi", "300ppi"];

/// The iOS and Android rows (the engine's `mobile` and `density` presets) and their labels.
pub(crate) const PLATFORMS: [(&str, &str); 2] = [("mobile", "iOS"), ("density", "Android")];

/// The columns of the format rows: scale, suffix and format share the room the remove button leaves. When the
/// fields cannot narrow that far, the grid stacks the cells.
const COLUMNS: &[ColumnWidth] = &[ColumnWidth::fill(1.0), ColumnWidth::fill(1.0), ColumnWidth::fill(1.0), ColumnWidth::Content];

/// The text of `key` in `fields`, "" when it has none.
fn text<'a>(fields: &'a Map<String, Value>, key: &str) -> &'a str {
    fields.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// The flag `key` in `fields`, false when it has none.
fn flag(fields: &Map<String, Value>, key: &str) -> bool {
    fields.get(key).and_then(Value::as_bool).unwrap_or_default()
}

/// Sets `key` of `row` when the row is an object; a row the dialog was given as anything else is
/// left as it is.
fn put(row: &mut Value, key: &str, value: Value) {
    if let Some(o) = row.as_object_mut() {
        o.insert(key.into(), value);
    }
}

/// Is `name` a built-in PDF preset's? Those are ours (translated where listed); the saved ones are
/// names. The name is matched exactly, so `default` and `Default` are not the built-in.
fn is_builtin_preset(name: &str) -> bool {
    static NAMES: LazyLock<Vec<String>> =
        LazyLock::new(|| vectorcraft_pdf::builtin_presets(vectorcraft_pdf::APP).into_iter().map(|p| p.name).collect());
    NAMES.iter().any(|n| n == name)
}

/// What the format presets menu (or the Asset Export panel's menu) asked for.
#[derive(Debug, PartialEq)]
pub(crate) enum PresetPick {
    /// Use this preset (`{name, builtIn, settings}`).
    Apply(Value),
    /// Save the settings as the preset of this name (Save, after a name was typed).
    Save(String),
    /// Delete this saved preset.
    Delete(String),
    /// Import Preset…: add the presets of a file.
    Import,
    /// Export Preset…: write this preset to a file.
    Export(String),
}

/// The format presets (the ☰ by iOS and Android) over `presets` (the rows of
/// `document.exportPresets.list`): the built-in and saved ones to apply, Save Preset… (which sets
/// `__savePreset`, shown by [`save_preset_row`]) and Delete Preset (the saved one applied last) → the
/// pick to act on with [`preset_pick`].
pub(crate) fn preset_menu_of(presets: &[Value], ui: &mut Ui, fields: &mut Map<String, Value>) -> Option<PresetPick> {
    let current = text(fields, "exportPreset").to_string();
    let saved = presets.iter().any(|p| p["name"] == current.as_str() && p["builtIn"] == false);
    let name = Label::from(tl!("Format Presets"));
    let opener = IconButton::new(IconId::new("menu")).label(name.clone()).tooltip(name.clone()).id_salt("efs-presets").show(ui);
    let mut entries: Vec<Entry<usize>> = presets
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let n = p["name"].as_str().unwrap_or_default();
            Entry::new(i, n.to_string()).checked(n == current)
        })
        .collect();
    // The rows after the presets: Save Preset… and Delete Preset, by their place past the presets.
    let (save_at, delete_at) = (presets.len(), presets.len() + 1);
    entries.push(Entry::separator());
    entries.push(Entry::new(save_at, tl!("Save Preset…")));
    entries.push(Entry::new(delete_at, tl!("Delete Preset")).enabled(saved));
    let picked = Menu::new(name).opened_by(&opener.response, Trigger::Click).pick(ui, &entries).picked?;
    if picked == save_at {
        fields.insert("__savePreset".into(), json!(if saved { current } else { String::new() }));
        None
    } else if picked == delete_at {
        Some(PresetPick::Delete(current))
    } else {
        presets.get(picked).cloned().map(PresetPick::Apply)
    }
}

/// The row that names a format preset being saved (Save Preset… in the ☰ menu) → Save, once a name
/// is typed and Save is pressed. Cancel drops `__savePreset`.
pub(crate) fn save_preset_row(ui: &mut Ui, fields: &mut Map<String, Value>) -> Option<PresetPick> {
    if !fields.contains_key("__savePreset") {
        return None;
    }
    let mut typed = text(fields, "__savePreset").to_string();
    let (mut pick, mut cancel) = (None, false);
    Row::new().show(ui, |ui, _| {
        Text::new(tl!("Name:")).shade(Shade::Muted).show(ui);
        TextField::new(tl!("Name:"), Bind::bound(&mut typed)).id_salt("efs-save-preset").show(ui);
        let name = typed.trim();
        if Button::new(tl!("Save")).enabled(!name.is_empty()).show(ui).clicked() {
            pick = Some(PresetPick::Save(name.to_string()));
        }
        cancel |= Button::new(tl!("Cancel")).show(ui).clicked();
    });
    fields.insert("__savePreset".into(), json!(typed));
    if cancel {
        fields.remove("__savePreset");
    }
    pick
}

/// Import Preset…: the presets of a picked file are added to the saved ones; `then` gets the names
/// added, or the error (`cancelled` when no file was picked). Browser picks use the file's contents.
pub(crate) fn import_presets(app: &mut VectorcraftApp, then: impl FnOnce(&mut VectorcraftApp, Result<Vec<String>, String>) + 'static) {
    let request = PickRequest::open("export-presets").filter("Export presets", fileio::SCREEN_EXPORT_PRESET_EXTS);
    // `then` reports its own result, so the continuation never fails (a failure would reach the status line twice).
    let _ = file_asks::ask(app, request, Owner::Free, move |app, picked| {
        let result = file_asks::open_params(&picked).ok_or_else(|| "cancelled".to_owned()).and_then(|mut params| {
            params.remove("name");
            io::import_presets(app, "document.exportPresets.import", io::EXPORT_PRESETS, Value::Object(params))
                .map(|r| r["imported"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect())
        });
        then(app, result);
        Ok(Value::Null)
    });
}

/// Export Preset…: preset `name` written to a file named after it (the web downloads it); `then`
/// gets where it went. The file question is asked by `io::save_command_output_named` through the
/// files service, since it takes no path.
pub(crate) fn export_preset(app: &mut VectorcraftApp, name: &str, then: impl FnOnce(&mut VectorcraftApp, Result<String, String>) + 'static) {
    // `then` runs once: with the path once the file is written, or with the error that stopped it first.
    let then = Rc::new(Cell::new(Some(then)));
    let written = Rc::clone(&then);
    let result =
        io::save_command_output_named(app, "document.exportPresets.export", &preset_file_name(name), json!({ "names": [name] }), move |app, path| {
            if let Some(then) = written.take() {
                then(app, Ok(path.clone()));
            }
            Ok(json!({ "path": path }))
        });
    if let Err(e) = result
        && let Some(then) = then.take()
    {
        then(app, Err(e));
    }
}

/// The file name Export Preset… suggests for preset `name`: the name made safe as a file name, as
/// Export Library… makes a library's (`library::file_stem_for`), with the `.vcexportpresets`
/// extension.
pub(crate) fn preset_file_name(name: &str) -> String {
    format!("{}.{}", vectorcraft_engine::cmd::library::file_stem_for(name), fileio::SCREEN_EXPORT_PRESET_FORMAT)
}

/// `list` (the rows of `document.exportPresets.list`) made the rows for the saved presets `saved`:
/// built again only when it does not list them as they are, so that a frame that changes nothing
/// builds nothing → whether it was built again.
pub(crate) fn preset_list(list: &mut Value, saved: &[fileio::ScreenExportPreset]) -> bool {
    // The built-in rows never change; the saved ones are compared in place.
    let current = list.as_array().is_some_and(|rows| {
        let mut mine = rows.iter().filter(|p| p["builtIn"] == false);
        rows.iter().any(|p| p["builtIn"] == true)
            && saved.iter().all(|q| mine.next().is_some_and(|row| row["name"] == q.name.as_str() && row["settings"].as_object() == Some(&q.settings)))
            && mine.next().is_none()
    });
    if !current {
        *list = fileio::screen_export_preset_list(saved);
    }
    !current
}

/// `ui.dialog.set` of `exportPreset` in the dialog's `fields`: the preset of `presets` (the rows of
/// `document.exportPresets.list`) it names is applied as a pick in the ☰ menu applies it, and `""` is
/// none. A name that is not a preset's is an error, and `exportPreset` is then cleared.
pub(crate) fn preset_set(presets: &[Value], fields: &mut Map<String, Value>) -> Result<(), String> {
    let name = text(fields, "exportPreset").to_string();
    let preset = presets.iter().find(|p| p["name"] == name.as_str()).cloned();
    match preset {
        Some(preset) => {
            apply_preset(fields, &preset);
            Ok(())
        }
        None => {
            fields.insert("exportPreset".into(), json!(""));
            if name.is_empty() { Ok(()) } else { Err(format!("no export preset `{name}` (see document.exportPresets.list)")) }
        }
    }
}

/// Act on what the format presets menu of the dialog with `fields` asked for. Save keeps the applied
/// preset's keys that the dialog does not show (a file name pattern, Full Document, a folder).
pub(crate) fn preset_pick(app: &mut VectorcraftApp, fields: &mut Map<String, Value>, pick: PresetPick) {
    let done = match pick {
        PresetPick::Apply(preset) => {
            apply_preset(fields, &preset);
            Ok(())
        }
        // The name as saved (trimmed and capped).
        PresetPick::Save(name) => {
            let applied = text(fields, "exportPreset");
            let mut settings = fileio::screen_export_preset_list(&app.session.prefs.screen_export_presets)
                .as_array()
                .and_then(|rows| rows.iter().find(|p| p["name"] == applied))
                .and_then(|p| p["settings"].as_object())
                .cloned()
                .unwrap_or_default();
            for key in ["formats", "preset", "settings", "prefix", "folders", "subfolders", "includeBleed", "includeArtboardFill"] {
                settings.remove(key);
            }
            settings.insert("formats".into(), fields.get("formats").cloned().unwrap_or(json!([])));
            settings.insert("settings".into(), fields.get("settings").cloned().unwrap_or(json!({})));
            settings.insert("prefix".into(), json!(text(fields, "prefix")));
            settings.insert("folders".into(), json!(folders(fields)));
            for key in ["includeBleed", "includeArtboardFill"] {
                settings.insert(key.into(), json!(flag(fields, key)));
            }
            app.run("document.exportPresets.save", json!({"name": name, "settings": settings})).map(|r| {
                fields.remove("__savePreset");
                fields.insert("exportPreset".into(), r["name"].clone());
            })
        }
        PresetPick::Delete(name) => app.run("document.exportPresets.delete", json!({ "name": name })).map(|_| {
            fields.insert("exportPreset".into(), json!(""));
        }),
        // The dialog's menu has no Import Preset… or Export Preset….
        PresetPick::Import | PresetPick::Export(_) => Ok(()),
    };
    if let Err(e) = done
        && e != "cancelled"
    {
        app.ui.status = e;
    }
}

/// Set the dialog's `fields` to format preset `preset` (`{name, settings}`): its rows (or the mobile
/// or density rows it names), their settings, the prefix and the sub-folders; what it doesn't name
/// keeps the dialog's value.
pub(crate) fn apply_preset(fields: &mut Map<String, Value>, preset: &Value) {
    let st = &preset["settings"];
    if let Some(rows) = st["formats"].as_array() {
        fields.insert("formats".into(), json!(rows.iter().filter(|r| r.is_object()).map(editable_row).collect::<Vec<_>>()));
        fields.insert("preset".into(), json!(""));
    } else if let Some((p, rows)) = st["preset"].as_str().and_then(|p| Some((p, fileio::screen_preset_rows(p)?))) {
        // A preset saved with the mobile or density rows (the Asset Export panel's).
        fields.insert("formats".into(), json!(rows));
        fields.insert("preset".into(), json!(p));
    }
    // Settings for a format the export doesn't write, and keys it no longer reads, are left out.
    let mut given = Map::from_iter([("settings".to_string(), st["settings"].clone())]);
    drop_unread_screen_settings(&mut given);
    if let (Some(all), Some(given)) = (fields.get_mut("settings").and_then(Value::as_object_mut), given.get("settings").and_then(Value::as_object)) {
        for (id, o) in given {
            match (all.get_mut(id).and_then(Value::as_object_mut), o.as_object()) {
                (Some(mine), Some(o)) => mine.extend(o.iter().map(|(k, v)| (k.clone(), v.clone()))),
                _ => _ = all.insert(id.clone(), o.clone()),
            }
        }
    }
    // An empty `folders` is none, as an export reads it.
    let folders = st["folders"]
        .as_str()
        .map(|f| if f.is_empty() { "none" } else { f })
        .or_else(|| st["subfolders"].as_bool().map(|b| if b { "rows" } else { "none" }));
    if let Some(f) = folders {
        fields.insert("subfolders".into(), json!(f != "none"));
        fields.insert("subfolderBy".into(), json!(if f == "format" { "format" } else { "scale" }));
        fields.insert("__itemFolders".into(), json!(if matches!(f, "item" | "itemFormat") { f } else { "" }));
    }
    if let Some(prefix) = st["prefix"].as_str() {
        fields.insert("prefix".into(), json!(prefix));
    }
    for key in ["includeBleed", "includeArtboardFill"] {
        if let Some(b) = st[key].as_bool() {
            fields.insert(key.into(), json!(b));
        }
    }
    fields.insert("exportPreset".into(), json!(preset["name"]));
}

/// The dialog's format presets menu, drawn in the Formats row over the dialog's fields.
pub(crate) type FormatMenu<'a> = &'a mut dyn FnMut(&mut Ui, &mut Map<String, Value>);

/// Formats: the iOS and Android buttons (each sets the rows that platform uses), `menu` (the
/// dialog's format presets), the Format Settings gear (which sets `__settings`), the format rows
/// (scale, suffix, format) and Add Scale, as wide as `ui` allows: the dialog's and the Asset Export
/// panel's rows. A row edit makes `preset` "". Nothing is kept between passes: the rows are built
/// over `fields` each pass.
pub(crate) fn rows(ui: &mut Ui, fields: &mut Map<String, Value>, mut menu: Option<FormatMenu<'_>>) -> Response {
    let mut rows: Vec<Value> = fields.get("formats").and_then(Value::as_array).cloned().unwrap_or_default();
    let preset = text(fields, "preset").to_string();
    let sizes: Vec<Entry<String>> = SIZES.iter().map(|s| Entry::new((*s).to_string(), *s)).collect();
    let formats: Vec<Entry<usize>> = FORMAT_LABELS.iter().enumerate().map(|(i, l)| Entry::new(i, *l)).collect();
    let (mut edited, mut add, mut open_settings) = (false, false, false);
    let (mut platform, mut remove) = (None, None);
    let one = rows.len() == 1;
    let first = settings_format(&rows);
    let shown = Stack::new().show(ui, |ui, _| {
        // The buttons end at the right edge; a column too narrow for them puts them on more lines.
        Row::new().fill_width().align(RowAlign::SpaceBetween).show(ui, |ui, _| {
            Text::new(tl!("Formats:")).shade(Shade::Muted).show(ui);
            Row::new().wrap(true).show(ui, |ui, _| {
                for (id, label) in PLATFORMS {
                    if Button::new(tl!(label)).selected(preset == id).id_salt(id).show(ui).clicked() {
                        platform = Some(id);
                    }
                }
                if let Some(menu) = menu.as_mut() {
                    menu(ui, fields);
                }
                let name = Label::from(tl!("Format Settings"));
                open_settings |= IconButton::new(IconId::new("settings")).label(name.clone()).tooltip(name).id_salt("efs-gear").show(ui).clicked();
            });
        });
        Grid::new(COLUMNS).show(ui, |ui, _| {
            for head in [tl!("Scale"), tl!("Suffix"), tl!("Format")] {
                Text::new(head).shade(Shade::Muted).show(ui);
            }
            empty_cell(ui);
            for (i, row) in rows.iter_mut().enumerate() {
                let raster = row["format"].as_str().and_then(fileio::format).is_none_or(|f| f.raster);
                let scale = row["scale"].as_str().unwrap_or("1x").to_string();
                let typed = ComboBox::new(tl!("Scale"), Bind::given(scale.clone()), &sizes).id_salt(("efs-scale", i)).enabled(raster).show(ui);
                if let Some(s) = typed.edit.events.committed().filter(|s| **s != scale) {
                    // The suffix follows the size while it is the automatic one.
                    let auto = |text: &str| ScreenSize::parse(text).map(ScreenSize::suffix);
                    if row["suffix"].as_str().is_none_or(|x| Some(x.to_string()) == auto(&scale))
                        && let Some(suffix) = auto(s)
                    {
                        put(row, "suffix", json!(suffix));
                    }
                    put(row, "scale", json!(s));
                    edited = true;
                }
                let suffix = row["suffix"].as_str().unwrap_or_default().to_string();
                let typed = TextField::new(tl!("Suffix"), Bind::given(suffix.clone())).id_salt(("efs-suffix", i)).show(ui);
                if let Some(s) = typed.events.committed().filter(|s| **s != suffix) {
                    put(row, "suffix", json!(s));
                    edited = true;
                }
                let (format, quality) = (row["format"].as_str().unwrap_or("png"), row["quality"].as_u64());
                let marked = row.get("marks").is_some_and(|m| !m.is_null());
                let current =
                    FORMATS.iter().position(|(_, f, q, print)| *f == format && *print == marked && q.is_none_or(|q| Some(u64::from(q)) == quality));
                let picked = Select::new(tl!("Format"), Bind::given(current.unwrap_or(usize::MAX)), &formats)
                    .placeholder(format.to_uppercase())
                    .id_salt(("efs-fmt", i))
                    .show(ui);
                if let Some((_, f, q, print)) = picked.events.committed().and_then(|at| FORMATS.get(*at)) {
                    put(row, "format", json!(f));
                    if let Some(o) = row.as_object_mut() {
                        match q {
                            Some(q) => _ = o.insert("quality".into(), json!(q)),
                            None => _ = o.remove("quality"),
                        }
                        if *print {
                            o.insert("marks".into(), print_marks());
                            // A file of its own next to the plain PDF's.
                            if o.get("suffix").and_then(Value::as_str).is_none_or(str::is_empty) {
                                o.insert("suffix".into(), json!("-print"));
                            }
                        } else {
                            o.remove("marks");
                        }
                    }
                    edited = true;
                }
                let name = Label::from(tl!("Remove"));
                if IconButton::new(IconId::new("x")).label(name.clone()).tooltip(name).enabled(!one).id_salt(("efs-remove", i)).show(ui).clicked() {
                    remove = Some(i);
                }
            }
        });
        add |= Button::new(tl!("+ Add Scale")).show(ui).clicked();
    });
    if open_settings {
        fields.insert("__settings".into(), json!(first));
    }
    if let Some(id) = platform
        && let Some(preset_rows) = fileio::screen_preset_rows(id)
    {
        rows = preset_rows;
        fields.insert("subfolders".into(), json!(true));
        fields.insert("subfolderBy".into(), json!("scale"));
        fields.remove("__itemFolders");
        fields.insert("formats".into(), json!(rows));
        fields.insert("preset".into(), json!(id));
    }
    if let Some(i) = remove.filter(|i| *i < rows.len()) {
        rows.remove(i);
        edited = true;
    }
    if add {
        // One more than the largest factor so far.
        let largest = rows
            .iter()
            .filter_map(|r| match r["scale"].as_str().and_then(ScreenSize::parse) {
                Some(ScreenSize::Scale(f)) => Some(f),
                _ => None,
            })
            .fold(0.0, f64::max);
        let size = ScreenSize::Scale(largest.floor() + 1.0);
        rows.push(json!({"format": "png", "scale": size.label(), "suffix": size.suffix()}));
        edited = true;
    }
    // Rows changed by hand are no longer a preset's.
    if edited {
        fields.insert("formats".into(), json!(rows));
        fields.insert("preset".into(), json!(""));
    }
    shown.output
}
/// The format Format Settings opens on for format rows `rows`: the first row's format when it has
/// settings of its own, else PNG. Row spellings (`jpeg`, `tif`) give the canonical id.
pub(crate) fn settings_format(rows: &[Value]) -> &'static str {
    rows.first()
        .and_then(|r| r["format"].as_str())
        .and_then(fileio::format)
        .map(|f| f.id)
        .filter(|f| SETTINGS.iter().any(|(id, _)| id == f))
        .unwrap_or("png")
}

/// Format Settings: the format `__settings` names (a format list switches it), the options of that
/// format that apply to every row of it, then Done (which sets `__settings` to ""). `presets` are
/// the PDF preset names the PDF Preset list offers. Only an edit writes `settings`, so an untouched
/// format keeps no keys.
pub(crate) fn settings(ui: &mut Ui, fields: &mut Map<String, Value>, presets: &[String]) -> Response {
    let wanted = text(fields, "__settings");
    let id = SETTINGS.iter().map(|(id, _)| *id).find(|id| *id == wanted).unwrap_or("png");
    let mine = fields.get("settings").and_then(|all| all.get(id)).and_then(Value::as_object).cloned().unwrap_or_default();
    let tiny = mine.get("profile").and_then(Value::as_str) == Some("tiny12");
    let params = match id {
        "svg" => svg_options::svg_params(true, tiny),
        "pdf" => Vec::new(),
        _ => png_options::raster_params(id, true, None),
    };
    let formats: Vec<Entry<&'static str>> = SETTINGS.iter().zip(SETTINGS_LABELS).map(|((f, _), label)| Entry::new(*f, label)).collect();
    let pdf_presets: Vec<Entry<String>> =
        presets.iter().map(|n| Entry::new(n.clone(), (if is_builtin_preset(n) { tl!(n) } else { n.as_str() }).to_string())).collect();
    let mut switch = None;
    let mut changed: Option<Map<String, Value>> = None;
    let mut done = false;
    let shown = Stack::new().show(ui, |ui, _| {
        Text::new(tl!("Format Settings")).show(ui);
        Row::new().show(ui, |ui, _| {
            Text::new(tl!("Format:")).shade(Shade::Muted).show(ui);
            if let Some(f) = Select::new(tl!("Format:"), Bind::given(id), &formats).id_salt("efs-settings-format").show(ui).events.committed() {
                switch = Some(*f);
            }
        });
        if id == "pdf" {
            Row::new().show(ui, |ui, _| {
                Text::new(tl!("PDF Preset:")).shade(Shade::Muted).show(ui);
                let current = mine.get("preset").and_then(Value::as_str).unwrap_or_default().to_string();
                if let Some(p) =
                    Select::new(tl!("PDF Preset:"), Bind::given(current), &pdf_presets).id_salt("efs-pdf-preset").show(ui).events.committed()
                {
                    let mut next = mine.clone();
                    next.insert("preset".into(), json!(p));
                    changed = Some(next);
                }
            });
        } else {
            let shown = ParamRows::new(tl!("Format Settings"), &params, Bind::given(form::fields_of(&params, &mine))).show(ui);
            if shown.changed
                && let Some(values) = shown.values
            {
                changed = Some(form::json_of(&params, &values));
            }
        }
        done |= Button::new(tl!("Done")).show(ui).clicked();
    });
    if let Some(f) = switch.filter(|f| *f != id) {
        fields.insert("__settings".into(), json!(f));
    }
    if let Some(next) = changed
        && let Some(all) = fields.entry("settings").or_insert_with(|| json!({})).as_object_mut()
    {
        all.insert(id.into(), Value::Object(next));
    }
    if done {
        fields.insert("__settings".into(), json!(""));
    }
    shown.output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::panel_harness::Panel;
    use craft::ui::contract::Role;
    use egui::{Key, Modifiers};

    fn export_panel() -> Panel {
        let mut app = crate::testing::windowed(VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default()));
        app.services.download = Some(Box::new(|_, _| Ok(())));
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let mut panel = Panel::new(app, crate::tests::picks::dialogs);
        panel.room(egui::vec2(1100.0, 1000.0));
        panel.app().run("file.exportForScreens", json!({})).unwrap();
        panel.frame();
        panel
    }

    fn fields(panel: &Panel) -> Value {
        super::super::tests_export::fields(&panel.app())
    }

    fn set(panel: &Panel, key: &str, value: Value) {
        super::super::tests_export::set(&mut panel.app(), key, value);
    }

    #[test]
    fn cancelled_format_choices_preserve_the_complete_row_and_preset() {
        for row in [
            json!({"format": "pdf", "marks": print_marks(), "scale": "1x", "suffix": "-print"}),
            json!({"format": "jpg", "quality": 100, "scale": "1x", "suffix": ""}),
        ] {
            let mut panel = export_panel();
            set(&panel, "formats", json!([row]));
            set(&panel, "preset", json!("mobile"));
            let before = fields(&panel);
            let document = panel.app().session.active().unwrap().doc.clone();
            let undo = panel.app().session.active().unwrap().history.undo.len();
            panel.click_role("Format", Role::COMBO_BOX);
            panel.key(Key::ArrowDown, Modifiers::NONE);
            panel.key(Key::Escape, Modifiers::NONE);
            assert_eq!(fields(&panel), before, "Escape retains quality, marks, suffix and the applied platform preset");
            panel.click_role("Format", Role::COMBO_BOX);
            panel.key(Key::ArrowDown, Modifiers::NONE);
            panel.key(Key::Enter, Modifiers::NONE);
            let after = fields(&panel);
            assert_eq!(after["preset"], "");
            assert_eq!(after["formats"][0]["suffix"], before["formats"][0]["suffix"]);
            if before["formats"][0]["format"] == "pdf" {
                assert_eq!(after["formats"][0]["format"], "eps");
                assert!(after["formats"][0].get("marks").is_none());
            } else {
                assert_eq!(after["formats"][0]["format"], "jpg");
                assert_eq!(after["formats"][0]["quality"], 80);
            }
            assert_eq!(panel.app().session.active().unwrap().doc, document);
            assert_eq!(panel.app().session.active().unwrap().history.undo.len(), undo);
        }
    }

    #[test]
    fn cancelled_scale_suffix_and_preset_name_typing_keep_the_opening_values() {
        for (label, role, key, typed) in [
            ("Scale", Role::COMBO_BOX, "scale", "2x"),
            ("Suffix", Role::TEXT_INPUT, "suffix", "-new"),
            ("Name:", Role::TEXT_INPUT, "__savePreset", "Changed"),
        ] {
            let mut panel = export_panel();
            set(&panel, "formats", json!([{"format": "png", "scale": "1x", "suffix": ""}]));
            set(&panel, "preset", json!("mobile"));
            if key == "__savePreset" {
                set(&panel, key, json!("Original"));
            }
            let before = fields(&panel);
            panel.click_role(label, role);
            panel.key(Key::A, Modifiers::COMMAND);
            panel.type_text(typed);
            panel.key(Key::Escape, Modifiers::NONE);
            if key == "scale" {
                assert_eq!(fields(&panel), before, "Escape discards the Scale choice and its side effects");
            } else {
                assert!(fields(&panel).is_null(), "Escape from {label} cancels the current export dialog");
                panel.app().run("file.exportForScreens", json!({})).unwrap();
                panel.frame();
                set(&panel, "formats", before["formats"].clone());
                set(&panel, "preset", before["preset"].clone());
                if key == "__savePreset" {
                    set(&panel, key, before[key].clone());
                }
            }
            assert!(panel.app().session.prefs.screen_export_presets.is_empty(), "Escape never saves a preset");
            panel.click_role(label, role);
            panel.key(Key::A, Modifiers::COMMAND);
            panel.type_text(typed);
            panel.key(Key::Tab, Modifiers::NONE);
            let after = fields(&panel);
            if key == "__savePreset" {
                assert_eq!(after[key], typed);
            } else {
                assert_eq!(after["formats"][0][key], typed);
                assert_eq!(after["preset"], "");
                if key == "scale" {
                    assert_eq!(after["formats"][0]["suffix"], "@2x");
                }
            }
        }
    }

    #[test]
    fn cancelled_settings_format_and_pdf_preset_choices_keep_all_options() {
        for pdf in [false, true] {
            let mut panel = export_panel();
            let presets = fileio::pdf::presets(&panel.app().session);
            assert!(presets.len() > 1);
            set(&panel, "settings", json!({"png": {"dpi": 144}, "pdf": {"preset": presets[0]}}));
            set(&panel, "__settings", json!(if pdf { "pdf" } else { "png" }));
            let before = fields(&panel);
            let label = if pdf { "PDF Preset:" } else { "Format:" };
            panel.click_role(label, Role::COMBO_BOX);
            panel.key(Key::ArrowDown, Modifiers::NONE);
            panel.key(Key::Escape, Modifiers::NONE);
            assert_eq!(fields(&panel), before);
            panel.click_role(label, Role::COMBO_BOX);
            panel.key(Key::ArrowDown, Modifiers::NONE);
            panel.key(Key::Enter, Modifiers::NONE);
            let after = fields(&panel);
            if pdf {
                assert_eq!(after["settings"]["pdf"]["preset"], presets[1]);
                assert_eq!(after["__settings"], "pdf");
            } else {
                assert_eq!(after["__settings"], "png8");
                assert_eq!(after["settings"], before["settings"]);
            }
        }
    }

    #[test]
    fn a_new_preset_name_can_be_saved_by_a_direct_click_without_tab() {
        let mut panel = export_panel();
        set(&panel, "__savePreset", json!(""));
        panel.click_role("Name:", Role::TEXT_INPUT);
        panel.type_text("Fleet");
        assert!(panel.enabled("Save"), "typing a name enables Save before the field loses focus");
        panel.click("Save");
        panel.frame();
        assert!(fields(&panel).get("__savePreset").is_none());
        assert_eq!(fields(&panel)["exportPreset"], "Fleet");
        let formats = fields(&panel)["formats"].clone();
        let app = panel.app();
        let saved = app.session.prefs.screen_export_presets.iter().find(|p| p.name == "Fleet").expect("Save uses the name just typed");
        assert_eq!(saved.settings["formats"], formats);
    }

    #[test]
    fn the_browser_preset_picker_routes_by_content_without_opening_drawings() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.services.download = Some(Box::new(|_, _| Ok(())));
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("document.exportPresets.save", json!({"name": "Fleet", "settings": {"formats": [{"format": "png"}]}})).unwrap();
        let export = app.run("document.exportPresets.export", json!({"names": ["Fleet"]})).unwrap();
        let bytes = export["data"].as_str().unwrap().as_bytes();
        app.run("document.exportPresets.delete", json!({"name": "Fleet"})).unwrap();
        let document = app.session.active().unwrap().uid;
        let picker = crate::tests::picks::scripted(&mut app);
        let ctx = app.ui_ctx.clone().unwrap();
        let picked = |app: &mut VectorcraftApp, name: &str, bytes: &[u8]| {
            let answer = Rc::new(Cell::new(None));
            let report = Rc::clone(&answer);
            import_presets(app, move |_, result| report.set(Some(result)));
            picker.answer_contents(&[(name, bytes)]);
            crate::tests::picks::poll(app, &ctx);
            answer.take().unwrap()
        };

        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="30" height="20"><rect width="10" height="10"/></svg>"#;
        let error = picked(&mut app, "drawing.svg", svg);
        assert_eq!(app.session.documents().len(), 1, "Import Preset must not open a drawing");
        assert!(error.unwrap_err().contains("not an export presets file"));

        assert_eq!(picked(&mut app, "different-extension.svg", bytes), Ok(vec!["Fleet".to_string()]));
        assert_eq!(app.ui.status, "Imported export presets: Fleet");
        assert_eq!(app.session.active().unwrap().uid, document);
    }
}
