//! File → Export → Save for Web (Legacy) (`saveForWeb`, Ctrl/Cmd+Alt+Shift+S): the art previewed
//! as the web image Save writes, beside the original (Original, Optimized, 2-Up and 4-Up views,
//! each with its file size and download time), and the settings: format (GIF, JPEG, PNG-8,
//! PNG-24), colour reduction, colours, dither, transparency and matte, interlacing, web snap,
//! lossy, JPEG quality, the colour table (lock, map to transparent, web shift, sort), image size
//! and anti-aliasing, Clip to Artboard, sRGB, metadata, slices and HTML output, and presets.
//!
//! Fields are the `document.exportForWeb` settings (`webExport.settings` gives them; set one with
//! `ui.dialog.set`, or `preset` to load a preset), plus the dialog's own: `__view` (`original`,
//! `optimized`, `2up`, `4up`), `__zoom` (`fit` or a percentage), `__kbps` (connection speed),
//! `__color` (the colour table's selected colour, by source), `__presetName` (shown while saving a
//! preset) and `__hasSlices`. Save… remembers
//! the settings and writes the files (`file.saveForWeb`); Done only remembers them; Preview in
//! Browser opens the HTML page (`file.saveForWeb.browser`). The previews are what Save writes:
//! [`webexport::preview_image`], decoded back, cached until the art or a setting changes.

use std::sync::Arc;

use serde_json::{Map, Value, json};
use vectorcraft_engine::cmd::webexport::{
    self, SPEEDS, SliceScope, TableColor, WebFormat, WebMetadata, WebOutput, WebRender, WebSettings, download_seconds, hex,
};
use vectorcraft_render::AntiAlias;
use vectorcraft_render::encode::quantize::{Dither, Reduction};
use vectorcraft_render::encode::web::ColorSort;

use super::png_options::color_button;
use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, io, widgets};

pub const KIND: &str = "saveForWeb";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Save for Web").into(),
    body,
    confirm,
    ok: Some("Save…"),
    ok_label: Some(|app| if io::is_web(app) { "Download" } else { "Save…" }),
    discard: Some("Done"),
    min_width: PREVIEW_W + GAP + SIDE_W,
    ..DialogSpec::FORM
};

/// The preview area (panes and their captions) and the settings column.
const PREVIEW_W: f32 = 620.0;
const PREVIEW_H: f32 = 470.0;
const SIDE_W: f32 = 296.0;
const GAP: f32 = 16.0;
/// Label column and field widths of the settings column.
const LABEL_W: f32 = 96.0;
const FIELD_W: f32 = 190.0;
/// A pane's caption height.
const CAPTION_H: f32 = 34.0;

/// The views (`__view`) and their tabs.
const VIEWS: [&str; 4] = ["original", "optimized", "2up", "4up"];
const VIEW_LABELS: [&str; 4] = ["Original", "Optimized", "2-Up", "4-Up"];
/// Zoom choices (`__zoom`: `fit` or a percentage).
const ZOOMS: [&str; 5] = ["fit", "50", "100", "200", "400"];
const ZOOM_LABELS: [&str; 5] = ["Fit in View", "50%", "100%", "200%", "400%"];
/// Palette sizes offered (any 2–256 can be typed).
const COLOR_COUNTS: [&str; 8] = ["2", "4", "8", "16", "32", "64", "128", "256"];
/// Matte values and their labels (`Other`: a colour).
const MATTES: [&str; 3] = ["none", "white", "black"];
const MATTE_LABELS: [&str; 4] = ["None", "White", "Black", "Other"];
/// JPEG quality steps: (label, quality).
const QUALITIES: [(&str, u8); 5] = [("Low", 10), ("Medium", 30), ("High", 60), ("Very High", 80), ("Maximum", 100)];

/// Open the dialog on the remembered settings.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let n = app.session.active().ok_or("no document")?.doc.artboards.len();
    let mut s = app.session.last_web_settings();
    // A remembered artboard this document doesn't have: its first.
    if s.artboard.is_some_and(|a| a >= n) {
        s.artboard = None;
    }
    let mut fields = settings_fields(&s);
    let slices = app.session.active().is_some_and(|st| !st.doc.slice_layout().is_empty());
    fields.extend([
        ("__view".into(), json!("2up")),
        ("__zoom".into(), json!("100")),
        ("__kbps".into(), json!(56.6)),
        ("__hasSlices".into(), json!(slices)),
    ]);
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields });
    Ok(Value::Null)
}

/// The settings as dialog fields.
fn settings_fields(s: &WebSettings) -> Map<String, Value> {
    s.to_json().as_object().cloned().unwrap_or_default()
}

/// The settings fields of the dialog (the `document.exportForWeb` params).
fn params(d: &Dialog) -> Value {
    let mut p = form::params(d);
    if let Some(o) = p.as_object_mut() {
        o.remove("discard");
    }
    p
}

/// A `preset` field (set by an agent, or the preset list) replaces the settings with the preset's.
fn apply_preset(app: &VectorcraftApp, d: &mut Dialog) -> Result<(), String> {
    let Some(name) = d.fields.remove("preset") else { return Ok(()) };
    let s = app.session.web_settings(&json!({ "preset": name }))?;
    d.fields.extend(settings_fields(&s));
    Ok(())
}

