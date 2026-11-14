//! File → Export → Export As…: the format and Use Artboards (All or a range: one file per
//! artboard, after a central file of the whole document for EPS with two or more when art shows,
//! or one page each in a PDF); without artboards the export covers the visible art. OK
//! picks the file, then the format's options dialog follows (PNG/JPEG/WebP/PNG-8/GIF Options, SVG
//! Options, Text Export Options, DXF Options, EPS Options, HP-GL Options, and Save PDF),
//! else the file is written. DWG is listed, greyed out, with what to use instead. The formats of
//! installed exporter plug-ins come after the built-in ones: they write the whole document (from
//! Export Selection, the selection), and one with parameters asks for them before writing
//! (`plugin::open_export`). Export Selection ([`export_selection`]) takes the same route after its
//! save panel, and every format then writes the selection alone, cropped to it.

use std::sync::LazyLock;

use serde_json::{Value, json};
use vectorcraft_engine::cmd::fileio::{self, ArtboardPick, Format};

use super::{DialogSpec, dxf_options, eps_options, form, hpgl_options, plugin, png_options, save_pdf, svg_options, text_export};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, io, widgets};

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Export As").into(), body, confirm, ok: Some("Export…"), min_width: 380.0, ..DialogSpec::FORM };

/// The built-in formats Export As writes (the native format and `.ai` are Save As's).
static BUILT_IN: LazyLock<Vec<&'static Format>> =
    LazyLock::new(|| fileio::FORMATS.iter().filter(|f| f.write && !matches!(f.id, "vectorcraft" | "ai")).collect());
/// Formats listed after them that can't be written (`fileio::unsupported` ids), greyed out.
const UNAVAILABLE: [(&str, &str); 1] = [("dwg", "DWG (use DXF)")];

/// The formats Export As writes: the built-in ones, then those of the installed exporter plug-ins.
pub(super) fn formats() -> Vec<&'static Format> {
    BUILT_IN.iter().copied().chain(fileio::plugin_formats().into_iter().filter(|f| f.write)).collect()
}

/// Is `f` an exporter plug-in's format?
fn from_plugin(f: &Format) -> bool {
    vectorcraft_plugins::export::by_format(f.id).is_some()
}

/// What to use instead when the field names a format that can't be written (DWG).
fn unavailable(d: &Dialog) -> Option<(&'static str, &'static str)> {
    let id = d.str("format");
    let (_, label) = UNAVAILABLE.iter().find(|u| u.0 == id)?;
    Some((label, fileio::unsupported(&id)?.hint))
}

/// Open the dialog on `format` (an id or extension, default PNG); `p` may preset `useArtboards`
/// and `range`.
pub fn open(app: &mut VectorcraftApp, format: Option<&str>, p: &Value) {
    let n = app.session.active().map_or(0, |s| s.doc.artboards.len());
    let missing = format.and_then(|f| UNAVAILABLE.iter().find(|u| u.0.eq_ignore_ascii_case(f.trim_start_matches('.'))));
    let format = match missing {
        Some((id, _)) => id,
        None => format.and_then(fileio::format).filter(|f| formats().iter().any(|g| g.id == f.id)).map_or("png", |f| f.id),
    };
    let range = p.get("range").and_then(Value::as_str);
    app.ui.dialog = Some(Dialog::new(
        "exportAs",
        json!({
            "format": format,
            "useArtboards": p.get("useArtboards").and_then(Value::as_bool).unwrap_or(false),
            "all": range.is_none(),
            "range": range.map_or_else(|| if n > 1 { format!("1-{n}") } else { "1".into() }, str::to_string),
        }),
    ));
}

