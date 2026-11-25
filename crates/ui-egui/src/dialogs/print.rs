//! File → Print: the Print dialog, over `craft::print::ui::HostedPrint`. The print preset and the
//! printer on top, then the sections General, Marks and Bleed, Output, Graphics, Color Management,
//! Advanced and Summary, with a preview of the printed page: the arrows step through the pages,
//! dragging the art moves its placement on the paper.
//!
//! The values are the print settings (`print.setup`'s; automation reads a section as an object such as
//! `marks`), and agents fill the dialog with `ui.dialog.set`, a row at a time by its dotted key (`marks.trim`). The printer, the copies and the like live in the
//! framework's `Settings`. Print (OK) keeps the settings with the document (`print.setup`) and
//! prints them (`file.print`); Done only keeps them. The preview, the inks and the Summary come
//! from `print.preview`: the dialog lays nothing out itself. OK is held until the pump has run the
//! checks, so a refused print leaves the dialog open with the reason.
//!
//! The same dialog edits a preset (`open_preset`, from Edit → Print Presets): a name instead of
//! the preset and printer, and OK saves the preset.

// PLAN: assumed import paths for the framework facade (`craft_ui`) and the `HostedPrint` builders
// (`layout`, `mode`, `header`, `check_name`, `held_ok`, `take_ok`, `answer_ok`, `take_setup`,
// `set_presets`, `set_message`, `set_sheet(Option<_>)`, `set_field`, `sheet_index`); export-w7 writes them.
use std::sync::{Arc, LazyLock, mpsc};

use craft::behavior::{ColumnWidth, EditEvent, EditEvents, IconId, Mixed};
use craft::print::ui::{Destination, HostedPrint, PrintAnswer, PrintFinish, PrintLayout, PrintMode, PrintSection, PrintSheet, Printer, State};
use craft::ui::buttons::ToggleButton;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{DialogId, StackDialog as _};
use craft::ui::display::{Shade, Text};
use craft::ui::layout::empty_cell;
use craft::ui::layout_components::Grid;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamKind};
use craft::ui::patterns::presets::{NamedPresetEvent, PresetError, PresetRow};
use craft::ui::pickers::{Anchor9, ReferencePoint, ReferencePointLook};
use craft::ui::primitives::IconSpec;
use craft::ui::runtime::env::{LengthUnits, UnitKind};
use craft::ui::shell::WindowDialogs;
use craft::ui::text::{LinkMode, LinkedFields, NumberField};
use serde_json::{Map, Value, json};
use vectorcraft_color::cms::Intent;
use vectorcraft_engine::cmd::printpresets::DEFAULT_PRESET;
use vectorcraft_geom::Rect as DRect;
use vectorcraft_pdf::{
    Choice, Emulsion, FontDownload, Media, Orientation, Origin, OutputMode, PrintArtboards, PrintImage, PrintLayers, PrintOverprints, PrintScaling,
    PrintSettings,
};

use super::form;
use crate::{VectorcraftApp, graphics};

pub(crate) const SECTIONS: [&str; 7] = ["General", "Marks and Bleed", "Output", "Graphics", "Color Management", "Advanced", "Summary"];

/// The default print settings as JSON: what a field an agent left out reads as.
static DEFAULTS: LazyLock<Value> = LazyLock::new(|| serde_json::to_value(PrintSettings::default()).unwrap_or_default());

/// The rows of every section: the keys `form::fields_of` and `form::json_of` map by.
static KEYS: LazyLock<Vec<Param>> = LazyLock::new(|| sections(&Lists::default()).into_iter().flat_map(|s| s.params).collect());

/// Set when the Print Tiling tool placed the pages (`tileOrigin`): the placement is ignored.
const PLACED: &str = "tileOrigin.placed";

/// The fields that aren't print settings.
const NOT_SETTINGS: [&str; 1] = ["directPdf"];

/// The Printer list's entries besides the system's printers; `PDF_FILE` is the destination that
/// saves the job as a PDF.
const DEFAULT_PRINTER: &str = "Default Printer";
const PDF_FILE: &str = "PDF File";

/// The Printer Profile entry for no conversion.
const SAME_AS_SOURCE: &str = "Same As Source";
/// The flattener Preset entry for transparency printed as it is.
const KEEP_TRANSPARENCY: &str = "None (keep transparency)";

/// The longest side, in pixels, of the picture of the art in the preview.
const ART_PX: f64 = 1024.0;

/// The nine points of the Placement picker, in `Origin::ALL`'s order.
const ANCHORS: [Anchor9; 9] = [
    Anchor9::TopLeft,
    Anchor9::Top,
    Anchor9::TopRight,
    Anchor9::Left,
    Anchor9::Centre,
    Anchor9::Right,
    Anchor9::BottomLeft,
    Anchor9::Bottom,
    Anchor9::BottomRight,
];

/// The Print dialog the app has open (`app.print_dialog`).
// PLAN: hub-w1 adds the field `print_dialog: dialogs::print::PrintDialogs` to the app; hub-w2 adds `print::pump` to `dialogs::host::pump`.
#[derive(Default)]
pub(crate) struct PrintDialogs {
    open: Option<Open>,
}

/// What the pump keeps of the open dialog: what the answer needs after the framework has answered, and the
/// texture and preview the dialog shows until the document or the settings change.
struct Open {
    id: DialogId,
    mode: PrintMode,
    /// The preset the settings were loaded from last (the Print Preset row's selection).
    preset: String,
    /// The saved preset the editor edits (empty: a new one).
    editing: String,
    /// The picture of the art, with what it was made for.
    art: Option<(ArtKey, egui::TextureHandle)>,
    /// `print.preview` of the settings and document it was run for.
    preview: Option<(Stamp, Arc<Result<Value, String>>)>,
}

/// The document (uid, revision) and the settings a cached preview belongs to.
type Stamp = (String, (u64, u64));

/// The document (uid, revision), layers, area and size a picture of the art was made for.
type ArtKey = (u64, u64, PrintLayers, [u64; 4], u32);

/// What the pump took from the dialog this pass.
struct Drained {
    refresh: bool,
    /// The preset editor's name.
    name: String,
    setup: Option<String>,
    moves: EditEvents<egui::Pos2>,
    events: Vec<NamedPresetEvent<String>>,
    ok: Option<PrintAnswer>,
    settings: Value,
    sheet: usize,
}

