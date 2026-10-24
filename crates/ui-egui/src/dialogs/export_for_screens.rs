//! File → Export → Export for Screens: the Artboards tab (All, a range or Full Document, Include
//! Bleed, thumbnails with checkboxes) and the Assets tab (the Asset Export panel's assets, with
//! checkboxes), the destination (folder picker, Open Location after Export, Create Sub-folders;
//! the web downloads instead), the format rows (scale as `2x`, `100w`, `100h` or `72ppi`, suffix,
//! format), the presets, Format Settings (the gear) and the prefix. It opens on the settings the
//! document last exported with, except that while the Artboard tool is in use the artboards it has
//! selected are the ones chosen; the format rows are the Asset Export panel's too ([`formats`]).
//!
//! The Layers tab exports each checked layer's art alone, and the Selection tab the selected art as
//! one file or a file per object. A file name pattern (`name`: `<filename>`, `<name>`, `####`,
//! `YYYYMMDD`…, see `document.exportForScreens`) names every file alike, with a live example, and
//! the files go into one folder, or a folder per row, format, item, or item and format
//! (`folders`). Export presets (built-in and saved) set the formats, naming and folders at once.
//! On the desktop with Background Export on, the files are encoded and written on the background
//! thread ([`crate::background`]) after the dialog closes.
//!
//! Fields (`ui.dialog.set`): `tab` (artboards|assets|layers|selection), `select` (all|range|full), `range`,
//! `boards` ([bool] per artboard), `assets` ([asset id] checked on the Assets tab), `includeBleed`,
//! `folder`, `openLocation`, `subfolders`, `prefix`, `preset` (""|mobile|density: where the rows
//! came from), `formats` (rows of `document.exportForScreens`, `scale` as text), `settings`
//! ({png|png8|jpg|webp|gif|tiff|svg|pdf: {…}}), `layers` ([layer id] checked on the Layers tab),
//! `selectionMode` (one|each), `name` (the pattern), `counterStart`, `folders`
//! (none|rows|format|item|itemFormat), `exportPreset` (the preset applied last, "" for none);
//! `__settings` names the format whose Format Settings show ("" when closed), `__savePreset` is the
//! name being typed to save a preset (absent when not saving).

use serde::Deserialize;
use serde_json::{Map, Value, json};
use vectorcraft_engine::cmd::fileio::{self, ArtboardPick, SCREEN_PRESETS, ScreenSize, pdf};

use vectorcraft_doc::{ExportAsset, NodeId};

use super::{DialogSpec, form, png_options, svg_options};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, io, widgets};

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Export for Screens").into(),
    body,
    confirm,
    ok: Some("Export Artboard"),
    ok_label: Some(|app| match (io::is_web(app), app.ui.dialog.as_ref().map(|d| d.str("tab")).as_deref()) {
        (true, _) => "Download",
        (false, Some("assets")) => "Export Asset",
        (false, Some("layers")) => "Export Layer",
        (false, Some("selection")) => "Export Selection",
        (false, _) => "Export Artboard",
    }),
    min_width: 660.0,
    ..DialogSpec::FORM
};

/// `Dialog::kind` of this dialog.
pub const KIND: &str = "exportForScreens";

/// The format choices of a row: (label, format, JPEG quality, with printer's marks).
const FORMATS: [(&str, &str, Option<u8>, bool); 14] = [
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
const FORMAT_LABELS: [&str; 14] =
    ["PNG", "PNG 8", "JPG 100", "JPG 80", "JPG 50", "WebP", "GIF", "TIFF", "SVG", "PDF", "PDF Print", "EPS", "AI", "VectorCraft"];

/// The printer's marks a PDF Print row writes: trim marks, registration targets, colour bars and
/// page information.
fn print_marks() -> Value {
    json!({"trim": true, "registration": true, "colorBars": true, "pageInfo": true})
}

/// Where the files go: (`folders` value, label).
const FOLDERS: [(&str, &str); 5] = [
    ("none", "All in One Folder"),
    ("rows", "By Size or Row"),
    ("format", "By Format"),
    ("item", "By Artboard or Item"),
    ("itemFormat", "By Item, Then Format"),
];
const FOLDER_LABELS: [&str; 5] = ["All in One Folder", "By Size or Row", "By Format", "By Artboard or Item", "By Item, Then Format"];

/// The tokens Insert adds to the file name pattern: (label, token).
const TOKENS: [(&str, &str); 8] = [
    ("Document Name", "<filename>"),
    ("Artboard or Item Name", "<name>"),
    ("Number (####)", "####"),
    ("Date (YYYYMMDD)", "YYYYMMDD"),
    ("Date (YYYY-MM-DD)", "YYYY-MM-DD"),
    ("Time (hhmmss)", "<time>"),
    ("Format", "<format>"),
    ("Scale", "<scale>"),
];
const TOKEN_LABELS: [&str; 8] =
    ["Document Name", "Artboard or Item Name", "Number (####)", "Date (YYYYMMDD)", "Date (YYYY-MM-DD)", "Time (hhmmss)", "Format", "Scale"];

/// The tabs: (`tab` value, label).
const TABS: [&str; 4] = ["artboards", "assets", "layers", "selection"];

/// The formats Format Settings has options for: (format, label).
const SETTINGS: [(&str, &str); 8] =
    [("png", "PNG"), ("png8", "PNG 8"), ("jpg", "JPG"), ("webp", "WebP"), ("gif", "GIF"), ("tiff", "TIFF"), ("svg", "SVG"), ("pdf", "PDF")];
const SETTINGS_LABELS: [&str; 8] = ["PNG", "PNG 8", "JPG", "WebP", "GIF", "TIFF", "SVG", "PDF"];

/// The sizes a row's scale list offers (any can be typed).
const SIZES: [&str; 12] = ["0.5x", "0.75x", "1x", "1.5x", "2x", "3x", "4x", "512w", "512h", "72ppi", "150ppi", "300ppi"];

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
    if let Some(d) = app.ui.dialog.as_mut() {
        d.fields.insert("tab".into(), json!("assets"));
        if let Some(c) = checked {
            d.fields.insert("assets".into(), json!(c));
            // The list scrolls to the first of them once: new assets land at its end.
            if let Some(first) = c.first() {
                d.fields.insert("__scrollTo".into(), json!(first));
            }
        }
    }
}