/// The chosen format (PNG when the field names none Export As writes).
fn chosen(d: &Dialog) -> Option<&'static Format> {
    let id = d.str("format");
    let all = formats();
    let find = |id: &str| all.iter().copied().find(|f| f.id == id);
    find(&id).or_else(|| find("png"))
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    let Some(f) = chosen(d) else { return false };
    let missing = unavailable(d);
    egui::Grid::new("export-as").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
        ui.label(egui::RichText::new(tl!("Format:")).color(t.text_dim));
        let shown = missing.map_or(f.label, |(label, _)| label);
        let all = formats();
        let labels: Vec<&str> = all.iter().map(|f| f.label).chain(UNAVAILABLE.iter().map(|u| u.1)).collect();
        if let Some(g) = widgets::dropdown_with(ui, "export-as-format", shown, &labels, 160.0, |i| i < all.len()).and_then(|i| all.get(i)) {
            d.fields.insert("format".into(), json!(g.id));
        }
        ui.end_row();
    });
    if let Some((_, hint)) = missing {
        ui.add_space(6.0);
        form::caption(ui, hint);
    }
    ui.add_space(10.0);
    // An exporter plug-in writes the whole document: Use Artboards is not its to apply.
    let whole = from_plugin(f);
    ui.add_enabled_ui(!whole, |ui| form::check(ui, d, "useArtboards", tl!("Use Artboards")));
    let use_artboards = d.bool("useArtboards") && !whole;
    ui.add_enabled_ui(use_artboards, |ui| {
        ui.horizontal(|ui| {
            ui.add_space(22.0);
            let mut all = d.bool("all");
            if ui.radio_value(&mut all, true, tl!("All")).changed() | ui.radio_value(&mut all, false, tl!("Range:")).changed() {
                d.fields.insert("all".into(), json!(all));
            }
            let mut range = d.str("range");
            let field = egui::TextEdit::singleline(&mut range).align(egui::Align2::LEFT_CENTER).desired_width(90.0).hint_text("1-3, 5");
            if ui.add_enabled(!all, field).changed() {
                d.fields.insert("range".into(), json!(range));
            }
        });
    });
    ui.add_space(8.0);
    let ext = f.extensions[0];
    let hint = match (use_artboards, f.id) {
        _ if whole => tl!("The plug-in writes the whole document.").to_string(),
        (true, "pdf") => tl!("One page per artboard.").to_string(),
        (true, _) => crate::i18n::fmt(tl!("One file per artboard: <name>-<artboard>.{ext}"), &[("ext", ext)]),
        (false, _) => tl!("The bounds of the visible art.").to_string(),
    };
    ui.label(egui::RichText::new(hint).color(t.text_dim).size(11.5));
    false
}

/// Check the artboard range, pick the file, then show the format's options or write the file.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    if let Some((_, hint)) = unavailable(d) {
        return Err(hint.into());
    }
    let f = chosen(d).ok_or("no format to export to")?;
    let plugin = vectorcraft_plugins::export::by_format(f.id);
    let use_artboards = d.bool("useArtboards") && plugin.is_none();
    let all = d.bool("all");
    let range = d.str("range");
    if use_artboards && !all {
        // A bad range keeps the dialog open.
        let n = app.session.active().map_or(0, |s| s.doc.artboards.len());
        ArtboardPick { range: Some(range.clone()), ..Default::default() }.resolve(n)?;
    }
    // Nothing to export keeps the dialog open too (before the save dialog, not after it). An
    // exporter plug-in decides for itself what it can write.
    if !use_artboards && plugin.is_none() && app.session.active().is_some_and(|s| vectorcraft_render::encode::art_bounds(&s.doc).is_none()) {
        return Err("nothing to export: the document has no visible art (or turn on Use Artboards)".into());
    }
    app.ui.dialog = None;
    let path = io::export_path(app, None, f)?;
    let range = if all { "all".to_string() } else { range };
    let r = options_or_write(app, f, plugin, path, Pages { use_artboards, all, range, selection: false });
    // The options follow the save panel: the Enter that closed it isn't theirs (as in Save As).
    super::opened_by_panel(app);
    r
}

/// The options dialogs' field that is true when Export Selection opened them: OK then writes the
/// selection alone ([`exported`]). Not remembered with the dialog's options.
pub(super) const SELECTION_FIELD: &str = "__selection";

/// `params` of an options dialog `d` for [`io::export`]: with [`io::EXPORT_SELECTION`] when
/// Export Selection opened `d`.
pub(super) fn exported(d: &Dialog, mut params: Value) -> Value {
    if d.bool(SELECTION_FIELD)
        && let Some(o) = params.as_object_mut()
    {
        o.insert(io::EXPORT_SELECTION.into(), json!(true));
    }
    params
}

