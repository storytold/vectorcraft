//! File → Print: the Print dialog. The print preset (and Save Preset…) and the printer (and
//! Setup…) on top, then the sections General,
//! Marks and Bleed, Output, Graphics, Color Management, Advanced and Summary, with a preview of the
//! printed page under the section list: the arrows step through the pages, dragging the art moves
//! its placement on the paper.
//!
//! The fields are the print settings (`print.setup`'s; sections are objects such as `marks`) plus
//! `printer` (a name from `print.printers`, "" the system's default), `toFile` (save the job as a
//! PDF instead) and UI-only `__` keys, so agents fill the dialog with `ui.dialog.set`. Print (OK)
//! keeps the settings with the document (`print.setup`) and prints them (`file.print`); Done
//! (`discard: true`, then confirm) only keeps them. The preview, the inks and the Summary come
//! from `print.preview`: the dialog lays nothing out itself.
//!
//! `preset` names the print preset whose settings were loaded last: setting it (the list, or
//! `ui.dialog.set`) loads that preset's settings. Save Preset… names the settings as a preset
//! (`print.presets.save`). The same dialog edits a preset (kind `printPreset`, from Edit → Print
//! Presets): a `name` instead of the preset and printer, and OK saves the preset.

use std::sync::{Arc, LazyLock};

use serde_json::{Value, json};
use vectorcraft_color::cms::Intent;
use vectorcraft_geom::Rect as DRect;
use vectorcraft_pdf::{
    Choice, Emulsion, FontDownload, Media, Orientation, Origin, OutputMode, PrintArtboards, PrintImage, PrintLayers, PrintOverprints, PrintScaling,
    PrintSettings,
};

use super::save_pdf::{TOP_LABEL_WIDTH, choice, current_section, flag, get, heading, length, note, number, pick, row, row_with, set, text};
use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::theme::{self, Tokens};
use crate::{VectorcraftApp, widgets};
use vectorcraft_engine::cmd::printpresets::DEFAULT_PRESET;

pub(super) const KIND: &str = "print";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Print").into(), body, confirm, ok: Some("Print"), discard: Some("Done"), ..DialogSpec::FORM };

/// The dialog kind of the preset editor (New / Edit in Edit → Print Presets).
pub(super) const PRESET_KIND: &str = "printPreset";

pub(super) const PRESET_SPEC: DialogSpec = DialogSpec {
    heading: |d| if d.str(EDITING).is_empty() { tl!("New Print Preset") } else { tl!("Edit Print Preset") }.into(),
    body,
    confirm: confirm_preset,
    ok: Some("Save Preset"),
    ..DialogSpec::FORM
};

/// The preset editor's field holding the name of the saved preset it edits (empty: a new one).
const EDITING: &str = "__editing";
/// The field holding the name typed for Save Preset… (present while that row shows).
const SAVE_AS: &str = "__savePresetAs";
/// The preset whose settings were loaded last (`preset` loads its settings when it differs).
const LOADED: &str = "__loaded";

pub(super) const SECTIONS: [&str; 7] = ["General", "Marks and Bleed", "Output", "Graphics", "Color Management", "Advanced", "Summary"];

/// The default print settings as JSON: what a field an agent left out reads as.
static DEFAULTS: LazyLock<Value> = LazyLock::new(|| serde_json::to_value(PrintSettings::default()).unwrap_or_default());

/// The defaults of the settings a dialog of `kind` edits, when it is a Print dialog.
pub(super) fn defaults(kind: &str) -> Option<&'static Value> {
    (kind == KIND || kind == PRESET_KIND).then(|| &*DEFAULTS)
}

/// Set when the Print Tiling tool placed the pages (`tileOrigin`): the placement is ignored.
const PLACED: &str = "tileOrigin.placed";

/// The fields that aren't print settings.
const NOT_SETTINGS: [&str; 5] = ["printer", "toFile", "discard", "preset", "name"];

/// The Printer list's entries besides the system's printers.
const DEFAULT_PRINTER: &str = "Default Printer";
const PDF_FILE: &str = "PDF File";

/// The preview's size in the section column.
const PREVIEW: egui::Vec2 = egui::vec2(super::save_pdf::LIST_WIDTH - 12.0, 168.0);

/// Open the Print dialog on the document's print settings and the system's default printer.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document")?;
    // The settings saved with the document, every field filled in (defaults if never set up).
    let saved: PrintSettings = st.doc.print_setup.clone().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
    let mut fields = fields_of(&saved)?;
    let printers = crate::print::printers(app);
    let list = printers["printers"].as_array().map(Vec::as_slice).unwrap_or_default();
    let default = list.iter().find(|p| p["default"] == true).or(list.first()).and_then(|p| p["name"].as_str()).unwrap_or_default();
    fields.insert("printer".into(), json!(default));
    fields.insert("toFile".into(), json!(!printers["service"].as_bool().unwrap_or(false)));
    fields.insert("__printers".into(), printers);
    fields.insert("__section".into(), json!(SECTIONS[0]));
    fields.insert("__sheet".into(), json!(0));
    // The saved preset these settings are, if any (else [Default], shown as [Custom] if they differ).
    let preset = app.session.prefs.print_presets.iter().find(|p| p.settings == saved).map_or(DEFAULT_PRESET, |p| p.name.as_str());
    fields.insert("preset".into(), json!(preset));
    fields.insert(LOADED.into(), json!(preset));
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields });
    Ok(Value::Null)
}

