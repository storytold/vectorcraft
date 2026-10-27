//! Image Trace panel: preset, view, mode, palette (Color), threshold or colour count, and the
//! Advanced section (open or closed as it was last left): fidelity, method, and Create: Fills /
//! Strokes with the widest line stroked. With an Image Trace object selected and Preview on (the
//! default, kept with the UI preferences), changing a setting re-traces it (one undo step per
//! change; sliders apply when released); with Preview off, the changes wait in the panel until
//! Trace applies them, or until Preview is turned on again; they stay with their object while
//! another one is selected. `imageTrace.preview` turns Preview on or off. Changing the view redraws
//! the object without tracing again. With an image selected, Trace makes a new Image Trace object.
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

/// Document Library's Colors choices with the document's swatches: All, then the document's color
/// groups (`current` checked; `None` is All) → the choice, `Some(None)` for All.
fn color_group_list(app: &VectorcraftApp, ui: &mut Ui, current: Option<&str>) -> Option<Option<String>> {
    let mut chosen = menu_item(ui, "All", true, current.is_none()).then_some(None);
    let groups: Vec<String> = app.session.active().map(|d| d.doc.swatch_groups.iter().map(|g| g.name.clone()).collect()).unwrap_or_default();
    if !groups.is_empty() {
        ui.separator();
    }
    for g in groups {
        if widgets::menu_item_name(ui, &g, true, current == Some(g.as_str())) {
            chosen = Some(Some(g));
        }
    }
    chosen
}

/// `params`' color group when the active document has it, found as `imageTrace.make` finds it (so
/// named exactly, else ignoring case); `None` is All. A group of another document, or a deleted
/// one, is All.
fn color_group<'a>(app: &VectorcraftApp, params: &'a Value) -> Option<&'a str> {
    let name = params["colorGroup"].as_str().filter(|g| !g.is_empty())?;
    let lower = name.to_lowercase();
    app.session.active()?.doc.swatch_groups.iter().any(|g| g.name.to_lowercase() == lower).then_some(name)
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
    /// The settings that wait for objects no longer selected, by document and object: the preset,
    /// the parameters, and the object's stored settings they were changed from.
    drafts: std::collections::HashMap<(u64, NodeId), (String, Value, Value)>,
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

/// The object Trace acts on: the first selected image or Image Trace object in paint order, as
/// `imageTrace.make` takes it, else the first selected object.
fn target_node(app: &VectorcraftApp) -> Option<Node> {
    let st = app.session.active()?;
    let traceable = |n: &&Node| is_trace(n) || matches!(n.kind, NodeKind::Image(_));
    st.selection.in_paint_order(&st.doc).into_iter().find_map(|id| st.doc.node(id).filter(traceable).cloned()).or_else(|| first_selected(app))
}

/// The active document and the object Trace acts on.
fn target_key(app: &VectorcraftApp) -> Option<(u64, NodeId)> {
    Some((app.session.active()?.uid, target_node(app)?.id))
}

/// What Trace acts on: an Image Trace object (with its stored settings), a plain image, or neither.
fn target(app: &VectorcraftApp) -> (bool, bool, Option<Value>) {
    let Some(n) = target_node(app) else { return (false, false, None) };
    (is_trace(&n), matches!(n.kind, NodeKind::Image(_)), n.extras().trace.as_deref().cloned())
}