/// Open the dialog on the settings the active document last exported with (else one PNG row at
/// 1x, into the Desktop), every asset checked. While the Artboard tool is in use, the artboards it
/// has selected (the rows the Artboards panel highlights) are the ones chosen instead.
pub fn open(app: &mut VectorcraftApp) {
    let (n, saved, assets) = app
        .session
        .active()
        .map(|st| (st.doc.artboards.len(), st.doc.export_settings.clone(), st.doc.assets.iter().map(|a| a.id).collect::<Vec<_>>()))
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
    // Where files go: the saved choice, else a folder per row as Create Sub-folders did.
    let folders = get("folders")
        .and_then(Value::as_str)
        .filter(|f| FOLDERS.iter().any(|(id, _)| id == f))
        .map_or_else(|| if flag("subfolders", preset.is_some()) { "rows" } else { "none" }.to_string(), str::to_string);
    let layers: Vec<u64> = app.session.active().map(|st| st.doc.layers.iter().map(|l| l.id.0).collect()).unwrap_or_default();
    let fields = json!({
        "tab": "artboards",
        "select": select,
        "range": range_text(&boards),
        "boards": boards,
        "assets": assets,
        "includeBleed": flag("includeBleed", false),
        "folder": folder.unwrap_or_default(),
        "openLocation": flag("openLocation", true),
        "subfolders": flag("subfolders", preset.is_some()),
        "prefix": get("prefix").and_then(Value::as_str).unwrap_or_default(),
        "preset": preset.unwrap_or_default(),
        "formats": rows,
        "settings": settings,
        "__settings": "",
        "__presets": pdf::presets(&app.session),
        "layers": layers,
        "selectionMode": get("selection").and_then(Value::as_str).filter(|m| *m == "each").unwrap_or("one"),
        "name": get("name").and_then(Value::as_str).unwrap_or_default(),
        "counterStart": get("counterStart").and_then(Value::as_u64).unwrap_or(1),
        "folders": folders,
        "exportPreset": "",
        "__exportPresets": export_presets(app),
    });
    app.ui.dialog = Some(Dialog::new(KIND, fields));
}

/// The options format `id` starts with in Format Settings: the raster options dialogs' defaults
/// (without quality and image maps: the rows choose quality, a map is another file), the SVG
/// Options' first-use ones (without artboards and images: a linked image is another file), or the
/// default PDF preset.
fn defaults(app: &VectorcraftApp, id: &str) -> Map<String, Value> {
    let mut o = match (id, fileio::format(id)) {
        ("svg", _) => svg_options::first_use(),
        ("pdf", _) => Map::from_iter([("preset".to_string(), json!(pdf::DEFAULT_PRESET))]),
        (_, Some(f)) => png_options::defaults(app, f),
        _ => Map::new(),
    };
    for k in ["quality", "imageMap", "useArtboards", "images"] {
        o.remove(k);
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
        options.extend(o.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    options
}

/// A row with its size as the text the Scale field shows (`2x`, `100w`, `100h`, `72ppi`).
fn editable_row(row: &Value) -> Value {
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

/// The art of layer `id` (sublayers unfolded), back to front: what its file holds.
fn layer_art(doc: &vectorcraft_doc::Document, id: NodeId, out: &mut Vec<NodeId>) {
    let Some(n) = doc.node(id) else { return };
    if n.is_layer() {
        for c in n.children().into_iter().flatten() {
            layer_art(doc, c.id, out);
        }
    } else {
        out.push(id);
    }
}

/// The checked layers that are still in the document, in stacking order.
fn checked_layers(app: &VectorcraftApp, d: &Dialog) -> Vec<u64> {
    let on: Vec<u64> = d.fields.get("layers").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
    app.session.active().map(|st| st.doc.layers.iter().map(|l| l.id.0).filter(|id| on.contains(id)).collect()).unwrap_or_default()
}

/// The `document.exportForScreens` params the dialog describes (the web: no folder, one ZIP for
/// several files).
fn params(app: &VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let (mut p, files) = settings_params(app, d)?;
    if io::is_web(app) {
        p.insert("zip".into(), json!(files > 1));
    } else {
        let folder = d.str("folder");
        if folder.trim().is_empty() {
            return Err("choose a folder to export to".into());
        }
        p.insert("folder".into(), json!(folder.trim()));
        p.insert("openLocation".into(), json!(d.bool("openLocation")));
    }
    Ok(Value::Object(p))
}

/// The counter's first value: the Start at field (a number, or text typed through `ui.dialog.set`).
fn counter_start(d: &Dialog) -> u64 {
    d.fields.get("counterStart").and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|t| t.trim().parse().ok()))).unwrap_or(1)
}

