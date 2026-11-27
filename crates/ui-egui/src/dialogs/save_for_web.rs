//! File → Export → Save for Web (Legacy) (`saveForWeb`, Ctrl/Cmd+Alt+Shift+S): the framework's
//! hosted Save for Web dialog ([`HostedSaveForWeb`]) over the engine's web export.
//!
//! The dialog owns its settings and draws; this module answers what it asks for, once per frame
//! ([`pump`]): the encoded pictures of its panes ([`webexport::render`] and
//! [`webexport::preview_image`], decoded back, so the previews are what Save writes), the colour
//! table edits ([`color_table`]), the preset list (`webExport.presets.*`), Optimize to File Size…
//! (`webExport.optimizeToFileSize`), Preview in Browser (`file.saveForWeb.browser`) and the path
//! of Save…. Save… remembers the settings and writes the files (`file.saveForWeb`); Done only
//! remembers them (`webExport.settings`).

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::Document;
use vectorcraft_engine::cmd::webexport::{self, Optimized, WebOutput, WebRender, WebSettings};
use vectorcraft_render::encode::quantize::Reduction;

use craft::behavior::PixelSize;
use craft::host::PickRequest;
use craft::theme::Srgba;
use craft::ui::dialog::{DialogId, DialogResult, StackDialog};
use craft::ui::patterns::form_dialogs::{
    BrowserPreview, ColorTable, ColorTableEdit, ColorTableEvent, HostedSaveForWeb, OptimizeAnswer, OptimizeRequest, OptimizeScope, OptimizeStart,
    PALETTE_COLOR_KEY, SaveForWebOptions, SaveForWebResult, WantedVariant, WebDocument, WebFormat, WebPreset, WebPresetEvent, WebSlice, WebSlices,
    WebVariant,
};
use craft::ui::primitives::{Revision, TextureStore, TextureStoreOptions};
use craft::ui::runtime::requests::PathRequest;
use egui::load::SizedTexture;

use super::preview_folders;
use crate::file_asks::{self, Owner};
use crate::{VectorcraftApp, io};

pub(super) mod color_table;

pub const KIND: &str = "saveForWeb";
const WRONG_DOCUMENT: &str = "Save for Web belongs to the document that opened the dialog.";

/// What the open dialog's pump keeps between frames: data keyed by its inputs, never UI state.
#[derive(Default)]
pub(crate) struct WebExport {
    document: Option<u64>,
    opening: Arc<()>,
    /// The rendering of the art before optimisation, by [`render_key`].
    render: Option<(String, Result<Arc<WebRender>, String>)>,
    /// The Original pane's picture, `None` while the dialog has not been given the current rendering's.
    original: Option<SizedTexture>,
    /// The last Optimize to File Size result with the art it measured ([`art_key`]).
    optimized: Option<Value>,
    /// The engine's `colorTable` value: the edits the colour table's buttons made.
    table: Value,
    /// The engine's settings the dialog has no option for ([`KEPT`]), as the dialog opened on them
    /// or Optimize to File Size found them: the previews, Save… and the presets use them. A preset
    /// picked in the dialog does not change them.
    kept: Value,
}

/// The engine's settings keys the dialog does not show; they pass through it unchanged.
const KEPT: [&str; 2] = ["transparencyDither", "transparencyDitherAmount"];

/// The [`KEPT`] keys of the engine's settings `s`.
fn kept_of(s: &WebSettings) -> Value {
    let engine = s.to_json();
    Value::Object(KEPT.iter().filter_map(|k| engine.get(*k).map(|v| ((*k).to_string(), v.clone()))).collect())
}

/// The key of the Original pane's texture in the pump's [`TextureStore`]; the panes use their index.
const ORIGINAL: usize = usize::MAX;

/// The pump's textures: one per pane and the original.
fn textures(ctx: &egui::Context) -> TextureStore<usize> {
    let nearest = egui::TextureOptions { magnification: egui::TextureFilter::Nearest, ..egui::TextureOptions::LINEAR };
    TextureStore::in_context(ctx, "save-for-web", TextureStoreOptions::frames().filter(nearest))
}

/// The engine's reduction for a colour-table choice of the dialog (which offers six of them).
fn reduction_of(table: ColorTable) -> Reduction {
    match table {
        ColorTable::Perceptual => Reduction::Perceptual,
        ColorTable::Selective => Reduction::Selective,
        ColorTable::Restrictive => Reduction::Web,
        ColorTable::BlackWhite => Reduction::BlackWhite,
        ColorTable::Grayscale => Reduction::Gray,
        // Exact, and a table the engine has no reduction of its own for, take the adaptive palette.
        _ => Reduction::Adaptive,
    }
}

