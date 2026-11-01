//! Window → Flattener Preview: what flattening the document would touch (`flattener.preview`),
//! highlighted in red over a grey preview of the art. Refresh takes a new snapshot of the
//! document; Highlight picks what shows; Overprints, the preset and the options (Show Options in
//! the panel menu) set how it flattens. Click the preview to zoom in, Alt-click to zoom out, drag
//! to pan and double-click to fit.
//!
//! The settings live in `UiState::flattener_preview` (`ui.inspect`); `ui.flattenerPreview` sets
//! them and refreshes.

use std::cell::RefCell;
use std::sync::Arc;

use egui::{Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_doc::Document;
use vectorcraft_engine::cmd::FlattenOptions;
use vectorcraft_engine::cmd::flatten::{FlattenReport, Highlight, Overprints};
use vectorcraft_geom::{Affine, Point, Rect as DRect};
use vectorcraft_render::RenderOptions;

use super::navigator::{doc_to_thumb, thumb_to_doc};
use crate::dialogs::flatten::{options_editor, preset_dropdown};
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// The panel id (Window → Flattener Preview).
pub const ID: &str = "flattenerPreview";

/// The panel's settings (kept with the UI state).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub highlight: Highlight,
    pub overprints: Overprints,
    /// The preset the options came from.
    pub preset: String,
    pub options: FlattenOptions,
    /// The options show under the preset (panel menu → Show Options).
    pub show_options: bool,
}

impl Default for Settings {
    fn default() -> Self {
        let preset = FlattenOptions::preset_label(FlattenOptions::PRESETS[1]).unwrap_or_default().to_string();
        Self { highlight: Highlight::None, overprints: Overprints::Preserve, preset, options: FlattenOptions::default(), show_options: false }
    }
}

impl Settings {
    /// `flattener.preview` parameters for these settings.
    fn params(&self) -> Value {
        json!({ "options": self.options, "overprints": self.overprints.id() })
    }
}

/// The document as the last Refresh saw it, and what flattening it would do.
struct Shot {
    doc: Arc<Document>,
    doc_index: usize,
    revision: u64,
    /// The settings it was taken with.
    options: FlattenOptions,
    overprints: Overprints,
    report: FlattenReport,
    /// What the preview shows at its widest: the artboards and the art.
    fit: DRect,
    /// Tells snapshots apart for the texture cache.
    generation: u64,
}

/// The preview's zoom (1 = fit) and centre.
#[derive(Clone, Copy, Default)]
struct View {
    zoom: f64,
    center: Option<Point>,
}

impl View {
    /// The part of `fit` shown.
    fn visible(&self, fit: DRect) -> DRect {
        let z = self.zoom.max(1.0);
        let c = self.center.unwrap_or(fit.center());
        DRect::from_center_size(c, (fit.width() / z, fit.height() / z))
    }

    /// Zoom by `factor` (1×–64×) keeping `at` under the pointer, panned within `fit`.
    fn zoom_at(&mut self, fit: DRect, at: Point, factor: f64) {
        let v = self.visible(fit);
        let z = (self.zoom.max(1.0) * factor).clamp(1.0, 64.0);
        let k = self.zoom.max(1.0) / z;
        let c = v.center();
        self.zoom = z;
        self.center = Some(Point::new(at.x + (c.x - at.x) * k, at.y + (c.y - at.y) * k));
        self.pan(fit, (0.0, 0.0));
    }

    /// Move the centre by `d` (document units), keeping the view inside `fit`.
    fn pan(&mut self, fit: DRect, d: (f64, f64)) {
        let v = self.visible(fit);
        let (hw, hh) = (v.width() / 2.0, v.height() / 2.0);
        let c = v.center();
        let x = (c.x + d.0).clamp(fit.x0 + hw, (fit.x1 - hw).max(fit.x0 + hw));
        let y = (c.y + d.1).clamp(fit.y0 + hh, (fit.y1 - hh).max(fit.y0 + hh));
        self.center = Some(Point::new(x, y));
    }
}

/// What the preview texture shows: (generation, highlight, simulated overprints, the visible rect's
/// bits, its pixel size).
type Key = (u64, Highlight, bool, [u64; 4], [u32; 2]);

#[derive(Default)]
struct Cache {
    shot: Option<Shot>,
    view: View,
    generation: u64,
    renderer: Option<vectorcraft_render::Renderer>,
    drawn: crate::graphics::TexCache<Drawn>,
}

