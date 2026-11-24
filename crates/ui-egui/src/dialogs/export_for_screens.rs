//! File → Export → Export for Screens: on the left the Artboards and
//! Assets tabs (thumbnails with checkboxes, in a grid or a list), Clear Selection and the prefix;
//! on the right Select (All, a range or Full Document, Include Bleed, Include Artboard Fill), the destination (folder
//! picker, Open Location after Export, Create Sub-folders by scale or by format; the web downloads
//! instead), Export PDFs As (one file of every artboard or a file each), and the format rows
//! ([`screen_formats::rows`]: scale, suffix, format, the iOS and Android rows, the format presets
//! and Format Settings); below, how many items are selected and how many files the export writes.
//! It opens on the settings the document last exported with, except that while the Artboard tool
//! is in use the artboards it has selected are the ones chosen. On the desktop with Background
//! Export on, the files are encoded and written on the background thread ([`crate::background`])
//! after the dialog closes.
//!
//! The dialog lives on the window's dialog host. Its `show` gets no app, so [`pump`] gives it what
//! it shows from the app (the document, the artboard thumbnails, the presets) and runs what it asks
//! for (a preset pick, an artboard rename, the export on OK).
//!
//! `document.exportForScreens` does more than the dialog shows (layers, the selection, file name
//! patterns, a folder per item): those are for scripts and MCP.
//!
//! Fields (`ui.dialog.set`): `tab` (artboards|assets), `select` (all|range|full), `range`,
//! `boards` ([bool] per artboard), `assets` ([asset id] checked on the Assets tab), `includeBleed`,
//! `includeArtboardFill`,
//! `folder`, `openLocation`, `subfolders`, `subfolderBy` (scale|format), `pdfs` (single|multiple),
//! `prefix`, `preset` (""|mobile|density: where the rows came from), `formats` (rows of
//! `document.exportForScreens`, `scale` as text), `settings` ({png|png8|jpg|webp|gif|tiff|svg|pdf:
//! {…}}), `view` (grid|list), `exportPreset` (the format preset applied last, "" for none);
//! `__settings` names the format whose Format Settings show ("" when closed), `__savePreset` is the
//! name being typed to save a preset (absent when not saving).

use std::borrow::Cow;
use std::sync::Arc;

use craft::behavior::{ColumnKey, ColumnWidth, DisplayOrder, GesturePhase, IconId, ItemSource, SelectionChange};
use craft::host::PickRequest;
use craft::ui::buttons::Button;
use craft::ui::choice::{Checkbox, Radio, RadioGroup, Segmented};
use craft::ui::contract::{
    Bind, CollectionEvent, ColumnSpec, Entry, Props as _, Show as _, ShowCollection as _, ShowContainer as _, SpaceAction, SweepApply, SweepStep,
    ToggleColumn,
};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, HeldAnswer, HeldOk, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{List, ListRow, ThumbAspect, ThumbSize, ToggleCell, ToggleState};
use craft::ui::patterns::form_dialogs::{FormDialog, PathRow};
use craft::ui::primitives::ContentPainterCx;
use craft::ui::runtime::env::Label;
use craft::ui::sections::Section;
use craft::ui::tabs::{Tab, TabKind, TabStrip};
use craft::ui::text::TextField;
use craft::ui::tile_grid::{Tile, TileGrid, TilePartEvent};
use egui::{Context, TextureHandle, Ui};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use vectorcraft_doc::{Document, ExportAsset};
use vectorcraft_engine::cmd::fileio::{self, ArtboardPick, pdf};
use vectorcraft_engine::cmd::params::drop_unread_row_options;

use super::screen_formats::{self, PresetPick, SETTINGS};
use super::{png_options, svg_options};
use crate::{VectorcraftApp, io};

/// The dialog's key on the host.
pub const KIND: &str = "exportForScreens";

/// The tabs, by their `tab` value.
const TABS: [&str; 2] = ["artboards", "assets"];

/// The fields `ui.dialog.set` takes and `ui.inspect` lists; the other `__` keys are the dialog's own.
const SETTABLE: [&str; 20] = [
    "tab",
    "view",
    "boards",
    "select",
    "range",
    "folder",
    "openLocation",
    "subfolders",
    "subfolderBy",
    "pdfs",
    "prefix",
    "preset",
    "formats",
    "settings",
    "includeBleed",
    "includeArtboardFill",
    "assets",
    "exportPreset",
    "__settings",
    "__savePreset",
];

/// The check column of the list view and its name column.
const CHECK: ColumnKey = ColumnKey::new("check");
const NAME: ColumnKey = ColumnKey::new("name");
/// Rows of the list view and of the grid view the items pane shows before it scrolls, a pane as tall as the options
/// column's first sections.
const LIST_ROWS: u16 = 9;
const GRID_ROWS: u16 = 3;

/// The longest side of an artboard thumbnail, in pixels.
const THUMB_PX: u32 = 92;

/// The text of `key` in `fields`, "" when it has none.
fn text<'a>(fields: &'a Map<String, Value>, key: &str) -> &'a str {
    fields.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// The flag `key` in `fields`, false when it has none.
fn flag(fields: &Map<String, Value>, key: &str) -> bool {
    fields.get(key).and_then(Value::as_bool).unwrap_or_default()
}

/// The active document.
fn active_doc(app: &VectorcraftApp) -> Option<&Document> {
    app.session.active().map(|st| &*st.doc)
}

/// The format rows of export settings `saved` (`document.exportSettings`), sizes shown as text:
/// the saved rows, else the saved preset's, else one PNG at 1x; and that preset.
pub(crate) fn saved_rows(saved: &Map<String, Value>) -> (Vec<Value>, Option<&str>) {
    let get = |k: &str| saved.get(k).filter(|v| !v.is_null());
    let preset = get("preset").and_then(Value::as_str).filter(|p| fileio::screen_preset_rows(p).is_some());
    let rows = match (get("formats").and_then(Value::as_array), preset) {
        (Some(rows), _) if rows.iter().any(Value::is_object) => rows.iter().filter(|r| r.is_object()).map(editable_row).collect(),
        (_, Some(p)) => fileio::screen_preset_rows(p).unwrap_or_default(),
        _ => vec![json!({"format": "png", "scale": "1x", "suffix": ""})],
    };
    (rows, preset)
}