/// The dialog's colour-table choice for an engine reduction.
fn table_of(reduction: Reduction) -> ColorTable {
    match reduction {
        Reduction::Perceptual => ColorTable::Perceptual,
        Reduction::Selective => ColorTable::Selective,
        Reduction::Adaptive => ColorTable::Adaptive,
        Reduction::Web => ColorTable::Restrictive,
        Reduction::BlackWhite => ColorTable::BlackWhite,
        Reduction::Gray => ColorTable::Grayscale,
    }
}

/// The dialog's options for the engine's settings `s`, over the dialog's defaults for what the engine has no setting
/// of. A width or a height given alone keeps the proportions of `natural`, the image at 100%. The engine's
/// `colorTable` is the host's edited table ([`WebExport::table`]); the dialog's colour-table choice is the engine's
/// reduction.
pub(super) fn options_of(s: &WebSettings, natural: Option<PixelSize>) -> SaveForWebOptions {
    let mut v = serde_json::to_value(SaveForWebOptions::default()).unwrap_or(Value::Null);
    if let (Some(o), Value::Object(engine)) = (v.as_object_mut(), s.to_json()) {
        for (key, value) in engine {
            if o.contains_key(&key) {
                o.insert(key, value);
            }
        }
        o.insert("colorTable".into(), serde_json::to_value(table_of(s.reduction)).unwrap_or(Value::Null));
        // The dialog names JPEG `jpeg` and holds the matte as RGB; it has no None matte, which shows as white.
        if s.format == webexport::WebFormat::Jpg {
            o.insert("format".into(), json!("jpeg"));
        }
        o.insert("matte".into(), json!(s.matte_rgb().ok().flatten().unwrap_or([255; 3])));
        o.insert("convertSrgb".into(), json!(s.convert_to_srgb));
        o.insert("percent".into(), json!(s.percent.unwrap_or(100.0)));
        if s.metadata == webexport::WebMetadata::Contact {
            o.insert("metadata".into(), json!("copyrightAndContact"));
        }
        // The dialog's whole image is the engine's no slices, and its user slices the engine's every slice.
        // shortcut: the selected slices are the document's in the engine; the dialog starts with none picked.
        let slices = match s.slices {
            webexport::SliceScope::None => json!("all"),
            webexport::SliceScope::All => json!("user"),
            webexport::SliceScope::Selected => json!({"selected": []}),
        };
        o.insert("slices".into(), slices);
        o.insert("html".into(), json!(s.output == WebOutput::Html));
        let scaled = |side: u32, from: u32, to: u32| ((f64::from(side) * f64::from(to) / f64::from(from.max(1))).round() as u32).max(1);
        let size = match (s.width, s.height, natural) {
            (Some(width), Some(height), _) => Some((width, height)),
            (Some(width), None, Some(n)) => Some((width, scaled(width, n.width, n.height))),
            (None, Some(height), Some(n)) => Some((scaled(height, n.height, n.width), height)),
            _ => None,
        };
        o.insert("size".into(), size.map_or(Value::Null, |(width, height)| json!({"width": width, "height": height})));
    }
    serde_json::from_value(v).unwrap_or_default()
}

/// The engine's settings JSON for the dialog's `options`, the edited colour `table`, the `kept`
/// settings the dialog does not show ([`WebExport::kept`]) and an optional `path`: what
/// `webExport.settings`, `document.exportForWeb` and `file.saveForWeb` read.
fn settings_of(options: &SaveForWebOptions, table: &Value, kept: &Value, path: Option<&str>) -> Value {
    let mut v = serde_json::to_value(options).unwrap_or(Value::Null);
    if let Some(o) = v.as_object_mut() {
        if let Some(kept) = kept.as_object() {
            o.extend(kept.clone());
        }
        let [r, g, b] = options.matte;
        o.insert("matte".into(), json!(format!("#{r:02x}{g:02x}{b:02x}")));
        o.remove("colorTable");
        o.insert("reduction".into(), json!(reduction_of(options.color_table).id()));
        if !table.is_null() {
            o.insert("colorTable".into(), table.clone());
        }
        // The keys the engine names its own way (see `options_of`).
        o.insert("convertToSrgb".into(), json!(options.convert_srgb));
        if o.get("metadata").and_then(Value::as_str) == Some("copyrightAndContact") {
            o.insert("metadata".into(), json!("contact"));
        }
        let slices = match &options.slices {
            WebSlices::All => "none",
            WebSlices::User => "all",
            _ => "selected",
        };
        o.insert("slices".into(), json!(slices));
        o.insert("output".into(), json!(if options.html { "html" } else { "images" }));
        let size = options.size;
        o.insert("width".into(), json!(size.map(|s| s.width)));
        o.insert("height".into(), json!(size.map(|s| s.height)));
        o.insert("percent".into(), if size.is_some() { Value::Null } else { json!(options.percent) });
        if let Some(p) = path {
            o.insert("path".into(), json!(p));
        }
    }
    v
}

