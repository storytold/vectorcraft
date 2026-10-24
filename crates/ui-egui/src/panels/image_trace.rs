//! Image Trace panel: preset, view, mode, palette (Color), threshold or colour count, and the
//! Advanced section (open or closed as it was last left): fidelity, method, and Create: Fills /
//! Strokes with the widest line stroked. With an Image Trace object selected and Preview on (the
//! default, kept with the UI preferences), changing a setting re-traces it (one undo step per
//! change; sliders apply when released); with Preview off, the changes wait in the panel until
//! Trace applies them, or until Preview is turned on again. Changing the view redraws the object
//! without tracing again. With an image selected, Trace makes a new Image Trace object.
//! The public documentation of the panel describes Preview as showing the result of the current
//! settings (<https://web.archive.org/web/20250831090623/https://helpx.adobe.com/illustrator/using/image-trace.html>).

use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_doc::{Node, NodeId, NodeKind, TraceView};
use vectorcraft_engine::cmd::swatchlib::DOCUMENT_SWATCHES;

use super::{first_selected, library_panel, pstate, set_pstate, swatches};
use crate::VectorcraftApp;
use crate::widgets::{self, menu_item};

const MODES: [(&str, &str); 4] = [("blackAndWhite", "Black and White"), ("grayscale", "Grayscale"), ("color", "Color"), ("logo", "Flat Logo")];

/// Color mode's palettes (`palette` param ids), in panel order.
const PALETTES: [&str; 4] = ["limited", "fullTone", "automatic", "documentLibrary"];

/// Palette `id`, in the UI language.
/// The colour Ignore Color leaves out in `params` (`ignoreColor`, `#rrggbb`): white when there is none.
fn ignore_color(params: &Value) -> [u8; 3] {
    let hex = params["ignoreColor"].as_str().unwrap_or("").trim_start_matches('#');
    let channel = |i: usize| hex.get(i * 2..i * 2 + 2).and_then(|h| u8::from_str_radix(h, 16).ok());
    match (hex.len(), channel(0), channel(1), channel(2)) {
        (6, Some(r), Some(g), Some(b)) => [r, g, b],
        _ => [255; 3],
    }
}

fn palette_label(id: &str) -> &'static str {
    match id {
        "fullTone" => tl!("Full Tone"),
        "automatic" => tl!("Automatic"),
        "documentLibrary" => tl!("Document Library"),
        _ => tl!("Limited"),
    }
}

/// The name of Document Library's library `key` (the document's swatches or a swatch library); a
/// library that is gone shows its key.
fn library_label(app: &VectorcraftApp, key: &str) -> String {
    match swatches::limit_name(app, key) {
        Some(_) if key == DOCUMENT_SWATCHES => tl!(swatches::DOCUMENT_SWATCHES).to_string(),
        Some(name) => library_panel::library_name(key, &name).to_string(),
        None => key.to_string(),
    }
}

/// Document Library's choices: the document's swatches, then the colour libraries (`current`
/// checked) → the key chosen.
fn library_list(app: &VectorcraftApp, ui: &mut Ui, current: &str) -> Option<String> {
    let document = menu_item(ui, swatches::DOCUMENT_SWATCHES, true, current == DOCUMENT_SWATCHES).then(|| DOCUMENT_SWATCHES.to_string());
    ui.separator();
    library_panel::library_items(ui, &swatches::colour_libraries(app), &|id| id == current).or(document)
}

/// Panel state: the preset name, the current parameters, the view and the last result's counts.
#[derive(Clone, Default)]
struct TraceUi {
    preset: String,
    params: Value,
    view: TraceView,
    info: Option<(u64, u64, u64)>,
    /// The selected Image Trace object's stored settings last adopted (so edits in progress
    /// aren't overwritten until the object changes).
    synced: Value,
    /// The document and object `synced` came from.
    object: Option<(u64, NodeId)>,
    /// Preview was off at the panel's last frame.
    preview_off: bool,
}

/// Each preset: its name, its parameters, and those of its enhanced version (null if it has none).
fn presets(app: &mut VectorcraftApp) -> Vec<(String, Value, Value)> {
    let v = app.session.execute("imageTrace.presets", &json!({})).unwrap_or_default();
    v["presets"]
        .as_array()
        .map(|a| a.iter().map(|p| (p["name"].as_str().unwrap_or("").to_string(), p["params"].clone(), p["enhanced"].clone())).collect())
        .unwrap_or_default()
}

/// Whether `n` is an Image Trace object: a group so named whose first child is the traced image.
pub(crate) fn is_trace(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Group { clip: false, .. })
        && n.name.as_deref() == Some("Image Trace")
        && n.children().is_some_and(|c| c.first().is_some_and(|i| matches!(i.kind, NodeKind::Image(_))))
}

/// The active document and the first selected object.
fn target_key(app: &VectorcraftApp) -> Option<(u64, NodeId)> {
    Some((app.session.active()?.uid, first_selected(app)?.id))
}

/// What the selection is: an Image Trace object (with its stored settings), a plain image, or neither.
fn target(app: &VectorcraftApp) -> (bool, bool, Option<Value>) {
    let Some(n) = first_selected(app) else { return (false, false, None) };
    (is_trace(&n), matches!(n.kind, NodeKind::Image(_)), n.extras().trace.as_deref().cloned())
}

/// The one selected Image Trace object's preset ("Custom" once its settings were changed) and view.
pub(crate) fn selected_trace(app: &VectorcraftApp) -> Option<(String, TraceView)> {
    let st = app.session.active()?;
    let [id] = st.selection.objects[..] else { return None };
    let n = st.doc.node(id).filter(|n| is_trace(n))?;
    Some((n.extras().trace.as_ref().and_then(|t| t["preset"].as_str()).unwrap_or("Custom").to_string(), n.trace_view()))
}

/// Image Trace › View, in the UI language.
fn view_label(v: TraceView) -> &'static str {
    match v {
        TraceView::Result => tl!("Tracing Result"),
        TraceView::ResultWithOutlines => tl!("Tracing Result with Outlines"),
        TraceView::Outlines => tl!("Outlines"),
        TraceView::OutlinesWithSource => tl!("Outlines with Source Image"),
        TraceView::Source => tl!("Source Image"),
    }
}