/// The settings as dialog fields.
fn fields_of(set: &PrintSettings) -> Result<serde_json::Map<String, Value>, String> {
    match serde_json::to_value(set).map_err(|e| e.to_string())? {
        Value::Object(fields) => Ok(fields),
        _ => Err("print settings are not an object".into()),
    }
}

/// Load the settings of the preset `preset` names once it changed (the list, or `ui.dialog.set`).
fn sync_preset(app: &VectorcraftApp, d: &mut Dialog) -> Result<(), String> {
    let name = d.str("preset");
    if d.kind != KIND || name == d.str(LOADED) {
        return Ok(());
    }
    d.fields.insert(LOADED.into(), json!(name));
    let set = app.session.print_preset_settings(&name).ok_or_else(|| format!("no print preset named `{name}`"))?;
    d.fields.extend(fields_of(&set)?);
    Ok(())
}

/// Save Preset…: save the settings under the typed name and pick that preset.
fn save_preset(app: &mut VectorcraftApp, d: &mut Dialog) -> Result<Value, String> {
    let r = app.run("print.presets.save", json!({ "name": d.str(SAVE_AS), "preset": DEFAULT_PRESET, "settings": settings(d) }))?;
    d.fields.remove(SAVE_AS);
    d.fields.insert("preset".into(), r["name"].clone());
    d.fields.insert(LOADED.into(), r["name"].clone());
    Ok(r)
}

/// Open the preset editor on the saved preset `name`, or on a new preset starting from `preset`
/// (default: [Default]).
pub fn open_preset(app: &mut VectorcraftApp, params: &Value) -> Result<Value, String> {
    let s = |k: &str| params.get(k).and_then(Value::as_str);
    let (base, name, editing) = match s("name") {
        Some(name) => {
            let p = app.session.prefs.print_presets.iter().find(|p| p.name.eq_ignore_ascii_case(name.trim()));
            let p = p.ok_or_else(|| format!("no saved print preset named `{name}` ({DEFAULT_PRESET} is protected: start a new one from it)"))?;
            (p.name.clone(), p.name.clone(), p.name.clone())
        }
        None => (s("preset").unwrap_or(DEFAULT_PRESET).to_string(), app.session.new_print_preset_name(), String::new()),
    };
    let set = app.session.print_preset_settings(&base).ok_or_else(|| format!("no print preset named `{base}`"))?;
    let mut fields = fields_of(&set)?;
    fields.insert("name".into(), json!(name));
    fields.insert(EDITING.into(), json!(editing));
    fields.insert("__section".into(), json!(SECTIONS[0]));
    fields.insert("__sheet".into(), json!(0));
    app.ui.dialog = Some(Dialog { kind: PRESET_KIND.into(), fields });
    Ok(Value::Null)
}

/// Save Preset (the preset editor's OK): `print.presets.save`, then back to Edit → Print Presets on
/// it. A new preset may not take the name of another one.
fn confirm_preset(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let (name, editing) = (d.str("name"), d.str(EDITING));
    let p = if editing.is_empty() {
        if app.session.print_preset_settings(&name).is_some() {
            return Err(format!("a preset named `{}` exists", name.trim()));
        }
        json!({ "name": name, "preset": DEFAULT_PRESET, "settings": settings(d) })
    } else {
        json!({ "name": editing, "newName": name, "settings": settings(d) })
    };
    let r = app.run("print.presets.save", p)?;
    super::print_presets::open(app, r["name"].as_str());
    Ok(r)
}

/// The print settings the dialog stands for (`print.setup`'s `settings`).
pub(super) fn settings(d: &Dialog) -> Value {
    let mut p = form::params(d);
    if let Some(o) = p.as_object_mut() {
        for k in NOT_SETTINGS {
            o.remove(k);
        }
    }
    p
}

/// Print, or Done (`discard`): keep the settings with the document (one undo step when they
/// changed), then print them; settings that can't print keep the dialog open, unchanged. While
/// Save Preset… asks for a name, it saves the preset instead.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut d = d.clone();
    // Done's flag holds for this press only: the dialog may stay open.
    let done = d.fields.remove("discard").is_some_and(|v| v == true);
    let synced = sync_preset(app, &mut d);
    app.ui.dialog = Some(d.clone());
    synced?;
    // While Save Preset… asks for a name, OK (and Enter) saves the preset instead.
    if d.fields.contains_key(SAVE_AS) {
        let r = save_preset(app, &mut d);
        app.ui.dialog = Some(d);
        return r;
    }
    let settings = settings(&d);
    let p = json!({ "settings": settings });
    // Laid out first, so a bad range or overlap is refused before anything is kept.
    app.session.execute("print.preview", &p).map_err(|e| e.to_string())?;
    app.run("print.setup", p)?;
    if done {
        app.ui.dialog = None;
        return Ok(json!({ "settings": settings }));
    }
    let r = crate::print::run(app, &json!({ "settings": settings, "printer": d.str("printer"), "toFile": d.bool("toFile") }))?;
    app.ui.dialog = None;
    Ok(r)
}

