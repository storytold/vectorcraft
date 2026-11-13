//! Edit → Tracing Presets: every tracing preset in a list (built-in ones in brackets, protected,
//! then the saved ones) with the selected one's settings beside it; New… (a preset starting from
//! the selected one), Edit… and Delete (saved presets), Import… and Export… (every saved preset)
//! under the list. New… and Edit… open the preset editor (kind `tracingPreset`): its OK saves the
//! preset with `imageTrace.presets.save` and comes back here on it, and its Cancel comes back with
//! nothing saved. An import whose file holds presets with the names of saved ones asks whether to
//! replace them (kind `tracingPresetsReplace`); Cancel keeps the saved ones. The saved presets are
//! the ones the Image Trace panel, the Control bar and Properties list.
//!
//! Fields: `selected` (a preset's name). The editor's: `name`, `params` (the tracing settings, as
//! `imageTrace.presets.save` takes them) and `editing` (the saved preset it edits; empty for a new
//! one). The question's: `names` (the presets the import skipped) and `file` (the file, as
//! `imageTrace.presets.import` takes it: `{path}` or `{dataBase64}`). Opening a `.vctracepresets`
//! file (File › Open, a drop, the web's Import… pick) imports it the same way.

use serde_json::{Value, json};
use vectorcraft_engine::cmd::swatchlib::DOCUMENT_SWATCHES;
use vectorcraft_engine::cmd::{TRACE_PRESET_EXTS, TRACE_PRESET_FORMAT, TRACE_PRESET_MAX_BYTES};

use super::{DialogSpec, form};
use crate::panels::image_trace::{self, MODES, PALETTES, Preset};
use crate::panels::swatches;
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Tracing Presets.
pub const KIND: &str = "tracingPresets";

/// The dialog kind of the tracing preset editor.
pub const PRESET_KIND: &str = "tracingPreset";

/// The dialog kind of the question whether an import replaces saved presets of the same names.
pub const REPLACE_KIND: &str = "tracingPresetsReplace";

/// The editor's field naming the saved preset it edits.
const EDITING: &str = "editing";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Tracing Presets").into(),
    body,
    confirm: |app, _| {
        app.ui.dialog = None;
        Ok(Value::Null)
    },
    ok: None,
    min_width: 620.0,
    ..DialogSpec::FORM
};

pub(super) const PRESET_SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Save Preset").into(),
    body: preset_body,
    confirm: confirm_preset,
    ok_enabled: Some(|_, d| !d.str("name").trim().is_empty()),
    min_width: 380.0,
    ..DialogSpec::FORM
};

pub(super) const REPLACE_SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Replace the saved presets that have these names?").into(),
    body: |_, ui, d| {
        let names: Vec<&str> = d.fields.get("names").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
        ui.label(egui::RichText::new(names.join("\n")).color(Tokens::get(ui.ctx()).text_dim));
        false
    },
    confirm: confirm_replace,
    ok: Some("Replace"),
    max_width: Some(420.0),
    ..DialogSpec::FORM
};

/// Open the presets manager on `selected` (default: the first preset, Default).
pub fn open(app: &mut VectorcraftApp, selected: Option<&str>) {
    app.ui.dialog = Some(Dialog::new(KIND, json!({ "selected": selected.unwrap_or("Default") })));
}

/// The index in `all` of the preset `selected` names, found as the engine finds presets (the first
/// when none).
fn shown(all: &[Preset], selected: &str) -> usize {
    image_trace::find_preset(all, selected).and_then(|p| all.iter().position(|q| std::ptr::eq(p, q))).unwrap_or(0)
}

/// A preset's name as the list shows it: a built-in one translated and in brackets, a saved one as
/// the user named it.
fn list_label(p: &Preset) -> String {
    if p.3 { format!("[{}]", image_trace::preset_label(&p.0)) } else { p.0.clone() }
}