/// The View dropdown showing `current` → the view chosen.
fn view_dropdown(ui: &mut Ui, current: TraceView, width: f32) -> Option<TraceView> {
    let labels = TraceView::ALL.map(view_label);
    widgets::dropdown_names(ui, "trace-view", view_label(current), &labels, width).and_then(|i| TraceView::ALL.get(i).copied())
}

/// Show the selected Image Trace objects with `view` (no new trace).
fn set_view(app: &mut VectorcraftApp, view: TraceView) {
    if let Err(e) = app.run("imageTrace.setView", json!({ "view": view.id() })) {
        app.ui.status = e;
    }
}

/// While `down`, the selected Image Trace objects show their source image on the canvas, whatever
/// their View. The document and the undo history stay as they are. The panel's eye button next to
/// View calls it; per the public documentation of the panel, that control is pressed and held to
/// view the source image
/// (<https://web.archive.org/web/20250831090623/https://helpx.adobe.com/illustrator/using/image-trace.html>).
fn hold_source(app: &mut VectorcraftApp, ctx: &egui::Context, down: bool) {
    let ids = app
        .session
        .active()
        .filter(|_| down)
        .map(|st| st.selection.objects.iter().copied().filter(|id| st.doc.node(*id).is_some_and(is_trace)).collect::<Vec<_>>())
        .filter(|ids| !ids.is_empty())
        .map(std::sync::Arc::new);
    if ids != app.ui.trace_hold_source {
        app.ui.trace_hold_source = ids;
        ctx.request_repaint();
    }
}

/// The selected Image Trace object's View row (Control bar, Properties).
pub(crate) fn view_row(app: &mut VectorcraftApp, ui: &mut Ui, current: TraceView, width: f32) {
    widgets::dim_label(ui, tl!("View:"));
    // No wider than what the label leaves of the row (#1358).
    if let Some(v) = view_dropdown(ui, current, width.min(ui.available_width())) {
        set_view(app, v);
    }
}

/// Trace the selected image, or trace the selected Image Trace object again, with built-in `preset`.
fn trace_with(app: &mut VectorcraftApp, preset: &str) {
    // A failure is reported in the status bar. The panel stays as it was: the Image Trace
    // button (Properties, Control bar) opens it.
    let _ = app.run("imageTrace.make", json!({ "preset": preset }));
}

/// Reveal the editable tracing controls, including fidelity, in the panel's current location.
pub(crate) fn open_settings(app: &mut VectorcraftApp) {
    // Icon panels toggle when activated, so an already open flyout must stay open.
    if app.ui.open_panel.as_deref() != Some("imageTrace")
        && let Err(e) = app.run("window.panel.activate", json!({ "panel": "imageTrace" }))
    {
        app.ui.status = e;
        return;
    }
    app.ui.image_trace_advanced = true;
}

/// The built-in presets to choose from (`current` highlighted) → the one chosen. Listed only
/// while the menu is open.
/// A preset's name as shown: translated as a preset's name where the catalog has it so (the
/// Outline preset is not the Outline view), as any other string else.
fn preset_label(name: &str) -> &str {
    crate::i18n::tr_ctx(crate::i18n::current(), "imageTrace", name)
}

fn preset_list(app: &mut VectorcraftApp, ui: &mut Ui, current: &str) -> Option<String> {
    let mut chosen = None;
    for (name, ..) in presets(app) {
        if ui.add(egui::Button::selectable(name == current, preset_label(&name))).clicked() {
            chosen = Some(name);
        }
    }
    chosen
}

/// Width of the Control bar's Image Trace button.
pub(crate) const TRACE_BUTTON_W: f32 = 104.0;

/// The Image Trace button for a selected image (Control bar, Properties): a click traces it with
/// the Default preset, its arrow lists the presets to trace it with.
pub(crate) fn trace_button(app: &mut VectorcraftApp, ui: &mut Ui, width: f32) {
    let (main, arrow) = widgets::split_button_named(ui, tl!("Image Trace"), tl!("Tracing Presets"), width);
    if main.on_hover_text(tl!("Trace the image with the Default preset")).clicked() {
        trace_with(app, "Default");
    }
    if let Some(p) = egui::Popup::menu(&arrow).show(|ui| preset_list(app, ui, "")).and_then(|r| r.inner) {
        trace_with(app, &p);
    }
}

/// The selected Image Trace object's preset (Control bar, Properties): choosing another traces it
/// again with that preset.
pub(crate) fn preset_dropdown(app: &mut VectorcraftApp, ui: &mut Ui, current: &str, width: f32) {
    if let Some(p) = widgets::combo(ui, "trace-preset", tl!(current), width, false, |ui| preset_list(app, ui, current)) {
        trace_with(app, &p);
    }
}

/// The Control bar for one selected Image Trace object: its preset, view, the Image Trace panel
/// and Expand. Whether it is one.
pub fn control_bar(app: &mut VectorcraftApp, ui: &mut Ui) -> bool {
    let Some((preset, view)) = selected_trace(app) else { return false };
    widgets::dim_label(ui, tl!("Preset:"));
    preset_dropdown(app, ui, &preset, 150.0);
    view_row(app, ui, view, 190.0);
    if widgets::icon_button(ui, "image", tl!("Image Trace"), false, 24.0).clicked() {
        open_settings(app);
    }
    if widgets::flat_button(ui, tl!("Expand"), 64.0).clicked() {
        app.run("imageTrace.expand", json!({})).ok();
    }
    ui.separator();
    true
}

fn trace(app: &mut VectorcraftApp, st: &mut TraceUi) {
    let preset = if st.preset == "Custom" { "Default" } else { st.preset.as_str() };
    match app.run("imageTrace.make", json!({ "preset": preset, "params": st.params, "view": st.view.id() })) {
        Ok(r) => {
            st.info = Some((r["paths"].as_u64().unwrap_or(0), r["anchors"].as_u64().unwrap_or(0), r["colors"].as_u64().unwrap_or(0)));
            // Adopt the committed state immediately: Undo before the next panel frame must
            // still be recognized as a change, even when it returns to the previous settings.
            if let Some(stored) = target(app).2 {
                st.synced = stored;
                st.object = target_key(app);
            }
        }
        Err(e) => app.ui.status = e,
    }
}