/// The settings the fields describe.
fn settings_of(app: &VectorcraftApp, d: &Dialog) -> Result<WebSettings, String> {
    app.session.web_settings(&params(d))
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut d = d.clone();
    apply_preset(app, &mut d)?;
    let p = params(&d);
    // Done remembers the settings; Save… remembers them and writes the files.
    app.run("webExport.settings", p.clone())?;
    if d.bool("discard") {
        app.ui.dialog = None;
        return Ok(json!({ "remembered": true }));
    }
    let r = save(app, p)?;
    app.ui.dialog = None;
    Ok(r)
}

/// `file.saveForWeb` with settings: write the files (`path`, else a picked file: the page with HTML
/// output, else the image); the web downloads them. The status bar shows the export's first
/// warning.
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
            None => app.status(format!("Downloaded {} file(s)", names.len())),
        }
        return Ok(json!({ "files": names, "bytes": r["bytes"], "warnings": r["warnings"] }));
    }
    let ext = if s.output == WebOutput::Html { "html" } else { s.format.ext() };
    let path = io::target_path(app, path, ext)?;
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
}

/// Whether an export's reply `r` has warnings (an image that couldn't be decoded):
/// [`io::report_saved`] puts the first in the status bar as a save's note.
fn has_warnings(r: &Value) -> bool {
    r["warnings"].as_array().is_some_and(|w| !w.is_empty())
}

/// `file.saveForWeb.browser`: the HTML page and its images in a temporary folder, opened in the
/// default browser (desktop). When the export has warnings, the status bar shows the first in
/// place of "Opened …".
pub(crate) fn browser_preview(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    if io::is_web(app) {
        return Err("Preview in Browser needs the desktop app (the web downloads files instead)".into());
    }
    let stem = app.session.active().map(|st| vectorcraft_engine::cmd::fileio::file_stem(&st.doc.title)).ok_or("no document")?;
    let dir = temp_folder()?;
    let path = format!("{dir}/{}.html", if stem.trim().is_empty() { "Untitled" } else { stem.as_str() });
    let mut q = if p.is_object() { p.clone() } else { json!({}) };
    if let Some(o) = q.as_object_mut() {
        o.insert("path".into(), json!(path));
        o.insert("output".into(), json!("html"));
    }
    let r = app.run("document.exportForWeb", q)?;
    app.open_url(&io::file_url(&path));
    if has_warnings(&r) {
        io::report_saved(app, &path, &r);
    }
    Ok(json!({ "path": path, "warnings": r["warnings"] }))
}

/// The folder browser previews go to.
#[cfg(not(target_arch = "wasm32"))]
fn temp_folder() -> Result<String, String> {
    let dir = std::env::temp_dir().join("VectorCraft Save for Web");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir.to_string_lossy().replace('\\', "/"))
}

#[cfg(target_arch = "wasm32")]
fn temp_folder() -> Result<String, String> {
    Err("no file system here".into())
}

// ---------- previews ----------

/// What a pane shows.
#[derive(Clone)]
struct Shot {
    texture: egui::TextureHandle,
    size: [u32; 2],
    /// The file's size (bytes); `None` for the original.
    bytes: Option<usize>,
    colors: Arc<Vec<TableColor>>,
    mapped: Arc<Vec<[u8; 3]>>,
}

/// The previews of the open dialog: the rendering (until the art or the image size changes) and
/// the optimised panes by settings.
#[derive(Clone, Default)]
struct Cache {
    render_key: String,
    render: Option<Result<Arc<WebRender>, String>>,
    original: Option<Shot>,
    shots: Vec<(String, Result<Shot, String>)>,
}

fn cache_id() -> egui::Id {
    egui::Id::new("save-for-web-previews")
}

/// The texture of straight RGBA pixels.
fn texture(ctx: &egui::Context, name: &str, px: &[u8], [w, h]: [u32; 2]) -> egui::TextureHandle {
    let img = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], px);
    let opts = egui::TextureOptions { magnification: egui::TextureFilter::Nearest, ..egui::TextureOptions::LINEAR };
    ctx.load_texture(name, img, opts)
}