/// What a button asked for, done once the list and the settings are drawn.
enum Action {
    Select(String),
    New,
    Edit,
    Delete,
    Import,
    Export,
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    let all = image_trace::presets(app, ui.ctx());
    let i = shown(&all, &d.str("selected"));
    let Some(preset) = all.get(i).cloned() else { return false };
    let builtin = preset.3;
    let any_saved = all.iter().any(|p| !p.3);
    let mut act = None;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(260.0);
            widgets::dim_label(ui, tl!("Presets:"));
            widgets::list_box(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("tracing-presets").min_scrolled_height(300.0).max_height(300.0).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_min_height(300.0);
                    for (k, p) in all.iter().enumerate() {
                        if ui.selectable_label(k == i, list_label(p)).clicked() {
                            act = Some(Action::Select(p.0.clone()));
                        }
                    }
                });
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if widgets::flat_button(ui, tl!("New…"), 48.0).on_hover_text(tl!("A new preset starting from the selected one")).clicked() {
                    act = Some(Action::New);
                }
                if ui.add_enabled_ui(!builtin, |ui| widgets::flat_button(ui, tl!("Edit…"), 48.0)).inner.clicked() {
                    act = Some(Action::Edit);
                }
                if ui.add_enabled_ui(!builtin, |ui| widgets::flat_button(ui, tl!("Delete"), 52.0)).inner.clicked() {
                    act = Some(Action::Delete);
                }
                if widgets::flat_button(ui, tl!("Import…"), 60.0).clicked() {
                    act = Some(Action::Import);
                }
                if ui.add_enabled_ui(any_saved, |ui| widgets::flat_button(ui, tl!("Export…"), 60.0)).inner.clicked() {
                    act = Some(Action::Export);
                }
            });
        });
        ui.add_space(18.0);
        ui.vertical(|ui| {
            ui.set_width(320.0);
            ui.label(egui::RichText::new(list_label(&preset)).color(t.text_strong).strong());
            ui.add_space(8.0);
            // The selected preset's settings, as the editor shows them, to read only.
            let mut params = preset.1.clone();
            ui.add_enabled_ui(false, |ui| settings_editor(app, ui, "tracing-presets", &mut params));
            if builtin {
                ui.add_space(6.0);
                ui.label(egui::RichText::new(tl!("Built-in presets don't change: New makes a copy you can edit.")).color(t.text_dim).size(11.5));
            }
        });
    });
    if let Some(a) = act {
        let r = match a {
            // Their file dialogs, shown off the UI thread, import into or export from the dialog
            // as it is then.
            Action::Import | Action::Export => {
                let export = matches!(a, Action::Export);
                crate::picks::in_app_dialog(app, d, move |app, d| run(app, d, if export { Action::Export } else { Action::Import }, ""))
            }
            a => run(app, d, a, &preset.0),
        };
        if let Err(e) = r {
            app.status(e);
        }
    }
    false
}

fn select(d: &mut Dialog, name: &str) {
    d.fields.insert("selected".into(), json!(name));
}

fn run(app: &mut VectorcraftApp, d: &mut Dialog, act: Action, current: &str) -> Result<(), String> {
    match act {
        Action::Select(name) => select(d, &name),
        // The editor replaces this dialog; its OK and its Cancel come back here.
        Action::New => *d = editor(app, d, current, false)?,
        Action::Edit => *d = editor(app, d, current, true)?,
        Action::Delete => {
            let i = app.session.prefs.trace_presets.iter().position(|p| p.name == current);
            app.run("imageTrace.presets.delete", json!({ "name": current }))?;
            // The saved preset below it takes its place, else the one above it, else Default.
            let saved = &app.session.prefs.trace_presets;
            let next = i.and_then(|i| saved.get(i).or_else(|| i.checked_sub(1).and_then(|k| saved.get(k)))).map(|p| p.name.clone());
            select(d, next.as_deref().unwrap_or("Default"));
        }
        Action::Import => {
            // On the web the picked file arrives later and imports through `io::open_bytes`.
            if crate::picks::open_async(app) {
                return Ok(());
            }
            let pick = crate::FilePick { filters: vec![crate::FilePick::filter("Tracing presets", TRACE_PRESET_EXTS)], ..Default::default() };
            let path = crate::picks::open(app, &pick).ok_or("cancelled")?;
            import(app, d, json!({ "path": path }))?;
        }
        Action::Export => {
            crate::io::save_command_output(app, "imageTrace.presets.export", TRACE_PRESET_FORMAT, json!({}))?;
        }
    }
    Ok(())
}

/// Import the tracing presets file `file` (`{path}` or `{dataBase64}`) into the saved presets,
/// keeping saved presets of the same names → the presets the file added and the names it skipped.
fn import_file(app: &mut VectorcraftApp, file: &Value) -> Result<(Value, Vec<Value>), String> {
    let r = crate::io::import_presets(app, "imageTrace.presets.import", "tracing presets", file.clone())?;
    let skipped = r["skipped"].as_array().cloned().unwrap_or_default();
    Ok((r["imported"].clone(), skipped))
}