/// Open the dialog on its Assets tab with `checked` assets checked (every asset when `None`).
pub(crate) fn open_assets(app: &mut VectorcraftApp, checked: Option<&[u64]>) {
    open(app);
    let shown: Vec<u64> = active_doc(app).map(|doc| picture_assets(doc).iter().map(|a| a.id).collect()).unwrap_or_default();
    let Some(dialogs) = app.dialogs() else { return };
    dialogs.with(|host| {
        let Some(d) = host.get_mut_by_id::<ExportForScreensDialog>(dialog_id(KIND)) else { return };
        d.fields.insert("tab".into(), json!("assets"));
        if let Some(c) = checked {
            d.fields.insert("assets".into(), json!(c));
            // The list scrolls to the first of them it shows: new assets land at its end.
            if let Some(first) = c.iter().find(|id| shown.contains(id)) {
                d.fields.insert("__scrollTo".into(), json!(first));
            }
        }
    });
}

/// Open the dialog on the settings the active document last exported with (else one PNG row at
/// 1x, into the Desktop, with Create Sub-folders on), every asset checked. While the Artboard tool
/// is in use, the artboards it has selected (the rows the Artboards panel highlights) are the ones
/// chosen instead.
pub fn open(app: &mut VectorcraftApp) {
    let (n, saved, assets) = active_doc(app)
        .map(|doc| (doc.artboards.len(), doc.export_settings.clone(), picture_assets(doc).iter().map(|a| a.id).collect::<Vec<_>>()))
        .unwrap_or_default();
    let get = |k: &str| saved.get(k).filter(|v| !v.is_null());
    let flag = |k: &str, default: bool| get(k).and_then(Value::as_bool).unwrap_or(default);
    // The artboards are those the Artboard tool has selected while it is in use, else Full
    // Document, a range (or a list of them), else all.
    let picked = (app.session.tool_id() == "artboard").then(|| crate::panels::artboards::picked(app, n));
    let full = picked.is_none() && flag("fullDocument", false);
    let chosen = picked.or_else(|| ArtboardPick::deserialize(&Value::Object(saved.clone())).ok().and_then(|p| p.resolve(n).ok().flatten()));
    let select = match (full, &chosen) {
        (true, _) => "full",
        (false, Some(c)) if c.len() < n => "range",
        _ => "all",
    };
    let boards: Vec<bool> = (0..n).map(|i| chosen.as_ref().is_none_or(|c| c.contains(&i))).collect();
    let (rows, preset) = saved_rows(&saved);
    // Every format's settings: its defaults, then the saved ones.
    let mut settings = Map::new();
    for (id, _) in SETTINGS {
        settings.insert(id.into(), Value::Object(saved_options(app, &saved, id)));
    }
    let folder = get("folder").and_then(Value::as_str).filter(|f| !f.is_empty()).map(str::to_string).or_else(fileio::export_folder);
    // Create Sub-folders is on until an export turns it off; a folder per format is the saved
    // `folders: "format"`, any other sorting a folder per scale.
    let folders = get("folders").and_then(Value::as_str);
    let subfolders = match folders {
        // An export reads empty folders as none.
        Some("none" | "") => false,
        Some(_) => true,
        None => flag("subfolders", saved.is_empty() || preset.is_some()),
    };
    let fields = json!({
        "tab": "artboards",
        "select": select,
        "range": range_text(&boards),
        "boards": boards,
        "assets": assets,
        "includeBleed": flag("includeBleed", false),
        "includeArtboardFill": flag("includeArtboardFill", true),
        "folder": folder.unwrap_or_default(),
        "openLocation": flag("openLocation", true),
        "subfolders": subfolders,
        "subfolderBy": if folders == Some("format") { "format" } else { "scale" },
        "__itemFolders": folders.filter(|f| matches!(*f, "item" | "itemFormat")).unwrap_or_default(),
        "pdfs": if get("pdfs").and_then(Value::as_str) == Some("single") { "single" } else { "multiple" },
        "prefix": get("prefix").and_then(Value::as_str).unwrap_or_default(),
        "preset": preset.unwrap_or_default(),
        "formats": rows,
        "settings": settings,
        "view": "grid",
        "__settings": "",
        "exportPreset": "",
    });
    let Value::Object(fields) = fields else { return };
    let Some(ctx) = app.ui_ctx.clone() else { return };
    let (doc, thumbs) = document_view(app, &ctx);
    let dialog = ExportForScreensDialog {
        fields,
        doc,
        web: io::is_web(app),
        thumbs,
        presets: export_presets(app),
        pdf_presets: pdf::presets(&app.session),
        pending: None,
        renaming_in: None,
        rename: None,
        held: HeldAnswer::new(),
    };
    if let Some(dialogs) = app.dialogs() {
        dialogs.with(|host| host.open(dialog));
    }
}

/// The options format `id` starts with in Format Settings: the raster options dialogs' defaults
/// (without image maps, a map being another file, and without quality but WebP's: JPEG rows choose
/// their quality), the SVG Options' first-use ones (without artboards and images: a linked image is
/// another file), or the default PDF preset.
fn defaults(app: &VectorcraftApp, id: &str) -> Map<String, Value> {
    let mut o = match (id, fileio::format(id)) {
        ("svg", _) => svg_options::first_use(),
        ("pdf", _) => Map::from_iter([("preset".to_string(), json!(vectorcraft_pdf::default_preset_name(vectorcraft_pdf::APP)))]),
        (_, Some(f)) => png_options::defaults(app, f),
        _ => Map::new(),
    };
    for k in ["imageMap", "useArtboards", "images"] {
        o.remove(k);
    }
    if id != "webp" {
        o.remove("quality");
    }
    o
}