/// The rendering for `s` and the optimised image for each of `panes`, from the cache when nothing
/// they depend on changed.
fn previews(app: &VectorcraftApp, ctx: &egui::Context, s: &WebSettings, panes: &[WebSettings]) -> (Result<Shot, String>, Vec<Result<Shot, String>>) {
    let Some(st) = app.session.active() else { return (Err("no document".into()), vec![]) };
    let doc = &st.doc;
    let mut cache: Cache = ctx.data(|m| m.get_temp(cache_id())).unwrap_or_default();
    let render_key = format!(
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
    );
    if cache.render_key != render_key || cache.render.is_none() {
        let r = webexport::render(doc, s).map(Arc::new);
        cache.original = r.as_ref().ok().map(|r| Shot {
            texture: texture(ctx, "save-for-web-original", &r.rgba, [r.width, r.height]),
            size: [r.width, r.height],
            bytes: None,
            colors: Arc::default(),
            mapped: Arc::default(),
        });
        cache = Cache { render_key, render: Some(r), original: cache.original, shots: vec![] };
    }
    let original = match (&cache.render, &cache.original) {
        (Some(Ok(_)), Some(o)) => Ok(o.clone()),
        (Some(Err(e)), _) => Err(e.clone()),
        _ => Err("nothing to preview".into()),
    };
    let mut shots = Vec::with_capacity(panes.len());
    let mut kept = Vec::with_capacity(panes.len());
    for (i, p) in panes.iter().enumerate() {
        let key = p.to_json().to_string();
        let shot = match cache.shots.iter().find(|(k, _)| *k == key) {
            Some((_, s)) => s.clone(),
            None => match &cache.render {
                Some(Ok(r)) => optimise(ctx, doc, r, p, i),
                Some(Err(e)) => Err(e.clone()),
                None => Err("nothing to preview".into()),
            },
        };
        kept.push((key, shot.clone()));
        shots.push(shot);
    }
    cache.shots = kept;
    ctx.data_mut(|m| m.insert_temp(cache_id(), cache));
    (original, shots)
}

fn optimise(ctx: &egui::Context, doc: &vectorcraft_doc::Document, r: &WebRender, s: &WebSettings, pane: usize) -> Result<Shot, String> {
    let o = webexport::preview_image(doc, &r.rgba, r.width, r.height, s)?;
    let (px, w, h) = webexport::decode(&o)?;
    Ok(Shot {
        texture: texture(ctx, &format!("save-for-web-{pane}"), &px, [w, h]),
        size: [w, h],
        bytes: Some(o.bytes.len()),
        colors: Arc::new(o.colors),
        mapped: Arc::new(o.mapped),
    })
}

/// The two lighter settings 4-Up offers beside the current ones: lower quality, fewer colours, or
/// for PNG-24 a palette and a JPEG.
fn alternates(s: &WebSettings) -> [WebSettings; 2] {
    let with = |f: &dyn Fn(&mut WebSettings)| {
        let mut a = s.clone();
        f(&mut a);
        a
    };
    match s.format {
        WebFormat::Jpg => [with(&|a| a.quality /= 2), with(&|a| a.quality /= 4)],
        WebFormat::Gif | WebFormat::Png8 => [with(&|a| a.colors = (a.colors / 2).max(2)), with(&|a| a.colors = (a.colors / 4).max(2))],
        WebFormat::Png24 => [
            with(&|a| {
                a.format = WebFormat::Png8;
                a.colors = 256;
            }),
            with(&|a| a.format = WebFormat::Jpg),
        ],
    }
}

// ---------- the dialog ----------

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    if app.session.active().is_none() {
        return true;
    }
    if let Err(e) = apply_preset(app, d) {
        app.status(e);
    }
    let settings = settings_of(app, d);
    let mut browser = false;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(PREVIEW_W);
            browser = preview_area(app, ui, d, &settings);
        });
        ui.add_space(GAP);
        ui.vertical(|ui| {
            ui.set_width(SIDE_W);
            side(app, ui, d, settings.as_ref().ok());
        });
    });
    if browser && let Err(e) = app.run("file.saveForWeb.browser", params(d)) {
        app.status(e);
    }
    false
}