/// The params of [`params`] without the destination (what is exported, at which sizes, and how the
/// files are named and sorted), and how many files that is.
fn settings_params(app: &VectorcraftApp, d: &Dialog) -> Result<(Map<String, Value>, usize), String> {
    let n = app.session.active().map_or(0, |st| st.doc.artboards.len());
    let rows = d.fields.get("formats").and_then(Value::as_array).cloned().unwrap_or_default();
    if rows.is_empty() {
        return Err("add a format to export".into());
    }
    let mut p = Map::new();
    // What each row writes a file of: the checked assets, layers, the selection, else the
    // artboards.
    let tab = d.str("tab");
    let pieces = if tab == "assets" {
        let checked = checked_assets(app, d);
        if checked.is_empty() {
            return Err("check an asset to export".into());
        }
        let count = checked.len();
        p.insert("assets".into(), json!(checked));
        count
    } else if tab == "layers" {
        let checked = checked_layers(app, d);
        if checked.is_empty() {
            return Err("check a layer to export".into());
        }
        let count = checked.len();
        p.insert("layers".into(), json!(checked));
        count
    } else if tab == "selection" {
        let selected = app.session.active().map_or(0, |st| st.selection.objects.len());
        if selected == 0 {
            return Err("select the art to export first".into());
        }
        let each = d.str("selectionMode") == "each";
        p.insert("selection".into(), json!(if each { "each" } else { "one" }));
        if each { selected } else { 1 }
    } else {
        match d.str("select").as_str() {
            "full" => {
                p.insert("fullDocument".into(), json!(true));
                1
            }
            "range" => {
                let range = d.str("range");
                // A bad range keeps the dialog open.
                let chosen = ArtboardPick { range: Some(range.clone()), ..Default::default() }.resolve(n)?.unwrap_or_default();
                p.insert("range".into(), json!(range));
                chosen.len()
            }
            _ => n,
        }
    };
    let files = pieces * rows.len();
    p.insert("includeBleed".into(), json!(d.bool("includeBleed")));
    let folders = d.str("folders");
    let folders = match (FOLDERS.iter().any(|(id, _)| *id == folders), d.bool("subfolders")) {
        // Create Sub-folders (as the Asset Export panel and older scripts set it) is a folder per
        // row while Folders is still All in One Folder.
        (true, true) if folders == "none" => "rows".to_string(),
        (true, _) => folders,
        (false, sub) => if sub { "rows" } else { "none" }.to_string(),
    };
    // Create Sub-folders, as the Asset Export panel (which shares these settings) still names it.
    p.insert("subfolders".into(), json!(folders == "rows"));
    p.insert("folders".into(), json!(folders));
    p.insert("prefix".into(), json!(d.str("prefix")));
    let name = d.str("name");
    if !name.trim().is_empty() {
        p.insert("name".into(), json!(name.trim()));
    }
    p.insert("counterStart".into(), json!(counter_start(d)));
    p.insert("formats".into(), Value::Array(rows));
    p.insert("settings".into(), d.fields.get("settings").cloned().unwrap_or(json!({})));
    Ok((p, files))
}