/// The format's defaults with the document's saved choices overlaid.
pub(crate) fn saved_options(app: &VectorcraftApp, saved: &Map<String, Value>, id: &str) -> Map<String, Value> {
    let mut options = defaults(app, id);
    // TIFF's new defaults must not mask an existing shared anti-aliasing choice.
    if id == "tiff"
        && let Some(aa) = saved.get("antiAlias")
    {
        options.insert("antiAlias".into(), aa.clone());
    }
    if let Some(o) = saved.get("settings").and_then(|v| v.get(id)).and_then(Value::as_object) {
        let mut o = o.clone();
        drop_unread_row_options(&mut o);
        options.extend(o);
    }
    options
}

/// Export settings `settings` (as `document.exportSettings` or a preset holds them) without the
/// keys the export no longer reads in its rows and per-format settings, and without settings for a
/// format it doesn't write.
pub(crate) fn drop_unread_screen_settings(settings: &mut Map<String, Value>) {
    if let Some(rows) = settings.get_mut("formats").and_then(Value::as_array_mut) {
        rows.iter_mut().filter_map(Value::as_object_mut).for_each(drop_unread_row_options);
    }
    if let Some(all) = settings.get_mut("settings").and_then(Value::as_object_mut) {
        let formats = vectorcraft_engine::cmd::params::screen_formats();
        all.retain(|id, _| formats.contains(&id.as_str()));
        all.values_mut().filter_map(Value::as_object_mut).for_each(drop_unread_row_options);
    }
}

/// A row with its size as the text the Scale field shows (`2x`, `100w`, `100h`, `72ppi`), without
/// the keys the export no longer reads.
pub(super) fn editable_row(row: &Value) -> Value {
    let num = |k: &str| row.get(k).and_then(Value::as_f64);
    let text = match (num("width"), num("height"), num("ppi"), row.get("scale")) {
        (Some(w), ..) => format!("{w}w"),
        (_, Some(h), ..) => format!("{h}h"),
        (_, _, Some(p), _) => format!("{p}ppi"),
        (.., Some(Value::String(s))) => s.clone(),
        (.., Some(v)) => format!("{}x", v.as_f64().unwrap_or(1.0)),
        _ => "1x".into(),
    };
    let mut r = row.clone();
    if let Some(o) = r.as_object_mut() {
        drop_unread_row_options(o);
        for k in ["width", "height", "ppi"] {
            o.remove(k);
        }
        o.insert("scale".into(), json!(text));
    }
    r
}

/// The checked artboards as a 1-based range (`1-3, 5`).
pub(super) fn range_text(boards: &[bool]) -> String {
    let mut parts = vec![];
    let mut start = None;
    // A false past the end closes the last run.
    for (i, on) in boards.iter().copied().chain([false]).enumerate() {
        match (on, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                parts.push(if i - 1 > s { format!("{}-{i}", s + 1) } else { i.to_string() });
                start = None;
            }
            _ => {}
        }
    }
    parts.join(", ")
}

/// The export presets: `[{name, builtIn, settings}]` (built-in first).
pub(super) fn export_presets(app: &mut VectorcraftApp) -> Value {
    app.run("document.exportPresets.list", json!({})).ok().and_then(|r| r.get("presets").cloned()).unwrap_or_else(|| json!([]))
}

/// The `document.exportForScreens` params the dialog's `fields` describe (the web: no folder, one
/// ZIP for several files).
pub(super) fn params(app: &VectorcraftApp, fields: &Map<String, Value>) -> Result<Value, String> {
    let (mut p, files) = settings_params(active_doc(app), fields)?;
    if io::is_web(app) {
        p.insert("zip".into(), json!(files > 1));
    } else {
        let folder = text(fields, "folder").trim();
        if folder.is_empty() {
            return Err("choose a folder to export to".into());
        }
        p.insert("folder".into(), json!(folder));
        p.insert("openLocation".into(), json!(flag(fields, "openLocation")));
    }
    Ok(Value::Object(p))
}

/// The params of [`params`] without the destination (what is exported, at which sizes, and how the
/// files are named and sorted), and how many files that is.
fn settings_params(doc: Option<&Document>, fields: &Map<String, Value>) -> Result<(Map<String, Value>, usize), String> {
    let (p, files, _) = selection_params(doc, fields)?;
    Ok((p, files))
}

/// [`settings_params`] and how many artboards or assets are chosen.
fn selection_params(doc: Option<&Document>, fields: &Map<String, Value>) -> Result<(Map<String, Value>, usize, usize), String> {
    let n = doc.map_or(0, |doc| doc.artboards.len());
    let rows = fields.get("formats").and_then(Value::as_array).cloned().unwrap_or_default();
    if rows.is_empty() {
        return Err("add a format to export".into());
    }
    let mut p = Map::new();
    // What each row writes a file of: the checked assets, else the artboards.
    let artboards = text(fields, "tab") != "assets";
    let pieces = if !artboards {
        let checked = checked_assets(doc, fields);
        if checked.is_empty() {
            return Err("check an asset to export".into());
        }
        let count = checked.len();
        p.insert("assets".into(), json!(checked));
        count
    } else {
        match text(fields, "select") {
            "full" => {
                p.insert("fullDocument".into(), json!(true));
                1
            }
            "range" => {
                let range = text(fields, "range");
                // A bad range keeps the dialog open.
                let chosen = ArtboardPick { range: Some(range.to_string()), ..Default::default() }.resolve(n)?.unwrap_or_default();
                if chosen.is_empty() {
                    return Err("check an artboard to export".into());
                }
                p.insert("range".into(), json!(range));
                chosen.len()
            }
            _ => n,
        }
    };
    // Export PDFs As › Single file: every chosen artboard in one PDF per PDF row.
    let single = artboards && pieces > 1 && text(fields, "select") != "full" && text(fields, "pdfs") == "single";
    let pdf_rows = rows.iter().filter(|r| is_pdf_row(r)).count();
    let files = if single { (rows.len() - pdf_rows) * pieces + pdf_rows } else { rows.len() * pieces };
    p.insert("includeBleed".into(), json!(flag(fields, "includeBleed")));
    p.insert("includeArtboardFill".into(), json!(flag(fields, "includeArtboardFill")));
    let folders = folders(fields);
    // Create Sub-folders, as the Asset Export panel (which shares these settings) names it.
    p.insert("subfolders".into(), json!(folders == "rows"));
    p.insert("folders".into(), json!(folders));
    p.insert("pdfs".into(), json!(if single { "single" } else { "multiple" }));
    p.insert("prefix".into(), json!(text(fields, "prefix")));
    p.insert("formats".into(), Value::Array(rows));
    p.insert("settings".into(), fields.get("settings").cloned().unwrap_or(json!({})));
    Ok((p, files, pieces))
}