/// Open on the document's active artboard, keeping the other remembered settings.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let document = app.session.active().ok_or("no document")?.uid;
    let host = app.dialogs().ok_or("no window")?;
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    // Settle a completed opening while its settings and source are still available.
    if let Some(DialogResult::Ok(result)) = host.with(|dialog| dialog.take::<HostedSaveForWeb>()) {
        finish(app, result);
    }
    let s = remembered(app);
    app.web_export = WebExport {
        document: Some(document),
        table: serde_json::to_value(&s.color_table).unwrap_or_default(),
        kept: kept_of(&s),
        ..WebExport::default()
    };
    let (name, artboards, slices) = {
        let st = app.session.active().ok_or("no document")?;
        let slices: Vec<WebSlice> = st
            .doc
            .slice_layout()
            .iter()
            .map(|a| {
                let rect = egui::Rect::from_min_max(egui::pos2(a.rect.x0 as f32, a.rect.y0 as f32), egui::pos2(a.rect.x1 as f32, a.rect.y1 as f32));
                WebSlice::new(a.number as u32, rect)
            })
            .collect();
        (st.doc.title.clone(), st.doc.artboards.iter().map(|a| a.name.clone()).collect::<Vec<_>>(), slices)
    };
    // The dialog sizes the output against the image at 100%.
    let natural = WebSettings { width: None, height: None, percent: None, ..s.clone() };
    let (size, original) = match render_for(app, &natural) {
        Ok((key, r)) => {
            let texture = original_texture(&ctx, &key, &r);
            app.web_export.original = Some(texture);
            (PixelSize::new(r.width, r.height), texture)
        }
        // The panes show why the rendering failed; the Original pane has nothing to show.
        Err(_) => {
            let blank = egui::ColorImage::from_rgba_unmultiplied([1, 1], &[0; 4]);
            (PixelSize::new(1, 1), textures(&ctx).put(&ctx, ORIGINAL, Revision::of(&"blank"), blank).sized())
        }
    };
    let mut dialog = HostedSaveForWeb::new(name, size, original)
        .document(WebDocument::vector(artboards))
        .formats(vec![WebFormat::Gif, WebFormat::Jpeg, WebFormat::Png8, WebFormat::Png24])
        .color_tables(vec![
            ColorTable::Perceptual,
            ColorTable::Selective,
            ColorTable::Adaptive,
            ColorTable::Restrictive,
            ColorTable::BlackWhite,
            ColorTable::Grayscale,
        ])
        .options(options_of(&s, Some(size)))
        .presets(dialog_presets(app))
        .optimize()
        .browser_preview(if io::is_web(app) { BrowserPreview::Unavailable(tl!("Needs the desktop app").into()) } else { BrowserPreview::Available });
    if !slices.is_empty() {
        dialog = dialog.slices(slices);
    }
    if io::is_web(app) {
        dialog = dialog.ok_label(tl!("Download"));
    }
    host.with(|h| h.open(dialog));
    Ok(Value::Null)
}

/// The remembered settings (the defaults when none are remembered) on the document's active
/// artboard; no artboard when the document has none.
fn remembered(app: &VectorcraftApp) -> WebSettings {
    let n = app.session.active().map_or(0, |st| st.doc.artboards.len());
    let mut s = app.session.last_web_settings();
    s.artboard = Some(crate::panels::artboards::selected(app, n)).filter(|a| *a < n);
    s
}

/// The saved presets and the built-in ones as the dialog lists them; the built-in ones come first.
fn dialog_presets(app: &VectorcraftApp) -> Vec<WebPreset> {
    let all = app.session.web_presets();
    let built_in = all.len().saturating_sub(app.session.prefs.web_export_presets.len());
    all.iter()
        .enumerate()
        .map(|(i, p)| {
            let mut preset = WebPreset::new(p.name.clone(), options_of(&p.settings, None));
            preset.built_in = i < built_in;
            preset
        })
        .collect()
}