// ---------- the dialog ----------

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    if let Err(e) = sync_preset(app, d) {
        app.status(e);
    }
    if d.kind == PRESET_KIND {
        row_with(ui, tl!("Preset name:"), TOP_LABEL_WIDTH, |ui| {
            form::text_edit(ui, d, "name", 288.0);
        });
    } else {
        preset_rows(app, ui, d);
        printer_row(app, ui, d);
    }
    ui.add_space(10.0);
    let section = current_section(d, &SECTIONS);
    let preview = preview_of(app, ui.ctx(), &settings(d));
    ui.horizontal_top(|ui| {
        super::save_pdf::section_frame(ui, |ui| {
            super::save_pdf::section_list(ui, d, &SECTIONS, section);
            ui.add_space(10.0);
            page_preview(app, ui, d, &preview);
        });
        ui.add_space(14.0);
        ui.vertical(|ui| {
            ui.set_width(500.0);
            ui.label(egui::RichText::new(tl!(section)).font(theme::semibold(14.0)).color(t.text_strong));
            egui::ScrollArea::vertical().id_salt(("print", section)).max_height(360.0).auto_shrink([false, false]).show(ui, |ui| match section {
                "Marks and Bleed" => super::save_pdf::marks_and_bleeds(app, ui, d),
                "Output" => output(ui, d, &preview),
                "Graphics" => graphics(ui, d),
                "Color Management" => color(ui, d),
                "Advanced" => advanced(app, ui, d),
                "Summary" => summary(ui, d, &preview, app.session.active().is_some()),
                _ => general(app, ui, d),
            });
        });
    });
    false
}

/// The Print Preset row ([Default], the saved presets; [Custom] once the settings differ from the
/// preset's) with Save Preset…, and while Save Preset… asks for a name, the row taking it.
fn preset_rows(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    const CUSTOM: &str = "[Custom]";
    let name = d.str("preset");
    let current = serde_json::from_value::<PrintSettings>(settings(d)).ok();
    let same = current.is_some() && current == app.session.print_preset_settings(&name);
    let names: Vec<&str> = std::iter::once(DEFAULT_PRESET).chain(app.session.prefs.print_presets.iter().map(|p| p.name.as_str())).collect();
    let mut picked = None;
    let asking = d.fields.contains_key(SAVE_AS);
    let mut ask = false;
    row_with(ui, tl!("Print Preset:"), TOP_LABEL_WIDTH, |ui| {
        // [Default] is ours (translated); the saved presets are names.
        let current = if same { name.as_str() } else { tl!(CUSTOM) };
        picked = super::mixed_dropdown(ui, "print-preset", current, &names, 300.0, |k| k == 0).and_then(|i| names.get(i)).map(|n| n.to_string());
        let save = ui.add_enabled_ui(!asking, |ui| widgets::flat_button(ui, tl!("Save Preset…"), 96.0)).inner;
        ask = save.on_hover_text(tl!("Save these settings as a print preset")).clicked();
    });
    if let Some(n) = picked {
        // Picking the preset shown again brings its settings back.
        d.fields.insert(LOADED.into(), json!(""));
        d.fields.insert("preset".into(), json!(n));
        if let Err(e) = sync_preset(app, d) {
            app.status(e);
        }
    }
    if ask {
        d.fields.insert(SAVE_AS.into(), json!(app.session.new_print_preset_name()));
    }
    if d.fields.contains_key(SAVE_AS) {
        row_with(ui, tl!("Save as preset:"), TOP_LABEL_WIDTH, |ui| {
            form::text_edit(ui, d, SAVE_AS, 200.0);
            if widgets::flat_button(ui, tl!("Save"), 52.0).clicked()
                && let Err(e) = save_preset(app, d)
            {
                app.status(e);
            }
            if widgets::flat_button(ui, tl!("Cancel"), 60.0).clicked() {
                d.fields.remove(SAVE_AS);
            }
        });
    }
}

/// What the Printer list offers.
enum Destination {
    Printer(String),
    /// The system's default printer (the web: the browser's print dialog).
    Default,
    File,
}