/// The view tabs, the panes and the zoom and speed row. Returns whether Preview in Browser was
/// clicked.
fn preview_area(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog, settings: &Result<WebSettings, String>) -> bool {
    let t = Tokens::get(ui.ctx());
    let view = VIEWS.iter().position(|v| *v == d.str("__view")).unwrap_or(2);
    if let Some(i) = widgets::tab_bar(ui, &VIEW_LABELS, view) {
        d.fields.insert("__view".into(), json!(VIEWS[i]));
    }
    ui.add_space(4.0);
    let (area, resp) = ui.allocate_exact_size(egui::vec2(PREVIEW_W, PREVIEW_H), egui::Sense::click_and_drag());
    let pan_id = egui::Id::new("save-for-web-pan");
    let mut pan: egui::Vec2 = ui.ctx().data(|m| m.get_temp(pan_id)).unwrap_or_default();
    if resp.dragged() {
        pan += resp.drag_delta();
        ui.ctx().data_mut(|m| m.insert_temp(pan_id, pan));
    }
    match settings {
        Err(e) => {
            ui.painter().rect_filled(area, 2.0, t.pasteboard);
            ui.painter().text(area.center(), egui::Align2::CENTER_CENTER, format!("⚠ {e}"), egui::FontId::proportional(12.5), t.text);
        }
        Ok(s) => {
            let alts = alternates(s);
            let panes: Vec<WebSettings> = if view == 3 { vec![s.clone(), alts[0].clone(), alts[1].clone()] } else { vec![s.clone()] };
            // The current settings' image is made in every view: the colour table shows its colours.
            let (original, shots) = previews(app, ui.ctx(), s, &panes);
            let zoom = d.str("__zoom").parse::<f32>().ok().filter(|z| z.is_finite() && *z > 0.0).map(|z| z / 100.0);
            let kbps = d.f64("__kbps", 56.6);
            let Some(checker) = app.session.active().map(|st| crate::canvas::checker_texture(ui.ctx(), &st.doc.setup)) else { return false };
            let cells: Vec<(egui::Rect, Pane)> = match view {
                0 => vec![(area, Pane::Original)],
                1 => vec![(area, Pane::Optimized(0))],
                2 => split(area, 2, 1).into_iter().zip([Pane::Original, Pane::Optimized(0)]).collect(),
                _ => split(area, 2, 2).into_iter().zip([Pane::Original, Pane::Optimized(0), Pane::Optimized(1), Pane::Optimized(2)]).collect(),
            };
            for (rect, pane) in cells {
                let (shot, settings) = match pane {
                    Pane::Original => (&original, None),
                    Pane::Optimized(i) => (shots.get(i).unwrap_or(&original), panes.get(i)),
                };
                let selected = view == 3 && matches!(pane, Pane::Optimized(0));
                draw_pane(ui, rect, shot, settings, zoom, pan, (checker.0.id(), checker.1), selected, kbps);
                // 4-Up: clicking a lighter alternate takes its settings.
                if let (Pane::Optimized(1..), Some(a)) = (pane, settings)
                    && resp.clicked()
                    && resp.interact_pointer_pos().is_some_and(|p| rect.contains(p))
                {
                    d.fields.extend(settings_fields(a));
                }
            }
        }
    }
    ui.add_space(6.0);
    let mut browser = false;
    ui.horizontal(|ui| {
        let z = ZOOMS.iter().position(|z| *z == d.str("__zoom")).unwrap_or(2);
        if let Some(i) = widgets::dropdown(ui, "sfw-zoom", ZOOM_LABELS[z], &ZOOM_LABELS, 110.0) {
            d.fields.insert("__zoom".into(), json!(ZOOMS[i]));
            ui.ctx().data_mut(|m| m.insert_temp(pan_id, egui::Vec2::ZERO));
        }
        let kbps = d.f64("__kbps", 56.6);
        let labels: Vec<String> = SPEEDS.iter().map(|s| speed_label(*s)).collect();
        let names: Vec<&str> = labels.iter().map(String::as_str).collect();
        if let Some(i) = widgets::dropdown(ui, "sfw-speed", &speed_label(kbps), &names, 110.0)
            && let Some(s) = SPEEDS.get(i)
        {
            d.fields.insert("__kbps".into(), json!(s));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let b = ui.add_enabled(!io::is_web(app), |ui: &mut egui::Ui| widgets::secondary_button(ui, tl!("Preview in Browser")));
            browser = b.on_disabled_hover_text(tl!("Needs the desktop app")).clicked();
        });
    });
    browser
}

#[derive(Clone, Copy)]
enum Pane {
    Original,
    /// The optimised image of the current settings (0) or an alternate (1, 2).
    Optimized(usize),
}

/// `area` cut into `cols` × `rows` panes with a gap.
fn split(area: egui::Rect, cols: usize, rows: usize) -> Vec<egui::Rect> {
    let gap = 6.0;
    let w = (area.width() - gap * (cols as f32 - 1.0)) / cols as f32;
    let h = (area.height() - gap * (rows as f32 - 1.0)) / rows as f32;
    (0..rows)
        .flat_map(|r| (0..cols).map(move |c| (c, r)))
        .map(|(c, r)| egui::Rect::from_min_size(area.min + egui::vec2(c as f32 * (w + gap), r as f32 * (h + gap)), egui::vec2(w, h)))
        .collect()
}

/// `kbps` as the speed list shows it.
fn speed_label(kbps: f64) -> String {
    if kbps >= 1024.0 { format!("{} Mbps", kbps / 1024.0) } else { format!("{kbps} Kbps") }
}

/// A file size as the captions show it.
fn size_label(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} bytes")
    } else if bytes < 1 << 20 {
        format!("{:.1}K", bytes as f64 / 1024.0)
    } else {
        format!("{:.2}M", bytes as f64 / f64::from(1 << 20))
    }
}

