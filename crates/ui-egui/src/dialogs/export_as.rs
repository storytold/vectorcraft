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

use craft::ui::contract::{Bind, Show as _};
use craft::ui::display::{Shade, Text};
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind};
use craft::ui::runtime::env::Label;
use craft::ui::style::TypeRole;
use serde_json::{Value, json};
use vectorcraft_engine::cmd::fileio::{self, ArtboardPick, Format};

use super::export_host::{self, After, Export};
use super::{dxf_options, eps_options, form, hpgl_options, plugin, png_options, save_pdf, svg_options, text_export};
use crate::{VectorcraftApp, io};

/// The dialog's key.
pub(super) const KIND: &str = "exportAs";

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

/// What to use instead when the format field names a format that can't be written (DWG).
fn unavailable(fields: &DialogFields) -> Option<(&'static str, &'static str)> {
    let id = fields.text("format")?;
    let (_, label) = UNAVAILABLE.iter().find(|u| u.0 == id)?;
    Some((label, fileio::unsupported(id)?.hint))
}

/// The chosen format (PNG when the field names none Export As writes).
fn chosen(fields: &DialogFields) -> Option<&'static Format> {
    let all = formats();
    let find = |id: &str| all.iter().copied().find(|f| f.id == id);
    find(fields.text("format").unwrap_or("png")).or_else(|| find("png"))
}

/// Whether the artboard choice is All; the radio's ids are `true` and `false`.
fn is_all(fields: &DialogFields) -> bool {
    fields.bool("all").or_else(|| fields.text("all").map(|t| t == "true")).unwrap_or(true)
}

/// The rows, from the formats installed now.
fn params() -> Vec<Param> {
    let all = formats();
    let choices = all.iter().map(|f| (f.id, f.label)).chain(UNAVAILABLE);
    // An exporter plug-in writes the whole document: Use Artboards is not its to apply.
    let whole: Vec<&str> = all.iter().filter(|f| !from_plugin(f)).map(|f| f.id).collect();
    vec![
        Param::new(
            "format",
            tl!("Format:"),
            ParamKind::choice(choices.map(|(id, label)| (id.to_string(), Label::from(label))).collect(), Some("png".into())),
        )
        .disabled_options(UNAVAILABLE.map(|u| u.0)),
        Param::new("useArtboards", tl!("Use Artboards"), ParamKind::bool(false)).enabled_when_in("format", whole),
        Param::new(
            "all",
            tl!("Artboards"),
            ParamKind::radio(vec![("true".into(), Label::from(tl!("All"))), ("false".into(), Label::from(tl!("Range:")))], Some("true".into())),
        )
        .enabled_when("useArtboards", true),
        Param::new("range", "", ParamKind::text("")).hint("1-3, 5").enabled_when("all", "false"),
    ]
}

/// The line under the rows: what the export writes, or what to use instead of DWG.
fn summary(fields: &DialogFields) -> String {
    if let Some((_, hint)) = unavailable(fields) {
        return hint.to_string();
    }
    let Some(f) = chosen(fields) else { return String::new() };
    let whole = from_plugin(f);
    let use_artboards = fields.bool("useArtboards").unwrap_or(false) && !whole;
    match (use_artboards, f.id) {
        _ if whole => tl!("The plug-in writes the whole document.").to_string(),
        (true, "pdf") => tl!("One page per artboard.").to_string(),
        (true, _) => crate::i18n::fmt(tl!("One file per artboard: <name>-<artboard>.{ext}"), &[("ext", f.extensions[0])]),
        (false, _) => tl!("The bounds of the visible art.").to_string(),
    }
}

/// Open the dialog on `format` (an id or extension, default PNG); `p` may preset `useArtboards`
/// and `range`.
pub fn open(app: &mut VectorcraftApp, format: Option<&str>, p: &Value) {
    if let Err(e) = open_dialog(app, format, p) {
        app.status(e);
    }
}

fn open_dialog(app: &mut VectorcraftApp, format: Option<&str>, p: &Value) -> Result<(), String> {
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    let source = app.session.active();
    let document = source.map(|source| (source.uid, source.reverts));
    let n = source.map_or(0, |source| source.doc.artboards.len());
    let missing = format.and_then(|f| UNAVAILABLE.iter().find(|u| u.0.eq_ignore_ascii_case(f.trim_start_matches('.'))));
    let format = match missing {
        Some((id, _)) => id,
        None => format.and_then(fileio::format).filter(|f| formats().iter().any(|g| g.id == f.id)).map_or("png", |f| f.id),
    };
    let range = p.get("range").and_then(Value::as_str);
    let initial = json!({
        "format": format,
        "useArtboards": p.get("useArtboards").and_then(Value::as_bool).unwrap_or(false),
        "all": range.is_none(),
        "range": range.map_or_else(|| if n > 1 { format!("1-{n}") } else { "1".into() }, str::to_string),
    });
    let fields = form::fields_of(&params(), &initial.as_object().cloned().unwrap_or_default());
    let dialog = HostedParamDialog::new(&ctx, KIND, fields, |fields| {
        ParamDialog::new(KIND, tl!("Export As"), params())
            .values(Bind::bound(fields))
            .below(|ui, fields| {
                Text::new(summary(fields)).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
            })
            .ok_label(Label::from("Export…"))
    });
    export_host::open(app, Export::As { document }, false, dialog).map(|_| ()).ok_or_else(|| "no window".into())
}