/// The lists the rows offer that the app holds (the profiles, the flattener presets).
#[derive(Default)]
struct Lists {
    profiles: Vec<String>,
    flatteners: Vec<String>,
}

impl Lists {
    fn of(app: &VectorcraftApp) -> Self {
        Self {
            profiles: vectorcraft_color::cms::profiles().iter().map(|p| p.name.clone()).collect(),
            flatteners: app.session.flattener_presets().iter().map(|p| p.name.clone()).collect(),
        }
    }
}

/// Open the Print dialog on the document's print settings and the system's default printer.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document")?;
    // The settings saved with the document, every field filled in (defaults if never set up).
    let saved: PrintSettings = st.doc.print_setup.clone().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
    // The saved preset these settings are, if any (else [Default], shown as [Custom] if they differ).
    let preset = app.session.prefs.print_presets.iter().find(|p| p.settings == saved).map_or(DEFAULT_PRESET, |p| p.name.as_str()).to_string();
    let fields = fields_of(&saved)?;
    start(app, PrintMode::Print, fields, preset, Default::default())
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
    start(app, PrintMode::PresetEditor, fields_of(&set)?, base, (editing, name))
}

/// Host the dialog over the settings `fields`; the preset editor edits the saved preset `editing` (empty: a new one)
/// and starts on the name `name`.
fn start(
    app: &mut VectorcraftApp,
    mode: PrintMode,
    fields: DialogFields,
    preset: String,
    (editing, name): (String, String),
) -> Result<Value, String> {
    let dialogs = app.dialogs().ok_or("no window to show the dialog in")?;
    let ctx = app.ui_ctx.clone().ok_or("no window to show the dialog in")?;
    let (printers, service) = destinations(app);
    let mut state = State::default();
    state.settings.destination = Destination::Printer(
        printers.iter().find(|p| p.default).or(printers.first().filter(|_| service)).map_or_else(|| PDF_FILE.to_string(), |p| p.name.clone()),
    );
    state.printers = printers;
    let header = if service && cfg!(windows) {
        vec![
            Param::new("directPdf", tl!("Send PDF directly (bypass driver)"), ParamKind::bool(false))
                .tooltip(tl!("Writes the print-ready PDF to the printer's TCP/IP RAW port, for PCL-only drivers that change the custom page size.")),
        ]
    } else {
        vec![]
    };
    let taken: Vec<String> = taken_names(app);
    let named = editing.clone();
    let mut hosted = HostedPrint::new(state, fields, sections(&Lists::of(app)))
        .mode(mode)
        .layout(PrintLayout::Sections)
        .header(header)
        .json_form(|fields, out| out.extend(nested(form::json_of(&KEYS, fields))))
        .check_name(move |name: &str| refusal(&taken, &named, name).map(|_| PresetError::NameTaken))
        .held_ok();
    // The name row is the dialog's own.
    if mode == PrintMode::PresetEditor {
        hosted.set_json_field(&ctx, "name", json!(name))?;
    }
    let id = dialogs.with(|h| h.open(hosted));
    app.print_dialog.open = Some(Open { id, mode, preset, editing, art: None, preview: None });
    Ok(Value::Null)
}

/// The settings as dialog fields: a section's settings (`output: {mode}`) under the rows' dotted keys
/// (`output.mode`).
pub(super) fn fields_of(set: &PrintSettings) -> Result<DialogFields, String> {
    match serde_json::to_value(set).map_err(|e| e.to_string())? {
        Value::Object(map) => {
            let mut flat = Map::new();
            flatten_into("", map, &mut flat);
            Ok(form::fields_of(&KEYS, &flat))
        }
        _ => Err("print settings are not an object".into()),
    }
}

/// `map`'s values into `out` under dotted keys below `prefix`: every object is opened up, Scale's too (its row keeps
/// `scale.width`, `scale.height` and `scale.linked`).
fn flatten_into(prefix: &str, map: Map<String, Value>, out: &mut Map<String, Value>) {
    for (k, v) in map {
        let key = if prefix.is_empty() { k } else { format!("{prefix}.{k}") };
        match v {
            Value::Object(inner) => flatten_into(&key, inner, out),
            v => {
                out.insert(key, v);
            }
        }
    }
}

/// `flat`'s dotted keys (`output.mode`) as nested objects (`output: {mode}`), as the print settings have them.
fn nested(flat: Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for (key, v) in flat {
        insert_at(&mut out, &key.split('.').collect::<Vec<_>>(), v);
    }
    out
}

/// `v` into `map` at the key path `path`, making the objects on the way.
fn insert_at(map: &mut Map<String, Value>, path: &[&str], v: Value) {
    match path {
        [] => {}
        [last] => {
            map.insert((*last).to_string(), v);
        }
        [first, rest @ ..] => {
            let slot = map.entry(*first).or_insert_with(|| Value::Object(Map::new()));
            if !slot.is_object() {
                *slot = Value::Object(Map::new());
            }
            if let Value::Object(next) = slot {
                insert_at(next, rest, v);
            }
        }
    }
}

/// The Printer list: the system's printers, the system's default printer by itself when the service names none,
/// and PDF File last; and whether there is a print service.
fn destinations(app: &mut VectorcraftApp) -> (Vec<Printer>, bool) {
    let service = app.services.print.is_some();
    let mut list: Vec<Printer> = app.services.print.as_mut().map(|s| s.printers()).unwrap_or_default();
    if service && list.is_empty() {
        list.push(Printer::new("").label(tl!(DEFAULT_PRINTER)));
    }
    list.push(Printer::new(PDF_FILE).label(tl!(PDF_FILE)));
    (list, service)
}

/// The names a new preset may not take: [Default] and the saved presets.
fn taken_names(app: &VectorcraftApp) -> Vec<String> {
    std::iter::once(DEFAULT_PRESET.to_string()).chain(app.session.prefs.print_presets.iter().map(|p| p.name.clone())).collect()
}