/// `file.saveForWeb` with settings: write the files (`path`, else a picked file: the page with HTML
/// output, else the image); the web downloads them. The status bar shows the export's warnings as
/// [`io::report_saved`] shows a save's notes.
pub(crate) fn save(app: &mut VectorcraftApp, mut p: Value) -> Result<Value, String> {
    // Bad settings never open a save panel.
    let s = app.session.web_settings(&p)?;
    let path = p.get("path").and_then(Value::as_str).map(str::to_string);
    if path.is_none() && io::is_web(app) {
        let r = app.run("document.exportForWeb", p)?;
        let mut names = vec![];
        for f in r["files"].as_array().into_iter().flatten() {
            let (Some(name), Some(data)) = (f["name"].as_str(), f["dataBase64"].as_str()) else { continue };
            let bytes = vectorcraft_format::base64_decode(data).ok_or("the export returned unreadable data")?;
            io::write_to(&mut app.services, name, &bytes)?;
            names.push(json!(name));
        }
        match names.first().and_then(Value::as_str).filter(|_| has_warnings(&r)) {
            Some(first) => io::report_saved(app, first, &r),
            None => app.status(crate::i18n::msg_n(names.len() as u64, "Downloaded {n} file", "Downloaded {n} files", &[])),
        }
        return Ok(json!({ "files": names, "bytes": r["bytes"], "warnings": r["warnings"] }));
    }
    let ext = if s.output == WebOutput::Html { "html" } else { s.format.ext() };
    io::target_path(app, path, ext, move |app, path| {
        if let Some(o) = p.as_object_mut() {
            o.insert("path".into(), json!(path));
        }
        let r = app.run("document.exportForWeb", p)?;
        let n = r["files"].as_array().map_or(0, Vec::len);
        if n > 1 && !has_warnings(&r) {
            app.status(format!("Saved {n} files beside {path}"));
        } else {
            io::report_saved(app, &path, &r);
        }
        Ok(r)
    })
}

/// Whether an export's reply `r` has warnings (an image that couldn't be decoded):
/// [`io::report_saved`] shows them in the status bar as a save's notes.
fn has_warnings(r: &Value) -> bool {
    r["warnings"].as_array().is_some_and(|w| !w.is_empty())
}

/// `file.saveForWeb.browser`: the HTML page and its images in a new temporary folder, opened in the
/// default browser (desktop). When the export has warnings, the status bar shows them as
/// [`io::report_saved`] shows a save's notes.
pub(crate) fn browser_preview(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    if io::is_web(app) {
        return Err("Preview in Browser needs the desktop app (the web downloads files instead)".into());
    }
    app.refuse_automation("file.saveForWeb.browser")?;
    let stem = app.session.active().map(|st| vectorcraft_engine::cmd::fileio::file_stem(&st.doc.title)).ok_or("no document")?;
    let dir = temp_folder()?;
    let path = format!("{dir}/{}.html", preview_folders::preview_name(&stem));
    let mut q = if p.is_object() { p.clone() } else { json!({}) };
    if let Some(o) = q.as_object_mut() {
        o.insert("path".into(), json!(path));
        o.insert("output".into(), json!("html"));
    }
    let r = app.run("document.exportForWeb", q).inspect_err(|_| preview_folders::remove_preview_folder(&dir))?;
    app.open_url(&io::file_url(&path));
    if has_warnings(&r) {
        io::report_saved(app, &path, &r);
    }
    Ok(json!({ "path": path, "warnings": r["warnings"] }))
}

/// The folder a browser preview goes to: a new one for each preview, in the system's temporary
/// folder ([`preview_folders::temp_folder_under`]).
fn temp_folder() -> Result<String, String> {
    preview_folders::temp_folder_under(preview_folders::ROOT)
}

/// What the rendering for `s` depends on: the active document, its revision and the settings that
/// change the image's pixels.
fn render_key(app: &VectorcraftApp, s: &WebSettings) -> Option<String> {
    let st = app.session.active()?;
    Some(format!(
        "{}:{}:{:?}:{:?}:{:?}:{}:{}:{:?}:{:?}",
        st.uid,
        st.revision,
        s.width,
        s.height,
        s.percent,
        s.anti_alias.id(),
        s.clip_to_artboard,
        s.artboard,
        (!s.clip_to_artboard).then_some(s.slices)
    ))
}

/// The last Optimize to File Size result while the art (from [`art_key`]) is the art it measured.
/// That the settings are the ones it found is the dialog's rule.
pub(super) fn optimized_result<'a>(optimized: &'a Option<Value>, art: &str) -> Option<&'a Value> {
    optimized.as_ref().filter(|r| r["art"] == json!(art))
}

/// The active document, its revision and its selected objects and slices, which an Optimize to File
/// Size result was measured on. The selection decides the slices Save writes with `slices: selected`
/// and the slice Current Slice measures, and a selection made on the canvas or with the Slice
/// Selection tool leaves the revision as it is.
pub(super) fn art_key(app: &VectorcraftApp) -> String {
    app.session.active().map_or_else(String::new, |st| format!("{}:{}:{:?}:{:?}", st.uid, st.revision, st.selection.objects, st.selection.slices))
}