fn slider(ui: &mut Ui, label: &str, v: &mut f64, range: std::ops::RangeInclusive<f64>, suffix: &str) -> (bool, bool) {
    let mut out = (false, false);
    ui.horizontal(|ui| {
        ui.add_sized([74.0, 22.0], egui::Label::new(egui::RichText::new(tl!(label)).size(12.0)));
        // The theme's widget fill matches the panel, which would hide the rail.
        let t = crate::theme::Tokens::get(ui.ctx());
        ui.visuals_mut().widgets.inactive.bg_fill = t.input_border;
        ui.visuals_mut().selection.bg_fill = t.accent;
        let r = ui.add(egui::Slider::new(v, range).suffix(suffix).integer().trailing_fill(true));
        if cfg!(test) {
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("trace-slider", label)), r.rect));
        }
        out = (r.changed(), r.drag_stopped() || (r.changed() && !r.dragged()));
    });
    out
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let all = presets(app);
    let mut st: TraceUi = pstate(ui.ctx(), "image-trace");
    if st.params.is_null() {
        st.preset = "Default".into();
        st.params = all.first().map(|p| p.1.clone()).unwrap_or_default();
    }
    let (is_trace, is_image, stored) = target(app);
    let object = target_key(app);
    if let Some(t) = stored.filter(|t| *t != st.synced || object != st.object) {
        // A change of the object's view alone (here or in the Control bar) keeps the settings that
        // wait in the panel with Preview off.
        if object != st.object || t["preset"] != st.synced["preset"] || t["params"] != st.synced["params"] {
            st.preset = t["preset"].as_str().unwrap_or("Custom").to_string();
            st.params = t["params"].clone();
        }
        st.view = TraceView::of(&t);
        st.synced = t;
        st.object = object;
    }
    let mut retrace = false;

    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Preset:"));
        let names: Vec<&str> = all.iter().map(|p| p.0.as_str()).collect();
        if let Some(i) = widgets::dropdown(ui, "it-preset", &st.preset, &names, 170.0) {
            st.preset = all[i].0.clone();
            st.params = all[i].1.clone();
            retrace = true;
        }
    });
    // The preset's enhanced version (with Shapes and Auto Grouping) or its legacy one.
    if let Some((_, legacy, better)) = all.iter().find(|p| p.0 == st.preset && !p.2.is_null()) {
        let on = *better == st.params;
        if widgets::check(ui, tl!("Enhanced"), on, true) {
            st.params = if on { legacy.clone() } else { better.clone() };
            retrace = true;
        }
    }
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("View:"));
        let eye = 24.0;
        if let Some(v) = view_dropdown(ui, st.view, (ui.available_width() - eye - ui.spacing().item_spacing.x).min(170.0)) {
            st.view = v;
            if is_trace {
                set_view(app, v);
            }
        }
        let held = app.ui.trace_hold_source.is_some();
        let r = widgets::icon_button_enabled(ui, "eye", tl!("Press and hold to view the source image"), held, is_trace, eye);
        if cfg!(test) {
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("trace-hold-source"), r.rect));
        }
        hold_source(app, ui.ctx(), is_trace && r.is_pointer_button_down_on());
    });
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Mode:"));
        let mode = st.params["mode"].as_str().unwrap_or("blackAndWhite").to_string();
        let label = MODES.iter().find(|m| m.0 == mode).map_or("Black and White", |m| m.1);
        let labels: Vec<&str> = MODES.iter().map(|m| m.1).collect();
        if let Some(i) = widgets::dropdown(ui, "it-mode", label, &labels, 170.0) {
            st.params["mode"] = json!(MODES[i].0);
            st.preset = "Custom".into();
            retrace = true;
        }
    });
    let mode = st.params["mode"].as_str().unwrap_or("blackAndWhite").to_string();
    let palette = st.params["palette"].as_str().unwrap_or("limited").to_string();
    if mode == "color" {
        ui.horizontal(|ui| {
            widgets::dim_label(ui, tl!("Palette:"));
            let labels = PALETTES.map(palette_label);
            if let Some(p) = widgets::dropdown_names(ui, "it-palette", palette_label(&palette), &labels, 170.0).and_then(|i| PALETTES.get(i)) {
                st.params["palette"] = json!(p);
                st.preset = "Custom".into();
                retrace = true;
            }
        });
        if palette == "documentLibrary" {
            ui.horizontal(|ui| {
                widgets::dim_label(ui, tl!("Library:"));
                let key = st.params["library"].as_str().unwrap_or(DOCUMENT_SWATCHES).to_string();
                if let Some(k) = widgets::combo(ui, "it-library", &library_label(app, &key), 170.0, false, |ui| library_list(app, ui, &key)) {
                    st.params["library"] = json!(k);
                    st.preset = "Custom".into();
                    retrace = true;
                }
            });
        }
    }
    // Full Tone and Automatic: how many colours (Less … More); else a count or the threshold.
    let (key, label, range, suffix) = match mode.as_str() {
        "blackAndWhite" => ("threshold", tl!("Threshold"), 0.0..=255.0, ""),
        "grayscale" => ("colors", tl!("Grays"), 2.0..=256.0, ""),
        // Flat Logo finds a handful of flat colours: more than ten is almost always noise.
        "logo" => ("colors", tl!("Colors"), 2.0..=10.0, ""),
        _ if matches!(palette.as_str(), "fullTone" | "automatic") => ("colorDetail", tl!("Colors"), 0.0..=100.0, "%"),
        _ => ("colors", tl!("Colors"), 2.0..=256.0, ""),
    };
    let mut v = st.params[key].as_f64().unwrap_or(0.0);
    let (changed, release) = slider(ui, label, &mut v, range, suffix);
    if changed {
        st.params[key] = if key == "colorDetail" { json!(v.round()) } else { json!(v.round() as u64) };
    }
    // Keep the preset's Enhanced row (and widget identities) until the drag finishes.
    // Switching to Custom mid-drag removes that row and loses the slider's release event.
    if release {
        st.preset = "Custom".into();
    }
    retrace |= release;

    ui.add_space(4.0);
    let open = app.ui.image_trace_advanced;
    if widgets::section_toggle(ui, tl!("Advanced"), open) {
        app.ui.image_trace_advanced = !open;
    }
    if open {
        for (key, label, range, suffix) in
            [("paths", tl!("Paths"), 0.0..=100.0, "%"), ("corners", tl!("Corners"), 0.0..=100.0, "%"), ("noise", tl!("Noise"), 1.0..=100.0, " px")]
        {
            let mut v = st.params[key].as_f64().unwrap_or(0.0);
            let (changed, release) = slider(ui, label, &mut v, range, suffix);
            if changed {
                st.params[key] = if key == "noise" { json!(v.round() as u64) } else { json!(v.round()) };
            }
            if release {
                st.preset = "Custom".into();
            }
            retrace |= release;
        }
        // Flat Logo always paints biggest shapes first with a hair of overlap, so there is no choice.
        if st.params["mode"] != "logo" {
            ui.horizontal(|ui| {
                widgets::dim_label(ui, tl!("Method:"));
                for (m, l) in [("abutting", tl!("Abutting")), ("overlapping", tl!("Overlapping"))] {
                    if ui.selectable_label(st.params["method"] == m, l).clicked() && st.params["method"] != m {
                        st.params["method"] = json!(m);
                        st.preset = "Custom".into();
                        retrace = true;
                    }
                }
            });
            // Create: Fills and/or Strokes (one of them stays on), and the widest line stroked.
            let create = |p: &Value, key: &str| p[key].as_bool().unwrap_or(key == "fills");
            let strokes = create(&st.params, "strokes");
            ui.horizontal(|ui| {
                widgets::dim_label(ui, tl!("Create:"));
                for (key, other, label) in [("fills", "strokes", tl!("Fills")), ("strokes", "fills", tl!("Strokes"))] {
                    let on = create(&st.params, key);
                    if widgets::check(ui, label, on, !on || create(&st.params, other)) {
                        st.params[key] = json!(!on);
                        st.preset = "Custom".into();
                        retrace = true;
                    }
                }
            });
            let mut v = st.params["strokeWidth"].as_f64().unwrap_or(10.0);
            let (changed, release) = ui.add_enabled_ui(strokes, |ui| slider(ui, tl!("Stroke"), &mut v, 1.0..=100.0, " px")).inner;
            if changed {
                st.params["strokeWidth"] = json!(v.round());
            }
            if release {
                st.preset = "Custom".into();
            }
            retrace |= release;
        }
        let snap = st.params["snapCurvesToLines"].as_bool().unwrap_or(false);
        if widgets::check(ui, tl!("Snap Curves To Lines"), snap, true) {
            st.params["snapCurvesToLines"] = json!(!snap);
            st.preset = "Custom".into();
            retrace = true;
        }
        let shapes = st.params["shapes"].as_bool().unwrap_or(false);
        if widgets::check(ui, tl!("Shapes"), shapes, true) {
            st.params["shapes"] = json!(!shapes);
            st.preset = "Custom".into();
            retrace = true;
        }
        let grouping = st.params["autoGrouping"].as_bool().unwrap_or(false);
        if widgets::check(ui, tl!("Auto Grouping"), grouping, true) {
            st.params["autoGrouping"] = json!(!grouping);
            st.preset = "Custom".into();
            retrace = true;
        }
        // (Black and White has one colour and Flat Logo flat ones: no gradients to find.)
        let gradients = st.params["gradients"].as_bool().unwrap_or(false);
        if widgets::check(ui, tl!("Gradients"), gradients, mode == "grayscale" || mode == "color") {
            st.params["gradients"] = json!(!gradients);
            st.preset = "Custom".into();
            retrace = true;
        }
        // Ignore Color: white, unless another colour is picked.
        ui.horizontal(|ui| {
            let on = st.params["ignoreWhite"].as_bool().unwrap_or(false);
            if widgets::check(ui, tl!("Ignore Color"), on, true) {
                st.params["ignoreWhite"] = json!(!on);
                st.preset = "Custom".into();
                retrace = true;
            }
            if on {
                let mut rgb = ignore_color(&st.params);
                if ui.color_edit_button_srgb(&mut rgb).changed() {
                    st.params["ignoreColor"] = json!(format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]));
                    st.preset = "Custom".into();
                    retrace = true;
                }
            }
        });
    }
    widgets::divider(ui);
    if let Some((p, a, c)) = st.info {
        widgets::dim_label(
            ui,
            &crate::i18n::fmt(
                tl!("Paths: {paths}    Anchors: {anchors}    Colors: {colors}"),
                &[("paths", &p.to_string()), ("anchors", &a.to_string()), ("colors", &c.to_string())],
            ),
        );
    }
    ui.horizontal(|ui| {
        let preview = app.ui.image_trace_preview;
        if widgets::check(ui, tl!("Preview"), preview, true) {
            app.ui.image_trace_preview = !preview;
        }
        let r = ui.add_enabled_ui(is_trace || is_image, |ui| widgets::flat_button(ui, tl!("Trace"), 80.0)).inner;
        if r.on_disabled_hover_text(tl!("Select an image to trace")).clicked() {
            trace(app, &mut st);
        }
        if ui.add_enabled_ui(is_trace, |ui| widgets::flat_button(ui, tl!("Expand"), 80.0)).inner.clicked() {
            app.run("imageTrace.expand", json!({})).ok();
        }
    });
    // Preview turned on (the checkbox, or `ui.set`): the selected object shows the settings that
    // waited in the panel. With no Image Trace object selected, they wait until it is selected again.
    let preview = app.ui.image_trace_preview;
    retrace |= preview && st.preview_off && is_trace && st.params != st.synced["params"];
    if is_trace || !preview {
        st.preview_off = !preview;
    }
    if retrace && is_trace && preview {
        trace(app, &mut st);
    }
    set_pstate(ui.ctx(), "image-trace", st);
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let (is_trace, _, _) = target(app);
    if menu_item(ui, tl!("Release"), is_trace, false) {
        app.run("imageTrace.release", json!({})).ok();
    }
    if menu_item(ui, tl!("Expand"), is_trace, false) {
        app.run("imageTrace.expand", json!({})).ok();
    }
    ui.separator();
    if menu_item(ui, tl!("Reset to Default"), true, false) {
        set_pstate(ui.ctx(), "image-trace", TraceUi::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, PointerButton, Pos2, Rect, vec2};

    /// One headless frame of the panel (and its menu) with `events`: the texts drawn, with where.
    fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>) -> Vec<(String, Rect)> {
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(280.0, 700.0));
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
        crate::tests_removeanchors::shapes_text(&out.shapes.iter().map(|c| c.shape.clone()).collect::<Vec<_>>())
    }

    /// Click the panel's `label` (the first one drawn).
    fn click(app: &mut VectorcraftApp, ctx: &egui::Context, label: &str) {
        let texts = frame(app, ctx, vec![]);
        let at = texts.iter().find(|(t, _)| t == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no `{label}` in {texts:?}"));
        let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(app, ctx, vec![Event::PointerMoved(at), press(true)]);
        frame(app, ctx, vec![press(false)]);
    }

    fn traced_app() -> VectorcraftApp {
        traced_app_with_ink(0)
    }

    /// A quick trace, then the Image Trace button.
    fn trace_and_open(app: &mut VectorcraftApp, preset: &str) {
        trace_with(app, preset);
        open_settings(app);
    }

    fn traced_app_with_ink(ink: u8) -> VectorcraftApp {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        let img = image::RgbaImage::from_fn(20, 20, |x, _| image::Rgba(if x < 10 { [ink, ink, ink, 255] } else { [255, 255, 255, 255] }));
        let mut png = vec![];
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let data = vectorcraft_format::base64_encode(&png);
        app.run("file.place", json!({"name": "half.png", "dataBase64": data, "link": false})).unwrap();
        app.run("imageTrace.make", json!({"preset": "Default"})).unwrap();
        app
    }

    #[test]
    fn a_large_image_traces_and_the_status_bar_says_it_is_slower() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        let img = image::RgbaImage::from_fn(1601, 12, |x, _| image::Rgba(if x < 800 { [0, 0, 0, 255] } else { [255, 255, 255, 255] }));
        let mut png = vec![];
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let data = vectorcraft_format::base64_encode(&png);
        app.run("file.place", json!({"name": "wide.png", "dataBase64": data, "link": false})).unwrap();
        trace_with(&mut app, "Default");
        assert!(target(&app).2.is_some(), "the image is traced");
        assert!(app.ui.status.contains("1601 × 12") && app.ui.status.contains("more slowly"), "{}", app.ui.status);
    }

    #[test]
    fn a_large_image_traced_as_a_flat_logo_shows_the_size_it_was_traced_at() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        let img = image::RgbaImage::from_fn(1602, 1602, |x, _| image::Rgba(if x < 800 { [0, 0, 0, 255] } else { [255, 255, 255, 255] }));
        let mut png = vec![];
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let data = vectorcraft_format::base64_encode(&png);
        app.run("file.place", json!({"name": "logo.png", "dataBase64": data, "link": false})).unwrap();
        trace_with(&mut app, "Flat Logo");
        assert!(target(&app).2.is_some(), "the image is traced: {}", app.ui.status);
        assert_eq!(app.ui.status, "Flat Logo traced the image at 801 × 801 pixels, averaged down from 1602 × 1602.");
    }

    #[test]
    fn sketch_quick_trace_reveals_all_adjustments_and_keeps_the_source() {
        let mut app = traced_app();
        app.run("imageTrace.release", json!({})).unwrap();
        app.ui.image_trace_advanced = false;
        trace_and_open(&mut app, "Sketched Art");
        assert_eq!(app.ui.open_panel.as_deref(), Some("imageTrace"));
        assert!(app.ui.image_trace_advanced);
        assert_eq!(selected_trace(&app).unwrap().0, "Sketched Art");
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let texts = frame(&mut app, &ctx, vec![]);
        for label in ["Threshold", "Paths", "Corners", "Noise", "Method:", "Fills", "Strokes", "Ignore Color"] {
            assert!(texts.iter().any(|(t, _)| t == label), "missing {label}: {texts:?}");
        }
        let n = first_selected(&app).unwrap();
        assert!(is_trace(&n));
        assert!(matches!(n.children().unwrap()[0].kind, NodeKind::Image(_)));
        // Opening controls again neither closes the flyout nor adds a document undo step.
        let steps = app.session.active().unwrap().history.undo.len();
        open_settings(&mut app);
        assert_eq!(app.ui.open_panel.as_deref(), Some("imageTrace"));
        assert_eq!(app.session.active().unwrap().history.undo.len(), steps);
        // Optional local visual QA; no reference images or generated captures are committed.
        if let Some(path) = std::env::var_os("VECTORCRAFT_TRACE_CAPTURE") {
            let mut ready = false;
            let mut harness = egui_kittest::Harness::builder().with_size(vec2(340.0, 820.0)).wgpu().build_ui_state(
                move |ui, app: &mut VectorcraftApp| {
                    if !ready {
                        egui_extras::install_image_loaders(ui.ctx());
                        crate::theme::install_fonts(ui.ctx());
                        crate::theme::apply(ui.ctx(), crate::theme::Brightness::MediumDark);
                        ready = true;
                        return;
                    }
                    show(app, ui);
                },
                app,
            );
            harness.run_steps(4);
            harness.render().unwrap().save(path).unwrap();
        }
    }

    #[test]
    fn a_quick_trace_leaves_the_panel_as_it_was() {
        let mut app = traced_app();
        app.run("imageTrace.release", json!({})).unwrap();
        app.ui.open_panel = None;
        app.ui.image_trace_advanced = false;
        trace_with(&mut app, "Sketched Art");
        assert!(is_trace(&first_selected(&app).unwrap()));
        assert!(app.ui.open_panel.is_none());
        assert!(!app.ui.image_trace_advanced);
    }

    #[test]
    fn quick_trace_activates_the_existing_floating_panel() {
        let mut app = traced_app();
        app.run("window.panel.float", json!({"panel": "imageTrace"})).unwrap();
        let before = app.ui.docking.clone().unwrap();
        trace_and_open(&mut app, "Sketched Art");
        let after = app.ui.docking.as_ref().unwrap();
        assert_eq!(after.floating.len(), before.floating.len());
        assert!(after.floating.iter().any(|g| g.panels == ["imageTrace"]));
        assert!(app.ui.open_panel.is_none(), "the existing floating group owns the panel");
    }

    #[test]
    fn adjusting_sketch_threshold_changes_the_vector_result_and_can_be_undone() {
        let mut app = traced_app_with_ink(100);
        trace_and_open(&mut app, "Sketched Art");
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        frame(&mut app, &ctx, vec![]);
        let rail = ctx.data(|d| d.get_temp::<Rect>(egui::Id::new(("trace-slider", "Threshold")))).unwrap();
        let at = rail.left_center() + vec2(20.0, 0.0);
        assert_eq!(first_selected(&app).unwrap().children().unwrap().len(), 2, "source and ink path");
        for pressed in [true, false] {
            frame(
                &mut app,
                &ctx,
                vec![
                    Event::PointerMoved(at),
                    Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() },
                ],
            );
        }
        assert_eq!(first_selected(&app).unwrap().children().unwrap().len(), 1, "only the source remains below the ink's luminance");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(first_selected(&app).unwrap().children().unwrap().len(), 2);
        frame(&mut app, &ctx, vec![]);
        let state: TraceUi = pstate(&ctx, "image-trace");
        assert_eq!(state.params["threshold"], 150);
    }

    #[test]
    fn with_preview_off_a_slider_change_waits_for_trace() {
        let mut app = traced_app_with_ink(100);
        trace_and_open(&mut app, "Sketched Art");
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        assert!(app.ui.image_trace_preview, "on at first");
        click(&mut app, &ctx, "Preview");
        assert!(!app.ui.image_trace_preview);
        let steps = app.session.active().unwrap().history.undo.len();
        let rail = ctx.data(|d| d.get_temp::<Rect>(egui::Id::new(("trace-slider", "Threshold")))).unwrap();
        let at = rail.left_center() + vec2(20.0, 0.0);
        for pressed in [true, false] {
            frame(
                &mut app,
                &ctx,
                vec![
                    Event::PointerMoved(at),
                    Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() },
                ],
            );
        }
        assert_eq!(app.session.active().unwrap().history.undo.len(), steps, "no trace while Preview is off");
        assert_eq!(first_selected(&app).unwrap().children().unwrap().len(), 2, "source and ink path, as before");
        let draft: TraceUi = pstate(&ctx, "image-trace");
        assert_ne!(draft.params["threshold"], target(&app).2.unwrap()["params"]["threshold"], "the change waits in the panel");
        click(&mut app, &ctx, "Trace");
        assert_eq!(app.session.active().unwrap().history.undo.len(), steps + 1, "Trace applies it as one undo step");
        assert_eq!(first_selected(&app).unwrap().children().unwrap().len(), 1, "only the source remains below the ink's luminance");
        assert_eq!(target(&app).2.unwrap()["params"], pstate::<TraceUi>(&ctx, "image-trace").params);
        assert!(!app.ui.image_trace_preview, "Trace leaves Preview off");
    }

    #[test]
    fn turning_preview_on_traces_the_changes_that_waited() {
        let mut app = traced_app();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let strokes = |app: &VectorcraftApp| target(app).2.map(|t| t["params"]["strokes"].clone());
        let undo = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
        let steps = undo(&app);
        click(&mut app, &ctx, "Preview");
        click(&mut app, &ctx, "Preview");
        assert!(app.ui.image_trace_preview);
        assert_eq!(undo(&app), steps, "nothing waited, so nothing is traced");
        click(&mut app, &ctx, "Preview");
        click(&mut app, &ctx, "Strokes");
        assert_eq!(strokes(&app), Some(json!(false)));
        assert_eq!(undo(&app), steps);
        click(&mut app, &ctx, "Preview");
        assert_eq!(strokes(&app), Some(json!(true)), "traced again with strokes");
        assert_eq!(undo(&app), steps + 1);
        // A change of view keeps the settings that wait, and Preview turned on with `ui.set` traces them.
        click(&mut app, &ctx, "Preview");
        click(&mut app, &ctx, "Fills");
        assert_eq!(target(&app).2.unwrap()["params"]["fills"], json!(true));
        click(&mut app, &ctx, "Tracing Result");
        click(&mut app, &ctx, "Outlines");
        assert_eq!(selected_trace(&app).map(|t| t.1), Some(TraceView::Outlines));
        assert_eq!(undo(&app), steps + 2, "the view changes without a trace");
        frame(&mut app, &ctx, vec![]);
        assert_eq!(pstate::<TraceUi>(&ctx, "image-trace").params["fills"], json!(false), "the change still waits");
        app.ui.image_trace_preview = true;
        frame(&mut app, &ctx, vec![]);
        assert_eq!(target(&app).2.unwrap()["params"]["fills"], json!(false), "traced with the waiting change");
        assert_eq!(selected_trace(&app).map(|t| t.1), Some(TraceView::Outlines), "in the view chosen");
        assert_eq!(undo(&app), steps + 3);
    }

    #[test]
    fn preview_turned_on_with_nothing_selected_traces_when_the_object_is_selected_again() {
        let mut app = traced_app();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let a = first_selected(&app).unwrap().id;
        let undo = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
        click(&mut app, &ctx, "Preview");
        click(&mut app, &ctx, "Strokes");
        assert_eq!(target(&app).2.unwrap()["params"]["strokes"], json!(false), "the change waits");
        app.run("select.none", json!({})).unwrap();
        let steps = undo(&app);
        click(&mut app, &ctx, "Preview");
        assert!(app.ui.image_trace_preview);
        assert_eq!(undo(&app), steps, "nothing to trace with nothing selected");
        app.run("select.set", json!({"ids": [a.0]})).unwrap();
        frame(&mut app, &ctx, vec![]);
        assert_eq!(target(&app).2.unwrap()["params"]["strokes"], json!(true), "traced with the change that waited");
        assert_eq!(undo(&app), steps + 1);
    }

    #[test]
    fn settings_that_wait_stay_with_their_object() {
        let mut app = traced_app();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let a = first_selected(&app).unwrap().id;
        app.run("edit.duplicate", json!({})).unwrap();
        let b = first_selected(&app).unwrap().id;
        assert_ne!(a, b);
        assert_eq!(target(&app).2.unwrap()["params"]["strokes"], json!(false), "the copy has the same settings");
        app.run("select.set", json!({"ids": [a.0]})).unwrap();
        click(&mut app, &ctx, "Preview");
        click(&mut app, &ctx, "Strokes");
        assert_eq!(pstate::<TraceUi>(&ctx, "image-trace").params["strokes"], json!(true), "the change waits for A");
        app.run("select.set", json!({"ids": [b.0]})).unwrap();
        frame(&mut app, &ctx, vec![]);
        assert_eq!(pstate::<TraceUi>(&ctx, "image-trace").params["strokes"], json!(false), "the panel shows B's own settings");
        click(&mut app, &ctx, "Trace");
        assert_eq!(target(&app).2.unwrap()["params"]["strokes"], json!(false), "B is traced with its own settings");
    }

    #[test]
    fn preview_is_kept_with_the_preferences_and_reset_to_default_leaves_it() {
        let mut app = traced_app();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        click(&mut app, &ctx, "Preview");
        let menu_frame = |app: &mut VectorcraftApp, events: Vec<Event>| {
            let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| menu(app, ui));
            out.textures_delta.clear();
            crate::tests_removeanchors::shapes_text(&out.shapes.iter().map(|c| c.shape.clone()).collect::<Vec<_>>())
        };
        let texts = menu_frame(&mut app, vec![]);
        // Menu items draw their label after the check mark's column.
        let at = texts.iter().find(|(t, _)| t.trim() == "Reset to Default").map(|(_, r)| r.center()).unwrap_or_else(|| panic!("{texts:?}"));
        for pressed in [true, false] {
            menu_frame(
                &mut app,
                vec![
                    Event::PointerMoved(at),
                    Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() },
                ],
            );
        }
        assert!(pstate::<TraceUi>(&ctx, "image-trace").params.is_null(), "the panel's settings were reset");
        assert!(!app.ui.image_trace_preview, "Reset to Default leaves Preview");
        let saved: crate::state::UiState = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
        assert!(!saved.image_trace_preview);
        let old: crate::state::UiState = serde_json::from_value(json!({})).unwrap();
        assert!(old.image_trace_preview, "on for preferences saved before Preview existed");
    }

    #[test]
    fn sketch_adjustment_sliders_retrace_on_release() {
        let mut app = traced_app();
        trace_and_open(&mut app, "Sketched Art");
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        for (label, key) in [("Threshold", "threshold"), ("Paths", "paths"), ("Corners", "corners"), ("Noise", "noise")] {
            frame(&mut app, &ctx, vec![]);
            let rail = ctx.data(|d| d.get_temp::<Rect>(egui::Id::new(("trace-slider", label)))).unwrap();
            let at = rail.left_center() + vec2(20.0, 0.0);
            let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
            let before = target(&app).2.unwrap()["params"][key].clone();
            let preset_before = pstate::<TraceUi>(&ctx, "image-trace").preset;
            let steps = app.session.active().unwrap().history.undo.len();
            frame(&mut app, &ctx, vec![Event::PointerMoved(at), press(true)]);
            assert_eq!(app.session.active().unwrap().history.undo.len(), steps, "{label} waits for release");
            let draft: TraceUi = pstate(&ctx, "image-trace");
            assert_ne!(draft.params[key], before, "{label} updates while held");
            assert_eq!(draft.preset, preset_before, "the preset row stays in place during {label}");
            frame(&mut app, &ctx, vec![press(false)]);
            let stored = target(&app).2.unwrap();
            assert_ne!(
                stored["params"][key],
                before,
                "{label} reaches the tracer; rail {rail:?}, draft {:?}",
                pstate::<TraceUi>(&ctx, "image-trace").params
            );
            assert_eq!(stored["preset"], "Custom");
            assert_eq!(app.session.active().unwrap().history.undo.len(), steps + 1, "one undo step for {label}");
            frame(&mut app, &ctx, vec![]);
            let state: TraceUi = pstate(&ctx, "image-trace");
            assert_eq!(state.params, stored["params"], "{label} survives the next frame");
        }
    }

    /// Holding the eye button next to View down lists the selected Image Trace object for the
    /// canvas to draw with its source image; releasing it shows the object's View again. The
    /// document, its View and the undo history stay as they were. With a plain image selected the
    /// button is disabled.
    #[test]
    fn holding_the_eye_button_shows_the_source_image_until_released() {
        let mut app = traced_app();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        frame(&mut app, &ctx, vec![]);
        let id = first_selected(&app).unwrap().id;
        let undo = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
        let revision = |app: &VectorcraftApp| app.session.active().unwrap().revision;
        let (steps, rev) = (undo(&app), revision(&app));
        let at = ctx.data(|d| d.get_temp::<Rect>(egui::Id::new("trace-hold-source"))).unwrap().center();
        let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut app, &ctx, vec![Event::PointerMoved(at), press(true)]);
        assert_eq!(app.ui.trace_hold_source.as_deref(), Some(&vec![id]), "held: the object shows its source image");
        frame(&mut app, &ctx, vec![]);
        assert!(app.ui.trace_hold_source.is_some(), "still held on the next frame");
        frame(&mut app, &ctx, vec![press(false)]);
        assert_eq!(app.ui.trace_hold_source, None, "released: the object shows its View again");
        assert_eq!(undo(&app), steps, "no undo step");
        assert_eq!(revision(&app), rev, "the document is unchanged");
        assert_eq!(selected_trace(&app).map(|t| t.1), Some(TraceView::Result), "its View is unchanged");
        app.run("imageTrace.release", json!({})).unwrap();
        assert!(matches!(first_selected(&app).unwrap().kind, NodeKind::Image(_)));
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![Event::PointerMoved(at), press(true)]);
        assert_eq!(app.ui.trace_hold_source, None, "a plain image has no source image to show");
        frame(&mut app, &ctx, vec![press(false)]);
    }

    #[test]
    fn properties_can_reopen_the_trace_controls() {
        let mut app = traced_app();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut at = None;
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::super::properties::show(&mut app, ui));
            let texts = crate::tests_removeanchors::shapes_text(&out.shapes.iter().map(|c| c.shape.clone()).collect::<Vec<_>>());
            // The first Image Trace text is the section heading, the last is the button.
            at = texts.iter().rev().find(|(t, _)| t == "Image Trace").map(|(_, r)| r.center());
            out.textures_delta.clear();
        }
        let at = at.unwrap();
        for pressed in [true, false] {
            let events = vec![
                Event::PointerMoved(at),
                Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() },
            ];
            let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| super::super::properties::show(&mut app, ui));
            out.textures_delta.clear();
        }
        assert_eq!(app.ui.open_panel.as_deref(), Some("imageTrace"));
    }

    /// #1358: in Properties the View row fits the panel, so the panel doesn't widen and clip its
    /// labels (the row took the panel's whole width for its menu, beside its label).
    #[test]
    fn the_view_row_fits_the_properties_panel() {
        let mut app = traced_app();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        // Where the panel's content ends after a few frames.
        let right = |app: &mut VectorcraftApp| {
            let mut right = 0.0;
            for _ in 0..3 {
                let screen = Rect::from_min_size(Pos2::ZERO, vec2(280.0, 700.0));
                let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| {
                    crate::panels::properties::show(app, ui);
                    right = ui.min_rect().right();
                });
                out.textures_delta.clear();
            }
            right
        };
        let traced = right(&mut app);
        app.run("select.none", json!({})).unwrap();
        assert_eq!(traced, right(&mut app));
    }

    #[test]
    fn advanced_is_a_section_that_stays_as_it_was_left() {
        let mut app = traced_app();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let has = |texts: &[(String, Rect)], s: &str| texts.iter().any(|(t, _)| t == s);
        let texts = frame(&mut app, &ctx, vec![]);
        assert!(app.ui.image_trace_advanced && has(&texts, "Advanced") && has(&texts, "Corners"), "open at first: {texts:?}");
        click(&mut app, &ctx, "Advanced");
        assert!(!app.ui.image_trace_advanced, "the header closes it");
        assert!(!has(&frame(&mut app, &ctx, vec![]), "Corners"));
        // Remembered with the UI preferences, across launches.
        let saved: crate::state::UiState = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
        assert!(!saved.image_trace_advanced);
        let old: crate::state::UiState = serde_json::from_value(json!({})).unwrap();
        assert!(old.image_trace_advanced, "open for preferences saved before the section existed");
        click(&mut app, &ctx, "Advanced");
        assert!(app.ui.image_trace_advanced && has(&frame(&mut app, &ctx, vec![]), "Corners"));
    }

    #[test]
    fn the_view_dropdown_shows_and_sets_the_objects_view() {
        let mut app = traced_app();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let texts = frame(&mut app, &ctx, vec![]);
        assert!(texts.iter().any(|(t, _)| t == "View:") && texts.iter().any(|(t, _)| t == "Tracing Result"), "{texts:?}");
        let steps = app.session.active().unwrap().history.undo.len();
        click(&mut app, &ctx, "Tracing Result");
        click(&mut app, &ctx, "Source Image");
        assert_eq!(selected_trace(&app).map(|t| t.1), Some(TraceView::Source));
        assert_eq!(app.session.active().unwrap().history.undo.len(), steps + 1, "one undo step, no new trace");
        // Another object's view is shown when it is selected.
        app.run("imageTrace.setView", json!({"view": "outlines"})).unwrap();
        assert!(frame(&mut app, &ctx, vec![]).iter().any(|(t, _)| t == "Outlines"));
    }

    #[test]
    fn the_palette_shows_in_color_mode_and_traces_again() {
        let mut app = traced_app();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let has = |texts: &[(String, Rect)], s: &str| texts.iter().any(|(t, _)| t == s);
        let palette = |app: &VectorcraftApp| target(app).2.map(|t| t["params"]["palette"].clone());
        assert!(!has(&frame(&mut app, &ctx, vec![]), "Palette:"), "Black and White has no palette");
        app.run("imageTrace.make", json!({"preset": "6 Colors"})).unwrap();
        let texts = frame(&mut app, &ctx, vec![]);
        assert!(has(&texts, "Palette:") && has(&texts, "Limited") && !has(&texts, "Library:"), "{texts:?}");
        // Document Library traces again with the document's swatches, and names them.
        click(&mut app, &ctx, "Limited");
        click(&mut app, &ctx, "Document Library");
        assert_eq!(palette(&app), Some(json!("documentLibrary")));
        let texts = frame(&mut app, &ctx, vec![]);
        assert!(has(&texts, "Library:") && has(&texts, "Document Swatches"), "{texts:?}");
        // Full Tone's Colors is how many, Less (0%) … More (100%).
        click(&mut app, &ctx, "Document Library");
        click(&mut app, &ctx, "Full Tone");
        assert_eq!(palette(&app), Some(json!("fullTone")));
        let texts = frame(&mut app, &ctx, vec![]);
        assert!(has(&texts, "50") && !has(&texts, "Library:"), "{texts:?}");
    }

    #[test]
    fn create_strokes_traces_again_and_keeps_one_kind_on() {
        let mut app = traced_app();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let create = |app: &VectorcraftApp| target(app).2.map(|t| (t["params"]["fills"].clone(), t["params"]["strokes"].clone()));
        let texts = frame(&mut app, &ctx, vec![]);
        for s in ["Create:", "Fills", "Strokes", "Stroke", "10"] {
            assert!(texts.iter().any(|(t, _)| t == s), "{s}: {texts:?}");
        }
        // Fills alone can't be turned off.
        click(&mut app, &ctx, "Fills");
        assert_eq!(create(&app), Some((json!(true), json!(false))));
        click(&mut app, &ctx, "Strokes");
        assert_eq!(create(&app), Some((json!(true), json!(true))), "traced again with strokes");
        click(&mut app, &ctx, "Fills");
        assert_eq!(create(&app), Some((json!(false), json!(true))));
        click(&mut app, &ctx, "Strokes");
        assert_eq!(create(&app), Some((json!(false), json!(true))), "Strokes alone stays on");
    }

    #[test]
    fn panel_and_menu_draw_headless() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        for _ in 0..2 {
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                show(&mut app, ui);
                menu(&mut app, ui);
            });
            out.textures_delta.clear();
        }
    }
}