/// Why the preset editor may not save under `name` (a new preset, `editing` empty, may not take a taken name).
fn refusal(taken: &[String], editing: &str, name: &str) -> Option<String> {
    let name = name.trim();
    (editing.is_empty() && taken.iter().any(|t| t.eq_ignore_ascii_case(name))).then(|| format!("a preset named `{name}` exists"))
}

/// The print settings the fields stand for (`print.setup`'s `settings`).
pub(super) fn settings(fields: &DialogFields) -> Value {
    let mut p = form::json_of(&KEYS, fields);
    for k in NOT_SETTINGS {
        p.remove(k);
    }
    Value::Object(nested(p))
}

/// The work of the open dialog, once per frame: the printer list, Setup…, the preview, the placement the preview was
/// dragged to, the presets and OK.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(mut open) = app.print_dialog.open.take() else { return };
    let Some(dialogs) = app.dialogs() else {
        app.print_dialog.open = Some(open);
        return;
    };
    if serve(app, &dialogs, &mut open) {
        app.print_dialog.open = Some(open);
    }
}

/// Run `f` on the open dialog.
fn on<R>(dialogs: &WindowDialogs, f: impl FnOnce(&mut HostedPrint) -> R) -> Option<R> {
    dialogs.with(|h| h.get_mut::<HostedPrint>().map(f))
}

/// One pass of the pump over the open dialog; false once it has closed.
fn serve(app: &mut VectorcraftApp, dialogs: &WindowDialogs, open: &mut Open) -> bool {
    let Some(mut d) = on(dialogs, |p| Drained {
        refresh: p.take_refresh(),
        name: p.preset_name().to_string(),
        // Print Settings… opens the driver's dialog of the printer chosen.
        setup: if p.take_setup() { p.json_fields().and_then(|f| f["printer"].as_str().map(str::to_string)) } else { None },
        moves: p.take_moves(),
        events: p.take_preset_events(),
        ok: p.take_ok(),
        settings: settings(p.fields_mut()),
        sheet: p.sheet_index(),
    }) else {
        // Closed: its answer was acted on when the OK was held.
        let _ = dialogs.with(|h| h.take_by_id::<HostedPrint>(open.id));
        return false;
    };
    if d.refresh {
        let (list, _) = destinations(app);
        let (tx, rx) = mpsc::channel();
        // The receiver is held by the dialog below, so the send cannot fail.
        let _ = tx.send(list);
        on(dialogs, |p| p.listen(rx));
    }
    if let Some(printer) = d.setup
        && let Err(e) = app.run("print.printerSetup", json!({ "printer": printer }))
    {
        app.status(e);
    }
    for event in std::mem::take(&mut d.events) {
        apply_preset_event(app, dialogs, open, &mut d.settings, event);
    }
    let doc = app.session.active().map_or((0, 0), |s| (s.uid, s.revision));
    let stamp = (d.settings.to_string(), doc);
    let changed = open.preview.as_ref().is_none_or(|(s, _)| *s != stamp);
    if changed {
        let r = app.session.execute("print.preview", &json!({ "settings": d.settings })).map_err(|e| e.to_string());
        open.preview = Some((stamp, Arc::new(r)));
    }
    let Some(pv) = open.preview.as_ref().map(|(_, p)| p.clone()) else { return true };
    let pv: &Result<Value, String> = &pv;
    let tiling = serde_json::from_value::<PrintScaling>(d.settings["scaling"].clone()).is_ok_and(PrintScaling::tiles);
    if let (Ok(v), false) = (pv, tiling)
        && let Some(sheet) = v["sheets"].as_array().and_then(|s| s.get(d.sheet.min(s.len().saturating_sub(1))))
    {
        for delta in drags(std::mem::take(&mut d.moves)) {
            on(dialogs, |p| move_placement(p.fields_mut(), sheet, delta));
        }
    }
    let has_doc = app.session.active().is_some();
    let sheet = pv.as_ref().ok().and_then(|v| print_sheet(app, open, v, d.sheet, &d.settings));
    let message = match pv {
        _ if sheet.is_some() => None,
        _ if !has_doc => Some(tl!("Open a document to preview its pages")),
        Ok(_) => Some(tl!("Nothing to print")),
        Err(_) => Some(tl!("Can't print these settings: see the Summary")),
    };
    on(dialogs, |p| {
        p.set_sheet(sheet);
        p.set_message(message.map(Into::into));
    });
    if changed {
        let inks = pv.as_ref().ok().and_then(|v| v.get("documentInks")).cloned().unwrap_or(Value::Null);
        let [pages, options, warnings] = summary_texts(pv, &d.settings, has_doc);
        on(dialogs, |p| {
            p.set_field("__summary.pages", pages.into());
            p.set_field("__summary.options", options.into());
            p.set_field("__summary.warnings", warnings.into());
            p.set_field("__inks", FieldValue::Json(inks));
            for (key, on) in bitmap_companions(&d.settings) {
                p.set_field(key, on.into());
            }
        });
    }
    if open.mode == PrintMode::Print {
        let same = serde_json::from_value::<PrintSettings>(d.settings.clone())
            .ok()
            .is_some_and(|s| app.session.print_preset_settings(&open.preset) == Some(s));
        let held = same.then(|| open.preset.clone());
        let rows: Vec<PresetRow<String>> = std::iter::once(PresetRow::item(DEFAULT_PRESET.to_string(), tl!(DEFAULT_PRESET), 0))
            .chain(app.session.prefs.print_presets.iter().map(|p| PresetRow::item(p.name.clone(), p.name.clone(), 0)))
            .collect();
        on(dialogs, |p| p.set_presets(rows, held, vec![DEFAULT_PRESET.to_string()]));
    }
    if let Some(answer) = d.ok {
        let result = accept(app, open, &answer, &d.name);
        on(dialogs, |p| p.answer_ok(result.map_err(Into::into)));
    }
    true
}

