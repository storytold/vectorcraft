//! HP-GL Options (Export As → HP-GL): the language version, the curve tolerance, where the plot's
//! origin lies, the pens (one per color the art draws in, or a pen table of colors), and the order
//! the paths are drawn in (the nearest path next, inner paths first).
//!
//! Fields: `document.export` params (`format: hpgl`, `path?`, the artboard choice) and the HP-GL
//! options (`hpglVersion`, `curveTolerance`, `origin`, `pens`, `sortPaths`, `insideFirst`). OK
//! remembers the options for next time and writes the file (asking where when there is no path);
//! an option or artboard choice the export refuses keeps the dialog open, and a save panel that closes
//! without a file opens it again. A canceled save panel keeps the options remembered before. Opened by Export
//! Selection, it checks and writes the selection alone. A typed Curve Tolerance is kept within 0.01
//! to 10 points. OK after another document has become the active one closes the dialog and writes
//! nothing.

use craft::behavior::IconId;
use craft::theme::variant::Tone;
use craft::theme::{Rgba, Srgba};
use craft::ui::buttons::{Button, IconButton};
use craft::ui::choice::{Radio, RadioGroup, Swatch};
use craft::ui::contract::{Bind, Politeness, Props as _, Scroll, Show as _, ShowContainer as _};
use craft::ui::dialog::DialogId;
use craft::ui::display::Text;
use craft::ui::layout_components::Stack;
use craft::ui::notices::{Notice, NoticeLayout};
use craft::ui::patterns::form_dialogs::{CustomContext, DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog};
use craft::ui::text::TextField;
use serde_json::{Map, Value, json};
use vectorcraft_cad::hpgl::write::{DEFAULT_TOLERANCE, MAX_PENS, TOLERANCE_RANGE};
use vectorcraft_cad::hpgl::{Origin, Version};
use vectorcraft_engine::cmd::fileio;

use super::dxf_options::{check_param, choice_param};
use super::export_host::{self, After, Export, Open};
use super::form;
use crate::{VectorcraftApp, io};

pub(super) const KIND: &str = "hpglOptions";

/// Fields that are not remembered between exports (they belong to one export).
const PER_EXPORT: [&str; 7] = ["format", "path", "useArtboards", "range", "artboard", "artboards", "selectedOnly"];

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

/// The rows.
fn rows() -> Vec<Param> {
    vec![
        choice_param("hpglVersion", tl!("Version:"), Version::ALL.iter().map(|v| (v.id(), v.label()))),
        Param::number("curveTolerance", tl!("Curve Tolerance:"), DEFAULT_TOLERANCE, 0.1, 2)
            .bounds(Some(*TOLERANCE_RANGE.start()), Some(*TOLERANCE_RANGE.end())),
        choice_param("origin", tl!("Origin:"), Origin::ALL.iter().map(|o| o.id()).zip(ORIGIN_NAMES)),
        pens_param(),
        check_param("sortPaths", tl!("Draw the Nearest Path Next")),
        check_param("insideFirst", tl!("Draw Inner Paths First")),
    ]
}

/// Open HP-GL Options for an export whose params (path, artboard choice, any HP-GL option) are
/// `params`, over the options used last.
pub fn open(app: &mut VectorcraftApp, params: &Value) {
    open_with(app, params, false);
}

