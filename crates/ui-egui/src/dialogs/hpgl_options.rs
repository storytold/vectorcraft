//! HP-GL Options (Export As → HP-GL): the language version, the curve tolerance, where the plot's
//! origin lies, the pens (one per color the art draws in, or a pen table of colors), and the order
//! the paths are drawn in (the nearest path next, inner paths first).
//!
//! Fields: `document.export` params (`format: hpgl`, `path?`, the artboard choice) and the HP-GL
//! options (`hpglVersion`, `curveTolerance`, `origin`, `pens`, `sortPaths`, `insideFirst`). OK
//! remembers the options for next time and writes the file (asking where when there is no path);
//! an option or artboard choice the export refuses, or a save panel shown in line that is canceled,
//! keeps the dialog open. A canceled save panel keeps the options remembered before. Opened by Export
//! Selection, it checks and writes the selection alone. A typed Curve Tolerance is kept within 0.01
//! to 10 points.

use serde_json::{Map, Value, json};
use vectorcraft_cad::hpgl::write::{DEFAULT_TOLERANCE, MAX_PENS, TOLERANCE_RANGE};
use vectorcraft_cad::hpgl::{Origin, Version};
use vectorcraft_engine::cmd::fileio;

use super::DialogSpec;
use super::document_setup::{LABEL, check, choice};
use crate::state::Dialog;
use crate::{VectorcraftApp, io, widgets};

pub(super) const KIND: &str = "hpglOptions";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("HP-GL Options").into(),
    body,
    confirm,
    ok: Some("Export"),
    min_width: 420.0,
    max_width: Some(460.0),
    inputs: |_| &["path"],
    ..DialogSpec::FORM
};

/// Fields that are not remembered between exports (they belong to one export).
const PER_EXPORT: [&str; 8] = ["format", "path", "useArtboards", "range", "artboard", "artboards", "selectedOnly", io::EXPORT_SELECTION];

/// The pen table set aside while One Pen per Color is chosen, so Pen Table shows it again.
const SET_ASIDE: &str = "__pens";

/// Why Pen Table could not start from the art's pens, shown under the Pens row.
const PEN_ERROR: &str = "__penError";

/// The Origin menu's names, in [`Origin::ALL`] order.
pub(crate) const ORIGIN_NAMES: [&str; 2] = ["Bottom Left", "Center"];

/// The options a first HP-GL Options dialog starts from (the export's defaults).
fn defaults() -> Map<String, Value> {
    let mut m = Map::new();
    for (k, v) in [
        ("hpglVersion", json!(Version::default().id())),
        ("curveTolerance", json!(DEFAULT_TOLERANCE)),
        ("origin", json!(Origin::default().id())),
        ("pens", Value::Null),
        ("sortPaths", json!(false)),
        ("insideFirst", json!(false)),
    ] {
        m.insert(k.into(), v);
    }
    m
}

