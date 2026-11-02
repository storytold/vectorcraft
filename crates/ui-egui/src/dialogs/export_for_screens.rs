//! File → Export → Export for Screens, laid out as Illustrator's: on the left the Artboards and
//! Assets tabs (thumbnails with checkboxes, in a grid or a list), Clear Selection and the prefix;
//! on the right Select (All, a range or Full Document, Include Bleed, Include Artboard Fill), the destination (folder
//! picker, Open Location after Export, Create Sub-folders by scale or by format; the web downloads
//! instead), Export PDFs As (one file of every artboard or a file each), and the format rows (scale
//! as `2x`, `100w`, `100h` or `72ppi`, suffix, format) with the iOS and Android rows, the format
//! presets (the ☰ menu) and Format Settings (the gear); below, how many items are selected and
//! how many files the export writes. It opens on the settings the document last exported with,
//! except that while the Artboard tool is in use the artboards it has selected are the ones chosen;
//! the format rows are the Asset Export panel's too ([`formats`]). On the desktop with Background
//! Export on, the files are encoded and written on the background thread
//! ([`crate::background`]) after the dialog closes.
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

use serde::Deserialize;
use serde_json::{Map, Value, json};
use vectorcraft_engine::cmd::fileio::{self, ArtboardPick, ScreenSize, pdf};

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
        (false, _) => "Export Artboard",
    }),
    min_width: 720.0,
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

/// The tabs: (`tab` value, label).
const TABS: [&str; 2] = ["artboards", "assets"];