/// The question whether the import of `file` replaces the saved presets `skipped` kept, over the
/// dialog `under` (Cancel shows it again).
fn ask_replace(under: Option<&Dialog>, skipped: Vec<Value>, file: Value) -> Dialog {
    let under = under.and_then(|d| serde_json::to_value(d).ok()).unwrap_or(Value::Null);
    Dialog::new(REPLACE_KIND, json!({ "names": skipped, "file": file, super::PARENT: under }))
}

/// The error for a tracing presets file over the size limit.
fn too_big() -> String {
    format!("the tracing presets file is larger than {} MiB", TRACE_PRESET_MAX_BYTES >> 20)
}

/// The tracing presets file `file` as `{dataBase64}`: a `{path}` is read here, once, so the
/// question and its Replace act on the contents the first import checked.
fn file_contents(app: &VectorcraftApp, file: Value) -> Result<Value, String> {
    let Some(path) = file.get("path").and_then(Value::as_str) else { return Ok(file) };
    vectorcraft_engine::file_access::check_read(path)?;
    if std::fs::metadata(path).is_ok_and(|m| m.len() > TRACE_PRESET_MAX_BYTES as u64) {
        return Err(too_big());
    }
    // One byte past the limit tells a larger file apart without reading all of it.
    let bytes = match (&app.services.read_head, &app.services.read) {
        (Some(read_head), _) => read_head(path, TRACE_PRESET_MAX_BYTES + 1)?,
        (None, Some(read)) => read(path)?,
        (None, None) => return Err("no file reader".into()),
    };
    if bytes.len() > TRACE_PRESET_MAX_BYTES {
        return Err(too_big());
    }
    Ok(json!({ "dataBase64": vectorcraft_format::base64_encode(&bytes) }))
}

/// Import… on the manager `d`: the import of `file` ([`import_file`]) selects the first preset it
/// added, and when it kept saved presets of the file's names, `d` goes under the question whether
/// to replace them.
fn import(app: &mut VectorcraftApp, d: &mut Dialog, file: Value) -> Result<(), String> {
    let file = file_contents(app, file)?;
    let (imported, skipped) = import_file(app, &file)?;
    if let Some(first) = imported.get(0).and_then(Value::as_str) {
        select(d, first);
    }
    if !skipped.is_empty() {
        *d = ask_replace(Some(d), skipped, file);
    }
    Ok(())
}

/// A tracing presets file opened (File › Open, a drop, or the file picked for Import… on the web):
/// it imports as Import… does, over the dialog open now (the manager selects the first preset it
/// added), and asks whether to replace saved presets of the file's names.
pub(crate) fn open_file(app: &mut VectorcraftApp, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > TRACE_PRESET_MAX_BYTES {
        return Err(too_big());
    }
    let file = json!({ "dataBase64": vectorcraft_format::base64_encode(bytes) });
    let (imported, skipped) = import_file(app, &file)?;
    let mut under = app.ui.dialog.take();
    if let (Some(d), Some(first)) = (under.as_mut().filter(|d| d.kind == KIND), imported.get(0).and_then(Value::as_str)) {
        select(d, first);
    }
    app.ui.dialog = if skipped.is_empty() { under } else { Some(ask_replace(under.as_ref(), skipped, file)) };
    Ok(())
}

/// Replace (the question's OK): the file imports again, replacing the saved presets of its names,
/// and the dialog it was asked over shows again (the manager on the first of them).
fn confirm_replace(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut p = d.fields.get("file").cloned().filter(Value::is_object).ok_or("no tracing presets file")?;
    p["replace"] = json!(true);
    let r = app.run("imageTrace.presets.import", p)?;
    let mut back = super::parent(d);
    if let (Some(b), Some(first)) = (back.as_mut().filter(|b| b.kind == KIND), d.fields.get("names").and_then(|n| n.get(0)).and_then(Value::as_str)) {
        select(b, first);
    }
    app.ui.dialog = back;
    Ok(r)
}