/// The rendering for `s`, made again only when [`render_key`] differs from the kept one. A new
/// rendering makes the Original pane's picture stale ([`WebExport::original`]).
fn render_for(app: &mut VectorcraftApp, s: &WebSettings) -> Result<(String, Arc<WebRender>), String> {
    let key = render_key(app, s).ok_or("no document")?;
    if app.web_export.render.as_ref().is_none_or(|(k, _)| *k != key) {
        let st = app.session.active().ok_or("no document")?;
        app.web_export.render = Some((key.clone(), webexport::render(&st.doc, s).map(Arc::new)));
        app.web_export.original = None;
    }
    match &app.web_export.render {
        Some((_, Ok(r))) => Ok((key, r.clone())),
        Some((_, Err(e))) => Err(e.clone()),
        None => Err("nothing to preview".into()),
    }
}

/// The Original pane's picture of the rendering `r` made under `key`.
fn original_texture(ctx: &egui::Context, key: &str, r: &WebRender) -> SizedTexture {
    let image = egui::ColorImage::from_rgba_unmultiplied([r.width as usize, r.height as usize], &r.rgba);
    textures(ctx).get_or_put(ctx, ORIGINAL, Revision::of(&key), move || image).sized()
}

/// The optimised image for `s`: the file Save writes, with the colour table of an indexed format.
fn encoded(app: &mut VectorcraftApp, s: &WebSettings) -> Result<Optimized, String> {
    let (_, r) = render_for(app, s)?;
    let st = app.session.active().ok_or("no document")?;
    webexport::preview_image(&st.doc, &r.rgba, r.width, r.height, s)
}

/// The pane of `wanted`: `s`'s optimised image decoded back, as a picture of the pump's textures.
fn optimise(ctx: &egui::Context, doc: &Document, r: &WebRender, key: &str, s: &WebSettings, wanted: &WantedVariant) -> WebVariant {
    let (index, revision) = (wanted.index, wanted.request.revision);
    let decoded = webexport::preview_image(doc, &r.rgba, r.width, r.height, s).and_then(|o| webexport::decode(&o).map(|d| (o, d)));
    let (o, (px, w, h)) = match decoded {
        Ok(d) => d,
        Err(e) => return WebVariant::failed(index, revision, e),
    };
    let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &px);
    let texture = textures(ctx).get_or_put(ctx, index, Revision::of(&(key, s.to_json().to_string())), move || image);
    let variant = WebVariant::new(index, revision, texture.sized(), o.bytes.len() as u64);
    if s.format.palette() {
        variant.colors(u16::try_from(o.colors.len()).unwrap_or(256)).palette(o.colors.iter().map(|c| c.source).collect())
    } else {
        variant
    }
}

/// Everything the open dialog asked for in its last pass.
struct Asked {
    id: DialogId,
    options: SaveForWebOptions,
    tables: Vec<ColorTableEvent>,
    presets: Vec<WebPresetEvent>,
    optimize_open: bool,
    optimize: Option<OptimizeRequest>,
    browser: bool,
    path: Option<PathRequest>,
}

impl Asked {
    /// Takes the requests, so a second pass of the frame finds none.
    fn take(d: &mut HostedSaveForWeb) -> Self {
        Self {
            id: d.id(),
            options: d.current().clone(),
            tables: d.take_table_events(),
            presets: d.take_preset_events(),
            optimize_open: d.take_optimize_open(),
            optimize: d.take_optimize_request(),
            browser: d.take_browser_preview(),
            path: d.pick_path(),
        }
    }
}

/// Runs `f` on the open Save for Web dialog; `None` when none is open.
fn on_dialog<R>(host: &craft::ui::shell::WindowDialogs, f: impl FnOnce(&mut HostedSaveForWeb) -> R) -> Option<R> {
    host.with(|h| h.get_mut::<HostedSaveForWeb>().map(f))
}

fn on_original_document(app: &VectorcraftApp) -> bool {
    app.session.active().is_some_and(|active| app.web_export.document == Some(active.uid))
}