/// Whether format row `row` writes a PDF.
fn is_pdf_row(row: &Value) -> bool {
    row["format"].as_str().and_then(fileio::format).is_some_and(|f| f.id == "pdf")
}

/// The `folders` the dialog's sub-folder controls give: `none`, `format`, or a folder per scale
/// (`rows`), or the `item` or `itemFormat` of an applied preset (`__itemFolders`) while the
/// controls have not changed.
pub(super) fn folders(fields: &Map<String, Value>) -> &'static str {
    match (flag(fields, "subfolders"), text(fields, "subfolderBy")) {
        (false, _) => "none",
        (true, "format") => "format",
        (true, _) => match text(fields, "__itemFolders") {
            "item" => "item",
            "itemFormat" => "itemFormat",
            _ => "rows",
        },
    }
}

/// Export PDFs As applies to artboards (not Full Document) with a PDF row.
fn pdfs_enabled(fields: &Map<String, Value>) -> bool {
    let pdf = fields.get("formats").and_then(Value::as_array).is_some_and(|rows| rows.iter().any(is_pdf_row));
    text(fields, "tab") != "assets" && pdf && text(fields, "select") != "full"
}

/// How many artboards or assets are chosen and how many files the export writes (0 while the
/// settings choose nothing to export).
fn counts(doc: Option<&Document>, fields: &Map<String, Value>) -> (usize, usize) {
    match selection_params(doc, fields) {
        Ok((_, files, pieces)) => (pieces, files),
        Err(_) => (if text(fields, "tab") == "assets" { checked_assets(doc, fields).len() } else { 0 }, 0),
    }
}

/// Clear Selection: no artboard of `n` (or, on the Assets tab, no asset) is checked.
fn clear_selection(n: usize, fields: &mut Map<String, Value>) {
    if text(fields, "tab") == "assets" {
        fields.insert("assets".into(), json!([]));
    } else {
        fields.insert("boards".into(), json!(vec![false; n]));
        set_select(fields, "range");
        fields.insert("range".into(), json!(""));
    }
}

fn set_select(fields: &mut Map<String, Value>, select: &str) {
    fields.insert("select".into(), json!(select));
}

/// The checked artboards, one per artboard of `n` (all checked when the field is short).
fn boards(n: usize, fields: &Map<String, Value>) -> Vec<bool> {
    let mut boards: Vec<bool> =
        fields.get("boards").and_then(Value::as_array).map(|a| a.iter().map(|b| b.as_bool().unwrap_or(false)).collect()).unwrap_or_default();
    boards.resize(n, true);
    boards
}

/// The document's assets exported as pictures, in panel order: 3D assets export as mesh files from
/// the Asset Export panel (`assets.export3D`).
fn picture_assets(doc: &Document) -> Vec<&ExportAsset> {
    doc.assets.iter().filter(|a| a.format_3d.is_none()).collect()
}

/// The checked picture assets that are still in the document, in panel order.
fn checked_assets(doc: Option<&Document>, fields: &Map<String, Value>) -> Vec<u64> {
    let on: Vec<u64> = fields.get("assets").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
    doc.map(|doc| picture_assets(doc).iter().map(|a| a.id).filter(|id| on.contains(id)).collect()).unwrap_or_default()
}

/// Renames the artboard with id `board` to `text` (trimmed); an empty or unchanged name does nothing.
fn rename_artboard(app: &mut VectorcraftApp, board: u32, text: &str) {
    let name = text.trim();
    let Some(index) = active_doc(app).and_then(|doc| doc.artboards.iter().position(|a| a.id == board && a.name != name)) else { return };
    if !name.is_empty()
        && let Err(err) = app.run("artboard.rename", json!({"indices": [index], "name": name}))
    {
        app.status(err);
    }
}

/// A small rendering of artboard `i` of the active document, kept for the document's revision.
pub(super) fn artboard_thumb(app: &mut VectorcraftApp, ctx: &Context, i: usize) -> Option<TextureHandle> {
    let st = app.session.active()?;
    let key = (st.uid, st.revision);
    let (doc, n) = (Arc::clone(&st.doc), st.doc.artboards.len());
    let rect = doc.artboards.get(i)?.rect;
    let memo = app.canvas.artboard_thumbs.get_or_insert_with(|| (key, Vec::new()));
    if memo.0 != key {
        *memo = (key, Vec::new());
    }
    memo.1.resize(n, None);
    if let Some(tex) = memo.1.get(i).cloned().flatten() {
        return Some(tex);
    }
    let tex = ctx.load_texture(format!("ab-thumb-{i}"), crate::graphics::region_image(&doc, rect, THUMB_PX), egui::TextureOptions::LINEAR);
    if let Some(slot) = memo.1.get_mut(i) {
        *slot = Some(tex.clone());
    }
    Some(tex)
}

/// The active document as the dialog shows it (with its uid and revision) and its artboards'
/// thumbnails.
fn document_view(app: &mut VectorcraftApp, ctx: &Context) -> (Option<Shown>, Vec<Option<TextureHandle>>) {
    let Some(st) = app.session.active() else { return (None, Vec::new()) };
    let shown = Shown { key: (st.uid, st.revision), doc: Arc::clone(&st.doc) };
    let thumbs = (0..shown.doc.artboards.len()).map(|i| artboard_thumb(app, ctx, i)).collect();
    (Some(shown), thumbs)
}