/// The left pane's width and the height of its thumbnails' area; the right pane's width.
const LEFT_W: f32 = 300.0;
const PANE_H: f32 = 360.0;
const RIGHT_W: f32 = 370.0;

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
/// 1x, into the Desktop, with Create Sub-folders on), every asset checked. While the Artboard tool
/// is in use, the artboards it has selected (the rows the Artboards panel highlights) are the ones
/// chosen instead.
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
    // Create Sub-folders is on until an export turns it off; a folder per format is the saved
    // `folders: "format"`, any other sorting a folder per scale.
    let folders = get("folders").and_then(Value::as_str);
    let subfolders = match folders {
        Some("none") => false,
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
        "pdfs": if get("pdfs").and_then(Value::as_str) == Some("single") { "single" } else { "multiple" },
        "prefix": get("prefix").and_then(Value::as_str).unwrap_or_default(),
        "preset": preset.unwrap_or_default(),
        "formats": rows,
        "settings": settings,
        "view": "grid",
        "__settings": "",
        "__presets": pdf::presets(&app.session),
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

/// The params of [`params`] without the destination (what is exported, at which sizes, and how the
/// files are named and sorted), and how many files that is.
fn settings_params(app: &VectorcraftApp, d: &Dialog) -> Result<(Map<String, Value>, usize), String> {
    let (p, files, _) = selection_params(app, d)?;
    Ok((p, files))
}

/// [`settings_params`] and how many artboards or assets are chosen.
fn selection_params(app: &VectorcraftApp, d: &Dialog) -> Result<(Map<String, Value>, usize, usize), String> {
    let n = app.session.active().map_or(0, |st| st.doc.artboards.len());
    let rows = d.fields.get("formats").and_then(Value::as_array).cloned().unwrap_or_default();
    if rows.is_empty() {
        return Err("add a format to export".into());
    }
    let mut p = Map::new();
    // What each row writes a file of: the checked assets, else the artboards.
    let artboards = d.str("tab") != "assets";
    let pieces = if !artboards {
        let checked = checked_assets(app, d);
        if checked.is_empty() {
            return Err("check an asset to export".into());
        }
        let count = checked.len();
        p.insert("assets".into(), json!(checked));
        count
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
    let single = artboards && pieces > 1 && d.str("select") != "full" && d.str("pdfs") == "single";
    let pdf_rows = rows.iter().filter(|r| r["format"].as_str().and_then(fileio::format).is_some_and(|f| f.id == "pdf")).count();
    let files = if single { (rows.len() - pdf_rows) * pieces + pdf_rows } else { rows.len() * pieces };
    p.insert("includeBleed".into(), json!(d.bool("includeBleed")));
    p.insert("includeArtboardFill".into(), json!(d.bool("includeArtboardFill")));
    let folders = match (d.bool("subfolders"), d.str("subfolderBy").as_str()) {
        (false, _) => "none",
        (true, "format") => "format",
        (true, _) => "rows",
    };
    // Create Sub-folders, as the Asset Export panel (which shares these settings) names it.
    p.insert("subfolders".into(), json!(folders == "rows"));
    p.insert("folders".into(), json!(folders));
    p.insert("pdfs".into(), json!(if single { "single" } else { "multiple" }));
    p.insert("prefix".into(), json!(d.str("prefix")));
    p.insert("formats".into(), Value::Array(rows));
    p.insert("settings".into(), d.fields.get("settings").cloned().unwrap_or(json!({})));
    Ok((p, files, pieces))
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
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(LEFT_W);
            if let Some(i) = widgets::tab_bar(ui, &[tl!("Artboards"), tl!("Assets")], tab) {
                d.fields.insert("tab".into(), json!(TABS.get(i).copied().unwrap_or("artboards")));
            }
            ui.add_space(6.0);
            let list = d.str("view") == "list";
            let (pane, _) = ui.allocate_exact_size(egui::vec2(LEFT_W, PANE_H), egui::Sense::hover());
            ui.painter().rect_filled(pane, 2.0, t.panel_darker);
            ui.painter().rect_stroke(pane, 2.0, egui::Stroke::new(1.0, t.divider), egui::StrokeKind::Inside);
            let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(pane.shrink(8.0)).layout(egui::Layout::top_down(egui::Align::Min)));
            match tab {
                1 => assets(app, &mut inner, d, list),
                _ => artboard_tiles(app, &mut inner, d, list),
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                for (view, icon, tip) in [("grid", "layout-grid", tl!("Grid View")), ("list", "dc-list-view", tl!("List View"))] {
                    if widgets::icon_button(ui, icon, tip, d.str("view") == view || (view == "grid" && !list && d.str("view") != "list"), 22.0)
                        .clicked()
                    {
                        d.fields.insert("view".into(), json!(view));
                    }
                }
                ui.add_space(6.0);
                if ui.small_button(tl!("Clear Selection")).clicked() {
                    clear_selection(app, d, tab);
                }
                widgets::dim_label(ui, tl!("Prefix:"));
                form::text(ui, d, "prefix", (ui.available_width() - 4.0).max(60.0));
            });
        });
        let (sep, _) = ui.allocate_exact_size(egui::vec2(17.0, PANE_H + 60.0), egui::Sense::hover());
        ui.painter().line_segment([sep.center_top(), sep.center_bottom()], egui::Stroke::new(1.0, t.divider));
        ui.vertical(|ui| {
            ui.set_width(RIGHT_W);
            let shown = d.str("__settings");
            if SETTINGS.iter().any(|(id, _)| *id == shown) {
                format_settings(ui, d, &shown);
            } else {
                if tab == 0 {
                    select_options(app, ui, d);
                    ui.add_space(8.0);
                }
                destination(app, ui, d);
                ui.add_space(6.0);
                pdf_mode(ui, d, tab == 0);
                ui.add_space(10.0);
                formats(ui, d, Some(&mut |ui: &mut egui::Ui, d: &mut Dialog| preset_menu(app, ui, d)));
                save_preset_row(app, ui, d);
            }
        });
    });
    ui.add_space(8.0);
    let (picked, files) = counts(app, d);
    let status =
        crate::i18n::fmt(tl!("Selected: {n}, Total number of files to export: {m}"), &[("n", &picked.to_string()), ("m", &files.to_string())]);
    widgets::dim_label(ui, &status);
    false
}

/// How many artboards or assets are chosen and how many files the export writes (0 while the
/// settings choose nothing to export).
fn counts(app: &VectorcraftApp, d: &Dialog) -> (usize, usize) {
    match selection_params(app, d) {
        Ok((_, files, pieces)) => (pieces, files),
        Err(_) => (if d.str("tab") == "assets" { checked_assets(app, d).len() } else { 0 }, 0),
    }
}

/// Clear Selection: no artboard (or, on the Assets tab, no asset) is checked.
fn clear_selection(app: &VectorcraftApp, d: &mut Dialog, tab: usize) {
    if tab == 1 {
        d.fields.insert("assets".into(), json!([]));
    } else {
        let n = app.session.active().map_or(0, |st| st.doc.artboards.len());
        d.fields.insert("boards".into(), json!(vec![false; n]));
        set_select(d, "range");
        d.fields.insert("range".into(), json!(""));
    }
}