/// Export; the dialog stays open when the export fails.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let p = params(app, d)?;
    let r = io::export_for_screens(app, p);
    if r.is_ok() {
        app.ui.dialog = None;
    }
    r
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    let tab = TABS.iter().position(|k| *k == d.str("tab")).unwrap_or(0);
    if let Some(i) = widgets::tab_bar(ui, &[tl!("Artboards"), tl!("Assets"), tl!("Layers"), tl!("Selection")], tab) {
        d.fields.insert("tab".into(), json!(TABS.get(i).copied().unwrap_or("artboards")));
    }
    ui.add_space(8.0);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(270.0);
            match tab {
                1 => assets(app, ui, d),
                2 => layers(app, ui, d),
                3 => selection(app, ui, d),
                _ => artboards(app, ui, d),
            }
        });
        let (sep, _) = ui.allocate_exact_size(egui::vec2(17.0, 380.0), egui::Sense::hover());
        ui.painter().line_segment([sep.center_top(), sep.center_bottom()], egui::Stroke::new(1.0, t.divider));
        ui.vertical(|ui| {
            ui.set_width(350.0);
            let shown = d.str("__settings");
            if SETTINGS.iter().any(|(id, _)| *id == shown) {
                format_settings(ui, d, &shown);
            } else {
                presets(app, ui, d);
                ui.add_space(8.0);
                destination(app, ui, d);
                ui.add_space(10.0);
                formats(ui, d);
                ui.add_space(8.0);
                naming(app, ui, d);
            }
        });
    });
    false
}

fn set_select(d: &mut Dialog, select: &str) {
    d.fields.insert("select".into(), json!(select));
}

/// Select (All, Range, Full Document), Include Bleed, and the artboards' thumbnails and checkboxes.
fn artboards(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    let n = app.session.active().map_or(0, |st| st.doc.artboards.len());
    let mut boards: Vec<bool> =
        d.fields.get("boards").and_then(Value::as_array).map(|a| a.iter().map(|b| b.as_bool().unwrap_or(false)).collect()).unwrap_or_default();
    boards.resize(n, true);
    let before = boards.clone();
    let select = d.str("select");
    widgets::dim_label(ui, tl!("Select:"));
    ui.horizontal(|ui| {
        if widgets::radio(ui, tl!("All"), select == "all", true) {
            set_select(d, "all");
            boards.iter_mut().for_each(|b| *b = true);
        }
        if widgets::radio(ui, tl!("Range:"), select == "range", true) {
            set_select(d, "range");
        }
        ui.add_enabled_ui(select == "range", |ui| {
            if form::text(ui, d, "range", 90.0)
                && let Ok(Some(chosen)) = (ArtboardPick { range: Some(d.str("range")), ..Default::default() }).resolve(n)
            {
                boards = (0..n).map(|i| chosen.contains(&i)).collect();
            }
        });
    });
    if widgets::radio(ui, tl!("Full Document"), select == "full", true) {
        set_select(d, "full");
    }
    ui.add_space(4.0);
    form::check(ui, d, "includeBleed", tl!("Include Bleed"));
    ui.add_space(6.0);
    let typed = boards != before;
    let checked = boards.clone();
    ui.add_enabled_ui(d.str("select") != "full", |ui| {
        ui.horizontal(|ui| {
            if ui.small_button(tl!("Select All")).clicked() {
                boards.iter_mut().for_each(|b| *b = true);
            }
            if ui.small_button(tl!("Clear")).clicked() {
                boards.iter_mut().for_each(|b| *b = false);
            }
        });
        let names: Vec<String> = app.session.active().map(|s| s.doc.artboards.iter().map(|a| a.name.clone()).collect()).unwrap_or_default();
        egui::ScrollArea::vertical().max_height(250.0).show(ui, |ui| {
            for (i, name) in names.iter().enumerate() {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(46.0, 46.0), egui::Sense::hover());
                    match artboard_thumb(app, ui.ctx(), i) {
                        Some(tex) => {
                            let sz = tex.size_vec2();
                            let s = (46.0 / sz.x.max(sz.y)).min(1.0);
                            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                            ui.painter().image(tex.id(), egui::Rect::from_center_size(r.center(), sz * s), uv, egui::Color32::WHITE);
                        }
                        None => {
                            ui.painter().rect_filled(r, 2.0, egui::Color32::WHITE);
                        }
                    }
                    if let Some(b) = boards.get_mut(i)
                        && crate::widgets::check_name(ui, name, *b)
                    {
                        *b ^= true;
                    }
                });
            }
        });
    });
    // Checking artboards picks them as the range (all of them: All); a typed range keeps its text.
    if boards != checked {
        set_select(d, if boards.iter().all(|b| *b) { "all" } else { "range" });
        d.fields.insert("range".into(), json!(range_text(&boards)));
    }
    if typed || boards != checked {
        d.fields.insert("boards".into(), json!(boards));
    }
}

/// The checked assets that are still in the document, in panel order.
fn checked_assets(app: &VectorcraftApp, d: &Dialog) -> Vec<u64> {
    let on: Vec<u64> = d.fields.get("assets").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
    app.session.active().map(|st| st.doc.assets.iter().map(|a| a.id).filter(|id| on.contains(id)).collect()).unwrap_or_default()
}