/// The preset editor over the manager `d`: on the saved preset `name` (`edit`), else on a new
/// preset starting from the preset `name`, under the first free "Tracing Preset N".
fn editor(app: &mut VectorcraftApp, d: &Dialog, name: &str, edit: bool) -> Result<Dialog, String> {
    let (editing, new_name, params) = if edit {
        let saved = app.session.trace_preset(name).ok_or_else(|| format!("no saved preset named `{name}`"))?;
        (saved.name.clone(), saved.name.clone(), serde_json::to_value(&saved.params).map_err(|e| e.to_string())?)
    } else {
        let params = app.session.execute("imageTrace.presets", &json!({})).map_err(|e| e.to_string())?["presets"]
            .as_array()
            .and_then(|all| {
                let found = |p: &&Value| p["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(name));
                all.iter().find(|p| p["builtIn"] != false && found(p)).or_else(|| all.iter().find(found)).map(|p| p["params"].clone())
            })
            .ok_or_else(|| format!("no preset named `{name}`"))?;
        (String::new(), app.session.new_trace_preset_name(), params)
    };
    let under = serde_json::to_value(d).map_err(|e| e.to_string())?;
    Ok(Dialog::new(PRESET_KIND, json!({ "name": new_name, "params": params, EDITING: editing, super::PARENT: under })))
}

fn preset_body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Name:"));
        form::text(ui, d, "name", 240.0);
    });
    ui.add_space(8.0);
    let mut params = d.fields.get("params").cloned().filter(Value::is_object).unwrap_or_else(|| json!({}));
    if settings_editor(app, ui, "tracing-preset", &mut params) {
        d.fields.insert("params".into(), params);
    }
    false
}

/// Save Preset (the editor's OK): `imageTrace.presets.save`, then back to the manager on the
/// preset. A preset may not take the name of a built-in preset, of Custom or of another saved
/// preset: the editor stays open to change it.
fn confirm_preset(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let (name, editing) = (d.str("name"), d.str(EDITING));
    let name = name.trim();
    let params = d.fields.get("params").cloned().unwrap_or(Value::Null);
    let p = if editing.is_empty() {
        if app.session.trace_preset(name).is_some() {
            return Err(format!("a preset named `{name}` exists"));
        }
        // The settings are whole: a new preset starts from none of the saved ones.
        json!({ "name": name, "preset": "Default", "params": params })
    } else {
        json!({ "name": editing, "newName": name, "params": params })
    };
    let r = app.run("imageTrace.presets.save", p)?;
    let mut back = super::parent(d).filter(|p| p.kind == KIND).unwrap_or_else(|| Dialog::new(KIND, json!({})));
    select(&mut back, r["name"].as_str().unwrap_or_default());
    app.ui.dialog = Some(back);
    Ok(r)
}