/// Open HP-GL Options for an export whose params (path, artboard choice, any HP-GL option) are
/// `params`, over the options used last.
pub fn open(app: &mut VectorcraftApp, params: &Value) {
    let mut fields = defaults();
    for given in [&app.ui.hpgl_options, params] {
        if let Some(o) = given.as_object() {
            fields.extend(o.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
    }
    fields.insert("format".into(), json!("hpgl"));
    app.ui.dialog = Some(Dialog::new(KIND, Value::Object(fields)));
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let versions: Vec<(&str, &str)> = Version::ALL.iter().map(|v| (v.id(), v.label())).collect();
    choice(ui, d, "hpglVersion", tl!("Version:"), &versions);
    ui.add_space(4.0);
    widgets::label_row(ui, tl!("Curve Tolerance:"), LABEL, |ui| {
        let x = d.f64("curveTolerance", DEFAULT_TOLERANCE);
        if let Some(v) = widgets::plain_field(ui, "hpgl-tolerance", x, " pt", 2, 80.0) {
            let (lo, hi) = (*TOLERANCE_RANGE.start(), *TOLERANCE_RANGE.end());
            d.fields.insert("curveTolerance".into(), json!(v.clamp(lo, hi)));
        }
    });
    ui.add_space(4.0);
    let origins: Vec<(&str, &str)> = Origin::ALL.iter().map(|o| o.id()).zip(ORIGIN_NAMES).collect();
    choice(ui, d, "origin", tl!("Origin:"), &origins);
    ui.add_space(10.0);
    pens(app, ui, d);
    ui.add_space(8.0);
    check(ui, d, "sortPaths", tl!("Draw the Nearest Path Next"));
    check(ui, d, "insideFirst", tl!("Draw Inner Paths First"));
    false
}

/// Pens: One Pen per Color (`pens` null) or Pen Table (`pens` the colors, pen 1's first), with a
/// row per pen (its number, color and a Remove button) and Add.
fn pens(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    let table = d.fields.get("pens").is_some_and(Value::is_array);
    widgets::label_row(ui, tl!("Pens:"), LABEL, |ui| {
        if widgets::radio(ui, tl!("One Pen per Color"), !table, true) {
            let _ = set_pen_table(d, false, || Ok(Vec::new()));
            d.fields.remove(PEN_ERROR);
        }
        ui.add_space(12.0);
        if widgets::radio(ui, tl!("Pen Table"), table, true) {
            let shown = d.clone();
            match set_pen_table(d, true, || automatic_pens(app, &shown)) {
                Ok(()) => d.fields.remove(PEN_ERROR),
                Err(e) => d.fields.insert(PEN_ERROR.into(), json!(e)),
            };
        }
    });
    if let Some(e) = d.fields.get(PEN_ERROR).and_then(Value::as_str) {
        let t = crate::theme::Tokens::get(ui.ctx());
        ui.label(egui::RichText::new(crate::i18n::message(crate::i18n::current(), e)).color(t.error));
    }
    // The table as the radio buttons left it, so a row edit committed this frame can't undo a switch.
    // A table longer than a plot takes (set through `ui.dialog.set`) draws no rows: OK refuses it.
    let Some(mut list) = d.fields.get("pens").and_then(Value::as_array).filter(|a| a.len() <= MAX_PENS).cloned() else { return };
    let t = crate::theme::Tokens::get(ui.ctx());
    let n = list.len();
    let (mut changed, mut remove) = (false, None);
    egui::ScrollArea::vertical().id_salt("hpgl-pens").max_height(180.0).auto_shrink([false, true]).show(ui, |ui| {
        for (i, v) in list.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.add_space(LABEL + 8.0);
                ui.add_sized([28.0, 18.0], egui::Label::new(egui::RichText::new((i + 1).to_string()).color(t.text)));
                let hex = v.as_str().and_then(|s| s.strip_prefix('#')).unwrap_or_default().to_string();
                let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 18.0), egui::Sense::hover());
                if let Some([r, g, b]) = rgb(&hex) {
                    ui.painter().rect_filled(rect, 2.0, egui::Color32::from_rgb(r, g, b));
                }
                ui.painter().rect_stroke(rect, 2.0, egui::Stroke::new(1.0, t.input_border), egui::StrokeKind::Inside);
                if let Some(typed) = widgets::hex_field(ui, ("hpgl-pen", i), &hex)
                    && let Some(color) = pen_color(&typed)
                {
                    *v = json!(color);
                    changed = true;
                }
                if widgets::icon_button_enabled(ui, "x", tl!("Remove"), false, n > 1, 22.0).clicked() {
                    remove = Some(i);
                }
            });
        }
    });
    ui.horizontal(|ui| {
        ui.add_space(LABEL + 8.0);
        if ui.add_enabled(n < MAX_PENS, |ui: &mut egui::Ui| widgets::flat_button(ui, tl!("Add"), 60.0)).clicked() {
            // A new pen starts as the last one's color, ready to be changed.
            list.push(list.last().cloned().unwrap_or_else(|| json!("#000000")));
            changed = true;
        }
    });
    if let Some(i) = remove.filter(|_| list.len() > 1) {
        list.remove(i);
        changed = true;
    }
    if changed {
        d.fields.insert("pens".into(), Value::Array(list));
    }
}

/// The pens an export from dialog `d` with no pen table would get: its options, artboard choice and
/// source art (the selection with Export Selected Art Only, or when Export Selection opened `d`);
/// an error when there is no document or the export refuses those.
pub(super) fn automatic_pens(app: &VectorcraftApp, d: &Dialog) -> Result<Vec<String>, String> {
    let st = app.session.active().ok_or("no document")?;
    let (doc, params) = source(st, d)?;
    fileio::hpgl::automatic_pens(&doc, &params)
}