/// A preset the Print Preset row picked or named.
fn apply_preset_event(app: &mut VectorcraftApp, dialogs: &WindowDialogs, open: &mut Open, settings: &mut Value, event: NamedPresetEvent<String>) {
    match event {
        NamedPresetEvent::Apply(key) => {
            let loaded =
                app.session.print_preset_settings(&key).ok_or_else(|| format!("no print preset named `{key}`")).and_then(|set| fields_of(&set));
            match loaded {
                Ok(new) => {
                    if let Some(s) = on(dialogs, |p| {
                        for (k, v) in new.iter() {
                            p.fields_mut().set(k.to_string(), v.clone());
                        }
                        settings_of(p)
                    }) {
                        *settings = s;
                    }
                    open.preset = key;
                }
                Err(e) => app.status(e),
            }
        }
        NamedPresetEvent::Create { name, .. } => {
            match app.run("print.presets.save", json!({ "name": name, "preset": DEFAULT_PRESET, "settings": settings })) {
                Ok(r) => open.preset = r["name"].as_str().unwrap_or(&name).to_string(),
                Err(e) => app.status(e),
            }
        }
        // Rename and Delete are the Print Presets dialog's.
        _ => {}
    }
}

fn settings_of(p: &mut HostedPrint) -> Value {
    settings(p.fields_mut())
}

/// The committed moves of the preview's art, each as the paper points it moved by; a click that moved nothing is none.
fn drags(moves: EditEvents<egui::Pos2>) -> Vec<(f64, f64)> {
    moves
        .into_iter()
        // PLAN: assumed `EditEvents: IntoIterator`; a gesture folded into one `Set` is not matched here.
        .filter_map(|e| match e {
            EditEvent::Commit { before: Mixed::Same(from), after, .. } => Some((f64::from(after.x - from.x), f64::from(after.y - from.y))),
            _ => None,
        })
        .filter(|d| *d != (0.0, 0.0))
        .collect()
}

/// Print, or Done: keep the settings with the document (one undo step when they changed), then print them; settings
/// that can't print are refused, and the dialog stays open. The preset editor saves the preset under `name` instead.
fn accept(app: &mut VectorcraftApp, open: &Open, answer: &PrintAnswer, name: &str) -> Result<(), String> {
    let settings = settings(&answer.fields);
    if open.mode == PrintMode::PresetEditor {
        return save_preset(app, open, name, settings);
    }
    // Laid out first, so a bad range or overlap is refused before anything is kept.
    app.session.execute("print.preview", &json!({ "settings": settings })).map_err(|e| e.to_string())?;
    app.run("print.setup", json!({ "settings": settings }))?;
    match answer.finish {
        PrintFinish::Print => {
            let (printer, to_file) = match &answer.settings.destination {
                Destination::Printer(name) if name != PDF_FILE => (name.as_str(), false),
                _ => ("", true),
            };
            let direct_pdf = answer.fields.bool("directPdf").unwrap_or(false);
            crate::print::run(app, &json!({ "settings": settings, "printer": printer, "toFile": to_file, "directPdf": direct_pdf })).map(|_| ())
        }
        // Done only keeps the settings.
        _ => Ok(()),
    }
}

/// Save Preset (the preset editor's OK): `print.presets.save`, then back to Edit → Print Presets on it.
fn save_preset(app: &mut VectorcraftApp, open: &Open, name: &str, settings: Value) -> Result<(), String> {
    if let Some(why) = refusal(&taken_names(app), &open.editing, name) {
        return Err(why);
    }
    let p = if open.editing.is_empty() {
        json!({ "name": name, "preset": DEFAULT_PRESET, "settings": settings })
    } else {
        json!({ "name": open.editing, "newName": name, "settings": settings })
    };
    let r = app.run("print.presets.save", p)?;
    super::print_presets::open(app, r["name"].as_str());
    Ok(())
}

/// A `[x0, y0, x1, y1]` array as a rectangle.
fn rect_of(v: &Value) -> Option<DRect> {
    let n = |i: usize| v.get(i).and_then(Value::as_f64);
    Some(DRect::new(n(0)?, n(1)?, n(2)?, n(3)?))
}

fn egui_rect(r: &DRect) -> egui::Rect {
    egui::Rect::from_min_max(egui::pos2(r.x0 as f32, r.y0 as f32), egui::pos2(r.x1 as f32, r.y1 as f32))
}

/// The sheet the preview shows: sheet `index` (clamped to the pages there are) of the `print.preview` reply `pv`.
fn print_sheet(app: &mut VectorcraftApp, open: &mut Open, pv: &Value, index: usize, settings: &Value) -> Option<PrintSheet> {
    let sheets = pv["sheets"].as_array()?;
    let sheet = sheets.get(index.min(sheets.len().checked_sub(1)?))?;
    let paper = egui::vec2(sheet["width"].as_f64().unwrap_or(0.0) as f32, sheet["height"].as_f64().unwrap_or(0.0) as f32);
    if !(paper.x.is_finite() && paper.y.is_finite() && paper.x > 0.0 && paper.y > 0.0) {
        return None;
    }
    let coeffs: Vec<f64> = sheet["transform"].as_array()?.iter().filter_map(Value::as_f64).collect();
    let [a, b, c, d, e, f] = <[f64; 6]>::try_from(coeffs).ok()?;
    let area = rect_of(&sheet["area"])?;
    let at = |x: f64, y: f64| egui::pos2((a * x + c * y + e) as f32, (b * x + d * y + f) as f32);
    let image = egui::Rect::from_points(&[at(area.x0, area.y0), at(area.x1, area.y0), at(area.x1, area.y1), at(area.x0, area.y1)]);
    let layers = serde_json::from_value::<PrintLayers>(settings["printLayers"].clone()).unwrap_or(PrintLayers::VisiblePrintable);
    let texture = art_texture(app, open, layers, area)?;
    // With copies per sheet, `copies` holds each copy's artboard on the page.
    let copies: Vec<egui::Rect> =
        sheet["copies"].as_array().map(|a| a.iter().filter_map(rect_of).map(|r| egui_rect(&r)).collect()).unwrap_or_default();
    let count = copies.len().max(1);
    let outlines = if copies.is_empty() { rect_of(&sheet["trim"]).map(|r| egui_rect(&r)).into_iter().collect() } else { copies };
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
    let size = format!("{} × {}", unit.number(f64::from(paper.x)), unit.format(f64::from(paper.y)));
    let readout = format!("{}\n{size} · {scale}", what.join(" · "));
    let mut out = PrintSheet::new(paper, image, egui::load::SizedTexture::new(texture.id(), texture.size_vec2()), readout.into())
        .count(count)
        .outlines(outlines);
    // The imageable area inside the device's margin.
    let margin = settings["margin"].as_f64().unwrap_or(0.0) as f32;
    if margin > 0.5 {
        out = out.printable(egui::Rect::from_min_max(egui::pos2(margin, margin), egui::pos2(paper.x - margin, paper.y - margin)));
    }
    Some(out)
}