/// The document the dialog shows, as the pump last gave it.
struct Shown {
    /// The document's uid and revision.
    key: (u64, u64),
    doc: Arc<Document>,
}

/// Export for Screens on the window's dialog host. OK files the fields; [`pump`] runs the export
/// and refuses OK with the reason when it fails, so the dialog stays open on the same fields.
pub(crate) struct ExportForScreensDialog {
    fields: Map<String, Value>,
    /// The active document, from the pump.
    doc: Option<Shown>,
    /// The web build: the files download.
    web: bool,
    /// The artboards' thumbnails, from the pump.
    thumbs: Vec<Option<TextureHandle>>,
    /// The format presets (`[{name, builtIn, settings}]`), from the pump.
    presets: Value,
    /// The PDF preset names Format Settings offers, from the pump.
    pdf_presets: Vec<String>,
    /// What the format presets menu asked for, for the pump to run.
    pending: Option<PresetPick>,
    /// The document an artboard rename began in, while it runs.
    renaming_in: Option<u64>,
    /// A committed artboard name: (document uid, artboard id, name), for the pump to run.
    rename: Option<(u64, u32, String)>,
    held: HeldAnswer<Map<String, Value>>,
}

impl ExportForScreensDialog {
    /// The saved and built-in format presets.
    fn preset_rows(&self) -> &[Value] {
        self.presets.as_array().map_or(&[], Vec::as_slice)
    }

    /// The left pane: the tabs, the artboards or assets, the view switch, Clear Selection and the prefix.
    fn items_pane(&mut self, ui: &mut Ui, fields: &mut Map<String, Value>) {
        let tab = TABS.iter().position(|k| *k == text(fields, "tab")).unwrap_or(0);
        let titles = Items(vec![(0, tl!("Artboards").to_string()), (1, tl!("Assets").to_string())]);
        let strip = TabStrip::new(tl!("Export for Screens")).id_salt("efs-tabs").kind(TabKind::Text).shown(tab).natural_height(true).show_items(
            ui,
            &titles,
            &mut |k, _| Tab::new(titles.name(*k)).id_salt(("efs-tab", *k)),
        );
        let picked = strip.response.events.into_iter().find_map(|e| match e {
            CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } => keys.last().copied(),
            _ => None,
        });
        if let Some(k) = picked.and_then(|k| TABS.get(k)) {
            fields.insert("tab".into(), json!(k));
        }
        if text(fields, "tab") == "assets" {
            self.asset_items(ui, fields);
        } else {
            self.artboard_items(ui, fields);
        }
        Row::new().show(ui, |ui, _| {
            let views = [
                Entry::new("grid".to_string(), tl!("Grid View")).icon(IconId::new("layout-grid")),
                Entry::new("list".to_string(), tl!("List View")).icon(IconId::new("dc-list-view")),
            ];
            let mut view = if text(fields, "view") == "list" { "list" } else { "grid" }.to_string();
            Segmented::new(tl!("View"), Bind::bound(&mut view), &views).show(ui);
            if view != text(fields, "view") {
                fields.insert("view".into(), json!(view));
            }
            if Button::new(tl!("Clear Selection")).show(ui).clicked() {
                clear_selection(self.doc.as_ref().map_or(0, |s| s.doc.artboards.len()), fields);
            }
            Text::new(tl!("Prefix:")).shade(Shade::Muted).show(ui);
            let prefix = text(fields, "prefix").to_string();
            if let Some(p) =
                TextField::new(tl!("Prefix:"), Bind::given(prefix.clone())).id_salt("efs-prefix").show(ui).value().filter(|p| **p != prefix)
            {
                fields.insert("prefix".into(), json!(p));
            }
        });
    }

    /// The artboards, each with its checkbox, number and name; checking them picks them as the range
    /// (all of them: All). Disabled with Full Document. A name committed in the rename editor goes to
    /// the pump with the document it was typed in.
    fn artboard_items(&mut self, ui: &mut Ui, fields: &mut Map<String, Value>) {
        let Some(shown) = &self.doc else { return };
        let doc = &shown.doc;
        let mut on = boards(doc.artboards.len(), fields);
        let items = Items(doc.artboards.iter().map(|a| (u64::from(a.id), a.name.clone())).collect());
        let caption = |at: usize| format!("{}  {}", at + 1, items.0.get(at).map_or("", |(_, name)| name.as_str()));
        let thumbs = &self.thumbs;
        let paint = |at: usize, cx: &ContentPainterCx, rect: egui::Rect| match thumbs.get(at).cloned().flatten() {
            Some(tex) => cx.picture(tex.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), 0.0),
            None => {
                cx.painter.rect_filled(rect, 0.0, egui::Color32::WHITE);
            }
        };
        let checked = |at: usize| on.get(at).copied().unwrap_or(false);
        let shelf =
            Shelf { items: &items, enabled: text(fields, "select") != "full", reveal: None, caption: &caption, checked: &checked, paint: &paint };
        let picks = shelf.show(ui, text(fields, "view") == "list");
        for key in picks.toggled {
            if let Some(b) = items.index_of(&key).and_then(|at| on.get_mut(at)) {
                *b ^= true;
            }
        }
        if picks.rename_began {
            self.renaming_in = Some(shown.key.0);
        }
        // The name goes to the document the rename began in.
        if let Some((key, name)) = picks.renamed
            && let Ok(board) = u32::try_from(key)
        {
            self.rename = Some((self.renaming_in.take().unwrap_or(shown.key.0), board, name));
        }
        if on != boards(doc.artboards.len(), fields) {
            set_select(fields, if on.iter().all(|b| *b) { "all" } else { "range" });
            fields.insert("range".into(), json!(range_text(&on)));
            fields.insert("boards".into(), json!(on));
        }
    }

    /// The Assets tab: each picture asset's thumbnail with its checkbox and name.
    fn asset_items(&mut self, ui: &mut Ui, fields: &mut Map<String, Value>) {
        let Some(shown) = &self.doc else { return };
        let (key, doc) = (shown.key, &*shown.doc);
        let assets = picture_assets(doc);
        let mut on = checked_assets(Some(doc), fields);
        let items = Items(assets.iter().map(|a| (a.id, a.name.clone())).collect());
        let caption = |at: usize| items.0.get(at).map(|(_, name)| name.clone()).unwrap_or_default();
        let paint = |at: usize, cx: &ContentPainterCx, rect: egui::Rect| {
            if let Some(a) = assets.get(at) {
                crate::panels::asset_export::paint_thumb(&cx.painter, key, doc, a, rect);
            }
        };
        let checked = |at: usize| items.0.get(at).is_some_and(|(id, _)| on.contains(id));
        let reveal = fields.get("__scrollTo").and_then(Value::as_u64).filter(|id| items.index_of(id).is_some());
        let shelf = Shelf { items: &items, enabled: true, reveal, caption: &caption, checked: &checked, paint: &paint };
        let picks = shelf.show(ui, text(fields, "view") == "list");
        for id in picks.toggled {
            if on.contains(&id) {
                on.retain(|a| *a != id);
            } else {
                on.push(id);
            }
        }
        // The grid reveals the asset once; the request is then spent.
        fields.remove("__scrollTo");
        if on != checked_assets(Some(doc), fields) {
            fields.insert("assets".into(), json!(on));
        }
    }

    /// The right column: Select, Export to, Export PDFs As, the format rows and Save Preset's name; or
    /// the Format Settings while `__settings` names a format.
    fn options_pane(&mut self, ui: &mut Ui, fields: &mut Map<String, Value>) {
        let wanted = text(fields, "__settings");
        if SETTINGS.iter().any(|(id, _)| *id == wanted) {
            screen_formats::settings(ui, fields, &self.pdf_presets);
            return;
        }
        if text(fields, "tab") != "assets" {
            let n = self.doc.as_ref().map_or(0, |s| s.doc.artboards.len());
            Section::fixed(tl!("Select:")).show(ui, |ui, _| select_options(ui, fields, n));
        }
        let web = self.web;
        Section::fixed(tl!("Export to:")).show(ui, |ui, _| destination(ui, fields, web));
        pdf_mode(ui, fields);
        let presets = self.presets.as_array().map_or(&[][..], Vec::as_slice);
        let mut picked = None;
        let mut menu = |ui: &mut Ui, fields: &mut Map<String, Value>| {
            if let Some(pick) = screen_formats::preset_menu_of(presets, ui, fields) {
                picked = Some(pick);
            }
        };
        screen_formats::rows(ui, fields, Some(&mut menu));
        let saved = screen_formats::save_preset_row(ui, fields);
        if let Some(pick) = picked.or(saved) {
            self.pending = Some(pick);
        }
    }
}