/// The Assets tab: each asset's thumbnail and checkbox, Select All and Clear.
fn assets(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    let Some(st) = app.session.active() else { return };
    if st.doc.assets.is_empty() {
        ui.add_space(24.0);
        widgets::dim_label(ui, tl!("No assets yet. Collect art with Object › Collect for Export, or drag it into the Asset Export panel."));
        return;
    }
    let mut on = checked_assets(app, d);
    let before = on.clone();
    ui.horizontal(|ui| {
        if ui.small_button(tl!("Select All")).clicked() {
            on = st.doc.assets.iter().map(|a| a.id).collect();
        }
        if ui.small_button(tl!("Clear")).clicked() {
            on.clear();
        }
    });
    // Asked for until the row is in view: the dialog's first frame only measures it.
    let scroll_to = d.fields.get("__scrollTo").and_then(Value::as_u64).filter(|id| st.doc.assets.iter().any(|a| a.id == *id));
    let mut in_view = scroll_to.is_none();
    egui::ScrollArea::vertical().max_height(330.0).show(ui, |ui| {
        for a in &st.doc.assets {
            let row = ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(egui::vec2(46.0, 46.0), egui::Sense::hover());
                crate::panels::asset_export::paint_thumb(ui, st, a, r);
                let checked = on.contains(&a.id);
                if crate::widgets::check_name(ui, &a.name, checked) {
                    on.retain(|id| *id != a.id);
                    if !checked {
                        on.push(a.id);
                    }
                }
            });
            if scroll_to == Some(a.id) {
                if ui.clip_rect().contains_rect(row.response.rect) {
                    in_view = true;
                } else {
                    row.response.scroll_to_me(Some(egui::Align::Center));
                }
            }
        }
    });
    if in_view {
        d.fields.remove("__scrollTo");
    }
    if on != before {
        d.fields.insert("assets".into(), json!(on));
    }
}

/// Export to: the folder (typed or picked), Open Location after Export and Create Sub-folders; the
/// web downloads instead.
fn destination(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    widgets::dim_label(ui, tl!("Export to:"));
    if io::is_web(app) {
        widgets::dim_label(ui, tl!("Your browser downloads the files (several as one .zip)."));
    } else {
        let pick = app.services.pick_folder.is_some();
        ui.horizontal(|ui| {
            form::text(ui, d, "folder", if pick { 300.0 } else { 334.0 });
            if pick && widgets::icon_button(ui, "folder-open", tl!("Choose a folder"), false, 26.0).clicked() {
                crate::picks::folder_field(app, d, "folder");
            }
        });
        form::check(ui, d, "openLocation", tl!("Open Location after Export"));
    }
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Folders:"));
        let current = d.str("folders");
        let at = FOLDERS.iter().position(|(id, _)| *id == current).unwrap_or(0);
        if let Some((id, _)) = widgets::dropdown(ui, "efs-folders", FOLDER_LABELS[at], &FOLDER_LABELS, 190.0).and_then(|i| FOLDERS.get(i)) {
            d.fields.insert("folders".into(), json!(id));
            d.fields.insert("subfolders".into(), json!(*id == "rows"));
        }
    });
}

/// Export Preset: apply a built-in or saved preset (its formats, settings, naming and folders),
/// save the dialog's settings as one (Save…, then a name), or delete a saved one.
fn presets(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    let list: Vec<Value> = d.fields.get("__exportPresets").and_then(Value::as_array).cloned().unwrap_or_default();
    let names: Vec<&str> = list.iter().filter_map(|p| p["name"].as_str()).collect();
    let current = d.str("exportPreset");
    let saved = list.iter().any(|p| p["name"] == current && p["builtIn"] == false);
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Export Preset:"));
        let labels: Vec<&str> = std::iter::once("Custom").chain(names.iter().copied()).collect();
        let at = names.iter().position(|n| *n == current).map_or(0, |i| i + 1);
        if let Some(i) = widgets::dropdown(ui, "efs-export-preset", labels[at], &labels, 150.0)
            && let Some(preset) = i.checked_sub(1).and_then(|i| list.get(i))
        {
            apply_preset(d, preset);
        }
        if widgets::icon_button(ui, "save", tl!("Save Preset…"), false, 24.0).clicked() {
            let name = if saved { current.clone() } else { String::new() };
            d.fields.insert("__savePreset".into(), json!(name));
        }
        ui.add_enabled_ui(saved, |ui| {
            if widgets::icon_button(ui, "trash-2", tl!("Delete Preset"), false, 24.0).clicked()
                && app.run("document.exportPresets.delete", json!({"name": current})).is_ok()
            {
                d.fields.insert("exportPreset".into(), json!(""));
                d.fields.insert("__exportPresets".into(), export_presets(app));
            }
        });
    });
    if d.fields.contains_key("__savePreset") {
        ui.horizontal(|ui| {
            widgets::dim_label(ui, tl!("Name:"));
            form::text(ui, d, "__savePreset", 150.0);
            let name = d.str("__savePreset").trim().to_string();
            if ui.add_enabled(!name.is_empty(), egui::Button::new(tl!("Save"))).clicked() {
                let settings = params(app, d).unwrap_or_else(|_| json!({"formats": d.fields.get("formats").cloned().unwrap_or(json!([]))}));
                match app.run("document.exportPresets.save", json!({"name": name, "settings": settings})) {
                    Ok(_) => {
                        d.fields.remove("__savePreset");
                        d.fields.insert("exportPreset".into(), json!(name));
                        d.fields.insert("__exportPresets".into(), export_presets(app));
                    }
                    Err(e) => app.ui.status = e,
                }
            }
            if ui.small_button(tl!("Cancel")).clicked() {
                d.fields.remove("__savePreset");
            }
        });
    }
}