/// File › Export Selection (`document.exportSelection` without params): the save panel picks the
/// file and its format, then the format's options dialog follows as in Export As, else the file is
/// written. Each route writes the selected objects alone, cropped to them. Nothing selected to
/// export stops before the save panel.
pub fn export_selection(app: &mut VectorcraftApp) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document")?;
    fileio::selection_doc(st, "document.exportSelection", false).map_err(|e| e.to_string())?;
    let path = io::selection_path(app)?;
    let f = fileio::writable_format(None, Some(&path))?;
    let plugin = vectorcraft_plugins::export::by_format(f.id);
    let pages = Pages { use_artboards: false, all: true, range: String::new(), selection: true };
    let r = options_or_write(app, f, plugin, path, pages);
    super::opened_by_panel(app);
    r
}

/// What an export writes: with `use_artboards` the artboards (`all`, else the 1-based `range`),
/// else the bounds of the visible art, or with `selection` the selection alone.
struct Pages {
    use_artboards: bool,
    all: bool,
    range: String,
    selection: bool,
}

/// After the save panel picked `path`: the options dialog of format `f` (or of its exporter
/// plug-in), else the file written.
fn options_or_write(
    app: &mut VectorcraftApp,
    f: &'static Format,
    plugin: Option<std::sync::Arc<vectorcraft_plugins::Plugin>>,
    path: String,
    pages: Pages,
) -> Result<Value, String> {
    let Pages { use_artboards, all, range, selection } = pages;
    let r = match f.id {
        "svg" | "svgz" => {
            svg_options::open(app, svg_options::Mode::Export, Some(&path));
            if let Some(o) = app.ui.dialog.as_mut() {
                o.fields.extend([("useArtboards".into(), json!(use_artboards)), ("allArtboards".into(), json!(all)), ("range".into(), json!(range))]);
            }
            Ok(Value::Null)
        }
        // Save PDF writes the artboards as pages, or one page of the art's bounds or the selection.
        "pdf" if use_artboards => save_pdf::open(app, &json!({ "path": path, "range": (!all).then_some(range) })),
        "pdf" => save_pdf::open(app, &json!({ "path": path, "useArtboards": false })),
        "txt" => {
            text_export::open(app, &path);
            if let Some(o) = app.ui.dialog.as_mut().filter(|_| selection) {
                o.fields.insert("selectionOnly".into(), json!(true));
            }
            Ok(Value::Null)
        }
        _ => {
            let mut params = json!({ "format": f.id, "path": path });
            // A format that holds the whole document (a template) has no Use Artboards: the
            // export is not given the option, which its warnings would name as not used.
            if !selection && (plugin.is_some() || f.options.iter().any(|o| o.name == "useArtboards")) {
                params["useArtboards"] = json!(use_artboards);
                if use_artboards {
                    params["range"] = json!(range);
                }
            }
            let plugin = plugin.filter(|p| !p.manifest().params.is_empty());
            let dialog = plugin.is_none() && (f.raster || matches!(f.id, "dxf" | "eps"));
            if selection {
                // An options dialog keeps the mark as a field of its own, which OK hands on.
                params[if dialog { SELECTION_FIELD } else { io::EXPORT_SELECTION }] = json!(true);
            }
            if let Some(plugin) = plugin {
                plugin::open_export(app, &plugin, params);
                return Ok(Value::Null);
            }
            if f.raster {
                png_options::open(app, f, params);
                return Ok(Value::Null);
            }
            if f.id == "dxf" {
                dxf_options::open(app, &params);
                return Ok(Value::Null);
            }
            if f.id == "eps" {
                eps_options::open(app, &params);
                return Ok(Value::Null);
            }
            if f.id == "hpgl" {
                hpgl_options::open(app, &params);
                return Ok(Value::Null);
            }
            return io::export(app, Some(f.id), Some(path), &params);
        }
    };
    // Export Selection's mark on the options dialog that opened.
    if let Some(o) = app.ui.dialog.as_mut().filter(|_| selection) {
        o.fields.insert(SELECTION_FIELD.into(), json!(true));
    }
    r
}