/// What the grid or the list of the left pane reported.
#[derive(Default)]
struct Picks {
    /// The items whose check flipped.
    toggled: Vec<u64>,
    /// A rename began.
    rename_began: bool,
    /// A committed rename: the item and the name typed.
    renamed: Option<(u64, String)>,
}

/// The items of the left pane, as the grid or the list shows them: each with its picture
/// (`paint`), caption and check box, by display position.
#[derive(Clone, Copy)]
struct Shelf<'a> {
    items: &'a Items<u64>,
    enabled: bool,
    /// The item to scroll to. Renaming is offered for the artboards, which have none.
    reveal: Option<u64>,
    caption: &'a dyn Fn(usize) -> String,
    checked: &'a dyn Fn(usize) -> bool,
    paint: &'a dyn Fn(usize, &ContentPainterCx, egui::Rect),
}

impl Shelf<'_> {
    /// Shows the items as a list of rows (`list`) or a grid of tiles → what the user did to them.
    fn show(self, ui: &mut Ui, list: bool) -> Picks {
        let Shelf { items, enabled, reveal, caption, checked, paint } = self;
        let mut picks = Picks::default();
        let rename = reveal.is_none();
        let at = |key: &u64| items.index_of(key).unwrap_or(usize::MAX);
        let state = |at: usize| if checked(at) { ToggleState::On } else { ToggleState::Off };
        if list {
            let columns =
                vec![ColumnSpec::toggle(CHECK, Label::Empty, ToggleColumn::default()), ColumnSpec::new(NAME, Label::Empty, ColumnWidth::fill(1.0))];
            let mut view = List::<u64>::new(tl!("Export for Screens"))
                .header(false)
                .space(SpaceAction::ToggleCell)
                .columns(columns)
                .rename(rename)
                .enabled(enabled)
                .max_rows(LIST_ROWS);
            if rename {
                view = view.rename_column(NAME);
            }
            let out = view.show_items(ui, items, &mut |key, icx| {
                let at = at(key);
                let name = caption(at);
                ListRow::new(name.clone())
                    .cells(icx.cells())
                    .toggle(ToggleCell::check(CHECK, state(at), name))
                    .thumb(ThumbSize::Medium, move |cx, rect| paint(at, cx, rect))
            });
            for event in out.events {
                match event {
                    CollectionEvent::Toggle(edit) => {
                        if let Some(SweepStep::Flip(toggle)) = SweepApply::default().feed(&edit) {
                            picks.toggled.push(toggle.key);
                        }
                    }
                    other => picks.rename(&other),
                }
            }
        } else {
            let mut grid = TileGrid::<u64>::new(tl!("Export for Screens")).checks(true).rename(rename).enabled(enabled).max_rows(GRID_ROWS);
            if let Some(key) = reveal {
                grid = grid.reveal(key);
            }
            let out = grid.show_items(ui, items, &mut |key, _| {
                let at = at(key);
                Tile::new(caption(at)).check(state(at)).thumb(ThumbAspect::SQUARE, move |cx, rect| paint(at, cx, rect))
            });
            for event in out.response.events {
                match event {
                    CollectionEvent::Item(key, TilePartEvent::Check { .. }) => picks.toggled.push(key),
                    other => picks.rename(&other),
                }
            }
        }
        picks
    }
}