/// Move the placement so the art of `sheet` moves `(dx, dy)` points on the page: the move turned
/// back onto the paper (the page may be turned or mirrored), at the art's scale.
pub(super) fn move_placement(fields: &mut DialogFields, sheet: &Value, (dx, dy): (f64, f64)) {
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
    let num = |fields: &DialogFields, k: &str| fields.number(k).unwrap_or(0.0);
    if fields.bool(PLACED) == Some(true) {
        // Pages the Print Tiling tool placed: their corner moves the other way over the art.
        let x = num(fields, "tileOrigin.x") - px / sx;
        let y = num(fields, "tileOrigin.y") - py / sy;
        fields.set("tileOrigin.x", round2(x));
        fields.set("tileOrigin.y", round2(y));
        return;
    }
    let x = num(fields, "placement.x") + px;
    let y = num(fields, "placement.y") + py;
    fields.set("placement.x", round2(x));
    fields.set("placement.y", round2(y));
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// The art of `area` as it prints (only the layers that print); kept until the document, the layers or the area change.
fn art_texture(app: &mut VectorcraftApp, open: &mut Open, layers: PrintLayers, area: DRect) -> Option<egui::TextureHandle> {
    let st = app.session.active()?;
    let key: ArtKey = (st.uid, st.revision, layers, [area.x0, area.y0, area.x1, area.y1].map(f64::to_bits), ART_PX as u32);
    if let Some((k, tex)) = &open.art
        && *k == key
    {
        return Some(tex.clone());
    }
    // The artboards' background colors print behind the art.
    let mut doc = st.doc.with_artboard_fills(0..st.doc.artboards.len()).unwrap_or_else(|| (*st.doc).clone());
    vectorcraft_pdf::keep_layers(&mut doc.layers, layers);
    let image = graphics::region_image(&doc, area, ART_PX as u32);
    let ctx = app.ui_ctx.clone()?;
    let tex = ctx.load_texture("print-preview-art", image, egui::TextureOptions::LINEAR);
    open.art = Some((key, tex.clone()));
    Some(tex)
}

// ---------- sections ----------

/// A select over the choices of `T`.
fn pick<T: Choice>(key: &str, label: &'static str) -> Param {
    Param::new(key, label, ParamKind::choice(T::IDS.iter().zip(T::LABELS).map(|(i, l)| (i.to_string(), tl!(*l).into())).collect(), None))
}

fn flag(key: &str, label: &'static str) -> Param {
    Param::new(key, label, ParamKind::bool(false))
}

fn note(key: &str, text: &'static str) -> Param {
    Param::read_only_text(key, "", text).full_width()
}

/// A length in points, shown in the document's unit.
fn length(key: &str, label: &'static str) -> Param {
    // PLAN: the shown unit is the document's general unit; the framework's env supplies it once `Param::length` is written (wg.W21).
    Param::number(key, label, 0.0, 1.0, 2).length(LengthUnits::new(UnitKind::Points, UnitKind::Points, 72.0))
}

/// The ids of the scalings that tile (`tiles`), or that don't.
fn scaling_ids(tiles: bool) -> Vec<&'static str> {
    // PLAN: assumed `PrintScaling::ALL`.
    PrintScaling::ALL.iter().filter(|s| s.tiles() == tiles).map(|s| s.id()).collect()
}

/// Set by the pump while Print as Bitmap is on for composite output: Advanced › Resolution takes input.
const BITMAP_ON: &str = "__bitmap.on";
/// Set by the pump while `advanced.bitmapResolution` is 0: the note on the raster effects resolution shows.
const BITMAP_AT_DOCUMENT: &str = "__bitmap.atDocument";