/// The Printer row: the system's printers (or its default one), PDF File; Setup….
fn printer_row(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    let printers = d.fields.get("__printers");
    let names: Vec<String> = printers
        .and_then(|p| p["printers"].as_array())
        .map(|a| a.iter().filter_map(|p| p["name"].as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let service = printers.and_then(|p| p["service"].as_bool()).unwrap_or(false);
    let setup = printers.and_then(|p| p["setup"].as_bool()).unwrap_or(false);
    let mut entries: Vec<Destination> = names.iter().cloned().map(Destination::Printer).collect();
    if service && names.is_empty() {
        entries.push(Destination::Default);
    }
    entries.push(Destination::File);
    // The system's printers by their names; the default printer and PDF File are ours.
    let options: Vec<&str> = entries
        .iter()
        .map(|e| match e {
            Destination::Printer(name) => name.as_str(),
            Destination::Default => tl!(DEFAULT_PRINTER),
            Destination::File => tl!(PDF_FILE),
        })
        .collect();
    let printer = d.str("printer");
    let current = if d.bool("toFile") {
        tl!(PDF_FILE)
    } else if printer.is_empty() {
        tl!(DEFAULT_PRINTER)
    } else {
        printer.as_str()
    };
    row_with(ui, tl!("Printer:"), TOP_LABEL_WIDTH, |ui| {
        match widgets::dropdown_names(ui, "print-printer", current, &options, 300.0).and_then(|i| entries.get(i)) {
            Some(Destination::Printer(name)) => {
                d.fields.insert("printer".into(), json!(name));
                d.fields.insert("toFile".into(), json!(false));
            }
            Some(Destination::Default) => {
                d.fields.insert("printer".into(), json!(""));
                d.fields.insert("toFile".into(), json!(false));
            }
            Some(Destination::File) => {
                d.fields.insert("toFile".into(), json!(true));
            }
            None => {}
        }
        let to_file = d.bool("toFile");
        let tip = if setup { tl!("The system's settings of this printer") } else { tl!("No printer settings to open here") };
        if ui.add_enabled_ui(setup && !to_file, |ui| widgets::flat_button(ui, tl!("Setup…"), 72.0)).inner.on_hover_text(tip).clicked()
            && let Err(e) = app.run("print.printerSetup", json!({ "printer": d.str("printer") }))
        {
            app.status(e);
        }
    });
}

/// `print.preview` of `settings` (an error when they can't print), cached until they or the
/// document change.
fn preview_of(app: &mut VectorcraftApp, ctx: &egui::Context, settings: &Value) -> Arc<Result<Value, String>> {
    let doc = app.session.active().map_or((0, 0), |s| (s.uid, s.revision));
    let stamp = egui::Id::new((settings.to_string(), doc));
    let key = egui::Id::new("print-preview");
    if let Some((s, v)) = ctx.data(|m| m.get_temp::<(egui::Id, Arc<Result<Value, String>>)>(key))
        && s == stamp
    {
        return v;
    }
    let v = Arc::new(app.session.execute("print.preview", &json!({ "settings": settings })).map_err(|e| e.to_string()));
    ctx.data_mut(|m| m.insert_temp(key, (stamp, v.clone())));
    v
}

/// A `[x0, y0, x1, y1]` array as a rectangle.
fn rect_of(v: &Value) -> Option<DRect> {
    let n = |i: usize| v.get(i).and_then(Value::as_f64);
    Some(DRect::new(n(0)?, n(1)?, n(2)?, n(3)?))
}

/// The page `__sheet` shows (clamped to the pages there are) and how many there are.
fn current_sheet<'a>(d: &Dialog, pv: &'a Value) -> Option<(usize, usize, &'a Value)> {
    let sheets = pv["sheets"].as_array()?;
    let i = (d.f64("__sheet", 0.0).max(0.0) as usize).min(sheets.len().checked_sub(1)?);
    Some((i, sheets.len(), sheets.get(i)?))
}

/// The preview: the page with its imageable area (dashed) and the art as it prints; ◀ ▶ step
/// through the pages; dragging the art moves the placement (not when tiling).
fn page_preview(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog, pv: &Result<Value, String>) {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(PREVIEW, egui::Sense::drag());
    ui.painter().rect_filled(rect, 3.0, t.pasteboard);
    let sheet = pv.as_ref().ok().and_then(|v| current_sheet(d, v));
    let Some((index, count, sheet)) = sheet else {
        let text = match pv {
            _ if app.session.active().is_none() => tl!("Open a document to preview its pages"),
            Ok(_) => tl!("Nothing to print"),
            Err(_) => tl!("Can't print these settings: see the Summary"),
        };
        let font = egui::FontId::proportional(11.0);
        let galley = ui.painter().layout(text.into(), font, t.text_dim, rect.width() - 16.0);
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, t.text_dim);
        return;
    };
    let size = egui::vec2(sheet["width"].as_f64().unwrap_or(0.0) as f32, sheet["height"].as_f64().unwrap_or(0.0) as f32);
    let fit = ((rect.width() - 16.0) / size.x).min((rect.height() - 16.0) / size.y);
    if !(fit.is_finite() && fit > 0.0) {
        return;
    }
    let page = egui::Rect::from_center_size(rect.center(), size * fit);
    let to_screen = |x: f64, y: f64| page.min + egui::vec2(x as f32, y as f32) * fit;
    let painter = ui.painter();
    painter.rect_filled(page.translate(egui::vec2(2.0, 2.0)), 0.0, egui::Color32::from_black_alpha(60));
    painter.rect_filled(page, 0.0, egui::Color32::WHITE);
    // Dragging moves the art (and its trim box) on the paper; the placement changes on release.
    let tiling = choice::<PrintScaling>(d, "scaling").is_some_and(PrintScaling::tiles);
    let drag_id = egui::Id::new("print-preview-drag");
    let mut offset = ui.ctx().data(|m| m.get_temp::<egui::Vec2>(drag_id)).unwrap_or_default();
    if !tiling && resp.dragged() {
        offset += resp.drag_delta();
        ui.ctx().data_mut(|m| m.insert_temp(drag_id, offset));
    }
    let coeffs: Vec<f64> = sheet["transform"].as_array().map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
    let clip = ui.painter().with_clip_rect(page);
    if let ([a, b, c, dd, e, f], Some(area)) = (coeffs.as_slice(), rect_of(&sheet["area"])) {
        let px = (f64::from(page.width().max(page.height()) * ui.ctx().pixels_per_point())).min(1024.0);
        let layers = choice::<PrintLayers>(d, "printLayers").unwrap_or(PrintLayers::VisiblePrintable);
        if let Some(tex) = art_texture(app, ui.ctx(), layers, area, px) {
            let at = |x: f64, y: f64| to_screen(a * x + c * y + e, b * x + dd * y + f) + offset;
            let mut mesh = egui::Mesh::with_texture(tex.id());
            for (p, uv) in [
                ((area.x0, area.y0), (0.0, 0.0)),
                ((area.x1, area.y0), (1.0, 0.0)),
                ((area.x1, area.y1), (1.0, 1.0)),
                ((area.x0, area.y1), (0.0, 1.0)),
            ] {
                mesh.vertices.push(egui::epaint::Vertex { pos: at(p.0, p.1), uv: egui::pos2(uv.0, uv.1), color: egui::Color32::WHITE });
            }
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            clip.add(mesh);
        }
    }
    if resp.drag_stopped() && offset != egui::Vec2::ZERO {
        move_placement(d, sheet, (f64::from(offset.x / fit), f64::from(offset.y / fit)));
    }
    if !resp.dragged() {
        ui.ctx().data_mut(|m| m.remove::<egui::Vec2>(drag_id));
    }
    // The imageable area inside the device's margin, and the artboard's edge.
    let margin = get(d, "margin").as_f64().unwrap_or(0.0) as f32 * fit;
    if margin > 0.5 {
        let r = page.shrink(margin);
        let stroke = egui::Stroke::new(1.0, t.text_dim);
        let corners = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
        clip.extend(egui::Shape::dashed_line(&corners, stroke, 3.0, 2.0));
    }
    if let Some(trim) = rect_of(&sheet["trim"]) {
        let r = egui::Rect::from_min_max(to_screen(trim.x0, trim.y0), to_screen(trim.x1, trim.y1)).translate(offset);
        clip.rect_stroke(r, 0.0, egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Middle);
    }
    let resp = if tiling { resp } else { resp.on_hover_cursor(egui::CursorIcon::Grab) };
    resp.on_hover_text(if tiling { tl!("The page as it prints") } else { tl!("Drag the art to move it on the paper") });
    // ◀ n of N ▶, then what the page is.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        if widgets::icon_button_enabled(ui, "chevron-left", tl!("Previous page"), false, index > 0, 20.0).clicked() {
            d.fields.insert("__sheet".into(), json!(index - 1));
        }
        let text = crate::i18n::fmt(tl!("{page} of {count}"), &[("page", &(index + 1).to_string()), ("count", &count.to_string())]);
        ui.add_sized([PREVIEW.x - 44.0, 20.0], egui::Label::new(egui::RichText::new(text).size(11.5).color(t.text)));
        if widgets::icon_button_enabled(ui, "chevron-right", tl!("Next page"), false, index + 1 < count, 20.0).clicked() {
            d.fields.insert("__sheet".into(), json!(index + 1));
        }
    });
    let mut what = vec![];
    match sheet["artboard"].as_u64() {
        Some(a) => what.push(crate::i18n::fmt(tl!("Artboard {number}"), &[("number", &(a + 1).to_string())])),
        None => what.push(tl!("All the art").into()),
    }
    if let Some(tile) = sheet["tile"].as_u64() {
        what.push(crate::i18n::fmt(tl!("tile {number}"), &[("number", &tile.to_string())]));
    }
    if let Some(ink) = sheet["ink"].as_str() {
        what.push(ink.to_string());
    }
    let unit = app.session.general_unit();
    let scale = sheet["scale"].as_array().map(|s| s.iter().filter_map(Value::as_f64).collect::<Vec<_>>()).unwrap_or_default();
    let scale = match scale.as_slice() {
        [w, h] if (w - h).abs() < 1e-6 => format!("{}%", round2(*w)),
        [w, h] => format!("{}% × {}%", round2(*w), round2(*h)),
        _ => String::new(),
    };
    let paper = format!("{} × {}", unit.number(f64::from(size.x)), unit.format(f64::from(size.y)));
    for line in [what.join(" · "), format!("{paper} · {scale}")] {
        ui.label(egui::RichText::new(line).size(11.0).color(t.text_dim));
    }
}