fn set_select(d: &mut Dialog, select: &str) {
    d.fields.insert("select".into(), json!(select));
}

/// The checked artboards, one per artboard (all checked when the field is short).
fn boards(app: &VectorcraftApp, d: &Dialog) -> Vec<bool> {
    let n = app.session.active().map_or(0, |st| st.doc.artboards.len());
    let mut boards: Vec<bool> =
        d.fields.get("boards").and_then(Value::as_array).map(|a| a.iter().map(|b| b.as_bool().unwrap_or(false)).collect()).unwrap_or_default();
    boards.resize(n, true);
    boards
}

/// Select: All or a Range (which checks the artboards it names), Include Bleed, Include Artboard
/// Fill and Full Document.
fn select_options(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    let n = app.session.active().map_or(0, |st| st.doc.artboards.len());
    let select = d.str("select");
    widgets::dim_label(ui, tl!("Select:"));
    ui.horizontal(|ui| {
        if widgets::radio(ui, tl!("All"), select == "all", true) {
            set_select(d, "all");
            d.fields.insert("boards".into(), json!(vec![true; n]));
            d.fields.insert("range".into(), json!(range_text(&vec![true; n])));
        }
        if widgets::radio(ui, tl!("Range:"), select == "range", true) {
            set_select(d, "range");
        }
        ui.add_enabled_ui(select == "range", |ui| {
            if form::text_named(ui, d, "range", tl!("Range:"), 150.0)
                && let Ok(Some(chosen)) = (ArtboardPick { range: Some(d.str("range")), ..Default::default() }).resolve(n)
            {
                d.fields.insert("boards".into(), json!((0..n).map(|i| chosen.contains(&i)).collect::<Vec<_>>()));
            }
        });
    });
    ui.horizontal(|ui| {
        form::check(ui, d, "includeBleed", tl!("Include Bleed"));
        form::check(ui, d, "includeArtboardFill", tl!("Include Artboard Fill"));
    });
    if widgets::radio(ui, tl!("Full Document"), d.str("select") == "full", true) {
        set_select(d, "full");
    }
}

/// The artboards' thumbnails, each with its checkbox, number and name, in a grid or a list;
/// checking them picks them as the range (all of them: All). Dimmed with Full Document.
fn artboard_tiles(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog, list: bool) {
    let mut boards = boards(app, d);
    let before = boards.clone();
    let names: Vec<String> = app.session.active().map(|s| s.doc.artboards.iter().map(|a| a.name.clone()).collect()).unwrap_or_default();
    ui.add_enabled_ui(d.str("select") != "full", |ui| {
        tiles(ui, "efs-artboards", names.len(), list, |ui, i, r| {
            match artboard_thumb(app, ui.ctx(), i) {
                Some(tex) => {
                    let sz = tex.size_vec2();
                    let s = (r.width() / sz.x).min(r.height() / sz.y).min(1.0);
                    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                    ui.painter().image(tex.id(), egui::Rect::from_center_size(r.center(), sz * s), uv, egui::Color32::WHITE);
                }
                None => {
                    ui.painter().rect_filled(r, Tokens::get(ui.ctx()).cr_small(), egui::Color32::WHITE);
                }
            }
            let name = names.get(i).cloned().unwrap_or_default();
            (format!("{}  {name}", i + 1), name, boards.get(i).copied().unwrap_or(false))
        })
        .into_iter()
        .for_each(|i| {
            if let Some(b) = boards.get_mut(i) {
                *b ^= true;
            }
        });
    });
    if boards != before {
        set_select(d, if boards.iter().all(|b| *b) { "all" } else { "range" });
        d.fields.insert("range".into(), json!(range_text(&boards)));
        d.fields.insert("boards".into(), json!(boards));
    }
}