/// The preview as drawn: for what, and its texture.
#[derive(Default)]
struct Drawn {
    key: Option<Key>,
    tex: Option<egui::TextureHandle>,
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
}

/// Take a new snapshot of the active document and work out what flattening it would do →
/// `flattener.preview`'s answer for the panel's highlight.
pub fn refresh(app: &mut VectorcraftApp) -> Result<Value, String> {
    let set = &app.ui.flattener_preview;
    let (o, report) = app.session.flattener_report(&set.params()).map_err(|e| e.to_string())?;
    let (options, overprints) = (set.options.clone(), set.overprints);
    let (st, doc_index) = (app.session.active().ok_or("no document open")?, app.session.active_index().unwrap_or(0));
    let mut out = report.to_json(&st.doc, set.highlight);
    out["options"] = serde_json::to_value(&o).unwrap_or_default();
    let mut fit = st.doc.artboards.iter().map(|a| a.rect).reduce(|a, b| a.union(b)).unwrap_or(DRect::new(0.0, 0.0, 612.0, 792.0));
    if let Some(b) = st.doc.art_bounds() {
        fit = fit.union(b);
    }
    let fit = fit.inflate(fit.width() * 0.03 + 4.0, fit.height() * 0.03 + 4.0);
    let (doc, revision) = (st.doc.clone(), st.revision);
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.generation += 1;
        c.shot = Some(Shot { doc, doc_index, revision, options, overprints, report, fit, generation: c.generation });
        c.view = View::default();
    });
    Ok(out)
}

/// `ui.flattenerPreview {highlight?, overprints?, preset?, options?, showOptions?}`: set the panel's
/// settings (a preset loads its options, then `options` adjust them), show the panel and refresh.
pub(crate) fn command(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let mut set = app.ui.flattener_preview.clone();
    if let Some(h) = p.get("highlight").and_then(Value::as_str) {
        set.highlight = Highlight::parse(h).ok_or_else(|| format!("unknown highlight `{h}`"))?;
    }
    if let Some(o) = p.get("overprints").and_then(Value::as_str) {
        set.overprints = Overprints::parse(o).ok_or_else(|| format!("unknown overprints `{o}` (preserve, simulate or discard)"))?;
    }
    if let Some(name) = p.get("preset").and_then(Value::as_str) {
        let preset = app.session.flattener_preset(name).ok_or_else(|| format!("unknown preset `{name}` (see flattener.presets.list)"))?;
        (set.preset, set.options) = (preset.name, preset.options);
    }
    if let Some(o) = p.get("options") {
        set.options = app.session.flatten_options(&json!({ "options": merged(&set.options, o) }))?;
    }
    if let Some(b) = p.get("showOptions").and_then(Value::as_bool) {
        set.show_options = b;
    }
    app.ui.flattener_preview = set;
    app.ui.open_panel = Some(ID.into());
    app.ui.dock = true;
    refresh(app)
}

