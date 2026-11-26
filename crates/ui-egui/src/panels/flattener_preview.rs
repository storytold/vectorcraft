//! Window → Flattener Preview: what flattening the document would touch (`flattener.preview`),
//! highlighted in red over a grey preview of the art. Refresh takes a new snapshot of the
//! document; Highlight picks what shows; Overprints, the preset and the options (Show Options in
//! the panel menu) set how it flattens. The preview is a `PreviewPane`: a click zooms in about the
//! point clicked and an Alt-click (Option on macOS) zooms out, a drag pans, Ctrl or Cmd with the
//! wheel zooms, and a double click fits. The zoom row under it zooms from the keyboard.
//!
//! The settings live in `UiState::flattener_preview` (`ui.inspect`); `ui.flattenerPreview` sets
//! them and refreshes. The snapshot, the view and the texture live in [`FlattenerPreviewModel`]
//! on the app.

use std::sync::Arc;

use craft::behavior::CanvasTransform;
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::{Divider, Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::menu::PanelRow;
use craft::ui::patterns::form_dialogs::{DialogFields, ParamRows, PreviewPane, PreviewView, PreviewZoom};
use craft::ui::pickers::Select;
use craft::ui::runtime::commands::{CommandRef, CommandState, CommandStates};
use craft::ui::runtime::env::Env;
use craft::ui::style::{TypeRole, rgba_color};
use egui::load::SizedTexture;
use egui::{Ui, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_doc::Document;
use vectorcraft_engine::cmd::FlattenOptions;
use vectorcraft_engine::cmd::flatten::{FlattenReport, Highlight, Overprints};
use vectorcraft_geom::{Affine, Rect as DRect};
use vectorcraft_render::RenderOptions;

use crate::VectorcraftApp;
use crate::dialogs::{flatten, form};

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

/// The pane's well width over its height.
const ASPECT: f32 = 1.25;

/// The zoom is relative to the fitted picture: 1 is the fit, up to 8×.
fn fitting_view() -> PreviewView {
    PreviewView::fitting().zoom_range(1.0..=8.0)
}

/// The panel's state outside the settings: the last snapshot, its view and its texture.
pub(crate) struct FlattenerPreviewModel {
    shot: Option<Shot>,
    /// Counts snapshots, so the texture cache tells them apart.
    generation: u64,
    renderer: Option<vectorcraft_render::Renderer>,
    view: PreviewView,
    drawn: crate::graphics::TexCache<Drawn>,
}

impl Default for FlattenerPreviewModel {
    fn default() -> Self {
        Self { shot: None, generation: 0, renderer: None, view: fitting_view(), drawn: Default::default() }
    }
}

/// What the preview texture shows: (generation, highlight, simulated overprints, its pixel size). The
/// whole `fit` rect is rendered, so the view is not part of the key.
type Key = (u64, Highlight, bool, u32);

/// The preview as drawn: for what, and its texture.
#[derive(Default)]
struct Drawn {
    key: Option<Key>,
    tex: Option<egui::TextureHandle>,
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
    let m = &mut app.flattener_preview;
    m.generation += 1;
    m.shot = Some(Shot { doc, doc_index, revision, options, overprints, report, fit, generation: m.generation });
    m.view = fitting_view();
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
    crate::panel_docking::show_panels(app);
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

/// Whether the snapshot is behind the document or the settings.
fn stale(app: &VectorcraftApp) -> bool {
    let (Some(st), Some(s)) = (app.session.active(), app.flattener_preview.shot.as_ref()) else { return false };
    let set = &app.ui.flattener_preview;
    s.revision != st.revision || s.options != set.options || s.overprints != set.overprints
}

/// Whether the options are no preset's: the preset field then reads `[Custom]`.
fn custom(app: &VectorcraftApp) -> bool {
    let set = &app.ui.flattener_preview;
    !app.session.flattener_presets().iter().any(|p| p.name == set.preset && p.options == set.options)
}

/// The panel body.
pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    if app.session.active().is_none() {
        super::empty_state(ui, "eye", tl!("No document"), tl!("Open a document to preview its flattening."));
        return;
    }
    let index = app.session.active_index().unwrap_or(0);
    // A document the panel hasn't seen yet gets a snapshot right away.
    if app.flattener_preview.shot.as_ref().map(|s| s.doc_index) != Some(index)
        && let Err(e) = refresh(app)
    {
        app.status(e);
    }
    // The highlighted art takes the theme's error colour.
    let highlight = rgba_color(Env::of(ui).tokens.tokens().color.status.bad.solid);
    let mut set = app.ui.flattener_preview.clone();
    let mut refresh_now = false;
    Row::new().show(ui, |ui, _| {
        refresh_now = Button::new(tl!("Refresh")).tooltip(tl!("Preview the document as it is now")).show(ui).clicked();
        let entries = Highlight::ALL.map(|h| Entry::new(h, h.label()));
        Select::new(tl!("Highlight"), Bind::bound(&mut set.highlight), &entries).show(ui);
    });
    Row::new().show(ui, |ui, _| {
        Text::new(tl!("Overprints:")).shade(Shade::Muted).show(ui);
        let entries = Overprints::ALL.map(|o| Entry::new(o, o.label()));
        Select::new(tl!("Overprints"), Bind::bound(&mut set.overprints), &entries).show(ui);
    });
    Row::new().show(ui, |ui, _| {
        Text::new(tl!("Preset:")).shade(Shade::Muted).show(ui);
        let names = flatten::preset_names(app);
        let mut fields = DialogFields::from_pairs([("preset", set.preset.clone())]);
        flatten::preset_picker(ui, &mut fields, "preset", &names, custom(app));
        if let Some(name) = fields.text("preset")
            && name != set.preset
            && let Some(p) = app.session.flattener_preset(name)
        {
            (set.preset, set.options) = (p.name, p.options);
        }
    });
    if set.show_options {
        Divider::horizontal().show(ui);
        let params = flatten::option_params(&[], "");
        let mut fields = flatten::option_fields(&set.options);
        let out = ParamRows::new(tl!("Flattener options"), &params, Bind::bound(&mut fields)).show(ui);
        if out.changed
            && let Ok(o) = serde_json::from_value(Value::Object(form::json_of(&params, &fields)))
        {
            set.options = o;
        }
    }
    preview_area(app, ui, highlight, set.highlight, set.overprints);
    if set.highlight != Highlight::None {
        let count = app.flattener_preview.shot.as_ref().map_or(0, |s| s.report.objects(set.highlight).len() + s.report.areas(set.highlight).len());
        let areas = matches!(set.highlight, Highlight::RasterizedRegions | Highlight::AllRasterized);
        let n = count as u64;
        let msg = if areas {
            crate::i18n::tn(n, "{n} area highlighted", "{n} areas highlighted")
        } else {
            crate::i18n::tn(n, "{n} object highlighted", "{n} objects highlighted")
        };
        Text::new(msg).shade(Shade::Muted).show(ui);
    }
    if stale(app) {
        Text::new(tl!("Changed since the last Refresh.")).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
    }
    if set != app.ui.flattener_preview {
        app.ui.flattener_preview = set;
    }
    if refresh_now && let Err(e) = refresh(app) {
        app.status(e);
    }
}

/// The preview: the whole snapshot rendered once per [`Key`], grey under a highlight with the
/// highlighted art in the theme's error colour, in a pane that zooms and pans it.
/// A click zooms in 2× about the point clicked and an Alt-click zooms out 2×, within [`fitting_view`]'s range.
fn preview_area(app: &mut VectorcraftApp, ui: &mut Ui, highlight: egui::Color32, h: Highlight, overprints: Overprints) {
    let FlattenerPreviewModel { shot, renderer, view, drawn, .. } = &mut app.flattener_preview;
    let Some(shot) = shot.as_ref() else { return };
    let w = ui.available_width();
    // A dock resize renders again only when the width crosses a power of two; the picture stays within what the
    // window's textures can hold.
    let most = ui.ctx().input(|i| i.max_texture_side).min(4096) as u32;
    let px = ((w * ui.ctx().pixels_per_point() * 4.0).ceil().max(1.0) as u32).next_power_of_two().min(most);
    let mut drawn = drawn.borrow_mut();
    let Drawn { key, tex } = &mut *drawn;
    let k = (shot.generation, h, overprints == Overprints::Simulate, px);
    if *key != Some(k) || tex.is_none() {
        let r = renderer.get_or_insert_with(vectorcraft_render::Renderer::new);
        let img = render(r, shot, shot.fit, [px, px], h, overprints, highlight);
        match tex {
            Some(tx) => tx.set(img, egui::TextureOptions::LINEAR),
            None => *tex = Some(ui.ctx().load_texture("flattener-preview", img, egui::TextureOptions::LINEAR)),
        }
        *key = Some(k);
    }
    let Some(tx) = tex else { return };
    // At scale 1 the picture is the fit of the well (width `w`, height `w / ASPECT`), so the zoom reads relative to the fit.
    let s = (f64::from(w) / shot.fit.width()).min(f64::from(w / ASPECT) / shot.fit.height());
    let size = vec2((shot.fit.width() * s) as f32, (shot.fit.height() * s) as f32);
    let out = PreviewPane::new(tl!("Flattener Preview"), SizedTexture::new(tx.id(), size), view)
        .aspect(ASPECT)
        .click_centres(false)
        .tooltip(tl!("Click to zoom in, Alt-click to zoom out, drag to pan, double-click to fit"))
        .show(ui);
    *view = match out.clicked {
        Some(at) => {
            let factor = if craft::ui::runtime::pointer::frame(ui.ctx()).pick_mods.alt { 0.5 } else { 2.0 };
            zoomed_about(&out.shown, at, size, out.response.rect.size(), factor)
        }
        // The view as drawn, so the zoom row steps from what is on screen while the pane fits.
        None => out.view.unwrap_or(out.shown),
    };
    // The zoom row zooms from the keyboard too.
    if let Some(v) = PreviewZoom::new(view).show(ui).view {
        *view = v;
    }
}

/// `shown` zoomed by `factor` within its zoom range, keeping the point `at` of a picture of `picture` points (from its
/// top-left corner) under the pointer, settled in a well of `well` points. A zoom out to the fit or past it returns
/// to the fitting view.
fn zoomed_about(shown: &PreviewView, at: egui::Pos2, picture: egui::Vec2, well: egui::Vec2, factor: f32) -> PreviewView {
    let t = shown.transform;
    let fit = PreviewPane::fit_view(picture, well, &shown.range);
    let scale = (t.scale * factor).min(*shown.range.end());
    let mut next = shown.clone();
    if scale <= fit.scale {
        next.transform = PreviewPane::settled(fit, picture, well);
        next.fit = true;
        return next;
    }
    // Where the point lies from the well's center; the new offset keeps it there at the new zoom.
    let about = (at.to_vec2() - picture / 2.0) * t.scale + t.offset;
    let offset = about - (about - t.offset) * (scale / t.scale);
    next.transform = PreviewPane::settled(CanvasTransform::new(scale, offset), picture, well);
    next.fit = false;
    next
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

const REFRESH: &str = "ui.flattenerPreview.refresh";
const SHOW_OPTIONS: &str = "ui.flattenerPreview.showOptions";
const FIT_IN_WINDOW: &str = "ui.flattenerPreview.fitInWindow";
const SAVE_PRESET: &str = "ui.flattenerPreview.savePreset";

/// The rows of the panel menu.
pub(crate) fn menu_rows(_app: &VectorcraftApp) -> Vec<PanelRow> {
    // PLAN: owner hub-w2 registers these four commands in `menus/ui_commands.rs`.
    [REFRESH, SHOW_OPTIONS, FIT_IN_WINDOW, SAVE_PRESET].into_iter().map(|id| PanelRow::Command(CommandRef::new(id))).collect()
}

/// The states of the menu's rows: Refresh needs a document, Show Options is checked while the options show.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    states.set(CommandRef::new(REFRESH), CommandState::new().enabled(app.session.active().is_some()));
    states.set(CommandRef::new(SHOW_OPTIONS), CommandState::new().checked(app.ui.flattener_preview.show_options));
}

/// Runs a menu row; `None` for an id that is not this panel's.
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    let r = match id {
        REFRESH => refresh(app),
        SHOW_OPTIONS => {
            let set = &mut app.ui.flattener_preview;
            set.show_options = !set.show_options;
            Ok(Value::Null)
        }
        FIT_IN_WINDOW => {
            app.flattener_preview.view = fitting_view();
            Ok(Value::Null)
        }
        SAVE_PRESET => {
            // Saved under a new name, then shown in the presets manager to name it.
            app.run("flattener.presets.save", json!({ "options": app.ui.flattener_preview.options })).map(|r| {
                let name = r["name"].as_str().unwrap_or_default().to_string();
                app.ui.flattener_preview.preset = name.clone();
                let _ = app.run("ui.flattenerPresetsDialog", json!({ "selected": name }));
                Value::Null
            })
        }
        _ => return None,
    };
    Some(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;
    use vectorcraft_geom::Point;

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
        let ctx = crate::testing::ui_context();
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui));
            out.textures_delta.clear();
        }
        ctx
    }

    #[test]
    fn the_panel_takes_a_snapshot_and_highlights_in_red() {
        let mut app = app();
        frame(&mut app);
        assert!(app.flattener_preview.shot.is_some(), "a first look refreshes");
        assert!(app.flattener_preview.drawn.borrow().tex.is_some(), "the preview is drawn");
        let v = command(&mut app, &json!({"highlight": "allAffected"})).unwrap();
        assert_eq!(v["counts"]["allAffected"], 2);
        assert_eq!(app.ui.open_panel.as_deref(), Some(ID));
        let FlattenerPreviewModel { shot, renderer, .. } = &mut app.flattener_preview;
        let shot = shot.as_ref().unwrap();
        let r = renderer.get_or_insert_with(vectorcraft_render::Renderer::new);
        let fit = DRect::new(0.0, 0.0, 300.0, 200.0);
        assert!(shot.fit.contains(Point::new(300.0, 200.0)));
        // The highlight is red where the art is affected and grey elsewhere.
        let img = render(r, shot, fit, [300, 200], Highlight::AllAffected, Overprints::Preserve, egui::Color32::RED);
        let at = |x: usize, y: usize| img.pixels[y * img.width() + x];
        assert_eq!(at(100, 60), egui::Color32::RED, "inside the transparent rectangle");
        let apart = at(260, 160);
        assert!(apart.r() == apart.g() && apart.g() == apart.b(), "the lone square is grey: {apart:?}");
        // Without a highlight it is in colour.
        let plain = render(r, shot, fit, [300, 200], Highlight::None, Overprints::Preserve, egui::Color32::RED);
        assert!(plain.pixels.iter().any(|p| i32::from(p.b()) > i32::from(p.r()) + 40), "the blue shows");
    }

    #[test]
    fn edits_wait_for_refresh_and_settings_drive_the_report() {
        let mut app = app();
        frame(&mut app);
        assert!(!stale(&app));
        app.run("select.all", json!({})).unwrap();
        app.run("transparency.set", json!({"opacity": 100})).unwrap();
        assert!(stale(&app), "an edit waits for Refresh");
        let v = command(&mut app, &json!({"highlight": "transparentObjects"})).unwrap();
        assert_eq!(v["counts"]["transparentObjects"], 0, "an opaque document");
        assert!(!stale(&app), "command refreshes");
        // A preset and options: balance 0 rasterizes; the preset then reads [Custom].
        let v = command(&mut app, &json!({"preset": "low", "options": {"balance": 0}, "showOptions": true})).unwrap();
        assert_eq!((v["options"]["balance"].as_f64(), v["options"]["gradientPpi"].as_f64()), (Some(0.0), Some(150.0)));
        assert!(app.ui.flattener_preview.show_options);
        assert!(custom(&app));
        assert!(command(&mut app, &json!({"highlight": "nope"})).is_err());
        // The settings persist with the UI state.
        let back: crate::UiState = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
        assert_eq!(back.flattener_preview, app.ui.flattener_preview);
    }

    /// The panel in a fixture, and the preview pane's bounds.
    fn shown_pane() -> (std::rc::Rc<std::cell::RefCell<VectorcraftApp>>, craft::ui::testing::Fixture, egui::Rect) {
        let app = std::rc::Rc::new(std::cell::RefCell::new(app()));
        let drawn = std::rc::Rc::clone(&app);
        let mut fixture = craft::ui::testing::Fixture::new(move |ui| show(&mut drawn.borrow_mut(), ui));
        fixture.settle();
        let pane = fixture.by_label("Flattener Preview").unwrap_or_else(|| panic!("no preview pane"));
        (app, fixture, pane.rect)
    }

    /// One click at `at` with `modifiers` held, far enough from the last that the two make no double click.
    fn click(fixture: &mut craft::ui::testing::Fixture, at: egui::Pos2, modifiers: egui::Modifiers) {
        fixture.advance(2.0).hold(modifiers).click_at(at).hold(egui::Modifiers::NONE).idle(1);
    }

    fn view(app: &std::rc::Rc<std::cell::RefCell<VectorcraftApp>>) -> PreviewView {
        app.borrow().flattener_preview.view.clone()
    }

    #[test]
    fn a_click_zooms_in_about_the_point_and_an_alt_click_zooms_out() {
        let (app, mut fixture, well) = shown_pane();
        assert!(view(&app).fit);
        let at = well.center() + egui::vec2(well.width() * 0.1, well.height() * 0.1);
        click(&mut fixture, at, egui::Modifiers::NONE);
        let zoomed = view(&app);
        assert!(!zoomed.fit, "a click leaves fit");
        // 1 is about the fit: the picture is sized to the well.
        assert!(zoomed.transform.scale > 1.5, "a click zooms in: {zoomed:?}");
        // The point clicked stays under the pointer: off center, the picture moved away from the well's center.
        assert!(zoomed.transform.offset.x < 0.0 && zoomed.transform.offset.y < 0.0, "{zoomed:?}");
        click(&mut fixture, at, egui::Modifiers::NONE);
        let closer = view(&app);
        assert!(closer.transform.scale > zoomed.transform.scale * 1.9, "a second click zooms in further");
        click(&mut fixture, at, egui::Modifiers::ALT);
        let out = view(&app);
        assert!((out.transform.scale - zoomed.transform.scale).abs() < 1e-3, "an Alt-click zooms back out: {out:?}");
        click(&mut fixture, at, egui::Modifiers::ALT);
        assert!(view(&app).fit, "an Alt-click to the fit returns to fit");
    }

    #[test]
    fn a_click_with_alt_latched_by_the_app_zooms_out() {
        let mut app = app();
        let mut frames = craft::ui::testing::Frames::new();
        let mut pane = None;
        for _ in 0..60 {
            frames.frame(Vec::new(), |ui| show(&mut app, ui));
            let tree = frames.tree();
            pane = tree
                .keys()
                .into_iter()
                .filter(|key| tree.name_of(*key) == Some("Flattener Preview"))
                .filter_map(|key| tree.bounds_of(key))
                .max_by(|a, b| a.area().total_cmp(&b.area()));
            if pane.is_some() {
                break;
            }
        }
        let well = pane.unwrap_or_else(|| panic!("no preview pane"));
        let at = well.center();
        let primary =
            |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        let click = |frames: &mut craft::ui::testing::Frames, app: &mut VectorcraftApp| {
            frames.advance(2.0);
            frames.frame(vec![egui::Event::PointerMoved(at), primary(true)], |ui| show(app, ui));
            frames.frame(vec![primary(false)], |ui| show(app, ui));
            frames.idle(1, |ui| show(app, ui));
        };
        click(&mut frames, &mut app);
        assert!(!app.flattener_preview.view.fit, "a click zooms in");
        // Alt held for the user by the app (the Modifier Keys panel), with no Alt key down.
        frames.latched(craft::keymap::Mods { alt: true, ..Default::default() });
        click(&mut frames, &mut app);
        assert!(app.flattener_preview.view.fit, "a click with Alt latched zooms back out to fit");
    }

    #[test]
    fn click_zoom_stays_inside_the_zoom_range() {
        let (app, mut fixture, well) = shown_pane();
        // Alt-click at fit stays at fit.
        click(&mut fixture, well.center(), egui::Modifiers::ALT);
        assert!(view(&app).fit, "no zoom below the fit");
        for _ in 0..6 {
            click(&mut fixture, well.center(), egui::Modifiers::NONE);
        }
        let most = view(&app);
        assert!(most.transform.scale <= *most.range.end(), "no zoom past the range: {most:?}");
        assert!(!most.can_zoom_in(), "clicks reach the most zoom: {most:?}");
    }
}