/// The tracing settings `params` as the Image Trace panel shows them (mode, palette, the count or
/// threshold, Document Library's library and colors, and the Advanced settings), changed by the user
/// → whether they changed.
fn settings_editor(app: &VectorcraftApp, ui: &mut egui::Ui, id: &str, params: &mut Value) -> bool {
    let mut changed = false;
    let mode = params["mode"].as_str().unwrap_or("blackAndWhite").to_string();
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Mode:"));
        let label = MODES.iter().find(|m| m.0 == mode).map_or("Black and White", |m| m.1);
        let labels: Vec<&str> = MODES.iter().map(|m| m.1).collect();
        if let Some(i) = widgets::dropdown(ui, (id, "mode"), label, &labels, 170.0) {
            if MODES[i].0 != "color" {
                params["colorGroup"] = Value::Null;
            }
            params["mode"] = json!(MODES[i].0);
            changed = true;
        }
    });
    let palette = params["palette"].as_str().unwrap_or("limited").to_string();
    if mode == "color" {
        ui.horizontal(|ui| {
            widgets::dim_label(ui, tl!("Palette:"));
            let labels = PALETTES.map(image_trace::palette_label);
            if let Some(p) =
                widgets::dropdown_names(ui, (id, "palette"), image_trace::palette_label(&palette), &labels, 170.0).and_then(|i| PALETTES.get(i))
            {
                if *p != "documentLibrary" {
                    params["colorGroup"] = Value::Null;
                }
                params["palette"] = json!(p);
                changed = true;
            }
        });
        if palette == "documentLibrary" {
            let key = params["library"].as_str().unwrap_or(DOCUMENT_SWATCHES).to_string();
            ui.horizontal(|ui| {
                widgets::dim_label(ui, tl!("Library:"));
                let label = swatches::library_label(app, &key);
                if let Some(k) = widgets::combo(ui, (id, "library"), &label, 170.0, |ui| swatches::library_list(app, ui, &key)) {
                    if k != DOCUMENT_SWATCHES {
                        params["colorGroup"] = Value::Null;
                    }
                    params["library"] = json!(k);
                    changed = true;
                }
            });
            // The document's swatches: every one, or one color group's (kept by name, so a preset
            // names a group the open document may not have).
            if key == DOCUMENT_SWATCHES {
                ui.horizontal(|ui| {
                    widgets::dim_label(ui, tl!("Colors:"));
                    let group = params["colorGroup"].as_str().filter(|g| !g.is_empty()).map(str::to_string);
                    let label = group.clone().unwrap_or_else(|| tl!("All").to_string());
                    if let Some(g) =
                        widgets::combo(ui, (id, "color-group"), &label, 170.0, |ui| image_trace::color_group_list(app, ui, group.as_deref()))
                    {
                        params["colorGroup"] = json!(g);
                        changed = true;
                    }
                });
            }
        }
    }
    let (key, label, range, suffix) = match mode.as_str() {
        "blackAndWhite" => ("threshold", tl!("Threshold"), 0.0..=255.0, ""),
        "grayscale" => ("colors", tl!("Grays"), 2.0..=256.0, ""),
        "logo" => ("colors", tl!("Colors"), 2.0..=10.0, ""),
        _ if matches!(palette.as_str(), "fullTone" | "automatic") => ("colorDetail", tl!("Colors"), 0.0..=100.0, "%"),
        _ => ("colors", tl!("Colors"), 2.0..=256.0, ""),
    };
    let mut slider = |params: &mut Value, key: &str, label: &str, range: std::ops::RangeInclusive<f64>, suffix: &str, whole: bool| {
        let mut v = params[key].as_f64().unwrap_or(0.0);
        if image_trace::slider(ui, label, &mut v, range, suffix).0 {
            params[key] = if whole { json!(v.round() as u64) } else { json!(v.round()) };
            changed = true;
        }
    };
    slider(params, key, label, range, suffix, key != "colorDetail");
    for (key, label, range, suffix) in
        [("paths", tl!("Paths"), 0.0..=100.0, "%"), ("corners", tl!("Corners"), 0.0..=100.0, "%"), ("noise", tl!("Noise"), 1.0..=100.0, " px")]
    {
        slider(params, key, label, range, suffix, key == "noise");
    }
    if mode != "logo" {
        ui.horizontal(|ui| {
            widgets::dim_label(ui, tl!("Method:"));
            for (m, l) in [("abutting", tl!("Abutting")), ("overlapping", tl!("Overlapping"))] {
                if ui.selectable_label(params["method"] == m, l).clicked() && params["method"] != m {
                    params["method"] = json!(m);
                    changed = true;
                }
            }
        });
        // Create: Fills and/or Strokes (one of them stays on), and the widest line stroked.
        let create = |p: &Value, key: &str| p[key].as_bool().unwrap_or(key == "fills");
        ui.horizontal(|ui| {
            widgets::dim_label(ui, tl!("Create:"));
            for (key, other, label) in [("fills", "strokes", tl!("Fills")), ("strokes", "fills", tl!("Strokes"))] {
                let on = create(params, key);
                if widgets::check(ui, label, on, !on || create(params, other)) {
                    params[key] = json!(!on);
                    changed = true;
                }
            }
        });
        let mut v = params["strokeWidth"].as_f64().unwrap_or(10.0);
        let strokes = create(params, "strokes");
        if ui.add_enabled_ui(strokes, |ui| image_trace::slider(ui, tl!("Stroke"), &mut v, 1.0..=100.0, " px")).inner.0 {
            params["strokeWidth"] = json!(v.round());
            changed = true;
        }
    }
    let gradients = mode == "grayscale" || mode == "color";
    for (key, label, enabled) in [
        ("snapCurvesToLines", tl!("Snap Curves To Lines"), true),
        ("shapes", tl!("Shapes"), true),
        ("autoGrouping", tl!("Auto Grouping"), true),
        ("gradients", tl!("Gradients"), gradients),
    ] {
        let on = params[key].as_bool().unwrap_or(false);
        if widgets::check(ui, label, on, enabled) {
            params[key] = json!(!on);
            changed = true;
        }
    }
    ui.horizontal(|ui| {
        let on = params["ignoreWhite"].as_bool().unwrap_or(false);
        if widgets::check(ui, tl!("Ignore Color"), on, true) {
            params["ignoreWhite"] = json!(!on);
            changed = true;
        }
        if on {
            let mut rgb = image_trace::ignore_color(params);
            if ui.color_edit_button_srgb(&mut rgb).changed() {
                params["ignoreColor"] = json!(format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]));
                changed = true;
            }
        }
    });
    changed
}

#[cfg(test)]
mod tests;