/// `base` with the keys of `over` set.
fn merged(base: &FlattenOptions, over: &Value) -> Value {
    let mut v = serde_json::to_value(base).unwrap_or_default();
    if let (Some(m), Some(o)) = (v.as_object_mut(), over.as_object()) {
        m.extend(o.iter().map(|(k, x)| (k.clone(), x.clone())));
    }
    v
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else {
        super::empty_state(ui, "eye", tl!("No document"), tl!("Open a document to preview its flattening."));
        return;
    };
    let (index, revision) = (app.session.active_index().unwrap_or(0), st.revision);
    // A document the panel hasn't seen yet gets a snapshot right away.
    let seen = CACHE.with(|c| c.borrow().shot.as_ref().map(|s| s.doc_index));
    if seen != Some(index)
        && let Err(e) = refresh(app)
    {
        app.status(e);
    }
    let mut set = app.ui.flattener_preview.clone();
    let mut refresh_now = false;
    ui.horizontal(|ui| {
        if widgets::flat_button(ui, tl!("Refresh"), 64.0).on_hover_text(tl!("Preview the document as it is now")).clicked() {
            refresh_now = true;
        }
        let labels = Highlight::ALL.map(Highlight::label);
        if let Some(i) = widgets::dropdown(ui, "fp-highlight", set.highlight.label(), &labels, ui.available_width().max(120.0)) {
            set.highlight = Highlight::ALL[i];
        }
    });
    ui.add_space(4.0);
    egui::Grid::new("fp-settings").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
        widgets::dim_label(ui, tl!("Overprints:"));
        let labels = Overprints::ALL.map(Overprints::label);
        if let Some(i) = widgets::dropdown(ui, "fp-overprints", set.overprints.label(), &labels, 150.0) {
            set.overprints = Overprints::ALL[i];
        }
        ui.end_row();
        widgets::dim_label(ui, tl!("Preset:"));
        let presets = app.session.flattener_presets();
        if let Some(p) = preset_dropdown(ui, "fp-preset", &presets, &set.preset, Some(&set.options), 150.0) {
            set.preset = p.name.clone();
            set.options = p.options.clone();
        }
        ui.end_row();
    });
    if set.show_options {
        widgets::divider(ui);
        options_editor(ui, "flattener-preview", &mut set.options, true);
    }
    ui.add_space(6.0);
    preview_area(ui, &t, set.highlight, set.overprints);
    // What is highlighted, and whether the snapshot is behind the document or the settings.
    let (count, stale) = CACHE.with(|c| {
        let c = c.borrow();
        let s = c.shot.as_ref();
        let count = s.map_or(0, |s| s.report.objects(set.highlight).len() + s.report.areas(set.highlight).len());
        (count, s.is_some_and(|s| s.revision != revision || s.options != set.options || s.overprints != set.overprints))
    });
    if set.highlight != Highlight::None {
        let areas = matches!(set.highlight, Highlight::RasterizedRegions | Highlight::AllRasterized);
        let n = count as u64;
        let msg = if areas {
            crate::i18n::tn(n, "{n} area highlighted", "{n} areas highlighted")
        } else {
            crate::i18n::tn(n, "{n} object highlighted", "{n} objects highlighted")
        };
        widgets::dim_label(ui, &msg);
    }
    if stale {
        ui.label(egui::RichText::new(tl!("Changed since the last Refresh.")).color(t.text_dim).size(11.5));
    }
    if set != app.ui.flattener_preview {
        app.ui.flattener_preview = set;
    }
    if refresh_now && let Err(e) = refresh(app) {
        app.status(e);
    }
}

/// The preview: the snapshot rendered over the visible part, grey under a highlight with the
/// highlighted art in `t.highlight`; click, Alt-click, drag and double-click zoom and pan.
fn preview_area(ui: &mut Ui, t: &Tokens, h: Highlight, overprints: Overprints) {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, (w * 0.8).clamp(140.0, 320.0)), Sense::click_and_drag());
    let resp = resp.on_hover_text(tl!("Click to zoom in, Alt-click to zoom out, drag to pan, double-click to fit"));
    ui.painter().rect_filled(rect, 0.0, t.pasteboard);
    let ppp = ui.ctx().pixels_per_point();
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let Cache { shot, view, renderer, drawn, .. } = &mut *c;
        let Some(shot) = shot.as_ref() else { return };
        let mut drawn = drawn.borrow_mut();
        let Drawn { key, tex } = &mut *drawn;
        let visible = view.visible(shot.fit);
        if resp.double_clicked() {
            *view = View::default();
        } else if resp.clicked()
            && let Some(p) = resp.interact_pointer_pos()
        {
            let alt = ui.input(|i| i.modifiers.alt);
            view.zoom_at(shot.fit, thumb_to_doc(visible, rect, p), if alt { 0.5 } else { 2.0 });
        } else if resp.dragged() {
            let per_px = (visible.width() / rect.width() as f64).max(visible.height() / rect.height() as f64);
            let d = resp.drag_delta();
            view.pan(shot.fit, (-d.x as f64 * per_px, -d.y as f64 * per_px));
        }
        let visible = view.visible(shot.fit);
        let px = [(rect.width() * ppp) as u32, (rect.height() * ppp) as u32];
        let k = (shot.generation, h, overprints == Overprints::Simulate, [visible.x0, visible.y0, visible.x1, visible.y1].map(f64::to_bits), px);
        if *key != Some(k) || tex.is_none() {
            let r = renderer.get_or_insert_with(vectorcraft_render::Renderer::new);
            let img = render(r, shot, visible, px, h, overprints, t.highlight);
            match tex {
                Some(tx) => tx.set(img, egui::TextureOptions::LINEAR),
                None => *tex = Some(ui.ctx().load_texture("flattener-preview", img, egui::TextureOptions::LINEAR)),
            }
            *key = Some(k);
        }
        if let Some(tx) = tex {
            let r =
                Rect::from_min_max(doc_to_thumb(visible, rect, visible.origin()), doc_to_thumb(visible, rect, Point::new(visible.x1, visible.y1)));
            ui.painter().with_clip_rect(rect).image(tx.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
    });
    ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, t.border), StrokeKind::Outside);
}