/// Whether any selected object is an Image Trace object, wherever it is in the selection.
fn any_trace(app: &VectorcraftApp) -> bool {
    app.session.active().is_some_and(|st| st.selection.objects.iter().any(|id| st.doc.node(*id).is_some_and(is_trace)))
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
    // A group the active document does not have traces with All, as the panel shows.
    if color_group(app, &st.params).is_none() && !st.params["colorGroup"].is_null() {
        st.params["colorGroup"] = Value::Null;
    }
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
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique(("trace-slider", label)), r.rect));
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
    let any_trace = any_trace(app);
    let object = target_key(app);
    let same = |a: &Value, b: &Value| a["preset"] == b["preset"] && a["params"] == b["params"];
    let mut restored = false;
    // Another object: the settings that wait for the one left are kept until it is selected again,
    // also when a plain image or nothing is selected next.
    if object != st.object
        && let Some(left) = st.object
        && !st.synced.is_null()
        && st.params != st.synced["params"]
    {
        st.drafts.insert(left, (st.preset.clone(), st.params.clone(), st.synced.clone()));
    }
    // A plain image (or nothing) keeps the panel's settings, which no longer wait for the object left.
    if stored.is_none() && object != st.object {
        st.synced = Value::Null;
        st.object = object;
    }
    if let Some(t) = stored.filter(|t| *t != st.synced || object != st.object) {
        // A change of the object's view alone (here or in the Control bar) keeps the settings that
        // wait in the panel with Preview off.
        if object != st.object || !same(&t, &st.synced) {
            st.preset = t["preset"].as_str().unwrap_or("Custom").to_string();
            st.params = t["params"].clone();
        }
        // The object selected again shows the settings that wait for it, unless its own settings
        // changed since (Undo, or a trace from elsewhere).
        if object != st.object
            && let Some((preset, params, from)) = object.and_then(|k| st.drafts.remove(&k))
            && same(&from, &t)
        {
            st.preset = preset;
            st.params = params;
            restored = true;
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
            if any_trace {
                set_view(app, v);
            }
        }
        let held = app.ui.trace_hold_source.is_some();
        let r = widgets::icon_button_enabled(ui, "eye", tl!("Press and hold to view the source image"), held, any_trace, eye);
        if cfg!(test) {
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique("trace-hold-source"), r.rect));
        }
        hold_source(app, ui.ctx(), any_trace && r.is_pointer_button_down_on());
    });
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Mode:"));
        let mode = st.params["mode"].as_str().unwrap_or("blackAndWhite").to_string();
        let label = MODES.iter().find(|m| m.0 == mode).map_or("Black and White", |m| m.1);
        let labels: Vec<&str> = MODES.iter().map(|m| m.1).collect();
        if let Some(i) = widgets::dropdown(ui, "it-mode", label, &labels, 170.0) {
            // Only Color mode uses a color group.
            if MODES[i].0 != "color" {
                st.params["colorGroup"] = Value::Null;
            }
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
                // Every other palette refuses a color group.
                if *p != "documentLibrary" {
                    st.params["colorGroup"] = Value::Null;
                }
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
                    if k != DOCUMENT_SWATCHES {
                        st.params["colorGroup"] = Value::Null;
                    }
                    st.params["library"] = json!(k);
                    st.preset = "Custom".into();
                    retrace = true;
                }
            });
            // The document's swatches: every one, or one color group's.
            if st.params["library"].as_str().unwrap_or(DOCUMENT_SWATCHES) == DOCUMENT_SWATCHES {
                ui.horizontal(|ui| {
                    widgets::dim_label(ui, tl!("Colors:"));
                    let group = color_group(app, &st.params).map(str::to_string);
                    let label = group.clone().unwrap_or_else(|| tl!("All").to_string());
                    if let Some(g) = widgets::combo(ui, "it-color-group", &label, 170.0, false, |ui| color_group_list(app, ui, group.as_deref())) {
                        st.params["colorGroup"] = json!(g);
                        st.preset = "Custom".into();
                        retrace = true;
                    }
                });
            }
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
        if widgets::check(ui, tl!("Preview"), preview, true)
            && let Err(e) = app.run("imageTrace.preview", json!({}))
        {
            app.ui.status = e;
        }
        let r = ui.add_enabled_ui(is_trace || is_image, |ui| widgets::flat_button(ui, tl!("Trace"), 80.0)).inner;
        if r.on_disabled_hover_text(tl!("Select an image to trace")).clicked() {
            trace(app, &mut st);
        }
        if ui.add_enabled_ui(any_trace, |ui| widgets::flat_button(ui, tl!("Expand"), 80.0)).inner.clicked() {
            app.run("imageTrace.expand", json!({})).ok();
        }
    });
    // Preview turned on (the checkbox, `imageTrace.preview` or `ui.set`): the selected object is
    // traced with the settings that waited in the panel. With no Image Trace object selected, it is
    // traced when it is selected again. An object whose settings waited while another object was
    // selected is traced when it is selected again with Preview on.
    let preview = app.ui.image_trace_preview;
    retrace |= preview && (st.preview_off || restored) && is_trace && st.params != st.synced["params"];
    if is_trace || !preview {
        st.preview_off = !preview;
    }
    if retrace && is_trace && preview {
        trace(app, &mut st);
    }
    set_pstate(ui.ctx(), "image-trace", st);
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let any_trace = any_trace(app);
    if menu_item(ui, tl!("Release"), any_trace, false) {
        app.run("imageTrace.release", json!({})).ok();
    }
    if menu_item(ui, tl!("Expand"), any_trace, false) {
        app.run("imageTrace.expand", json!({})).ok();
    }
    ui.separator();
    if menu_item(ui, tl!("Reset to Default"), true, false) {
        set_pstate(ui.ctx(), "image-trace", TraceUi::default());
    }
}

#[cfg(test)]
mod tests;