/// [`open`] for Export Selection when `selection` is set: the options then plot the selection alone.
pub fn open_with(app: &mut VectorcraftApp, params: &Value, selection: bool) {
    let Some(doc) = app.session.active().map(|st| st.uid) else { return };
    // Opened afresh, it no longer waits to show again after a save panel.
    app.export_dialogs.1 = None;
    // Export Selection's flag in `params` says what the options plot: it is no option.
    let mut params = params.clone();
    let asked = params.as_object_mut().and_then(|o| o.remove(io::EXPORT_SELECTION));
    let selection = selection || asked == Some(json!(true));
    let mut fields = defaults();
    for given in [&app.ui.hpgl_options, &params] {
        if let Some(o) = given.as_object() {
            fields.extend(o.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
    }
    fields.insert("format".into(), json!("hpgl"));
    show(app, doc, selection, form::fields_of(&rows(), &fields));
}

/// HP-GL Options closed for the save panel its OK asked for: the document and the values it showed.
pub(super) struct Asked {
    doc: u64,
    selection: bool,
    fields: DialogFields,
}

/// The save panel `asked` waited on closed without a file: the dialog shows again with its values, unless another
/// document has become the active one.
pub(super) fn reopen(app: &mut VectorcraftApp, asked: Asked) {
    if app.session.active().map(|st| st.uid) == Some(asked.doc) {
        show(app, asked.doc, asked.selection, asked.fields);
    }
}

/// Shows HP-GL Options for document `doc` with `values`.
fn show(app: &mut VectorcraftApp, doc: u64, selection: bool, values: DialogFields) {
    let ctx = app.ui_ctx.clone().unwrap_or_default();
    let dialog = HostedParamDialog::new(&ctx, KIND, values, |fields| {
        ParamDialog::new(KIND, tl!("HP-GL Options"), rows()).values(Bind::bound(fields)).ok_label(tl!("Export").into())
    });
    export_host::open(app, Export::Hpgl { doc }, selection, dialog);
}

/// The pen table `fields` hold: `None` for One Pen per Color.
fn table(fields: &DialogFields) -> Option<Vec<Value>> {
    match fields.get("pens")? {
        FieldValue::Json(Value::Array(a)) => Some(a.clone()),
        FieldValue::List(l) => Some(l.iter().filter_map(|v| serde_json::to_value(v).ok()).collect()),
        _ => None,
    }
}

/// The key a pen's picked colour comes back under.
fn pen_key(i: usize) -> String {
    format!("__penColor{i}")
}

/// Pens: One Pen per Color (`pens` null) or Pen Table (`pens` the colors, pen 1's first), with a
/// row per pen (its number, color and a Remove button) and Add.
fn pens_param() -> Param {
    Param::custom("pens", tl!("Pens:"), |ui, fields, cx| ui.vertical(|ui| pens_ui(ui, fields, cx)).response)
        .companions([SET_ASIDE, PEN_ERROR])
        .full_width()
}

/// The pens row. Switching to Pen Table asks the host (`penTable`): the table starts from the document's pens, which
/// this draw cannot reach.
fn pens_ui(ui: &mut egui::Ui, fields: &mut DialogFields, cx: &mut CustomContext) {
    let mut list = table(fields);
    let mut changed = false;
    // Colours the picker answered.
    if let Some(pens) = list.as_mut() {
        for (i, pen) in pens.iter_mut().enumerate() {
            if let Some([r, g, b]) = fields.color(&pen_key(i)) {
                let byte = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
                *pen = json!(format!("#{:02x}{:02x}{:02x}", byte(r), byte(g), byte(b)));
                fields.remove(&pen_key(i));
                changed = true;
            }
        }
    }
    let mut on = list.is_some();
    RadioGroup::new(tl!("Pens:"), Bind::bound(&mut on)).show(ui, |ui, _| {
        Radio::new(false).label(tl!("One Pen per Color")).show(ui);
        Radio::new(true).label(tl!("Pen Table")).show(ui);
    });
    if on != list.is_some() {
        if on {
            cx.request_action("penTable");
        } else {
            let _ = set_pen_table(fields, false, || Ok(Vec::new()));
            fields.remove(PEN_ERROR);
            list = None;
        }
    }
    if let Some(e) = fields.text(PEN_ERROR) {
        Notice::new(Tone::Bad, crate::i18n::message(crate::i18n::current(), e).to_string())
            .layout(NoticeLayout::Inline)
            .announce(Politeness::Assertive)
            .show(ui);
    }
    // A table longer than a plot takes (set through `ui.dialog.set`) draws no rows: OK refuses it.
    let Some(mut pens) = list.filter(|a| a.len() <= MAX_PENS) else { return };
    let n = pens.len();
    let mut remove = None;
    Stack::new().scroll(Scroll::vertical()).room_height(180.0).show(ui, |ui, _| {
        for (i, pen) in pens.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                Text::new((i + 1).to_string()).show(ui);
                let mut color = pen.as_str().and_then(Rgba::from_hex).unwrap_or(Rgba::rgb(0, 0, 0));
                if Swatch::new(tl!("Pen color"), Srgba::from(color)).show(ui).clicked {
                    cx.pick_color(&pen_key(i), Srgba::from(color));
                }
                let before = color;
                TextField::hex(tl!("Pen color"), Bind::bound(&mut color)).id_salt(("hpgl-pen", i)).show(ui);
                if color != before
                    && let Some(typed) = pen_color(&color.to_hex())
                {
                    *pen = json!(typed);
                    changed = true;
                }
                if IconButton::new(IconId::new("trash")).label(tl!("Remove")).enabled(n > 1).show(ui).clicks > 0 {
                    remove = Some(i);
                }
            });
        }
    });
    if Button::new(tl!("Add")).enabled(n < MAX_PENS).show(ui).clicks > 0 {
        // A new pen starts as the last one's color, ready to be changed.
        pens.push(pens.last().cloned().unwrap_or_else(|| json!("#000000")));
        changed = true;
    }
    if let Some(i) = remove.filter(|_| pens.len() > 1) {
        pens.remove(i);
        changed = true;
    }
    if changed {
        fields.set("pens", FieldValue::Json(Value::Array(pens)));
    }
}