/// `visible` of the snapshot at most `px` pixels: in colour without a highlight, else in grey with
/// the highlighted art in `color`.
fn render(
    r: &mut vectorcraft_render::Renderer,
    shot: &Shot,
    visible: DRect,
    px: [u32; 2],
    h: Highlight,
    overprints: Overprints,
    color: egui::Color32,
) -> egui::ColorImage {
    let s = (px[0] as f64 / visible.width()).min(px[1] as f64 / visible.height());
    let (w, hh) = ((visible.width() * s).round().max(1.0) as u32, (visible.height() * s).round().max(1.0) as u32);
    let view = Affine::scale(s) * Affine::translate((-visible.x0, -visible.y0));
    let opts = RenderOptions {
        background: Some([255; 4]),
        skip_templates: true,
        overprint_preview: overprints == Overprints::Simulate,
        ..Default::default()
    };
    let mut img = r.render(&shot.doc, w, hh, view, &opts);
    if h != Highlight::None {
        let mask = shot
            .report
            .highlight_art(&shot.doc, h)
            .map(|art| r.render(&art, w, hh, view, &RenderOptions { skip_templates: true, ..Default::default() }));
        let red = [color.r(), color.g(), color.b()].map(f32::from);
        for (i, p) in img.pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            // A light grey of the art, so the highlight stands out.
            let luma = 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32;
            let grey = 96.0 + luma * 0.62;
            let a = mask.as_ref().map_or(0.0, |m| m.pixels[i * 4 + 3] as f32 / 255.0);
            for c in 0..3 {
                p[c] = (grey + (red[c] - grey) * a).round() as u8;
            }
        }
    }
    crate::canvas::color_image(&img)
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    if widgets::menu_item(ui, tl!("Refresh"), app.session.active().is_some(), false)
        && let Err(e) = refresh(app)
    {
        app.status(e);
    }
    let shown = app.ui.flattener_preview.show_options;
    if widgets::menu_item(ui, tl!("Show Options"), true, shown) {
        app.ui.flattener_preview.show_options = !shown;
    }
    if widgets::menu_item(ui, tl!("Fit in Window"), true, false) {
        CACHE.with(|c| c.borrow_mut().view = View::default());
    }
    if widgets::menu_item(ui, tl!("Save Transparency Flattener Preset…"), true, false) {
        // Saved under a new name, then shown in the presets manager to name it.
        match app.run("flattener.presets.save", json!({ "options": app.ui.flattener_preview.options })) {
            Ok(r) => {
                let name = r["name"].as_str().unwrap_or_default().to_string();
                app.ui.flattener_preview.preset = name.clone();
                let _ = app.run("ui.flattenerPresetsDialog", json!({ "selected": name }));
            }
            Err(e) => app.status(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    /// A half-transparent blue rectangle over type, and a green square apart.
    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        app.run("text.create", json!({"x": 20, "y": 80, "text": "Under", "size": 48})).unwrap();
        let top = app.run("shape.rectangle", json!({"x": 10, "y": 30, "width": 200, "height": 80})).unwrap()["id"].clone();
        app.run("paint.setFill", json!({"color": "#3366cc", "ids": [top]})).unwrap();
        app.run("transparency.set", json!({"ids": [top], "opacity": 50})).unwrap();
        app.run("shape.rectangle", json!({"x": 240, "y": 140, "width": 40, "height": 40})).unwrap();
        app
    }

    /// Two headless frames of the panel (the texture uploads in the first).
    fn frame(app: &mut VectorcraftApp) -> egui::Context {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui));
            out.textures_delta.clear();
        }
        ctx
    }

    fn pixels() -> egui::ColorImage {
        CACHE.with(|c| {
            let mut c = c.borrow_mut();
            let Cache { shot, view, renderer, .. } = &mut *c;
            let shot = shot.as_ref().unwrap();
            render(
                renderer.get_or_insert_with(vectorcraft_render::Renderer::new),
                shot,
                view.visible(shot.fit),
                [300, 200],
                Highlight::None,
                Overprints::Preserve,
                egui::Color32::RED,
            )
        })
    }

    #[test]
    fn the_panel_takes_a_snapshot_and_highlights_in_red() {
        let mut app = app();
        frame(&mut app);
        assert!(CACHE.with(|c| c.borrow().shot.is_some()), "a first look refreshes");
        assert!(CACHE.with(|c| c.borrow().drawn.borrow().tex.is_some()), "the preview is drawn");
        let v = command(&mut app, &json!({"highlight": "allAffected"})).unwrap();
        assert_eq!(v["counts"]["allAffected"], 2);
        assert_eq!(app.ui.open_panel.as_deref(), Some(ID));
        let text = crate::tests_labels::painted_text(&mut app, show);
        assert!(text.contains("2 objects highlighted") && text.contains("All Affected Objects"), "{text}");
        // The highlight is red where the art is affected and grey elsewhere.
        let (shot_fit, img) = CACHE.with(|c| {
            let mut c = c.borrow_mut();
            let Cache { shot, renderer, .. } = &mut *c;
            let shot = shot.as_ref().unwrap();
            let fit = DRect::new(0.0, 0.0, 300.0, 200.0);
            (
                shot.fit,
                render(
                    renderer.get_or_insert_with(vectorcraft_render::Renderer::new),
                    shot,
                    fit,
                    [300, 200],
                    Highlight::AllAffected,
                    Overprints::Preserve,
                    egui::Color32::RED,
                ),
            )
        });
        assert!(shot_fit.contains(Point::new(300.0, 200.0)));
        let at = |x: usize, y: usize| img.pixels[y * img.width() + x];
        assert_eq!(at(100, 60), egui::Color32::RED, "inside the transparent rectangle");
        let apart = at(260, 160);
        assert!(apart.r() == apart.g() && apart.g() == apart.b(), "the lone square is grey: {apart:?}");
        // Without a highlight it is in colour.
        let plain = pixels();
        assert!(plain.pixels.iter().any(|p| i32::from(p.b()) > i32::from(p.r()) + 40), "the blue shows");
    }

    #[test]
    fn edits_wait_for_refresh_and_settings_drive_the_report() {
        let mut app = app();
        frame(&mut app);
        let text = crate::tests_labels::painted_text(&mut app, show);
        assert!(!text.contains("Changed since"), "{text}");
        app.run("select.all", json!({})).unwrap();
        app.run("transparency.set", json!({"opacity": 100})).unwrap();
        let text = crate::tests_labels::painted_text(&mut app, show);
        assert!(text.contains("Changed since the last Refresh."), "{text}");
        let v = command(&mut app, &json!({"highlight": "transparentObjects"})).unwrap();
        assert_eq!(v["counts"]["transparentObjects"], 0, "an opaque document");
        // A preset and options: balance 0 rasterizes; the dropdown then reads [Custom].
        let v = command(&mut app, &json!({"preset": "low", "options": {"balance": 0}, "showOptions": true})).unwrap();
        assert_eq!((v["options"]["balance"].as_f64(), v["options"]["gradientPpi"].as_f64()), (Some(0.0), Some(150.0)));
        assert!(app.ui.flattener_preview.show_options);
        let text = crate::tests_labels::painted_text(&mut app, show);
        assert!(text.contains("[Custom]") && text.contains("Raster/Vector Balance:"), "{text}");
        assert!(command(&mut app, &json!({"highlight": "nope"})).is_err());
        // The settings persist with the UI state.
        let back: crate::UiState = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
        assert_eq!(back.flattener_preview, app.ui.flattener_preview);
    }

    #[test]
    fn zoom_and_pan_stay_inside_the_document() {
        let fit = DRect::new(0.0, 0.0, 200.0, 100.0);
        let mut v = View::default();
        assert_eq!(v.visible(fit), fit);
        v.zoom_at(fit, Point::new(50.0, 25.0), 2.0);
        let r = v.visible(fit);
        assert!((r.width() - 100.0).abs() < 1e-9 && r.x0 >= 0.0 && r.y0 >= 0.0, "{r:?}");
        v.pan(fit, (-1000.0, 0.0));
        assert_eq!(v.visible(fit).x0, 0.0);
        v.zoom_at(fit, Point::new(0.0, 0.0), 0.25);
        assert_eq!(v.visible(fit), fit, "never smaller than fit");
    }
}