impl Picks {
    /// Takes what a rename `event` says: that it began, or the item and the name it committed.
    fn rename<E>(&mut self, event: &CollectionEvent<u64, E>) {
        let CollectionEvent::Rename(edit) = event else { return };
        match edit.phase() {
            Some(GesturePhase::Begin) => self.rename_began = true,
            Some(GesturePhase::Commit) | None => {
                if let Some(rename) = edit.value() {
                    self.renamed = Some((rename.key, rename.name.clone()));
                }
            }
            _ => {}
        }
    }
}

/// Select: All or a Range (which checks the artboards it names), Include Bleed, Include Artboard
/// Fill and Full Document, for `n` artboards.
fn select_options(ui: &mut Ui, fields: &mut Map<String, Value>, n: usize) {
    let before = text(fields, "select").to_string();
    let mut select = before.clone();
    let mut range = None;
    RadioGroup::new(tl!("Select:"), Bind::bound(&mut select)).show(ui, |ui, _| {
        Stack::new().show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                Radio::new("all".to_string()).label(tl!("All")).show(ui);
                Radio::new("range".to_string()).label(tl!("Range:")).show(ui);
                let shown = text(fields, "range").to_string();
                let typed = TextField::new(tl!("Range:"), Bind::given(shown.clone())).id_salt("efs-range").enabled(before == "range").show(ui);
                range = typed.value().filter(|r| **r != shown).cloned();
            });
            Row::new().show(ui, |ui, _| {
                check(ui, fields, "includeBleed", tl!("Include Bleed"));
                check(ui, fields, "includeArtboardFill", tl!("Include Artboard Fill"));
            });
            Radio::new("full".to_string()).label(tl!("Full Document")).show(ui);
        });
    });
    if select != before {
        set_select(fields, &select);
        if select == "all" {
            fields.insert("boards".into(), json!(vec![true; n]));
            fields.insert("range".into(), json!(range_text(&vec![true; n])));
        }
    }
    if let Some(range) = range {
        if let Ok(Some(chosen)) = (ArtboardPick { range: Some(range.clone()), ..Default::default() }).resolve(n) {
            fields.insert("boards".into(), json!((0..n).map(|i| chosen.contains(&i)).collect::<Vec<_>>()));
        }
        fields.insert("range".into(), json!(range));
    }
}

/// A check box over flag `key`.
fn check(ui: &mut Ui, fields: &mut Map<String, Value>, key: &str, label: &'static str) {
    let mut on = flag(fields, key);
    Checkbox::new(Bind::bound(&mut on)).label(label).show(ui);
    if on != flag(fields, key) {
        fields.insert(key.into(), json!(on));
    }
}

/// Export to: the folder (typed or picked), Open Location after Export, and Create Sub-folders,
/// a folder per scale or per format; the web downloads instead.
fn destination(ui: &mut Ui, fields: &mut Map<String, Value>, web: bool) {
    if web {
        Text::new(tl!("Your browser downloads the files (several as one .zip).")).shade(Shade::Muted).show(ui);
    } else {
        let mut folder = text(fields, "folder").to_string();
        let row = PathRow::new(tl!("Export to:"), Bind::bound(&mut folder)).pick(PickRequest::folder("export-for-screens")).show(ui);
        if let Some(picked) = row.picked {
            folder = picked;
        }
        if folder != text(fields, "folder") {
            fields.insert("folder".into(), json!(folder));
        }
        check(ui, fields, "openLocation", tl!("Open Location after Export"));
    }
    let before = (flag(fields, "subfolders"), text(fields, "subfolderBy").to_string());
    check(ui, fields, "subfolders", tl!("Create Sub-folders"));
    let mut by = if before.1 == "format" { "format" } else { "scale" }.to_string();
    RadioGroup::new(tl!("Create Sub-folders"), Bind::bound(&mut by)).enabled(flag(fields, "subfolders")).show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            Radio::new("scale".to_string()).label(tl!("Scale")).show(ui);
            Radio::new("format".to_string()).label(tl!("Format")).show(ui);
        });
    });
    if by != before.1 {
        fields.insert("subfolderBy".into(), json!(by));
    }
    // A change to the sub-folder controls replaces an applied preset's item folders.
    if (flag(fields, "subfolders"), text(fields, "subfolderBy")) != (before.0, before.1.as_str()) {
        fields.remove("__itemFolders");
    }
}

/// Export PDFs As: one file of every chosen artboard, or a file each.
fn pdf_mode(ui: &mut Ui, fields: &mut Map<String, Value>) {
    let on = pdfs_enabled(fields);
    let mut pdfs = if text(fields, "pdfs") == "single" { "single" } else { "multiple" }.to_string();
    Row::new().show(ui, |ui, _| {
        Text::new(tl!("Export PDFs As:")).shade(Shade::Muted).show(ui);
        RadioGroup::new(tl!("Export PDFs As:"), Bind::bound(&mut pdfs)).enabled(on).show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                Radio::new("single".to_string()).label(tl!("Single File")).show(ui);
                Radio::new("multiple".to_string()).label(tl!("Multiple Files")).show(ui);
            });
        });
    });
    if pdfs != text(fields, "pdfs") {
        fields.insert("pdfs".into(), json!(pdfs));
    }
}

impl StackDialog for ExportForScreensDialog {
    type Result = Map<String, Value>;

    fn id(&self) -> DialogId {
        dialog_id(KIND)
    }

    fn key(&self) -> Option<&str> {
        Some(KIND)
    }