/// Starts Pen Table for dialog `id`: the table the export would give the art, or the reason it cannot. It runs from
/// the host's pump, which holds no lock on the dialog while the document is read.
pub(super) fn pen_ask(app: &mut VectorcraftApp, id: DialogId, selection: bool) {
    let Some(dialogs) = app.dialogs() else { return };
    let Some(mut copy) = dialogs.with(|h| h.get_by_id::<HostedParamDialog>(id).map(|d| d.fields().clone())) else { return };
    let shown = copy.clone();
    let result = set_pen_table(&mut copy, true, || automatic_pens(app, &shown, selection));
    dialogs.with(|h| {
        let Some(dialog) = h.get_mut_by_id::<HostedParamDialog>(id) else { return };
        match result {
            Ok(()) => {
                for key in ["pens", SET_ASIDE] {
                    match copy.get(key) {
                        Some(v) => dialog.set_field(key, v.clone()),
                        None => dialog.clear_field(key),
                    }
                }
                dialog.clear_field(PEN_ERROR);
            }
            Err(e) => dialog.set_field(PEN_ERROR, FieldValue::Text(e)),
        }
    });
}

/// The pens an export from a dialog holding `fields` with no pen table would get: its options, artboard choice and
/// source art (the selection with Export Selected Art Only, or when Export Selection opened the dialog); an error
/// when there is no document or the export refuses those.
pub(super) fn automatic_pens(app: &VectorcraftApp, fields: &DialogFields, selection: bool) -> Result<Vec<String>, String> {
    let st = app.session.active().ok_or("no document")?;
    let (doc, params) = source(st, fields, selection)?;
    fileio::hpgl::automatic_pens(&doc, &params)
}

/// The document an export from a dialog holding `fields` writes and the export params it reads, as
/// [`io::export`] takes them ([`io::take_selection`]): with Export Selection the selection alone,
/// without the artboard choice, else the whole document or, with Export Selected Art Only, its
/// selected objects.
fn source<'a>(
    st: &'a vectorcraft_engine::DocState,
    fields: &DialogFields,
    selection: bool,
) -> Result<(std::borrow::Cow<'a, vectorcraft_doc::Document>, Value), String> {
    let params = Value::Object(form::json_of(&rows(), fields));
    let (selection, params) = io::take_selection(&super::export_as::exported(selection, params))?;
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
pub(super) fn set_pen_table(fields: &mut DialogFields, on: bool, automatic: impl FnOnce() -> Result<Vec<String>, String>) -> Result<(), String> {
    let table = table(fields);
    match (on, table) {
        (true, None) => {
            let kept = fields.get(SET_ASIDE).and_then(|v| serde_json::to_value(v).ok()).filter(|v| v.as_array().is_some_and(|a| !a.is_empty()));
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
            fields.remove(SET_ASIDE);
            fields.set("pens", FieldValue::Json(list));
        }
        (false, Some(list)) => {
            fields.set(SET_ASIDE, FieldValue::Json(Value::Array(list)));
            fields.set("pens", FieldValue::Json(Value::Null));
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

/// Check the options, remember them, and write the file(s). When another document has become the
/// active one since the dialog opened, it closes as Cancel does and writes nothing.
pub(super) fn confirm(app: &mut VectorcraftApp, open: &Open, fields: &DialogFields) -> Result<After, String> {
    let Export::Hpgl { doc } = &open.kind else { return Err("this is not an HP-GL export".into()) };
    if Some(*doc) != app.session.active().map(|st| st.uid) {
        return Ok(None);
    }
    let map = form::json_of(&rows(), fields);
    let params = Value::Object(map.clone());
    let st = app.session.active().ok_or("no document")?;
    // A bad option or artboard choice, nothing selected, or no art to export keeps the dialog open.
    let (source, checked) = source(st, fields, open.selection)?;
    fileio::hpgl::check(&source, &checked)?;
    let remembered = Value::Object(map.into_iter().filter(|(k, _)| !PER_EXPORT.contains(&k.as_str())).collect());
    let exported = super::export_as::exported(open.selection, params);
    let f = fileio::format("hpgl").ok_or("no HP-GL encoder")?;
    let path = fields.text("path").map(str::to_string);
    // The options are remembered once the file is known (a save panel answers later); a panel that closes without
    // a file shows the dialog again (`export_host::pump`), and the options remembered before stay.
    app.export_dialogs.1 = Some(Asked { doc: *doc, selection: open.selection, fields: fields.clone() });
    let r = io::export_path(app, path, f, move |app, path| {
        app.export_dialogs.1 = None;
        let before = std::mem::replace(&mut app.ui.hpgl_options, remembered);
        let r = io::export(app, Some("hpgl"), Some(path), &exported);
        if r.is_err() {
            app.ui.hpgl_options = before;
        }
        r
    });
    if r.is_err() {
        app.export_dialogs.1 = None;
    }
    r.map(|_| None)
}