/// The values of [`BITMAP_ON`] and [`BITMAP_AT_DOCUMENT`] for the print settings `settings`.
fn bitmap_companions(settings: &Value) -> [(&'static str, bool); 2] {
    let composite = settings["output"]["mode"].as_str() != Some(OutputMode::Separations.id());
    let on = composite && settings["advanced"]["printAsBitmap"].as_bool() == Some(true);
    // 0, the default, leaves the resolution to the document.
    let at_document = settings["advanced"]["bitmapResolution"].as_f64().is_none_or(|ppi| ppi == 0.0);
    [(BITMAP_ON, on), (BITMAP_AT_DOCUMENT, at_document)]
}

/// A summary line that the pump fills (`key` is a `__` companion of the fields).
fn shown(key: &'static str) -> Param {
    Param::custom(key, "", move |ui, fields, _| ui.label(fields.text(key).unwrap_or_default()))
}

/// The sections of the dialog, with the rows of each.
fn sections(lists: &Lists) -> Vec<PrintSection> {
    let tiles = scaling_ids(true);
    let flat = scaling_ids(false);
    let separations = OutputMode::Separations.id();
    let composite: Vec<&str> = OutputMode::IDS.iter().copied().filter(|i| *i != separations).collect();
    let scaling_on: Vec<&str> = tiles.iter().copied().chain([PrintScaling::Custom.id()]).collect();
    let counts: Vec<i64> = (2..=i64::from(vectorcraft_pdf::MAX_COPIES)).collect();
    let artboards = vec![
        (PrintArtboards::All.id().to_string(), tl!("All").into()),
        (PrintArtboards::Range.id().to_string(), tl!("Range:").into()),
        (PrintArtboards::Ignore.id().to_string(), tl!("Ignore Artboards (all the art as one page)").into()),
    ];
    let general = vec![
        Param::new("copies", tl!("Copies:"), ParamKind::int(Some(1), Some(i64::from(vectorcraft_pdf::MAX_COPIES)), 1)),
        flag("collate", tl!("Collate")).enabled_when_in("copies", counts),
        flag("reverse", tl!("Reverse Order")),
        Param::new("artboards", tl!("Artboards"), ParamKind::radio(artboards, None)),
        Param::new("range", tl!("Range:"), ParamKind::text("")).hint("1-3, 5").enabled_when("artboards", PrintArtboards::Range.id()),
        flag("skipBlank", tl!("Skip Blank Artboards")).enabled_when_in("artboards", [PrintArtboards::All.id(), PrintArtboards::Range.id()]),
        Param::section(tl!("Media")),
        pick::<Media>("media", tl!("Size:")),
        length("width", tl!("Width:")).enabled_when("media", "custom"),
        length("height", tl!("Height:")).enabled_when("media", "custom"),
        Param::section(tl!("Orientation")),
        flag("autoRotate", tl!("Auto-Rotate (turn the paper to each artboard)")),
        pick::<Orientation>("orientation", tl!("Orientation:")).enabled_when("autoRotate", false),
        flag("transverse", tl!("Transverse (the page a quarter turn on the paper)")),
        Param::section(tl!("Options")),
        pick::<PrintLayers>("printLayers", tl!("Print Layers:")),
        Param::custom("placement.origin", tl!("Placement:"), |ui, fields, _| {
            let at = fields.text("placement.origin").and_then(|id| Origin::ALL.iter().position(|o| o.id() == id)).unwrap_or(4);
            let mut anchor = ANCHORS.get(at).copied();
            let out = ReferencePoint::new(tl!("Placement"), Bind::bound(&mut anchor)).look(ReferencePointLook::Squares).show(ui);
            if let Some(i) = anchor.and_then(|a| ANCHORS.iter().position(|x| *x == a)).filter(|i| *i != at)
                && let Some(o) = Origin::ALL.get(i)
            {
                fields.set("placement.origin", o.id().to_string());
            }
            out.response
        })
        .enabled_when_in("scaling", flat.clone()),
        length("placement.x", "X:").enabled_when_in("scaling", flat.clone()),
        length("placement.y", "Y:").enabled_when_in("scaling", flat.clone()),
        flag(PLACED, tl!("The Print Tiling tool placed the pages (uncheck to place them with the placement again)")).visible_when(PLACED, true),
        length("tileOrigin.x", "X:").visible_when(PLACED, true),
        length("tileOrigin.y", "Y:").visible_when(PLACED, true),
        pick::<PrintScaling>("scaling", tl!("Scaling:")),
        Param::custom("scale", tl!("Scale:"), |ui, fields, _| {
            let mut parts = [Mixed::Same(fields.number("scale.width")), Mixed::Same(fields.number("scale.height"))];
            let mut linked = fields.bool("scale.linked").unwrap_or(false);
            let out = LinkedFields::<2>::new(tl!("Scale"), Bind::bound(&mut parts), Bind::bound(&mut linked))
                .mode(LinkMode::Equal)
                .range(1.0..=1000.0)
                .show(ui);
            for (key, part) in ["scale.width", "scale.height"].into_iter().zip(parts) {
                if let Mixed::Same(Some(v)) = part
                    && fields.number(key) != Some(v)
                {
                    fields.set(key, v);
                }
            }
            if fields.bool("scale.linked").unwrap_or(false) != linked {
                fields.set("scale.linked", linked);
            }
            out.edit.response
        })
        // A value set from outside: `scale` as `{width?, height?, linked?}`, or one of its parts.
        .on_set(|fields: &mut DialogFields, key: &str, value: Value| {
            let incoming = match (key.strip_prefix("scale."), value) {
                (None, Value::Object(parts)) => parts,
                (Some(part), value) => Map::from_iter([(part.to_string(), value)]),
                (None, _) => return Err("Scale takes {width, height, linked}".to_string()),
            };
            let current: Map<String, Value> = ["width", "height", "linked"]
                .into_iter()
                .filter_map(|k| {
                    let key = format!("scale.{k}");
                    let v = if k == "linked" { fields.bool(&key).map(Value::from) } else { fields.number(&key).map(Value::from) };
                    v.map(|v| (k.to_string(), v))
                })
                .collect();
            for (k, v) in merge_scale(&current, &incoming) {
                match v {
                    Value::Bool(b) => fields.set(format!("scale.{k}"), b),
                    Value::Number(n) => fields.set(format!("scale.{k}"), n.as_f64().unwrap_or(100.0)),
                    _ => {}
                }
            }
            Ok(())
        })
        .enabled_when_in("scaling", scaling_on),
        length("overlap", tl!("Overlap:")).enabled_when_in("scaling", tiles.clone()),
        Param::new("tileRange", tl!("Tile Range:"), ParamKind::text("")).enabled_when_in("scaling", tiles.clone()),
        note("tilesNote", tl!("Tiles are numbered across, then down; leave the range empty for every tile.")).visible_when_in("scaling", tiles),
        flag("copiesPerSheet", tl!("Copies per Sheet (as many as fit, in a grid)")).enabled_when_in("scaling", flat),
        length("gutter", tl!("Gutter:")).enabled_when("copiesPerSheet", true),
    ];
    let marks = marks_and_bleed();
    let output = vec![
        pick::<OutputMode>("output.mode", tl!("Mode:")),
        pick::<Emulsion>("output.emulsion", tl!("Emulsion:")),
        pick::<PrintImage>("output.image", tl!("Image:")),
        flag("output.spotsToProcess", tl!("Convert All Spot Colors to Process")).enabled_when("output.mode", separations),
        flag("output.overprintBlack", tl!("Overprint Black")).enabled_when("output.mode", separations),
        Param::section(tl!("Document Ink Options")),
        Param::custom("output.inks", "", ink_table),
    ];
    let graphics = vec![
        Param::section(tl!("Paths")),
        flag("graphics.autoFlatness", tl!("Automatic flatness")),
        Param::number("graphics.flatness", tl!("Flatness:"), 1.0, 0.1, 1)
            .bounds(Some(0.2), Some(100.0))
            .tooltip(tl!("0.2 (quality) to 100 (speed)"))
            .enabled_when("graphics.autoFlatness", false),
        Param::section(tl!("Fonts")),
        pick::<FontDownload>("graphics.fonts", tl!("Download:")),
    ];
    let profiles: Vec<(String, _)> =
        std::iter::once((String::new(), tl!(SAME_AS_SOURCE).into())).chain(lists.profiles.iter().map(|p| (p.clone(), p.clone().into()))).collect();
    let intents = Intent::ALL
        .iter()
        .map(|i| (serde_json::to_value(i).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default(), tl!(i.label()).into()))
        .collect();
    let color = vec![
        Param::new("color.profile", tl!("Printer profile:"), ParamKind::choice(profiles, None)),
        note("profileNote", tl!("Composite colours are converted to it; separations separate with it when it is a CMYK profile.")),
        Param::new("color.intent", tl!("Rendering intent:"), ParamKind::choice(intents, None)),
        note("intentNote", tl!("How colours outside the press's gamut are separated.")),
        flag("color.preserveNumbers", tl!("Preserve CMYK Numbers")),
        note("preserveNote", tl!("CMYK colours print with their own values; off, they go through the colour settings too.")),
    ];
    let flatteners: Vec<(String, _)> = std::iter::once((String::new(), tl!(KEEP_TRANSPARENCY).into()))
        .chain(lists.flatteners.iter().map(|p| (p.clone(), p.clone().into())))
        .collect();
    let advanced = vec![
        Param::section(tl!("Printer")),
        length("margin", tl!("Unprintable margin:")),
        note("marginNote", tl!("The edge of the paper the printer can't reach: the art goes inside it, and the preview shows it dashed.")),
        flag("advanced.printAsBitmap", tl!("Print as Bitmap")).enabled_when_in("output.mode", composite.clone()),
        Param::number("advanced.bitmapResolution", tl!("Resolution:"), 0.0, 1.0, 0)
            .bounds(Some(0.0), Some(vectorcraft_pdf::MAX_BITMAP_RESOLUTION))
            .unit(UnitKind::PixelsPerInch)
            .enabled_when(BITMAP_ON, true),
        note("bitmapNote", tl!("Each page prints as one image, at the document's raster effects resolution.")).visible_when(BITMAP_AT_DOCUMENT, true),
        Param::section(tl!("Overprint and Transparency Flattener Options")),
        pick::<PrintOverprints>("advanced.overprints", tl!("Overprints:")).enabled_when_in("output.mode", composite),
        Param::new("advanced.flattenerPreset", tl!("Preset:"), ParamKind::choice(flatteners, None)),
        note(
            "flattenerNote",
            tl!(
                "Simulate prints overprints as Overprint Preview shows them; separations always keep them. A preset flattens transparency before printing (PostScript files always flatten it)."
            ),
        ),
    ];
    let summary = vec![
        Param::section(tl!("Pages")),
        shown("__summary.pages"),
        Param::section(tl!("Options")),
        shown("__summary.options"),
        Param::section(tl!("Warnings")),
        shown("__summary.warnings"),
    ];
    [general, marks, output, graphics, color, advanced, summary]
        .into_iter()
        .zip(SECTIONS)
        .map(|(params, title)| PrintSection::new(tl!(title), params))
        .collect()
}

/// The Marks and Bleed rows: no link between the bleed sides.
fn marks_and_bleed() -> Vec<Param> {
    // PLAN: the marks' option rows follow the Save PDF dialog's `marks_and_bleeds` (export-w17 deletes it): the kinds, the weights and the bleed.
    let weights: Vec<(String, _)> = [0.125, 0.25, 0.5, 0.75, 1.0].iter().map(|w| (w.to_string(), format!("{w} pt").into())).collect();
    vec![
        Param::section(tl!("Marks")),
        flag("marks.trim", tl!("Trim Marks")),
        flag("marks.registration", tl!("Registration Marks")),
        flag("marks.colorBars", tl!("Color Bars")),
        flag("marks.pageInfo", tl!("Page Information")),
        pick::<vectorcraft_pdf::MarkKind>("marks.kind", tl!("Printer's Marks Type:")),
        Param::new("marks.weight", tl!("Trim Mark Weight:"), ParamKind::choice(weights, None)),
        length("marks.offset", tl!("Offset:")),
        Param::section(tl!("Bleeds")),
        flag("bleed.useDocument", tl!("Use document bleed settings")),
        length("bleed.top", tl!("Top:")).enabled_when("bleed.useDocument", false),
        length("bleed.bottom", tl!("Bottom:")).enabled_when("bleed.useDocument", false),
        length("bleed.left", tl!("Left:")).enabled_when("bleed.useDocument", false),
        length("bleed.right", tl!("Right:")).enabled_when("bleed.useDocument", false),
    ]
}

/// The Document Ink table: every ink of the document (the pump writes them into `__inks` from the preview), the spot
/// inks printing as process included. Nothing but a note unless the output is separations and there are inks.
fn ink_table(ui: &mut egui::Ui, fields: &mut DialogFields, _: &mut craft::ui::patterns::form_dialogs::CustomContext) -> egui::Response {
    let inks = match fields.get("__inks") {
        Some(FieldValue::Json(v)) => v.as_array().cloned().unwrap_or_default(),
        _ => Vec::new(),
    };
    if fields.text("output.mode") != Some(OutputMode::Separations.id()) || inks.is_empty() {
        return Text::new(tl!("Separations list the document's inks here: one page per ink that prints.")).shade(Shade::Muted).show(ui);
    }
    let all_spots = fields.bool("output.spotsToProcess") == Some(true);
    ui.scope(|ui| {
        Grid::new(&[ColumnWidth::Content; 5]).show(ui, |ui, _| {
            for head in [tl!("Print"), "", tl!("Ink"), tl!("Frequency"), tl!("Angle")] {
                if head.is_empty() {
                    empty_cell(ui);
                } else {
                    Text::new(head).shade(Shade::Muted).show(ui);
                }
            }
            for ink in &inks {
                let name = ink["name"].as_str().unwrap_or_default();
                let spot = ink["spot"] == true;
                // A spot ink printing as process has no plate: its plate options wait for it to print as a spot ink again.
                let process = ink["convertToProcess"] == true;
                // The row's label names the box, which the Ink column beside it shows.
                let on = ink["print"].as_bool().unwrap_or(true);
                let mut print = on;
                Checkbox::new(Bind::bound(&mut print)).label(name.to_string()).enabled(!process).id_salt(("print-ink", name)).show(ui);
                if print != on {
                    set_ink(fields, name, "print", json!(print));
                }
                match spot.then(|| ink_kind_button(ui, name, process, !all_spots)) {
                    Some(Some(on)) => set_ink(fields, name, "convertToProcess", json!(on)),
                    Some(None) => {}
                    None => empty_cell(ui),
                }
                Text::new(if spot { format!("{name} ({})", tl!("spot")) } else { name.to_string() }).show(ui);
                for (k, heading, unit) in [("frequency", tl!("Frequency"), tl!("lpi")), ("angle", tl!("Angle"), "°")] {
                    let v = ink[k].as_f64().unwrap_or(0.0);
                    let typed = NumberField::new(format!("{heading} {name}"), Bind::given(Some(v)))
                        .unit(unit)
                        .fixed_decimals(2)
                        .enabled(!process)
                        .id_salt(("print-ink", name, k))
                        .show(ui);
                    if let Some(Some(x)) = typed.edit.value() {
                        set_ink(fields, name, k, json!(x));
                    }
                }
            }
        });
        Text::new(tl!("Frequencies and angles are the output device's to apply.")).shade(Shade::Muted).show(ui);
    })
    .response
}

/// The icon before spot ink `name`'s name: the spot icon while it prints as a spot ink, the four process inks while it
/// prints as process (`process`). A click switches it; dimmed (`enabled` off) while Convert All Spot Colors to Process
/// converts every spot ink. Whether it prints as process after a click.
fn ink_kind_button(ui: &mut egui::Ui, name: &str, process: bool, enabled: bool) -> Option<bool> {
    let label = if process {
        crate::i18n::fmt(tl!("Keep {name} as spot"), &[("name", name)])
    } else {
        crate::i18n::fmt(tl!("Convert {name} to process"), &[("name", name)])
    };
    ToggleButton::new(label, Bind::given(process))
        .icon(IconSpec::svg(IconId::new("dc-swatch-spot")))
        .on_icon(crate::panels::swatches::cmyk_glyph())
        .icon_only()
        .enabled(enabled)
        .id_salt(("print-ink-kind", name))
        .show(ui)
        .value()
        .copied()
}

/// Set option `key` of ink `name` in `output.inks` (the ink gets its own entry the first time).
fn set_ink(fields: &mut DialogFields, name: &str, key: &str, value: Value) {
    let mut inks = match fields.get("output.inks") {
        Some(FieldValue::Json(Value::Array(a))) => a.clone(),
        _ => Vec::new(),
    };
    match inks.iter_mut().find(|i| i["name"] == name) {
        Some(ink) => ink[key] = value,
        None => {
            let mut ink = json!({ "name": name, "print": true });
            ink[key] = value;
            inks.push(ink);
        }
    }
    fields.set("output.inks", FieldValue::Json(Value::Array(inks)));
}

/// The Scale object after a value was set from outside (`ui.dialog.set`), from the one it was: an object with W and H
/// apart and no `linked` turns the link off; a missing or `null` W, H or `linked` keeps the current value; with the
/// link on, a changed W (or H) gives its value to the other; both changed apart while linked is kept as sent, for
/// `print.preview` to refuse.
fn merge_scale(current: &Map<String, Value>, incoming: &Map<String, Value>) -> Map<String, Value> {
    let num = |m: &Map<String, Value>, k: &str| m.get(k).and_then(Value::as_f64);
    let mut out = current.clone();
    for k in ["width", "height", "linked"] {
        if let Some(v) = incoming.get(k).filter(|v| !v.is_null()) {
            out.insert(k.into(), v.clone());
        }
    }
    if incoming.get("linked").is_none() && num(incoming, "width").zip(num(incoming, "height")).is_some_and(|(w, h)| w != h) {
        out.insert("linked".into(), json!(false));
    }
    let (w, h) = (num(&out, "width"), num(&out, "height"));
    if out.get("linked") == Some(&json!(true)) && w != h {
        let (w_moved, h_moved) = (w != num(current, "width"), h != num(current, "height"));
        if w.is_none() || (!w_moved && h_moved) {
            if let Some(h) = h {
                out.insert("width".into(), json!(h));
            }
        } else if !(w_moved && h_moved)
            && let Some(w) = w
        {
            out.insert("height".into(), json!(w));
        } else if let (false, Some(w)) = (h_moved, w) {
            out.insert("height".into(), json!(w));
        }
    }
    out
}

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

/// A line per changed option (`{option, value}`, after [`labelled`]): "Section › Option: value".
pub(super) fn changed_lines(changed: &[Value]) -> Vec<String> {
    changed
        .iter()
        .map(|c| {
            let value = match &c["value"] {
                Value::Bool(b) => if *b { tl!("On") } else { tl!("Off") }.to_string(),
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            format!("{} {value}", crate::i18n::colon(&option_label(c["option"].as_str().unwrap_or_default())))
        })
        .collect()
}

/// The Summary's pages, options and warnings texts for the preview `pv` of `settings`.
fn summary_texts(pv: &Result<Value, String>, settings: &Value, has_doc: bool) -> [String; 3] {
    let lang = crate::i18n::current();
    let warn = |w: &str| format!("⚠ {}", crate::i18n::message(lang, w));
    let pages = match pv {
        Ok(v) => {
            let per_copy = v["sheets"].as_array().map_or(0, Vec::len);
            crate::i18n::fmt(tl!("{pages} in all, {count} per copy"), &[("pages", &v["pages"].to_string()), ("count", &per_copy.to_string())])
        }
        // A preset edited without a document has no pages to count.
        Err(_) if !has_doc => String::new(),
        Err(e) => warn(e),
    };
    let mut changed = vec![];
    vectorcraft_engine::cmd::fileio::pdf::changed("", settings, &DEFAULTS, &mut changed);
    changed.sort_by_key(|c| section_of(c["option"].as_str().unwrap_or_default()).0);
    labelled(&mut changed);
    let options = if changed.is_empty() { tl!("Every option is at its default.").to_string() } else { changed_lines(&changed).join("\n") };
    let warnings = match pv {
        Ok(v) => {
            let lines: Vec<String> = v["warnings"].as_array().into_iter().flatten().filter_map(Value::as_str).map(warn).collect();
            if lines.is_empty() { tl!("None.").to_string() } else { lines.join("\n") }
        }
        Err(_) => String::new(),
    };
    [pages, options, warnings]
}