fn require_source(app: &VectorcraftApp, document: Option<(u64, u64)>) -> Result<(), String> {
    let (uid, reverts) = document.ok_or_else(|| tl!("Open Export As from the drawing you want to export").to_owned())?;
    let source =
        app.session.active().filter(|source| source.uid == uid).ok_or_else(|| tl!("Return to the original drawing to export it").to_owned())?;
    if source.reverts != reverts {
        return Err(tl!("The document was reverted. Reopen this dialog to continue.").into());
    }
    Ok(())
}

/// Check the artboard range and that there is something to export; the save panel and the format's options or
/// the file follow once the dialog has closed.
pub(super) fn confirm(app: &mut VectorcraftApp, document: Option<(u64, u64)>, fields: &DialogFields) -> Result<After, String> {
    require_source(app, document)?;
    if let Some((_, hint)) = unavailable(fields) {
        return Err(hint.into());
    }
    let f = chosen(fields).ok_or("no format to export to")?;
    let plugin = vectorcraft_plugins::export::by_format(f.id);
    let use_artboards = fields.bool("useArtboards").unwrap_or(false) && plugin.is_none();
    let all = is_all(fields);
    let range = fields.text("range").unwrap_or_default().to_string();
    if use_artboards && !all {
        // A bad range keeps the dialog open.
        let n = app.session.active().map_or(0, |s| s.doc.artboards.len());
        ArtboardPick { range: Some(range.clone()), ..Default::default() }.resolve(n)?;
    }
    // Nothing to export keeps the dialog open too (before the save dialog, not after it). An
    // exporter plug-in decides for itself what it can write.
    if !use_artboards
        && plugin.is_none()
        && app.session.active().is_some_and(|s| vectorcraft_engine::cmd::fileio::export_art_bounds(&s.doc, f.id).is_none())
    {
        return Err("nothing to export: the document has no visible art (or turn on Use Artboards)".into());
    }
    let range = if all { "all".to_string() } else { range };
    let pages = Pages { use_artboards, all, range, selection: false };
    Ok(Some(Box::new(move |app| {
        require_source(app, document)?;
        io::export_path(app, None, f, move |app, path| {
            require_source(app, document)?;
            options_or_write(app, f, plugin, path, pages)
        })
    })))
}

/// `params` of an options dialog for [`io::export`]: with [`io::EXPORT_SELECTION`] when
/// Export Selection opened the dialog (`selection`).
pub(super) fn exported(selection: bool, mut params: Value) -> Value {
    if selection && let Some(o) = params.as_object_mut() {
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
    io::selection_path(app, |app, path| {
        let f = fileio::writable_format(None, Some(&path))?;
        let plugin = vectorcraft_plugins::export::by_format(f.id);
        let pages = Pages { use_artboards: false, all: true, range: String::new(), selection: true };
        options_or_write(app, f, plugin, path, pages)
    })
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
    match f.id {
        "svg" | "svgz" => {
            svg_options::open_with(app, svg_options::Mode::Export, Some(&path), selection);
            // Export Selection shows no artboard choice: the selection is one page.
            if !selection && let Some(id) = app.export_dialogs.last_id() {
                // Each row as the user sets it: All or Range holds its choice's id, and a greyed-out row takes nothing.
                let mut rows = vec![("useArtboards", json!(use_artboards))];
                if use_artboards {
                    rows.push(("allArtboards", json!(all.to_string())));
                    if !all {
                        rows.push(("range", json!(range)));
                    }
                }
                export_host::set_fields(app, id, &rows)?;
            }
            Ok(Value::Null)
        }
        // Save PDF writes the artboards as pages, or one page of the art's bounds or the selection.
        "pdf" if use_artboards => save_pdf::open_with(app, &json!({ "path": path, "range": (!all).then_some(range) }), selection),
        "pdf" => save_pdf::open_with(app, &json!({ "path": path, "useArtboards": false }), selection),
        "txt" => text_export::open(app, &path, selection).map(|()| Value::Null),
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
            // A raster format's dialog is told about Export Selection by its record; the others read the param.
            let raster_dialog = plugin.is_none() && f.raster;
            if selection && !raster_dialog {
                params[io::EXPORT_SELECTION] = json!(true);
            }
            if let Some(plugin) = plugin {
                plugin::open_export(app, &plugin, params);
                return Ok(Value::Null);
            }
            if f.raster {
                png_options::open_with(app, f, params, selection);
                return Ok(Value::Null);
            }
            match f.id {
                "dxf" => dxf_options::open(app, &params),
                "eps" => eps_options::open(app, &params),
                "hpgl" => hpgl_options::open(app, &params),
                _ => return io::export(app, Some(f.id), Some(path), &params),
            }
            Ok(Value::Null)
        }
    }
}