/// Set the dialog to preset `preset` (`{name, settings}`): its formats, settings, naming and
/// folders; what it doesn't name keeps the dialog's value.
pub(super) fn apply_preset(d: &mut Dialog, preset: &Value) {
    let st = &preset["settings"];
    if let Some(rows) = st["formats"].as_array() {
        d.fields.insert("formats".into(), json!(rows.iter().filter(|r| r.is_object()).map(editable_row).collect::<Vec<_>>()));
        d.fields.insert("preset".into(), json!(""));
    }
    if let (Some(all), Some(given)) = (d.fields.get_mut("settings").and_then(Value::as_object_mut), st["settings"].as_object()) {
        for (id, o) in given {
            match (all.get_mut(id).and_then(Value::as_object_mut), o.as_object()) {
                (Some(mine), Some(o)) => mine.extend(o.iter().map(|(k, v)| (k.clone(), v.clone()))),
                _ => _ = all.insert(id.clone(), o.clone()),
            }
        }
    }
    let folders = st["folders"].as_str().or_else(|| st["subfolders"].as_bool().map(|b| if b { "rows" } else { "none" }));
    if let Some(f) = folders {
        d.fields.insert("folders".into(), json!(f));
        d.fields.insert("subfolders".into(), json!(f == "rows"));
    }
    for k in ["name", "prefix"] {
        d.fields.insert(k.into(), json!(st[k].as_str().unwrap_or_default()));
    }
    d.fields.insert("counterStart".into(), json!(st["counterStart"].as_u64().unwrap_or(1)));
    if let Some(b) = st["includeBleed"].as_bool() {
        d.fields.insert("includeBleed".into(), json!(b));
    }
    if let Some(f) = st["folder"].as_str().filter(|f| !f.is_empty()) {
        d.fields.insert("folder".into(), json!(f));
    }
    d.fields.insert("exportPreset".into(), json!(preset["name"]));
}

/// File Name: the pattern (with Insert for its tokens), the counter's start, the prefix and an
/// example of the first file's name.
fn naming(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("File Name:"));
        // A pattern typed by hand is no longer the preset's.
        if form::text(ui, d, "name", 170.0) {
            d.fields.insert("exportPreset".into(), json!(""));
        }
        if let Some((_, token)) = widgets::dropdown(ui, "efs-name-token", tl!("Insert"), &TOKEN_LABELS, 90.0).and_then(|i| TOKENS.get(i)) {
            let name = d.str("name");
            d.fields.insert("name".into(), json!(format!("{name}{token}")));
        }
    });
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Prefix:"));
        form::text(ui, d, "prefix", 110.0);
        widgets::dim_label(ui, tl!("Start at:"));
        if let Some(v) = widgets::range_field(ui, "efs-counter-start", counter_start(d) as f64, 0.0..=999_999.0, "", 0, 64.0) {
            d.fields.insert("counterStart".into(), json!(v.round().max(0.0) as u64));
        }
    });
    let example = example_name(app, d);
    if !example.is_empty() {
        widgets::dim_label(ui, &crate::i18n::fmt(tl!("Example: {name}"), &[("name", &example)]));
    }
}

/// The first file the export writes, relative to the export folder, as `document.exportForScreens`
/// names it ([`fileio::screen_first_file`]); empty while the dialog's settings choose nothing to
/// export (no format, nothing checked or selected, a bad range).
pub(super) fn example_name(app: &VectorcraftApp, d: &Dialog) -> String {
    settings_params(app, d).ok().and_then(|(p, _)| fileio::screen_first_file(&app.session, &Value::Object(p)).ok()).unwrap_or_default()
}