/// One pane: the image over the transparency grid at `zoom` (`None`: fit), moved by `pan`, and
/// its caption: what it is, its size and download time, and its colours or quality.
#[allow(clippy::too_many_arguments)]
fn draw_pane(
    ui: &egui::Ui,
    rect: egui::Rect,
    shot: &Result<Shot, String>,
    settings: Option<&WebSettings>,
    zoom: Option<f32>,
    pan: egui::Vec2,
    (checker, cell): (egui::TextureId, f32),
    selected: bool,
    kbps: f64,
) {
    let t = Tokens::get(ui.ctx());
    let image = egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, rect.max.y - CAPTION_H));
    let p = ui.painter().with_clip_rect(image);
    p.rect_filled(image, 0.0, t.pasteboard);
    let font = egui::FontId::proportional(11.5);
    let (first, second, third) = match shot {
        Err(e) => {
            p.text(image.center(), egui::Align2::CENTER_CENTER, format!("⚠ {e}"), font.clone(), t.text);
            ("".into(), String::new(), String::new())
        }
        Ok(shot) => {
            let size = egui::vec2(shot.size[0] as f32, shot.size[1] as f32);
            let scale = zoom.unwrap_or_else(|| (image.width() / size.x).min(image.height() / size.y).min(1.0));
            let shown = egui::Rect::from_center_size(image.center() + pan, size * scale);
            let uv = egui::Rect::from_min_max(egui::Pos2::ZERO, (shown.size() / (2.0 * cell)).to_pos2());
            p.image(checker, shown, uv, egui::Color32::WHITE);
            p.image(shot.texture.id(), shown, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
            match (settings, shot.bytes) {
                (Some(s), Some(bytes)) => {
                    let secs = download_seconds(bytes, kbps);
                    let time =
                        if secs < 1.0 { tl!("< 1 sec").into() } else { crate::i18n::fmt(tl!("{secs} sec"), &[("secs", &secs.ceil().to_string())]) };
                    let detail = match s.format {
                        WebFormat::Jpg => crate::i18n::fmt(tl!("Quality {quality}"), &[("quality", &s.quality.to_string())]),
                        WebFormat::Png24 => String::new(),
                        _ => crate::i18n::tn(shot.colors.len() as u64, "{n} color", "{n} colors"),
                    };
                    (s.format.label().to_string(), format!("{}  {time} @ {}", size_label(bytes), speed_label(kbps)), detail)
                }
                _ => (tl!("Original").into(), format!("{} × {} px", shot.size[0], shot.size[1]), String::new()),
            }
        }
    };
    let stroke = if selected { egui::Stroke::new(1.5, t.accent) } else { egui::Stroke::new(1.0, t.border) };
    ui.painter().rect_stroke(image, 0.0, stroke, egui::StrokeKind::Inside);
    let caption = egui::Rect::from_min_max(egui::pos2(rect.min.x, image.max.y), rect.max).shrink2(egui::vec2(4.0, 3.0));
    let p = ui.painter().with_clip_rect(caption);
    p.text(caption.left_top(), egui::Align2::LEFT_TOP, first, font.clone(), t.text_strong);
    p.text(caption.left_bottom(), egui::Align2::LEFT_BOTTOM, second, font.clone(), t.text_dim);
    p.text(caption.right_bottom(), egui::Align2::RIGHT_BOTTOM, third, font, t.text_dim);
}

// ---------- the settings column ----------

/// `(value, label)` pairs of a keyed enum.
fn pairs<T: Copy>(all: &[T], key: fn(T) -> &'static str, label: fn(T) -> &'static str) -> Vec<(&'static str, &'static str)> {
    all.iter().map(|v| (key(*v), label(*v))).collect()
}

fn side(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog, s: Option<&WebSettings>) {
    presets_row(app, ui, d, s);
    widgets::divider(ui);
    egui::ScrollArea::vertical().id_salt("sfw-settings").max_height(PREVIEW_H - 40.0).auto_shrink([false, true]).show(ui, |ui| {
        let choice = |ui: &mut egui::Ui, d: &mut Dialog, key: &str, label: &str, options: &[(&str, &str)]| {
            form::choice(ui, d, key, label, (LABEL_W, FIELD_W), options);
        };
        choice(ui, d, "format", tl!("Format:"), &pairs(&WebFormat::ALL, WebFormat::id, WebFormat::label));
        let format = s.map_or(WebFormat::Gif, |s| s.format);
        match format {
            WebFormat::Gif | WebFormat::Png8 => {
                choice(ui, d, "reduction", tl!("Reduction:"), &pairs(&Reduction::ALL, Reduction::id, Reduction::label));
                colors_row(ui, d);
                choice(ui, d, "dither", tl!("Dither:"), &pairs(&Dither::ALL, Dither::id, Dither::label));
                if Dither::from_id(&d.str("dither")).is_some_and(|x| x != Dither::None) {
                    slider_row(ui, d, "ditherAmount", tl!("Amount:"), "%");
                }
                check_pair(ui, d, ("transparency", tl!("Transparency")), ("interlaced", tl!("Interlaced")));
                matte_row(ui, d);
                slider_row(ui, d, "webSnap", tl!("Web Snap:"), "%");
                if format == WebFormat::Gif {
                    slider_row(ui, d, "lossy", tl!("Lossy:"), "%");
                }
            }
            WebFormat::Jpg => {
                quality_row(ui, d);
                check_pair(ui, d, ("progressive", tl!("Progressive")), ("optimized", tl!("Optimized")));
                widgets::label_row(ui, "", LABEL_W, |ui| form::check(ui, d, "embedProfile", tl!("ICC Profile")));
                matte_row(ui, d);
            }
            WebFormat::Png24 => {
                check_pair(ui, d, ("transparency", tl!("Transparency")), ("interlaced", tl!("Interlaced")));
                matte_row(ui, d);
            }
        }
        widgets::label_row(ui, "", LABEL_W, |ui| form::check(ui, d, "convertToSrgb", tl!("Convert to sRGB")));
        choice(ui, d, "metadata", tl!("Metadata:"), &pairs(&WebMetadata::ALL, WebMetadata::key, WebMetadata::label));
        if format.palette() {
            widgets::divider(ui);
            color_table(ui, d, s);
        }
        widgets::divider(ui);
        image_size(app, ui, d, s);
        widgets::divider(ui);
        if d.bool("__hasSlices") {
            choice(ui, d, "slices", tl!("Slices:"), &pairs(&SliceScope::ALL, SliceScope::key, SliceScope::label));
        }
        choice(ui, d, "output", tl!("Output:"), &pairs(&WebOutput::ALL, WebOutput::key, WebOutput::label));
    });
}