/// Move the placement so the art of `sheet` moves `(dx, dy)` points on the page: the move turned
/// back onto the paper (the page may be turned or mirrored), at the art's scale.
pub(super) fn move_placement(d: &mut Dialog, sheet: &Value, (dx, dy): (f64, f64)) {
    let n = |k: &str, i: usize| sheet[k].get(i).and_then(Value::as_f64);
    let (Some(a), Some(b), Some(c), Some(e)) = (n("transform", 0), n("transform", 1), n("transform", 2), n("transform", 3)) else { return };
    let (Some(sx), Some(sy)) = (n("scale", 0), n("scale", 1)) else { return };
    let (sx, sy) = (sx / 100.0, sy / 100.0);
    if !(sx > 0.0 && sy > 0.0) {
        return;
    }
    // The transform's linear part is the page's turn times the scale, and the turn's inverse is
    // its transpose: the columns over the scale.
    let (px, py) = ((a * dx + b * dy) / sx, (c * dx + e * dy) / sy);
    if get(d, PLACED).as_bool() == Some(true) {
        // Pages the Print Tiling tool placed: their corner moves the other way over the art.
        let x = get(d, "tileOrigin.x").as_f64().unwrap_or(0.0) - px / sx;
        let y = get(d, "tileOrigin.y").as_f64().unwrap_or(0.0) - py / sy;
        set(d, "tileOrigin.x", json!(round2(x)));
        set(d, "tileOrigin.y", json!(round2(y)));
        return;
    }
    let x = get(d, "placement.x").as_f64().unwrap_or(0.0) + px;
    let y = get(d, "placement.y").as_f64().unwrap_or(0.0) + py;
    set(d, "placement.x", json!(round2(x)));
    set(d, "placement.y", json!(round2(y)));
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// The art of `area` as it prints (only the layers that print), `px` pixels on its longest side;
/// cached until the document, the layers or the size change.
fn art_texture(app: &mut VectorcraftApp, ctx: &egui::Context, layers: PrintLayers, area: DRect, px: f64) -> Option<egui::TextureHandle> {
    let st = app.session.active()?;
    let bits = [area.x0, area.y0, area.x1, area.y1].map(f64::to_bits);
    let stamp = egui::Id::new((st.uid, st.revision, layers, bits, px.round() as u32));
    let key = egui::Id::new("print-preview-art");
    if let Some((s, tex)) = ctx.data(|m| m.get_temp::<(egui::Id, egui::TextureHandle)>(key))
        && s == stamp
    {
        return Some(tex);
    }
    let mut doc = (*st.doc).clone();
    vectorcraft_pdf::keep_layers(&mut doc.layers, layers);
    let tex = widgets::region_texture(ctx, &mut app.canvas.renderer, "print-preview-art", &doc, area, px);
    ctx.data_mut(|m| m.insert_temp(key, (stamp, tex.clone())));
    Some(tex)
}

// ---------- sections ----------

/// A whole number at `path` within `range`.
fn whole(ui: &mut egui::Ui, d: &mut Dialog, path: &str, range: std::ops::RangeInclusive<f64>, enabled: bool) {
    let v = get(d, path).as_f64().unwrap_or(*range.start());
    ui.add_enabled_ui(enabled, |ui| {
        if let Some(x) = widgets::range_field(ui, path, v, range, "", 0, 56.0) {
            set(d, path, json!(x as u64));
        }
    });
}

fn general(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    let unit = app.session.general_unit();
    row(ui, tl!("Copies:"), |ui| {
        whole(ui, d, "copies", 1.0..=f64::from(vectorcraft_pdf::MAX_COPIES), true);
        ui.add_space(12.0);
        let copies = get(d, "copies").as_u64().unwrap_or(1);
        flag(ui, d, "collate", tl!("Collate"), copies > 1);
        flag(ui, d, "reverse", tl!("Reverse Order"), true);
    });
    heading(ui, tl!("Artboards"));
    let artboards = choice::<PrintArtboards>(d, "artboards").unwrap_or(PrintArtboards::All);
    ui.horizontal(|ui| {
        for (a, label) in [(PrintArtboards::All, tl!("All")), (PrintArtboards::Range, tl!("Range:"))] {
            if widgets::radio(ui, label, artboards == a, true) {
                set(d, "artboards", json!(a.id()));
            }
        }
        let mut range = get(d, "range").as_str().unwrap_or_default().to_string();
        let edit = egui::TextEdit::singleline(&mut range).desired_width(140.0).hint_text("1-3, 5");
        if ui.add_enabled(artboards == PrintArtboards::Range, edit).changed() {
            set(d, "range", json!(range));
        }
    });
    if widgets::radio(ui, tl!("Ignore Artboards (all the art as one page)"), artboards == PrintArtboards::Ignore, true) {
        set(d, "artboards", json!(PrintArtboards::Ignore.id()));
    }
    flag(ui, d, "skipBlank", tl!("Skip Blank Artboards"), artboards != PrintArtboards::Ignore);
    heading(ui, tl!("Media"));
    row(ui, tl!("Size:"), |ui| {
        pick::<Media>(ui, d, "media", 160.0, |_| true);
    });
    // A paper size shows its size; Custom takes one.
    let media = choice::<Media>(d, "media").unwrap_or_default();
    row(ui, tl!("Width:"), |ui| match media.size() {
        Some((w, h)) => {
            ui.add_enabled_ui(false, |ui| widgets::num_field(ui, "print-media-w", Some(w), unit, 80.0));
            ui.label(tl!("Height:"));
            ui.add_enabled_ui(false, |ui| widgets::num_field(ui, "print-media-h", Some(h), unit, 80.0));
        }
        None => {
            length(ui, d, "width", unit, true);
            ui.label(tl!("Height:"));
            length(ui, d, "height", unit, true);
        }
    });
    heading(ui, tl!("Orientation"));
    flag(ui, d, "autoRotate", tl!("Auto-Rotate (turn the paper to each artboard)"), true);
    let auto = get(d, "autoRotate").as_bool() == Some(true);
    row(ui, tl!("Orientation:"), |ui| {
        ui.add_enabled_ui(!auto, |ui| pick::<Orientation>(ui, d, "orientation", 160.0, |_| true));
    });
    flag(ui, d, "transverse", tl!("Transverse (the page a quarter turn on the paper)"), true);
    heading(ui, tl!("Options"));
    row(ui, tl!("Print Layers:"), |ui| {
        pick::<PrintLayers>(ui, d, "printLayers", 220.0, |_| true);
    });
    let scaling = choice::<PrintScaling>(d, "scaling").unwrap_or(PrintScaling::None);
    let tiles = scaling.tiles();
    let placed = get(d, PLACED).as_bool() == Some(true);
    row(ui, tl!("Placement:"), |ui| {
        ui.add_enabled_ui(!tiles && !placed, |ui| {
            let origin = choice::<Origin>(d, "placement.origin").unwrap_or_default();
            let current = Origin::ALL.iter().position(|o| *o == origin).unwrap_or(4);
            if let Some(o) = widgets::reference_point(ui, current).and_then(|i| Origin::ALL.get(i)) {
                set(d, "placement.origin", json!(o.id()));
            }
            ui.label("X:");
            length(ui, d, "placement.x", unit, true);
            ui.label("Y:");
            length(ui, d, "placement.y", unit, true);
        });
    });
    if placed {
        row(ui, "", |ui| {
            note(ui, tl!("The Print Tiling tool placed the pages."));
            if widgets::flat_button(ui, tl!("Reset"), 56.0).on_hover_text(tl!("Place the pages with the placement again")).clicked() {
                set(d, PLACED, json!(false));
            }
        });
    }
    row(ui, tl!("Scaling:"), |ui| {
        pick::<PrintScaling>(ui, d, "scaling", 180.0, |_| true);
    });
    row(ui, tl!("Scale:"), |ui| {
        let on = scaling == PrintScaling::Custom || tiles;
        ui.label(tl!("W:"));
        number(ui, d, "scale.width", "%", on);
        ui.label(tl!("H:"));
        number(ui, d, "scale.height", "%", on);
    });
    row(ui, tl!("Overlap:"), |ui| length(ui, d, "overlap", unit, tiles));
    row(ui, tl!("Tile Range:"), |ui| text(ui, d, "tileRange", tiles));
    if tiles {
        note(ui, tl!("Tiles are numbered across, then down; leave the range empty for every tile."));
    }
}

fn output(ui: &mut egui::Ui, d: &mut Dialog, pv: &Result<Value, String>) {
    row(ui, tl!("Mode:"), |ui| {
        pick::<OutputMode>(ui, d, "output.mode", 160.0, |_| true);
    });
    row(ui, tl!("Emulsion:"), |ui| {
        pick::<Emulsion>(ui, d, "output.emulsion", 160.0, |_| true);
    });
    row(ui, tl!("Image:"), |ui| {
        pick::<PrintImage>(ui, d, "output.image", 160.0, |_| true);
    });
    let separations = choice::<OutputMode>(d, "output.mode") == Some(OutputMode::Separations);
    flag(ui, d, "output.spotsToProcess", tl!("Convert All Spot Colors to Process"), separations);
    heading(ui, tl!("Document Ink Options"));
    let inks = pv.as_ref().ok().and_then(|v| v["inks"].as_array()).map(Vec::as_slice).unwrap_or_default();
    if !separations || inks.is_empty() {
        note(ui, tl!("Separations list the document's inks here: one page per ink that prints."));
        return;
    }
    let t = Tokens::get(ui.ctx());
    egui::Grid::new("print-inks").num_columns(4).spacing([12.0, 4.0]).show(ui, |ui| {
        for h in [tl!("Print"), tl!("Ink"), tl!("Frequency"), tl!("Angle")] {
            ui.label(egui::RichText::new(h).size(11.5).color(t.text_dim));
        }
        ui.end_row();
        for ink in inks {
            let name = ink["name"].as_str().unwrap_or_default();
            let on = ink["print"].as_bool().unwrap_or(true);
            if widgets::check_box(ui, name, on, true) {
                set_ink(d, name, "print", json!(!on));
            }
            let spot = if ink["spot"] == true { format!(" ({})", tl!("spot")) } else { String::new() };
            ui.label(egui::RichText::new(format!("{name}{spot}")).color(t.text));
            for (k, suffix) in [("frequency", " lpi"), ("angle", "°")] {
                let v = ink[k].as_f64().unwrap_or(0.0);
                if let Some(x) = widgets::plain_field(ui, ("print-ink", name, k), v, suffix, 2, 72.0) {
                    set_ink(d, name, k, json!(x));
                }
            }
            ui.end_row();
        }
    });
    note(ui, tl!("Frequencies and angles are the output device's to apply."));
}

/// Set option `key` of ink `name` in `output.inks` (the ink gets its own entry the first time).
fn set_ink(d: &mut Dialog, name: &str, key: &str, value: Value) {
    let mut inks = get(d, "output.inks").as_array().cloned().unwrap_or_default();
    match inks.iter_mut().find(|i| i["name"] == name) {
        Some(ink) => ink[key] = value,
        None => {
            let mut ink = json!({ "name": name, "print": true });
            ink[key] = value;
            inks.push(ink);
        }
    }
    set(d, "output.inks", Value::Array(inks));
}

fn graphics(ui: &mut egui::Ui, d: &mut Dialog) {
    heading(ui, tl!("Paths"));
    flag(ui, d, "graphics.autoFlatness", tl!("Automatic flatness"), true);
    let auto = get(d, "graphics.autoFlatness").as_bool() == Some(true);
    row(ui, tl!("Flatness:"), |ui| {
        number(ui, d, "graphics.flatness", "", !auto);
        ui.label(egui::RichText::new(tl!("0.2 (quality) to 100 (speed)")).size(11.5).color(Tokens::get(ui.ctx()).text_dim));
    });
    heading(ui, tl!("Fonts"));
    row(ui, tl!("Download:"), |ui| {
        pick::<FontDownload>(ui, d, "graphics.fonts", 160.0, |_| true);
    });
}

fn color(ui: &mut egui::Ui, d: &mut Dialog) {
    let profiles = vectorcraft_color::cms::profiles();
    let names: Vec<&str> = std::iter::once(SAME_AS_SOURCE).chain(profiles.iter().map(|p| p.name.as_str())).collect();
    row(ui, tl!("Printer profile:"), |ui| super::save_pdf::profile_pick(ui, d, "color.profile", &names, 1, true));
    note(ui, tl!("Composite colours are converted to it; separations separate with it when it is a CMYK profile."));
    row(ui, tl!("Rendering intent:"), |ui| {
        let current = serde_json::from_value::<Intent>(get(d, "color.intent").clone()).unwrap_or_default();
        let labels: Vec<&str> = Intent::ALL.iter().map(Intent::label).collect();
        if let Some(i) = widgets::dropdown(ui, "color.intent", current.label(), &labels, 200.0).and_then(|i| Intent::ALL.get(i)) {
            set(d, "color.intent", serde_json::to_value(i).unwrap_or_default());
        }
    });
    note(ui, tl!("How colours outside the press's gamut are separated."));
    flag(ui, d, "color.preserveNumbers", tl!("Preserve CMYK Numbers"), true);
    note(ui, tl!("CMYK colours print with their own values; off, they go through the colour settings too."));
}

fn advanced(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    heading(ui, tl!("Printer"));
    row(ui, tl!("Unprintable margin:"), |ui| length(ui, d, "margin", app.session.general_unit(), true));
    note(ui, tl!("The edge of the paper the printer can't reach: the art goes inside it, and the preview shows it dashed."));
    let composite = choice::<OutputMode>(d, "output.mode") != Some(OutputMode::Separations);
    flag(ui, d, "advanced.printAsBitmap", tl!("Print as Bitmap"), composite);
    note(ui, tl!("Each page prints as one image, at the document's raster effects resolution."));
    heading(ui, tl!("Overprint and Transparency Flattener Options"));
    row(ui, tl!("Overprints:"), |ui| {
        ui.add_enabled_ui(composite, |ui| pick::<PrintOverprints>(ui, d, "advanced.overprints", 160.0, |_| true));
    });
    let presets = app.session.flattener_presets();
    let names: Vec<&str> = std::iter::once(KEEP_TRANSPARENCY).chain(presets.iter().map(|p| p.name.as_str())).collect();
    // None and the built-in presets are ours; the saved presets are names.
    let builtins = names.len().saturating_sub(app.session.prefs.flattener_presets.len());
    row(ui, tl!("Preset:"), |ui| super::save_pdf::profile_pick(ui, d, "advanced.flattenerPreset", &names, builtins, true));
    note(
        ui,
        tl!(
            "Simulate prints overprints as Overprint Preview shows them; separations always keep them. A preset flattens transparency before printing (PostScript files always flatten it)."
        ),
    );
}

/// The Printer Profile entry for no conversion.
const SAME_AS_SOURCE: &str = "Same As Source";
/// The flattener Preset entry for transparency printed as it is.
const KEEP_TRANSPARENCY: &str = "None (keep transparency)";

/// The section an option is set in, and whether its first key is the section's own object
/// (`output.mode` → Output › Mode).
pub(super) fn section_of(option: &str) -> (usize, bool) {
    match option.split('.').next().unwrap_or_default() {
        "marks" | "bleed" => (1, false),
        "output" => (2, true),
        "graphics" => (3, true),
        "color" => (4, true),
        "margin" => (5, false),
        "advanced" => (5, true),
        _ => (0, false),
    }
}

/// `output.mode` → "Output › Mode", `placement.x` → "General › Placement › X".
pub(super) fn option_label(option: &str) -> String {
    let (section, own) = section_of(option);
    let keys = option.split('.').skip(usize::from(own)).map(|k| form::humanize(k).trim_end_matches(':').to_string());
    std::iter::once(SECTIONS.get(section).copied().unwrap_or_default().to_string()).chain(keys).collect::<Vec<_>>().join(" › ")
}

/// Changed options (`{option, value}`) with their choices as the dialog names them
/// (`tileFull` → "Tile Full Pages").
pub(super) fn labelled(changed: &mut [Value]) {
    fn label<T: Choice>(id: &str) -> Option<&'static str> {
        T::IDS.iter().position(|i| *i == id).and_then(|i| T::LABELS.get(i)).copied()
    }
    for c in changed {
        let Some(id) = c["value"].as_str() else { continue };
        let named = match c["option"].as_str().unwrap_or_default() {
            "artboards" => label::<PrintArtboards>(id),
            "media" => label::<Media>(id),
            "orientation" => label::<Orientation>(id),
            "printLayers" => label::<PrintLayers>(id),
            "placement.origin" => label::<Origin>(id),
            "scaling" => label::<PrintScaling>(id),
            "marks.kind" => label::<vectorcraft_pdf::MarkKind>(id),
            "output.mode" => label::<OutputMode>(id),
            "output.emulsion" => label::<Emulsion>(id),
            "output.image" => label::<PrintImage>(id),
            "graphics.fonts" => label::<FontDownload>(id),
            "advanced.overprints" => label::<PrintOverprints>(id),
            "color.intent" => serde_json::from_value::<Intent>(c["value"].clone()).ok().map(|i| i.label()),
            _ => None,
        };
        if let Some(l) = named {
            c["value"] = json!(l);
        }
    }
}