/// `count` thumbnails in a grid (or, with `list`, a list) that scrolls: `paint` draws thumbnail `i`
/// into its rect and gives its caption, its checkbox's name and whether it is checked → the
/// thumbnails whose checkbox (or picture) was clicked.
fn tiles(
    ui: &mut egui::Ui,
    id: &str,
    count: usize,
    list: bool,
    mut paint: impl FnMut(&mut egui::Ui, usize, egui::Rect) -> (String, String, bool),
) -> Vec<usize> {
    let t = Tokens::get(ui.ctx());
    let mut toggled = vec![];
    let side = if list { 40.0 } else { 84.0 };
    let width = ui.available_width();
    let columns = if list { 1 } else { ((width + 10.0) / (side + 10.0)).floor().max(1.0) as usize };
    egui::ScrollArea::vertical().id_salt(id).auto_shrink([false, false]).show(ui, |ui| {
        for start in (0..count).step_by(columns) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                for i in start..(start + columns).min(count) {
                    let cell = if list { egui::vec2(width - 16.0, side) } else { egui::vec2(side, side + 18.0) };
                    let (r, resp) = ui.allocate_exact_size(cell, egui::Sense::click());
                    let pic = egui::Rect::from_min_size(r.min, egui::vec2(side, side));
                    let (caption, name, checked) = paint(ui, i, pic.shrink(3.0));
                    let stroke = if checked { egui::Stroke::new(2.0, t.accent) } else { egui::Stroke::new(1.0, t.divider) };
                    ui.painter().rect_stroke(pic, 2.0, stroke, egui::StrokeKind::Inside);
                    let label_at = if list { egui::pos2(pic.right() + 8.0, pic.center().y) } else { egui::pos2(pic.left(), pic.bottom() + 9.0) };
                    let galley =
                        ui.painter().layout(caption, egui::FontId::proportional(11.0), t.text, if list { cell.x - side - 12.0 } else { side });
                    ui.painter().galley(egui::pos2(label_at.x, label_at.y - galley.size().y / 2.0), galley, t.text);
                    // The checkbox sits on the thumbnail's lower left corner, as Illustrator's.
                    let bx = egui::Rect::from_min_size(egui::pos2(pic.left() + 4.0, pic.bottom() - 18.0), egui::vec2(14.0, 14.0));
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(bx));
                    if widgets::check_box(&mut child, &name, checked, true) || resp.clicked() {
                        toggled.push(i);
                    }
                }
            });
            ui.add_space(6.0);
        }
    });
    toggled
}

/// The checked assets that are still in the document, in panel order.
fn checked_assets(app: &VectorcraftApp, d: &Dialog) -> Vec<u64> {
    let on: Vec<u64> = d.fields.get("assets").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
    app.session.active().map(|st| st.doc.assets.iter().map(|a| a.id).filter(|id| on.contains(id)).collect()).unwrap_or_default()
}

/// The Assets tab: each asset's thumbnail with its checkbox and name, in a grid or a list.
fn assets(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog, list: bool) {
    let Some(st) = app.session.active() else { return };
    if st.doc.assets.is_empty() {
        ui.add_space(24.0);
        widgets::dim_label(ui, tl!("No assets yet. Collect art with Object › Collect for Export, or drag it into the Asset Export panel."));
        return;
    }
    let mut on = checked_assets(app, d);
    let before = on.clone();
    // Asked for until its row is in view: the dialog's first frame only measures it.
    let scroll_to = d.fields.get("__scrollTo").and_then(Value::as_u64).filter(|id| st.doc.assets.iter().any(|a| a.id == *id));
    let mut in_view = scroll_to.is_none();
    let clicked = tiles(ui, "efs-assets", st.doc.assets.len(), list, |ui, i, r| {
        let Some(a) = st.doc.assets.get(i) else { return (String::new(), String::new(), false) };
        crate::panels::asset_export::paint_thumb(ui, st, a, r);
        if scroll_to == Some(a.id) {
            if ui.clip_rect().contains_rect(r) {
                in_view = true;
            } else {
                ui.scroll_to_rect(r, Some(egui::Align::Center));
            }
        }
        (a.name.clone(), a.name.clone(), on.contains(&a.id))
    });
    for i in clicked {
        if let Some(a) = st.doc.assets.get(i) {
            if on.contains(&a.id) {
                on.retain(|id| *id != a.id);
            } else {
                on.push(a.id);
            }
        }
    }
    if in_view {
        d.fields.remove("__scrollTo");
    }
    if on != before {
        d.fields.insert("assets".into(), json!(on));
    }
}