/// Answers the open dialog once per frame: its result when it closed, else its requests.
/// Colour requests (the matte, a palette cell) are routed by the host-wide colour picker routing;
/// a palette cell's pick is applied to the table by [`take_palette_pick`].
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(host) = app.dialogs() else { return };
    match host.with(|h| h.take::<HostedSaveForWeb>()) {
        Some(DialogResult::Ok(result)) => return finish(app, result),
        Some(_) if on_dialog(&host, |_| ()).is_none() => {
            app.web_export = WebExport::default();
            return;
        }
        _ => {}
    }
    let Some(asked) = on_dialog(&host, Asked::take) else { return };
    if !on_original_document(app) {
        let restore = asked.tables.iter().any(|event| matches!(event.edit, ColorTableEdit::Restore));
        if restore {
            color_table::restore(&mut app.web_export.table);
            let marks = color_table::marks(&app.web_export.table);
            on_dialog(&host, |dialog| dialog.set_marks(marks));
        }
        let requested = asked.path.is_some()
            || asked.browser
            || asked.optimize_open
            || asked.optimize.is_some()
            || asked.tables.iter().any(|event| !matches!(event.edit, ColorTableEdit::Restore));
        update_presets(app, &host, asked.presets, restore);
        if requested {
            on_dialog(&host, HostedSaveForWeb::cancel_requests);
            app.status(tl!(WRONG_DOCUMENT).to_owned());
        }
        return;
    }
    // The framework keeps the result line while the settings are the ones it found; the art is the app's.
    let art = art_key(app);
    if app.web_export.optimized.is_some() && optimized_result(&app.web_export.optimized, &art).is_none() {
        on_dialog(&host, HostedSaveForWeb::forget_optimized);
        app.web_export.optimized = None;
    }
    if !asked.tables.is_empty() {
        edit_table(app, ctx, &host, &asked);
    }
    let marks = color_table::marks(&app.web_export.table);
    on_dialog(&host, |d| d.set_marks(marks));
    update_presets(app, &host, asked.presets, !asked.tables.is_empty());
    if asked.optimize_open {
        let bytes = optimize_start(app, &asked.options);
        on_dialog(&host, |d| d.answer_optimize_start(bytes));
    }
    if let Some(req) = &asked.optimize {
        let answer = run_optimize(app, req);
        on_dialog(&host, |d| d.answer_optimize(answer.map_err(Into::into)));
    }
    if asked.browser {
        let p = settings_of(&asked.options, &app.web_export.table, &app.web_export.kept, None);
        if let Err(e) = browser_preview(app, &p) {
            app.status(e);
        }
    }
    if let Some(req) = asked.path {
        ask_path(app, &host, asked.id, req, &asked.options);
    }
    let wanted = on_dialog(&host, HostedSaveForWeb::wanted_variants).unwrap_or_default();
    if !wanted.is_empty() {
        answer_variants(app, ctx, &host, &wanted);
    }
}

/// The encodes the dialog wants, as pictures, and the Original pane's picture when the rendering is new.
fn answer_variants(app: &mut VectorcraftApp, ctx: &egui::Context, host: &craft::ui::shell::WindowDialogs, wanted: &[WantedVariant]) {
    let (table, kept) = (app.web_export.table.clone(), app.web_export.kept.clone());
    let mut answers = Vec::with_capacity(wanted.len());
    for w in wanted {
        let (index, revision) = (w.index, w.request.revision);
        let settings = app.session.web_settings(&settings_of(&w.request.options, &table, &kept, None));
        let rendered = settings.and_then(|s| render_for(app, &s).map(|(key, r)| (s, key, r)));
        answers.push(match (rendered, app.session.active()) {
            (Ok((s, key, r)), Some(st)) => optimise(ctx, &st.doc, &r, &key, &s, w),
            (Err(e), _) => WebVariant::failed(index, revision, e),
            (Ok(_), None) => WebVariant::failed(index, revision, "no document"),
        });
    }
    let fresh = match (&app.web_export.original, &app.web_export.render) {
        (None, Some((key, Ok(r)))) => Some(original_texture(ctx, key, r)),
        _ => None,
    };
    if let Some(texture) = fresh {
        app.web_export.original = Some(texture);
        on_dialog(host, |d| d.set_original(texture));
    }
    on_dialog(host, |d| answers.into_iter().for_each(|v| d.set_variant(v)));
}

/// Applies the colour table edits the dialog filed. A Delete clears the dialog's colour selection.
fn edit_table(app: &mut VectorcraftApp, ctx: &egui::Context, host: &craft::ui::shell::WindowDialogs, asked: &Asked) {
    let shown = match shown_colors(app, &asked.options) {
        Ok(shown) => shown,
        Err(e) => return app.status(e),
    };
    let mut deleted = false;
    for event in &asked.tables {
        match color_table::apply(&mut app.web_export.table, &event.edit, &event.colors, &shown) {
            Ok(()) => deleted |= matches!(event.edit, ColorTableEdit::Delete),
            Err(e) => app.status(e),
        }
    }
    if deleted {
        let id = asked.id;
        host.with(|h| h.set_json_field(ctx, id, "colorSelection", json!([])).ok());
    }
}

/// The source colours of the palette the dialog's settings `options` show, with the table as it stands.
fn shown_colors(app: &mut VectorcraftApp, options: &SaveForWebOptions) -> Result<Vec<[u8; 3]>, String> {
    let settings = app.session.web_settings(&settings_of(options, &app.web_export.table, &app.web_export.kept, None))?;
    Ok(encoded(app, &settings)?.colors.iter().map(|c| c.source).collect())
}