/// The preset list (the settings' preset, else "[Unnamed]") with Save and Delete.
fn presets_row(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog, s: Option<&WebSettings>) {
    let presets = app.session.web_presets();
    let saved = app.session.prefs.web_export_presets.len();
    let current = s.and_then(|s| presets.iter().rposition(|p| p.settings == *s));
    let names: Vec<&str> = presets.iter().map(|p| p.name.as_str()).collect();
    // The built-in presets come first (translated); the saved ones are names.
    let builtins = presets.len().saturating_sub(saved);
    let labels = super::shown_names(crate::i18n::current(), &names, |k| k < builtins);
    let mut run: Option<(&str, Value)> = None;
    widgets::label_row(ui, tl!("Preset:"), LABEL_W, |ui| {
        let shown = current.and_then(|i| labels.get(i).copied()).unwrap_or(tl!("[Unnamed]"));
        if let Some(i) = widgets::dropdown_names(ui, "sfw-preset", shown, &labels, FIELD_W - 60.0)
            && let Some(p) = presets.get(i)
        {
            d.fields.extend(settings_fields(&p.settings));
        }
        let editing = d.fields.contains_key("__presetName");
        if widgets::icon_button(ui, "save", tl!("Save the settings as a preset"), editing, 24.0).clicked() {
            match editing {
                true => _ = d.fields.remove("__presetName"),
                false => _ = d.fields.insert("__presetName".into(), json!("")),
            }
        }
        let user = current.filter(|i| *i + saved >= presets.len());
        let del =
            ui.add_enabled(user.is_some(), |ui: &mut egui::Ui| widgets::icon_button(ui, "trash-2", tl!("Delete this saved preset"), false, 24.0));
        if del.clicked()
            && let Some(name) = user.and_then(|i| names.get(i))
        {
            run = Some(("webExport.presets.delete", json!({ "name": name })));
        }
    });
    if d.fields.contains_key("__presetName") {
        widgets::label_row(ui, tl!("Name:"), LABEL_W, |ui| {
            form::text(ui, d, "__presetName", FIELD_W - 70.0);
            let name = d.str("__presetName");
            if ui.add_enabled(!name.trim().is_empty(), |ui: &mut egui::Ui| widgets::flat_button(ui, tl!("Save"), 52.0)).clicked() {
                let mut p = params(d);
                p["name"] = json!(name.trim());
                run = Some(("webExport.presets.save", p));
            }
        });
    }
    if let Some((id, p)) = run {
        match app.run(id, p) {
            Ok(_) => _ = d.fields.remove("__presetName"),
            Err(e) => app.status(e),
        }
    }
}

/// Two checkboxes side by side.
fn check_pair(ui: &mut egui::Ui, d: &mut Dialog, (a, a_label): (&str, &str), (b, b_label): (&str, &str)) {
    widgets::label_row(ui, "", LABEL_W, |ui| {
        ui.allocate_ui(egui::vec2(FIELD_W / 2.0, 20.0), |ui| {
            ui.set_min_width(FIELD_W / 2.0);
            form::check(ui, d, a, a_label);
        });
        form::check(ui, d, b, b_label);
    });
}

/// A 0–100 slider bound to `d.fields[key]`, its value shown with `suffix`.
fn slider_row(ui: &mut egui::Ui, d: &mut Dialog, key: &str, label: &str, suffix: &str) {
    widgets::label_row(ui, label, LABEL_W, |ui| {
        let mut v = d.f64(key, 0.0).round().clamp(0.0, 100.0) as u8;
        ui.spacing_mut().slider_width = FIELD_W - 60.0;
        if widgets::slider(ui, egui::Slider::new(&mut v, 0..=100).suffix(suffix)).changed() {
            d.fields.insert(key.into(), json!(v));
        }
    });
}

/// The palette size: the usual counts, or any 2–256.
fn colors_row(ui: &mut egui::Ui, d: &mut Dialog) {
    widgets::label_row(ui, tl!("Colors:"), LABEL_W, |ui| {
        let mut n = d.f64("colors", 128.0).round().clamp(2.0, 256.0) as u16;
        if let Some(c) = widgets::dropdown(ui, "sfw-colors", &n.to_string(), &COLOR_COUNTS, 80.0).and_then(|i| COLOR_COUNTS.get(i)) {
            n = c.parse().unwrap_or(n);
            d.fields.insert("colors".into(), json!(n));
        }
        if let Some(n) = widgets::range_field(ui, "sfw-colors-n", f64::from(n), 2.0..=256.0, "", 0, 52.0) {
            d.fields.insert("colors".into(), json!(n as u16));
        }
    });
}