/// The Layers tab: each top-level layer's thumbnail and checkbox (a file of its art alone),
/// Select All and Clear.
fn layers(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    let Some(st) = app.session.active() else { return };
    let mut on = checked_layers(app, d);
    let before = on.clone();
    ui.horizontal(|ui| {
        if ui.small_button(tl!("Select All")).clicked() {
            on = st.doc.layers.iter().map(|l| l.id.0).collect();
        }
        if ui.small_button(tl!("Clear")).clicked() {
            on.clear();
        }
    });
    egui::ScrollArea::vertical().max_height(330.0).show(ui, |ui| {
        for l in &st.doc.layers {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(egui::vec2(46.0, 46.0), egui::Sense::hover());
                let mut art = vec![];
                layer_art(&st.doc, l.id, &mut art);
                // An id of its own for the thumbnail cache, away from the assets' ids.
                let a = ExportAsset { id: u64::MAX - l.id.0, name: l.display_name(), nodes: art };
                crate::panels::asset_export::paint_thumb(ui, st, &a, r);
                let checked = on.contains(&l.id.0);
                if crate::widgets::check_name(ui, &l.display_name(), checked) {
                    on.retain(|id| *id != l.id.0);
                    if !checked {
                        on.push(l.id.0);
                    }
                }
            });
        }
    });
    if on != before {
        d.fields.insert("layers".into(), json!(on));
    }
}

/// The Selection tab: what is selected (a thumbnail and the count), as one file or a file per
/// object.
fn selection(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    let Some(st) = app.session.active() else { return };
    let ids = st.selection.objects.clone();
    if ids.is_empty() {
        ui.add_space(24.0);
        widgets::dim_label(ui, tl!("Nothing is selected. Select the art to export, then open this tab again."));
        return;
    }
    let (r, _) = ui.allocate_exact_size(egui::vec2(120.0, 120.0), egui::Sense::hover());
    // The selection changes without a new revision: its ids name its thumbnail.
    let key = ids.iter().fold(0x005e_1ec7_u64, |h, id| h.rotate_left(7) ^ id.0);
    let a = ExportAsset { id: u64::MAX / 2 + (key % (u64::MAX / 4)), name: "Selection".into(), nodes: ids.clone() };
    crate::panels::asset_export::paint_thumb(ui, st, &a, r);
    let count = ids.len().to_string();
    widgets::dim_label(ui, &crate::i18n::fmt(tl!("{n} objects selected"), &[("n", &count)]));
    ui.add_space(6.0);
    let mode = d.str("selectionMode");
    if widgets::radio(ui, tl!("One File of the Selection"), mode != "each", true) {
        d.fields.insert("selectionMode".into(), json!("one"));
    }
    if widgets::radio(ui, tl!("A File per Object"), mode == "each", true) {
        d.fields.insert("selectionMode".into(), json!("each"));
    }
}

/// The preset, the Format Settings gear (which sets `__settings`), the format rows (scale, suffix,
/// format) and Add Scale, as wide as `ui` allows: the dialog's and the Asset Export panel's rows.
pub(crate) fn formats(ui: &mut egui::Ui, d: &mut Dialog) {
    let mut rows: Vec<Value> = d.fields.get("formats").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut edited = false;
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Formats:"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::icon_button(ui, "settings", tl!("Format Settings"), false, 24.0).clicked() {
                // Preserve row spellings; format settings use canonical IDs.
                let first = rows
                    .first()
                    .and_then(|r| r["format"].as_str())
                    .and_then(fileio::format)
                    .map(|f| f.id)
                    .filter(|f| SETTINGS.iter().any(|(id, _)| id == f));
                d.fields.insert("__settings".into(), json!(first.unwrap_or("png")));
            }
            let preset = d.str("preset");
            let labels: Vec<&str> = std::iter::once("Custom").chain(SCREEN_PRESETS.iter().map(|(_, l)| *l)).collect();
            let at = SCREEN_PRESETS.iter().position(|(id, _)| *id == preset).map_or(0, |i| i + 1);
            if let Some(i) = widgets::dropdown(ui, "efs-preset", labels[at], &labels, (ui.available_width() - 8.0).clamp(90.0, 180.0)) {
                let id = i.checked_sub(1).and_then(|i| SCREEN_PRESETS.get(i)).map_or("", |(id, _)| *id);
                if let Some(preset_rows) = fileio::screen_preset_rows(id) {
                    rows = preset_rows;
                    d.fields.insert("subfolders".into(), json!(true));
                    d.fields.insert("formats".into(), json!(rows));
                }
                d.fields.insert("preset".into(), json!(id));
            }
        });
    });
    let mut remove = None;
    let one = rows.len() == 1;
    // The columns at the dialog's widths, narrower in a narrower column (the panel): the three
    // fields share what the remove buttons, the spacing and the fields' frames leave.
    let k = ((ui.available_width() - 66.0) / 262.0).clamp(0.6, 1.0);
    let (scale_w, suffix_w, format_w) = (96.0 * k, 70.0 * k, 96.0 * k);
    egui::Grid::new("efs-formats").num_columns(4).spacing([8.0, 6.0]).show(ui, |ui| {
        for head in [tl!("Scale"), tl!("Suffix"), tl!("Format"), ""] {
            widgets::dim_label(ui, head);
        }
        ui.end_row();
        for (i, row) in rows.iter_mut().enumerate() {
            let raster = row["format"].as_str().and_then(fileio::format).is_none_or(|f| f.raster);
            let scale = row["scale"].as_str().unwrap_or("1x").to_string();
            ui.add_enabled_ui(raster, |ui| {
                if let Some(s) = widgets::text_presets(ui, ("efs-scale", i), &scale, &SIZES, scale_w) {
                    // The suffix follows the size while it is the automatic one.
                    let auto = |text: &str| ScreenSize::parse(text).map(ScreenSize::suffix);
                    if row["suffix"].as_str().is_none_or(|x| Some(x.to_string()) == auto(&scale))
                        && let Some(suffix) = auto(&s)
                    {
                        row["suffix"] = json!(suffix);
                    }
                    row["scale"] = json!(s);
                    edited = true;
                }
            });
            if let Some(s) = widgets::text_field(ui, ("efs-suffix", i), row["suffix"].as_str(), suffix_w, 1) {
                row["suffix"] = json!(s);
                edited = true;
            }
            let (format, quality) = (row["format"].as_str().unwrap_or("png"), row["quality"].as_u64());
            let marked = row.get("marks").is_some_and(|m| !m.is_null());
            let current =
                FORMATS.iter().position(|(_, f, q, print)| *f == format && *print == marked && q.is_none_or(|q| Some(u64::from(q)) == quality));
            let label = current.map_or_else(|| format.to_uppercase(), |i| FORMAT_LABELS[i].to_string());
            if let Some((_, f, q, print)) = widgets::dropdown(ui, ("efs-fmt", i), &label, &FORMAT_LABELS, format_w).and_then(|i| FORMATS.get(i)) {
                row["format"] = json!(f);
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
            ui.add_enabled_ui(!one, |ui| {
                if widgets::icon_button(ui, "x", tl!("Remove"), false, 22.0).clicked() {
                    remove = Some(i);
                }
            });
            ui.end_row();
        }
    });
    if let Some(i) = remove {
        rows.remove(i);
        edited = true;
    }
    if widgets::secondary_button(ui, tl!("+ Add Scale")).clicked() {
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
        d.fields.insert("formats".into(), json!(rows));
        d.fields.insert("preset".into(), json!(""));
    }
}