/// The Color Picker's OK for the colour table cell `key` of the open dialog `parent`: the cell's colour `from`
/// is shifted to `to` in the table at once, so Done or Save in the dialog's next pass keeps the shift. A colour
/// the palette no longer shows is not shifted, and a refused shift shows in the status line. `false` when
/// `parent` is not Save for Web or `key` is not its palette, and the picker answers the row instead.
pub(crate) fn take_palette_pick(app: &mut VectorcraftApp, parent: DialogId, key: &str, from: Srgba, to: Srgba) -> bool {
    let Some(host) = app.dialogs() else { return false };
    if key != PALETTE_COLOR_KEY {
        return false;
    }
    let Some(options) = host.with(|h| h.get_mut_by_id::<HostedSaveForWeb>(parent).map(|d| d.current().clone())) else {
        return false;
    };
    if !on_original_document(app) {
        app.status(tl!(WRONG_DOCUMENT).to_owned());
        return true;
    }
    let rgb = |c: Srgba| {
        let [r, g, b, _] = c.to_rgba().0;
        [r, g, b]
    };
    let edit = ColorTableEdit::Shift(rgb(to));
    let shifted = shown_colors(app, &options).and_then(|shown| color_table::apply(&mut app.web_export.table, &edit, &[rgb(from)], &shown));
    if let Err(e) = shifted {
        app.status(e);
    }
    true
}

/// Runs what the preset list asked for and shows a failure in the status line.
fn update_presets(app: &mut VectorcraftApp, host: &craft::ui::shell::WindowDialogs, events: Vec<WebPresetEvent>, table_changed: bool) {
    let refresh = table_changed || !events.is_empty();
    for event in events {
        let r = match event {
            WebPresetEvent::Saved(p) => {
                let mut v = settings_of(&p.options, &app.web_export.table, &app.web_export.kept, None);
                v["name"] = json!(p.name);
                app.run("webExport.presets.save", v)
            }
            // The reference app's list has no rename: it is a save under the new name and a delete of the old.
            WebPresetEvent::Renamed { from, to, .. } => rename_preset(app, &from, &to),
            WebPresetEvent::Deleted(name) => app.run("webExport.presets.delete", json!({ "name": name })),
            _ => continue,
        };
        if let Err(e) = r {
            app.status(e);
        }
    }
    if refresh {
        let presets = dialog_presets(app);
        on_dialog(host, |d| d.set_presets(presets));
    }
}

/// Saves the preset `from` under `to`, then deletes `from`.
fn rename_preset(app: &mut VectorcraftApp, from: &str, to: &str) -> Result<Value, String> {
    let preset = app.session.web_presets().into_iter().find(|p| p.name == from).ok_or_else(|| format!("no preset named {from}"))?;
    let mut v = preset.settings.to_json();
    v["name"] = json!(to);
    app.run("webExport.presets.save", v)?;
    app.run("webExport.presets.delete", json!({ "name": from }))
}

/// Whether the active document has slices.
fn has_slices(app: &VectorcraftApp) -> bool {
    app.session.active().is_some_and(|st| !st.doc.slice_layout().is_empty())
}

/// The `webExport.optimizeToFileSize` params: `options`, the desired size and, with slices, what it covers.
fn optimize_params(app: &VectorcraftApp, options: &SaveForWebOptions, desired_k: f64, start: Option<&str>, scope: &str) -> Value {
    let mut p = settings_of(options, &app.web_export.table, &app.web_export.kept, None);
    if let Some(o) = p.as_object_mut() {
        o.insert("desiredK".into(), json!(desired_k));
        if let Some(start) = start {
            o.insert("startWith".into(), json!(start));
        }
        if has_slices(app) {
            o.insert("use".into(), json!(scope));
        }
    }
    p
}

/// The size Optimize to File Size… starts at, in bytes: what the current settings give what Use
/// measures, the image slices' files together when there are slices, else the image's file.
fn optimize_start(app: &mut VectorcraftApp, options: &SaveForWebOptions) -> Option<u64> {
    if has_slices(app) {
        let p = optimize_params(app, options, webexport::MAX_DESIRED_K, None, "allSlices");
        let r = app.run("webExport.optimizeToFileSize", p).ok()?;
        return r["achieved"].as_bool().filter(|a| *a).and_then(|_| r["bytes"].as_u64());
    }
    let s = app.session.web_settings(&settings_of(options, &app.web_export.table, &app.web_export.kept, None)).ok()?;
    encoded(app, &s).ok().map(|o| o.bytes.len() as u64)
}