/// Export to: the folder (typed or picked), Open Location after Export, and Create Sub-folders,
/// a folder per scale or per format; the web downloads instead.
fn destination(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    widgets::dim_label(ui, tl!("Export to:"));
    if io::is_web(app) {
        widgets::dim_label(ui, tl!("Your browser downloads the files (several as one .zip)."));
    } else {
        let pick = app.services.pick_folder.is_some();
        ui.horizontal(|ui| {
            form::text(ui, d, "folder", if pick { RIGHT_W - 40.0 } else { RIGHT_W - 6.0 });
            if pick && widgets::icon_button(ui, "folder-open", tl!("Choose a folder"), false, 26.0).clicked() {
                crate::picks::folder_field(app, d, "folder");
            }
        });
        form::check(ui, d, "openLocation", tl!("Open Location after Export"));
    }
    form::check(ui, d, "subfolders", tl!("Create Sub-folders"));
    ui.horizontal(|ui| {
        ui.add_space(22.0);
        let on = d.bool("subfolders");
        let by = d.str("subfolderBy");
        if widgets::radio(ui, tl!("Scale"), by != "format", on) {
            d.fields.insert("subfolderBy".into(), json!("scale"));
        }
        if widgets::radio(ui, tl!("Format"), by == "format", on) {
            d.fields.insert("subfolderBy".into(), json!("format"));
        }
    });
}

/// Export PDFs As: one file of every chosen artboard, or a file each; only for artboards (not
/// Full Document) with a PDF row.
fn pdf_mode(ui: &mut egui::Ui, d: &mut Dialog, artboards: bool) {
    let pdf = d
        .fields
        .get("formats")
        .and_then(Value::as_array)
        .is_some_and(|rows| rows.iter().any(|r| r["format"].as_str().and_then(fileio::format).is_some_and(|f| f.id == "pdf")));
    let on = artboards && pdf && d.str("select") != "full";
    ui.horizontal(|ui| {
        ui.add_enabled_ui(on, |ui| widgets::dim_label(ui, tl!("Export PDFs As:")));
        let single = d.str("pdfs") == "single";
        if widgets::radio(ui, tl!("Single File"), single, on) {
            d.fields.insert("pdfs".into(), json!("single"));
        }
        if widgets::radio(ui, tl!("Multiple Files"), !single, on) {
            d.fields.insert("pdfs".into(), json!("multiple"));
        }
    });
}

/// The format presets (the ☰ by iOS and Android): apply a built-in or saved one (its rows, their
/// settings, the prefix and the sub-folders), save the dialog's rows as one, or delete the saved
/// one applied last.
fn preset_menu(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    let list: Vec<Value> = d.fields.get("__exportPresets").and_then(Value::as_array).cloned().unwrap_or_default();
    let current = d.str("exportPreset");
    let saved = list.iter().any(|p| p["name"] == current && p["builtIn"] == false);
    let resp = widgets::icon_button(ui, "menu", tl!("Format Presets"), false, 24.0);
    let clicked = resp.clicked();
    let _ = widgets::popover(&resp, clicked, |ui| {
        for p in &list {
            let name = p["name"].as_str().unwrap_or_default();
            if widgets::menu_item_name(ui, name, true, name == current) {
                apply_preset(d, p);
                ui.close();
            }
        }
        ui.separator();
        if widgets::menu_item(ui, tl!("Save Preset…"), true, false) {
            d.fields.insert("__savePreset".into(), json!(if saved { current.clone() } else { String::new() }));
            ui.close();
        }
        if widgets::menu_item(ui, tl!("Delete Preset"), saved, false) && app.run("document.exportPresets.delete", json!({"name": current})).is_ok() {
            d.fields.insert("exportPreset".into(), json!(""));
            d.fields.insert("__exportPresets".into(), export_presets(app));
            ui.close();
        }
    });
}