/// Format Settings: the options of format `id` that apply to every row of it, then Done.
fn format_settings(ui: &mut egui::Ui, d: &mut Dialog, id: &str) {
    let t = Tokens::get(ui.ctx());
    widgets::subheader(ui, tl!("Format Settings"));
    ui.add_space(6.0);
    let at = SETTINGS.iter().position(|(f, _)| *f == id).unwrap_or(0);
    if let Some((f, _)) = widgets::dropdown(ui, "efs-settings-format", SETTINGS_LABELS[at], &SETTINGS_LABELS, 150.0).and_then(|i| SETTINGS.get(i)) {
        d.fields.insert("__settings".into(), json!(f));
    }
    ui.add_space(8.0);
    // The format's options as a dialog of their own, for the options dialogs' rows.
    let fields = d.fields.get("settings").and_then(|all| all.get(id)).and_then(Value::as_object).cloned().unwrap_or_default();
    let mut s = Dialog { kind: KIND.into(), fields };
    match id {
        "svg" => svg_options::option_fields(ui, &mut s, true),
        "pdf" => {
            let labels: Vec<&str> =
                d.fields.get("__presets").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tl!("PDF Preset:")).color(t.text_dim));
                // The built-in presets are ours (translated); the saved ones are names.
                let builtin = |k: usize| labels.get(k).is_some_and(|n| super::save_pdf::is_builtin_preset(n));
                if let Some(p) = super::mixed_dropdown(ui, "efs-pdf-preset", &s.str("preset"), &labels, 220.0, builtin).and_then(|i| labels.get(i)) {
                    s.fields.insert("preset".into(), json!(p));
                }
            });
        }
        _ => {
            egui::Grid::new("efs-settings").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| png_options::option_rows(ui, &mut s, id, true));
        }
    }
    if let Some(all) = d.fields.get_mut("settings").and_then(Value::as_object_mut) {
        all.insert(id.into(), Value::Object(s.fields));
    }
    ui.add_space(10.0);
    if widgets::secondary_button(ui, tl!("Done")).clicked() {
        d.fields.insert("__settings".into(), json!(""));
    }
}

/// A small cached rendering of artboard `i` (keyed by document revision).
pub(super) fn artboard_thumb(app: &mut VectorcraftApp, ctx: &egui::Context, i: usize) -> Option<egui::TextureHandle> {
    let st = app.session.active()?;
    let key = egui::Id::new(("ab-thumb", i, st.revision));
    if let Some(t) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(key)) {
        return Some(t);
    }
    let doc = st.doc.clone();
    let r = doc.artboards.get(i)?.rect;
    let tex = crate::widgets::region_texture(ctx, &mut app.canvas.renderer, &format!("ab-thumb-{i}"), &doc, r, 92.0);
    ctx.data_mut(|d| d.insert_temp(key, tex.clone()));
    Some(tex)
}