/// JPEG quality: a named step, then the exact value.
fn quality_row(ui: &mut egui::Ui, d: &mut Dialog) {
    let q = d.f64("quality", 60.0).round().clamp(0.0, 100.0) as u8;
    let step = QUALITIES.iter().rev().find(|(_, v)| q >= *v).map_or("Low", |(l, _)| *l);
    widgets::label_row(ui, tl!("Quality:"), LABEL_W, |ui| {
        let labels = QUALITIES.map(|(l, _)| l);
        if let Some((_, v)) = widgets::dropdown(ui, "sfw-quality", step, &labels, FIELD_W).and_then(|i| QUALITIES.get(i)) {
            d.fields.insert("quality".into(), json!(v));
        }
    });
    slider_row(ui, d, "quality", "", "");
}

/// The matte: none, white, black or a colour.
fn matte_row(ui: &mut egui::Ui, d: &mut Dialog) {
    let matte = d.str("matte");
    let at = MATTES.iter().position(|v| matte.eq_ignore_ascii_case(v)).unwrap_or(3);
    widgets::label_row(ui, tl!("Matte:"), LABEL_W, |ui| {
        if let Some(i) = widgets::dropdown(ui, "sfw-matte", MATTE_LABELS[at], &MATTE_LABELS, 120.0) {
            d.fields.insert("matte".into(), json!(MATTES.get(i).copied().unwrap_or("#808080")));
        }
        if at == 3 {
            color_button(ui, d, "matte");
        }
    });
}

/// The colour table of the current settings' preview: swatches (the selected one ringed; locked
/// ones with a square, web-safe ones with a diamond, transparent ones over the grid) and the
/// buttons that edit the selected colour, plus the sort order.
fn color_table(ui: &mut egui::Ui, d: &mut Dialog, s: Option<&WebSettings>) {
    let t = Tokens::get(ui.ctx());
    let cache: Option<Cache> = ui.ctx().data(|m| m.get_temp(cache_id()));
    let key = s.map(|s| s.to_json().to_string());
    let shot = cache.and_then(|c| c.shots.into_iter().find(|(k, _)| Some(k) == key.as_ref())).and_then(|(_, s)| s.ok());
    let Some(shot) = shot else {
        widgets::subheader(ui, tl!("Color Table"));
        widgets::dim_label(ui, tl!("Shown with the optimised preview."));
        return;
    };
    widgets::subheader(ui, &crate::i18n::tn(shot.colors.len() as u64, "Color Table ({n} color)", "Color Table ({n} colors)"));
    let selected = d.str("__color");
    // (colour shown, source, transparent, locked, web-safe, mapped by hand)
    let rows: Vec<(TableColor, bool)> = shot
        .colors
        .iter()
        .map(|c| (*c, false))
        .chain(shot.mapped.iter().map(|c| (TableColor { color: *c, source: *c, transparent: true, locked: false, web_shifted: false }, true)))
        .collect();
    const CELL: f32 = 15.0;
    let per_row = ((SIDE_W - 8.0) / (CELL + 2.0)).floor().max(1.0) as usize;
    let lines = rows.len().div_ceil(per_row).max(1);
    let (area, resp) = ui.allocate_exact_size(egui::vec2(SIDE_W - 8.0, lines as f32 * (CELL + 2.0)), egui::Sense::click());
    let p = ui.painter();
    for (i, (c, mapped)) in rows.iter().enumerate() {
        let r = egui::Rect::from_min_size(
            area.min + egui::vec2((i % per_row) as f32 * (CELL + 2.0), (i / per_row) as f32 * (CELL + 2.0)),
            egui::vec2(CELL, CELL),
        );
        let [red, g, b] = c.color;
        let fill = egui::Color32::from_rgb(red, g, b);
        if c.transparent {
            // Half the colour (a mapped one), half the grid.
            p.rect_filled(r, 0.0, egui::Color32::WHITE);
            p.rect_filled(egui::Rect::from_min_size(r.min, r.size() / 2.0), 0.0, egui::Color32::from_gray(200));
            p.rect_filled(egui::Rect::from_min_size(r.center(), r.size() / 2.0), 0.0, egui::Color32::from_gray(200));
            if *mapped {
                p.add(egui::Shape::convex_polygon(vec![r.left_top(), r.right_top(), r.left_bottom()], fill, egui::Stroke::NONE));
            }
        } else {
            p.rect_filled(r, 0.0, fill);
        }
        let mark = if c.color.iter().map(|v| u32::from(*v)).sum::<u32>() > 380 { egui::Color32::BLACK } else { egui::Color32::WHITE };
        if c.locked {
            p.rect_filled(egui::Rect::from_min_size(r.right_bottom() - egui::vec2(5.0, 5.0), egui::vec2(4.0, 4.0)), 0.0, mark);
        }
        if !c.transparent && vectorcraft_render::encode::quantize::web_safe(c.color) == c.color {
            let m = r.center();
            p.add(egui::Shape::convex_polygon(
                vec![m + egui::vec2(0.0, -2.5), m + egui::vec2(2.5, 0.0), m + egui::vec2(0.0, 2.5), m + egui::vec2(-2.5, 0.0)],
                mark,
                egui::Stroke::NONE,
            ));
        }
        // The transparent entry itself isn't a colour to edit.
        let pickable = !c.transparent || *mapped;
        let picked = pickable && hex(c.source) == selected;
        p.rect_stroke(r, 0.0, egui::Stroke::new(if picked { 2.0 } else { 1.0 }, if picked { t.accent } else { t.border }), egui::StrokeKind::Outside);
        if pickable && resp.clicked() && resp.interact_pointer_pos().is_some_and(|pos| r.contains(pos)) {
            d.fields.insert("__color".into(), json!(hex(c.source)));
        }
    }
    ui.add_space(4.0);
    let table = s.map(|s| s.color_table.clone()).unwrap_or_default();
    let on = |list: &[String]| list.iter().any(|h| h.eq_ignore_ascii_case(&selected));
    ui.horizontal(|ui| {
        ui.add_enabled_ui(!selected.is_empty(), |ui| {
            for (list, on_label, off_label) in [
                ("transparent", tl!("Restore"), tl!("Transparent")),
                ("webShift", tl!("Unshift"), tl!("Web Shift")),
                ("locked", tl!("Unlock"), tl!("Lock")),
            ] {
                let current = match list {
                    "transparent" => &table.transparent,
                    "webShift" => &table.web_shift,
                    _ => &table.locked,
                };
                let is_on = on(current);
                if widgets::flat_button(ui, if is_on { on_label } else { off_label }, 88.0).clicked() {
                    toggle(d, list, &selected, !is_on);
                }
            }
        });
    });
    let sort = table.sort;
    widgets::label_row(ui, tl!("Order:"), LABEL_W, |ui| {
        let labels = ColorSort::ALL.map(ColorSort::label);
        if let Some(i) = widgets::dropdown(ui, "sfw-sort", sort.label(), &labels, FIELD_W)
            && let Some(o) = ColorSort::ALL.get(i)
        {
            set_table(d, "sort", json!(o.id()));
        }
    });
}