/// The row that names a format preset being saved (Save Preset… in the ☰ menu).
fn save_preset_row(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    if !d.fields.contains_key("__savePreset") {
        return;
    }
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Name:"));
        form::text(ui, d, "__savePreset", 150.0);
        let name = d.str("__savePreset").trim().to_string();
        if ui.add_enabled(!name.is_empty(), egui::Button::new(tl!("Save"))).clicked() {
            let settings = json!({
                "formats": d.fields.get("formats").cloned().unwrap_or(json!([])),
                "settings": d.fields.get("settings").cloned().unwrap_or(json!({})),
                "prefix": d.str("prefix"),
                "folders": if !d.bool("subfolders") { "none" } else if d.str("subfolderBy") == "format" { "format" } else { "rows" },
            });
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

/// Set the dialog to format preset `preset` (`{name, settings}`): its rows, their settings, the
/// prefix and the sub-folders; what it doesn't name keeps the dialog's value.
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
        d.fields.insert("subfolders".into(), json!(f != "none"));
        d.fields.insert("subfolderBy".into(), json!(if f == "format" { "format" } else { "scale" }));
    }
    if let Some(prefix) = st["prefix"].as_str() {
        d.fields.insert("prefix".into(), json!(prefix));
    }
    for key in ["includeBleed", "includeArtboardFill"] {
        if let Some(b) = st[key].as_bool() {
            d.fields.insert(key.into(), json!(b));
        }
    }
    d.fields.insert("exportPreset".into(), json!(preset["name"]));
}

/// What the dialog adds by the iOS and Android buttons: its format presets.
pub(crate) type FormatMenu<'a> = &'a mut dyn FnMut(&mut egui::Ui, &mut Dialog);

/// The iOS and Android rows (the engine's `mobile` and `density` presets) and their labels.
const PLATFORMS: [(&str, &str); 2] = [("mobile", "iOS"), ("density", "Android")];

/// Formats: the iOS and Android buttons (each sets the rows that platform uses), `menu` (the
/// dialog's format presets), the Format Settings gear (which sets `__settings`), the format rows
/// (scale, suffix, format) and Add Scale, as wide as `ui` allows: the dialog's and the Asset
/// Export panel's rows.
pub(crate) fn formats(ui: &mut egui::Ui, d: &mut Dialog, mut menu: Option<FormatMenu<'_>>) {
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
            if let Some(menu) = menu.as_mut() {
                menu(ui, d);
            }
            // Right to left: Android, then iOS before it.
            for (id, label) in PLATFORMS.iter().rev() {
                let on = d.str("preset") == *id;
                if ui.add(egui::Button::new(egui::RichText::new(tl!(label)).size(12.0)).selected(on).frame(on)).clicked()
                    && let Some(preset_rows) = fileio::screen_preset_rows(id)
                {
                    rows = preset_rows;
                    d.fields.insert("subfolders".into(), json!(true));
                    d.fields.insert("subfolderBy".into(), json!("scale"));
                    d.fields.insert("formats".into(), json!(rows));
                    d.fields.insert("preset".into(), json!(id));
                }
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
                if let Some(s) = widgets::text_presets_named(ui, ("efs-scale", i), Some(tl!("Scale")), &scale, &SIZES, scale_w) {
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
            if let Some(s) = widgets::text_field_named(ui, ("efs-suffix", i), Some(tl!("Suffix")), row["suffix"].as_str(), suffix_w, 1) {
                row["suffix"] = json!(s);
                edited = true;
            }
            let (format, quality) = (row["format"].as_str().unwrap_or("png"), row["quality"].as_u64());
            let marked = row.get("marks").is_some_and(|m| !m.is_null());
            let current =
                FORMATS.iter().position(|(_, f, q, print)| *f == format && *print == marked && q.is_none_or(|q| Some(u64::from(q)) == quality));
            let label = current.map_or_else(|| format.to_uppercase(), |i| FORMAT_LABELS[i].to_string());
            if let Some((_, f, q, print)) =
                widgets::dropdown_named(ui, ("efs-fmt", i), Some(tl!("Format")), &label, &FORMAT_LABELS, format_w).and_then(|i| FORMATS.get(i))
            {
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
    if let Some((f, _)) = widgets::dropdown_named(ui, "efs-settings-format", Some(tl!("Format:")), SETTINGS_LABELS[at], &SETTINGS_LABELS, 150.0)
        .and_then(|i| SETTINGS.get(i))
    {
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
    let key = egui::Id::unique(("ab-thumb", i, st.revision));
    if let Some(t) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(key)) {
        return Some(t);
    }
    let doc = st.doc.clone();
    let r = doc.artboards.get(i)?.rect;
    let tex = crate::widgets::region_texture(ctx, &mut app.canvas.renderer, &format!("ab-thumb-{i}"), &doc, r, 92.0);
    ctx.data_mut(|d| d.insert_temp(key, tex.clone()));
    Some(tex)
}