/// `webExport.optimizeToFileSize` for the dialog's request. The settings it finds replace the
/// dialog's, and the result is kept with the art it measured.
fn run_optimize(app: &mut VectorcraftApp, req: &OptimizeRequest) -> Result<OptimizeAnswer, String> {
    let start = match req.start {
        OptimizeStart::Auto => "auto",
        _ => "current",
    };
    let scope = match req.scope {
        OptimizeScope::EachSlice => "eachSlice",
        OptimizeScope::CurrentSlice => "currentSlice",
        _ => "allSlices",
    };
    let p = optimize_params(app, &req.options, req.desired_k, Some(start), scope);
    let r = app.run("webExport.optimizeToFileSize", p)?;
    let found = app.session.web_settings(&r["settings"])?;
    app.web_export.table = serde_json::to_value(&found.color_table).unwrap_or_default();
    app.web_export.kept = kept_of(&found);
    app.web_export.optimized = Some(
        json!({ "settings": r["settings"], "bytes": r["bytes"], "targetBytes": r["targetBytes"], "achieved": r["achieved"], "art": art_key(app) }),
    );
    let natural = WebSettings { width: None, height: None, percent: None, ..found.clone() };
    let natural = render_for(app, &natural).ok().map(|(_, r)| PixelSize::new(r.width, r.height));
    Ok(OptimizeAnswer::new(
        Some(options_of(&found, natural)),
        r["bytes"].as_u64().unwrap_or(0),
        r["targetBytes"].as_u64().unwrap_or(0),
        r["achieved"] == json!(true),
    ))
}

/// Answers Save…'s request for a path. Bad settings never open a save panel; the web downloads, so
/// the suggested name stands in for the path.
fn ask_path(app: &mut VectorcraftApp, host: &craft::ui::shell::WindowDialogs, id: DialogId, req: PathRequest, options: &SaveForWebOptions) {
    let settings = match app.session.web_settings(&settings_of(options, &app.web_export.table, &app.web_export.kept, None)) {
        Ok(s) => s,
        Err(e) => {
            on_dialog(host, HostedSaveForWeb::cancel_requests);
            return app.status(e);
        }
    };
    let key = req.key.clone();
    if io::is_web(app) {
        return answer_path(app, id, &key, PathBuf::from(req.file_name.unwrap_or_default()));
    }
    let ext = if settings.output == WebOutput::Html { "html" } else { settings.format.ext() };
    let mut pick = PickRequest::save("export").suggested_extension(ext);
    if let Some(name) = &req.file_name {
        pick = pick.file_name(name.clone());
    }
    for f in &req.filters {
        let extensions: Vec<&str> = f.extensions.iter().map(String::as_str).collect();
        pick = pick.filter(f.name.clone(), &extensions);
    }
    // A cancelled panel has no path.
    let opening = app.web_export.opening.clone();
    file_asks::ask(app, pick, Owner::Free, move |app, picked| {
        if !Arc::ptr_eq(&opening, &app.web_export.opening) {
            return Ok(Value::Null);
        }
        if !on_original_document(app) {
            if let Some(host) = app.dialogs() {
                on_dialog(&host, HostedSaveForWeb::cancel_requests);
            }
            app.status(tl!(WRONG_DOCUMENT).to_owned());
            return Ok(Value::Null);
        }
        match picked.first_path() {
            Some(path) => answer_path(app, id, &key, path),
            None => {
                if let Some(host) = app.dialogs() {
                    on_dialog(&host, HostedSaveForWeb::cancel_requests);
                }
            }
        }
        Ok(Value::Null)
    });
}

/// Gives the dialog `id` the path its request `key` asked for.
fn answer_path(app: &VectorcraftApp, id: DialogId, key: &str, path: PathBuf) {
    if let Some(host) = app.dialogs() {
        host.with(|h| h.get_mut_by_id::<HostedSaveForWeb>(id).map(|d| d.answer_path(key, path)));
    }
}

/// The dialog closed with Save… or Done: the settings are remembered, and with a path the files are
/// written. Failures show in the status line; the dialog has closed.
fn finish(app: &mut VectorcraftApp, result: SaveForWebResult) {
    let wrong_document = result.path.is_some() && !on_original_document(app);
    let table = std::mem::take(&mut app.web_export.table);
    let kept = std::mem::take(&mut app.web_export.kept);
    app.web_export = WebExport::default();
    if wrong_document {
        return app.status(tl!(WRONG_DOCUMENT).to_owned());
    }
    let params = settings_of(&result.options, &table, &kept, None);
    if let Err(e) = app.run("webExport.settings", params.clone()) {
        return app.status(e);
    }
    let Some(path) = result.path else { return };
    let mut p = params;
    // The web downloads, so the name the panel stood in for is no path.
    if !io::is_web(app) {
        p["path"] = json!(path.to_string_lossy().replace('\\', "/"));
    }
    if let Err(e) = save(app, p) {
        app.status(e);
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_table;