    /// Files the fields as OK does, and closes with them once `pump` exported them.
    fn confirm(&mut self, _: &Context) -> Option<Map<String, Value>> {
        if self.held.is_accepted() {
            return self.held.finish();
        }
        if self.held.is_waiting() || selection_params(self.doc.as_ref().map(|s| &*s.doc), &self.fields).is_err() {
            return None;
        }
        self.held.file(self.fields.clone());
        None
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    fn json_fields(&self) -> Option<Value> {
        Some(Value::Object(self.fields.iter().filter(|(k, _)| SETTABLE.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect()))
    }

    /// For `ui.dialog.set`: `exportPreset` applies the preset it names as the ☰ menu does, and an
    /// unknown name leaves the dialog as it was; `subfolders` and `subfolderBy` end an applied
    /// preset's item folders.
    fn set_json_field(&mut self, _: &Context, field: &str, value: Value) -> Result<(), String> {
        if !SETTABLE.contains(&field) {
            return Err(FieldRefusal::Unknown.message(field));
        }
        if field == "exportPreset" {
            let mut next = self.fields.clone();
            next.insert(field.into(), value);
            screen_formats::preset_set(self.preset_rows(), &mut next)?;
            self.fields = next;
        } else {
            // A change to the sub-folder controls replaces an applied preset's item folders, as in the dialog.
            if matches!(field, "subfolders" | "subfolderBy") {
                self.fields.remove("__itemFolders");
            }
            self.fields.insert(field.into(), value);
        }
        Ok(())
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Map<String, Value>>> {
        let doc = self.doc.as_ref().map(|s| Arc::clone(&s.doc));
        let ok = match (self.web, text(&self.fields, "tab")) {
            (true, _) => tl!("Download"),
            (false, "assets") => tl!("Export Asset"),
            (false, _) => tl!("Export Artboard"),
        };
        let chosen = selection_params(doc.as_deref(), &self.fields).err();
        let mut fields = std::mem::take(&mut self.fields);
        let mut held = std::mem::take(&mut self.held);
        let mut dialog =
            FormDialog::new(tl!("Export for Screens"), Bind::bound(&mut fields)).ok_label(ok).ok_enabled(chosen.is_none()).held(&mut held);
        if let Some(reason) = chosen {
            dialog = dialog.ok_reason(reason);
        }
        let shown = dialog.show(ui, |ui, _, fields| {
            Row::new().show(ui, |ui, _| {
                Stack::new().show(ui, |ui, _| self.items_pane(ui, fields));
                Stack::new().show(ui, |ui, _| self.options_pane(ui, fields));
            });
            let (picked, files) = counts(doc.as_deref(), fields);
            let status = crate::i18n::fmt(
                tl!("Selected: {n}, Total number of files to export: {m}"),
                &[("n", &picked.to_string()), ("m", &files.to_string())],
            );
            Text::new(status).shade(Shade::Muted).show(ui);
            selection_params(doc.as_deref(), fields).is_ok()
        });
        self.fields = fields;
        self.held = held;
        match (shown.accepted, shown.closed) {
            (Some(fields), _) => Some(DialogResult::Ok(fields)),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }
}

/// Runs once a frame while the dialog is open: gives it the active document, its artboards'
/// thumbnails and the presets when they changed, applies a preset the ☰ menu or Save Preset picked,
/// renames an artboard whose name was committed in the document it was typed in, and exports the
/// fields OK filed (a failure refuses OK with its reason and the dialog stays open).
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &Context) {
    let Some(dialogs) = app.dialogs() else { return };
    let id = dialog_id(KIND);
    // The export ran when OK was filed: the closed dialog's answer is not read.
    let _ = dialogs.with(|host| host.take_by_id::<ExportForScreensDialog>(id));
    let key = app.session.active().map(|st| (st.uid, st.revision));
    let pdf_presets = pdf::presets(&app.session);
    let saved = &app.session.prefs.screen_export_presets;
    let Some((stale, pending, rename, filed)) = dialogs.with(|host| {
        let d = host.get_mut_by_id::<ExportForScreensDialog>(id)?;
        screen_formats::preset_list(&mut d.presets, saved);
        if d.pdf_presets != pdf_presets {
            d.pdf_presets = pdf_presets;
        }
        Some((d.doc.as_ref().map(|s| s.key) != key, d.pending.take(), d.rename.take(), d.held.take()))
    }) else {
        return;
    };
    if stale {
        let (doc, thumbs) = document_view(app, ctx);
        dialogs.with(|host| {
            if let Some(d) = host.get_mut_by_id::<ExportForScreensDialog>(id) {
                (d.doc, d.thumbs) = (doc, thumbs);
            }
        });
    }
    if let Some(pick) = pending
        && let Some(mut fields) = dialogs.with(|host| host.get_mut_by_id::<ExportForScreensDialog>(id).map(|d| std::mem::take(&mut d.fields)))
    {
        screen_formats::preset_pick(app, &mut fields, pick);
        dialogs.with(|host| {
            if let Some(d) = host.get_mut_by_id::<ExportForScreensDialog>(id) {
                d.fields = fields;
            }
        });
    }
    // A name typed in another document than the active one is dropped.
    if let Some((uid, board, name)) = rename
        && key.is_some_and(|(active, _)| active == uid)
    {
        rename_artboard(app, board, &name);
    }
    if let Some(fields) = filed {
        let answer = params(app, &fields).and_then(|p| io::export_for_screens(app, p)).map(drop).map_err(Label::from);
        dialogs.with(|host| {
            if let Some(d) = host.get_mut_by_id::<ExportForScreensDialog>(id) {
                d.held.answer(answer);
            }
        });
    }
}

/// Items keyed by `K`, with their names, in display order.
struct Items<K>(Vec<(K, String)>);

impl<K: Copy + PartialEq> Items<K> {
    /// The name of the item `key`, "" when there is none.
    fn name(&self, key: K) -> String {
        self.0.iter().find(|(k, _)| *k == key).map(|(_, name)| name.clone()).unwrap_or_default()
    }
}

impl<K: Copy + PartialEq + Send + Sync + 'static> DisplayOrder<K> for Items<K> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> K {
        self.0[at].0
    }

    fn index_of(&self, key: &K) -> Option<usize> {
        self.0.iter().position(|(k, _)| k == key)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.0.get(at).map(|(_, name)| Cow::Borrowed(name.as_str()))
    }
}

impl<K: Copy + PartialEq + Send + Sync + 'static> ItemSource<K> for Items<K> {}