/// The document an export from dialog `d` writes and the export params it reads, as
/// [`io::export`] takes them ([`io::take_selection`]): with Export Selection the selection alone,
/// without the artboard choice, else the whole document or, with Export Selected Art Only, its
/// selected objects.
fn source<'a>(st: &'a vectorcraft_engine::DocState, d: &Dialog) -> Result<(std::borrow::Cow<'a, vectorcraft_doc::Document>, Value), String> {
    let (selection, params) = io::take_selection(&super::export_as::exported(d, super::form::params(d)))?;
    if !selection {
        let doc = fileio::export_source(st, &params).map_err(|e| e.to_string())?;
        return Ok((doc, params));
    }
    let (doc, _) = fileio::selection_doc(st, "document.exportSelection", false).map_err(|e| e.to_string())?;
    Ok((std::borrow::Cow::Owned(doc), params))
}

/// Choose Pen Table (`on`) or One Pen per Color. Pen Table shows the table set aside when One Pen
/// per Color was chosen, else the pens the export would give the art (`automatic`), else one black
/// pen when that art draws nothing; when `automatic` fails the dialog is left as it was and the
/// error returned. One Pen per Color sets the table aside.
pub(super) fn set_pen_table(d: &mut Dialog, on: bool, automatic: impl FnOnce() -> Result<Vec<String>, String>) -> Result<(), String> {
    let table = d.fields.get("pens").filter(|v| v.is_array()).cloned();
    match (on, table) {
        (true, None) => {
            let kept = d.fields.get(SET_ASIDE).filter(|v| v.as_array().is_some_and(|a| !a.is_empty())).cloned();
            let list = match kept {
                Some(list) => list,
                None => {
                    let mut pens = automatic()?;
                    pens.truncate(MAX_PENS);
                    if pens.is_empty() {
                        pens.push("#000000".into());
                    }
                    json!(pens)
                }
            };
            d.fields.remove(SET_ASIDE);
            d.fields.insert("pens".into(), list);
        }
        (false, Some(list)) => {
            d.fields.insert(SET_ASIDE.into(), list);
            d.fields.insert("pens".into(), Value::Null);
        }
        _ => {}
    }
    Ok(())
}

/// A typed pen color (`ff0000` or `#FF0000`) as the table holds it, `#ff0000`; `None` unless it is
/// six hex digits.
fn pen_color(typed: &str) -> Option<String> {
    let s = typed.trim();
    let s = s.strip_prefix('#').unwrap_or(s);
    rgb(s).map(|_| format!("#{}", s.to_ascii_lowercase()))
}

/// Six hex digits as a color.
fn rgb(hex: &str) -> Option<[u8; 3]> {
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| hex.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok());
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// Check the options, remember them, and write the file(s).
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let params = super::form::params(d);
    let st = app.session.active().ok_or("no document")?;
    // A bad option or artboard choice, nothing selected, or no art to export keeps the dialog open.
    let (source, checked) = source(st, d)?;
    fileio::hpgl::check(&source, &checked)?;
    let remembered = params.as_object().map(|o| o.iter().filter(|(k, _)| !PER_EXPORT.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())));
    let before = std::mem::replace(&mut app.ui.hpgl_options, Value::Object(remembered.into_iter().flatten().collect()));
    app.ui.dialog = None;
    let path = d.fields.get("path").and_then(Value::as_str).map(str::to_string);
    let waiting = app.picks.is_waiting();
    let r = io::export(app, Some("hpgl"), path, &super::export_as::exported(d, params));
    if !waiting && app.picks.is_waiting() {
        // A save panel shown off the UI thread confirms the dialog again when it answers, which
        // remembers the options then; until it does, and when it is canceled, the options
        // remembered before stay.
        app.ui.hpgl_options = before;
    } else if r.is_err() && app.ui.dialog.is_none() {
        // An export that stops before writing (a save panel that was canceled or could not be
        // shown) leaves the dialog open over the options remembered before. A question that a
        // save panel asked stays open.
        app.ui.hpgl_options = before;
        app.ui.dialog = Some(d.clone());
    }
    r
}