/// Add `color` to the colour table's `list`, or take it out.
pub(super) fn toggle(d: &mut Dialog, list: &str, color: &str, add: bool) {
    let mut v: Vec<Value> = d.fields.get("colorTable").and_then(|t| t.get(list)).and_then(Value::as_array).cloned().unwrap_or_default();
    v.retain(|c| !c.as_str().is_some_and(|c| c.eq_ignore_ascii_case(color)));
    if add {
        v.push(json!(color.to_ascii_lowercase()));
    }
    set_table(d, list, Value::Array(v));
}

/// Set one key of the `colorTable` field.
pub(super) fn set_table(d: &mut Dialog, key: &str, value: Value) {
    let table = d.fields.entry("colorTable").or_insert_with(|| json!({}));
    if !table.is_object() {
        *table = json!({});
    }
    table[key] = value;
}

/// Image Size: width and height in pixels (proportional) or a percentage, anti-aliasing, Clip to
/// Artboard and which artboard.
fn image_size(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog, s: Option<&WebSettings>) {
    widgets::subheader(ui, tl!("Image Size"));
    let Some(st) = app.session.active() else { return };
    let region = s.and_then(|s| webexport::region(&st.doc, s).ok());
    let (w, h) = region.map_or((0, 0), |(r, scale)| vectorcraft_render::region_pixels(r, scale));
    let set = |d: &mut Dialog, key: &str, v: Value| {
        for k in ["width", "height", "percent"] {
            d.fields.insert(k.into(), Value::Null);
        }
        d.fields.insert(key.into(), v);
    };
    widgets::label_row(ui, tl!("W:"), LABEL_W, |ui| {
        if let Some(v) = widgets::plain_field(ui, "sfw-w", f64::from(w), " px", 0, 80.0) {
            set(d, "width", json!(v.round().max(1.0)));
        }
        widgets::field_label(ui, tl!("H:"));
        if let Some(v) = widgets::plain_field(ui, "sfw-h", f64::from(h), " px", 0, 80.0) {
            set(d, "height", json!(v.round().max(1.0)));
        }
    });
    let percent = region.map_or(100.0, |(_, scale)| (scale * 100.0 * 100.0).round() / 100.0);
    widgets::label_row(ui, tl!("Percent:"), LABEL_W, |ui| {
        if let Some(v) = widgets::plain_field(ui, "sfw-pct", percent, "%", 2, 80.0) {
            set(d, "percent", json!(v));
        }
    });
    form::choice(ui, d, "antiAlias", tl!("Anti-aliasing:"), (LABEL_W, FIELD_W), &pairs(&AntiAlias::ALL, AntiAlias::id, AntiAlias::label));
    widgets::label_row(ui, "", LABEL_W, |ui| form::check(ui, d, "clipToArtboard", tl!("Clip to Artboard")));
    let boards = &st.doc.artboards;
    if boards.len() > 1 && d.bool("clipToArtboard") {
        let names: Vec<&str> = boards.iter().map(|a| a.name.as_str()).collect();
        let at = s.and_then(|s| s.artboard).unwrap_or(0).min(names.len() - 1);
        widgets::label_row(ui, tl!("Artboard:"), LABEL_W, |ui| {
            if let Some(i) = widgets::dropdown_names(ui, "sfw-artboard", names.get(at).copied().unwrap_or_default(), &names, FIELD_W) {
                d.fields.insert("artboard".into(), json!(i));
            }
        });
    }
}