fn summary(ui: &mut egui::Ui, d: &mut Dialog, pv: &Result<Value, String>, has_doc: bool) {
    let t = Tokens::get(ui.ctx());
    match pv {
        Ok(v) => {
            heading(ui, tl!("Pages"));
            let per_copy = v["sheets"].as_array().map_or(0, Vec::len);
            ui.label(
                egui::RichText::new(crate::i18n::fmt(
                    tl!("{pages} in all, {count} per copy"),
                    &[("pages", &v["pages"].to_string()), ("count", &per_copy.to_string())],
                ))
                .color(t.text),
            );
        }
        // A preset edited without a document has no pages to count.
        Err(_) if !has_doc => {}
        Err(e) => {
            heading(ui, tl!("Error"));
            ui.label(egui::RichText::new(format!("⚠ {e}")).color(t.text));
        }
    }
    heading(ui, tl!("Options"));
    let mut changed = vec![];
    vectorcraft_engine::cmd::fileio::pdf::changed("", &settings(d), &DEFAULTS, &mut changed);
    changed.sort_by_key(|c| section_of(c["option"].as_str().unwrap_or_default()).0);
    labelled(&mut changed);
    if changed.is_empty() {
        note(ui, tl!("Every option is at its default."));
    }
    super::save_pdf::option_rows(ui, &changed, option_label);
    if let Ok(v) = pv {
        heading(ui, tl!("Warnings"));
        super::save_pdf::warning_rows(ui, v["warnings"].as_array().map(Vec::as_slice).unwrap_or_default());
    }
}
