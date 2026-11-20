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

use std::sync::Arc;

use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_doc::{Node, NodeId, NodeKind, TraceView};
use vectorcraft_engine::cmd::TracePresetChange;
use vectorcraft_engine::cmd::swatchlib::DOCUMENT_SWATCHES;

use super::{first_node, pstate, set_pstate, swatches};
use crate::VectorcraftApp;
use crate::widgets::{self, menu_item};

pub(crate) const MODES: [(&str, &str); 4] =
    [("blackAndWhite", "Black and White"), ("grayscale", "Grayscale"), ("color", "Color"), ("logo", "Flat Logo")];

/// Color mode's palettes (`palette` param ids), in panel order.
pub(crate) const PALETTES: [&str; 4] = ["limited", "fullTone", "automatic", "documentLibrary"];

/// The colour Ignore Color leaves out in `params` (`ignoreColor`, `#rrggbb`): white when there is none.
pub(crate) fn ignore_color(params: &Value) -> [u8; 3] {
    let hex = params["ignoreColor"].as_str().unwrap_or("").trim_start_matches('#');
    let channel = |i: usize| hex.get(i * 2..i * 2 + 2).and_then(|h| u8::from_str_radix(h, 16).ok());
    match (hex.len(), channel(0), channel(1), channel(2)) {
        (6, Some(r), Some(g), Some(b)) if hex.bytes().all(|b| b.is_ascii_hexdigit()) => [r, g, b],
        _ => [255; 3],
    }
}

/// Palette `id`, in the UI language.
pub(crate) fn palette_label(id: &str) -> &'static str {
    match id {
        "fullTone" => tl!("Full Tone"),
        "automatic" => tl!("Automatic"),
        "documentLibrary" => tl!("Document Library"),
        _ => tl!("Limited"),
    }
}

/// Document Library's Colors choices with the document's swatches: All, then the document's color
/// groups (`current` checked; `None` is All) → the choice, `Some(None)` for All.
pub(crate) fn color_group_list(app: &VectorcraftApp, ui: &mut Ui, current: Option<&str>) -> Option<Option<String>> {
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
    /// The revision of the saved presets at the panel's last frame
    /// ([`vectorcraft_engine::Session::trace_presets_revision`]).
    revision: Option<u64>,
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

/// A preset: its name, its parameters, those of its enhanced version (null if it has none), and
/// whether it is built in (else the user saved it).
pub(crate) type Preset = (String, Value, Value, bool);

/// The presets as `imageTrace.presets` listed them at a revision of the saved presets.
#[derive(Clone)]
struct PresetList {
    revision: u64,
    presets: Arc<Vec<Preset>>,
}

#[cfg(test)]
thread_local! {
    /// How many times [`presets`] ran `imageTrace.presets` on this thread.
    static PRESET_READS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Every preset, built-in then saved. `imageTrace.presets` runs once for each revision of the saved
/// presets ([`vectorcraft_engine::Session::trace_presets_revision`]); the list is kept in `ctx`'s
/// memory, which the panel, the Control bar, Properties and Edit → Tracing Presets share.
pub(crate) fn presets(app: &mut VectorcraftApp, ctx: &egui::Context) -> Arc<Vec<Preset>> {
    let id = egui::Id::unique("trace-presets");
    let revision = app.session.trace_presets_revision();
    if let Some(list) = ctx.data(|d| d.get_temp::<PresetList>(id)).filter(|l| l.revision == revision) {
        return list.presets;
    }
    #[cfg(test)]
    PRESET_READS.with(|n| n.set(n.get() + 1));
    let v = app.session.execute("imageTrace.presets", &json!({})).unwrap_or_default();
    let presets: Vec<Preset> = v["presets"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|p| (p["name"].as_str().unwrap_or("").to_string(), p["params"].clone(), p["enhanced"].clone(), p["builtIn"] != false))
                .collect()
        })
        .unwrap_or_default();
    let presets = Arc::new(presets);
    ctx.data_mut(|d| d.insert_temp(id, PresetList { revision, presets: presets.clone() }));
    presets
}

/// The listed preset that `name` finds, as the engine finds presets: a built-in one by its name in
/// any case, also in brackets (`[Default]`), else a saved one by its exact name in any case.
pub(crate) fn find_preset<'a>(all: &'a [Preset], name: &str) -> Option<&'a Preset> {
    let name = name.trim();
    let key = name.trim_start_matches('[').trim_end_matches(']');
    all.iter().find(|p| p.3 && p.0.eq_ignore_ascii_case(key)).or_else(|| all.iter().find(|p| !p.3 && p.0.eq_ignore_ascii_case(name)))
}

/// Whether `n` is an Image Trace object: a group so named whose first child is the traced image.
pub(crate) fn is_trace(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Group { clip: false, .. })
        && n.name.as_deref() == Some("Image Trace")
        && n.children().is_some_and(|c| c.first().is_some_and(|i| matches!(i.kind, NodeKind::Image(_))))
}

/// What Trace acts on in the active document, and whether any selected object is an Image Trace
/// object (wherever it is in the selection).
#[derive(Clone, Debug, Default, PartialEq)]
struct Target {
    /// The active document's uid and the object Trace acts on.
    object: Option<(u64, NodeId)>,
    /// The object is an Image Trace object.
    is_trace: bool,
    /// The object is a plain image.
    is_image: bool,
    /// The object's stored trace settings.
    stored: Option<Value>,
    any_trace: bool,
}

#[cfg(test)]
thread_local! {
    /// How many times [`find_target`] looked the target up in an active document on this thread.
    static TARGET_LOOKUPS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// The image or Image Trace object `imageTrace.make` takes from the selected roots, else the
/// first selected non-traceable object's metadata. Release and Expand keep their own selection scope.
fn find_target(app: &VectorcraftApp) -> Target {
    let Some(st) = app.session.active() else { return Target::default() };
    #[cfg(test)]
    TARGET_LOOKUPS.with(|n| n.set(n.get() + 1));
    let any_trace = st.selection.objects.iter().any(|id| st.doc.node(*id).is_some_and(is_trace));
    let node = vectorcraft_engine::cmd::image_trace_target(&app.session, &Value::Null)
        .ok()
        .and_then(|id| st.doc.node(id))
        .or_else(|| first_node(app).filter(|n| !is_trace(n) && !matches!(n.kind, NodeKind::Image(_))));
    let Some(n) = node else { return Target { any_trace, ..Target::default() } };
    Target {
        object: Some((st.uid, n.id)),
        is_trace: is_trace(n),
        is_image: matches!(n.kind, NodeKind::Image(_)),
        stored: n.extras().trace.as_deref().cloned(),
        any_trace,
    }
}

/// [`find_target`], looked up once for each document and revision of it: the panel reads it on
/// every frame.
fn target(app: &VectorcraftApp, ctx: &egui::Context) -> Target {
    let Some(st) = app.session.active() else { return Target::default() };
    let key = (st.uid, st.revision);
    match pstate::<Option<((u64, u64), Target)>>(ctx, "image-trace-target") {
        Some((k, t)) if k == key => t,
        _ => {
            let t = find_target(app);
            set_pstate(ctx, "image-trace-target", Some((key, t.clone())));
            t
        }
    }
}

/// The one selected Image Trace object's preset ("Custom" once its settings were changed) and view.
pub(crate) fn selected_trace(app: &VectorcraftApp) -> Option<(String, TraceView)> {
    let st = app.session.active()?;
    let [id] = st.selection.objects[..] else { return None };
    let n = st.doc.node(id).filter(|n| is_trace(n))?;
    Some((n.extras().trace.as_ref().and_then(|t| t["preset"].as_str()).unwrap_or("Custom").to_string(), n.trace_view()))
}

/// The trace parameters the one selected Image Trace object records (null without one).
fn selected_trace_params(app: &VectorcraftApp) -> Value {
    let Some(st) = app.session.active() else { return Value::Null };
    let [id] = st.selection.objects[..] else { return Value::Null };
    st.doc.node(id).filter(|n| is_trace(n)).and_then(|n| n.extras().trace.as_ref()).map_or(Value::Null, |t| t["params"].clone())
}

/// Image Trace › View, in the UI language.
fn view_label(v: TraceView) -> &'static str {
    match v {
        TraceView::Result => tl!("Tracing Result"),
        TraceView::ResultWithOutlines => tl!("Tracing Result with Outlines"),
        TraceView::Outlines => tl!("Outlines"),
        TraceView::OutlinesWithSource => tl!("Outlines with Source Image"),
        TraceView::Source => tl!("Source Image"),
        TraceView::Split => tl!("Split View"),
    }
}

/// The Split view's divider of the one selected Image Trace object, when it shows that view.
fn selected_split(app: &VectorcraftApp) -> Option<f64> {
    let st = app.session.active()?;
    let [id] = st.selection.objects[..] else { return None };
    st.doc.node(id).filter(|n| is_trace(n) && n.trace_view() == TraceView::Split).map(Node::trace_split)
}

/// The Divider row of the one selected Image Trace object in the Split view: how much of the
/// source image's width, from its left edge, shows the image. A drag is one undo step, and the
/// arrow keys move it a percent at a time. Nothing without such an object.
pub(crate) fn split_row(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(split) = selected_split(app) else { return };
    ui.horizontal(|ui| {
        let label = widgets::dim_label(ui, tl!("Divider"));
        // The canvas sets the divider to a ten-thousandth: a hundredth of a percent.
        let mut pct = (split * 10_000.0).round() / 100.0;
        // Clamping only edits keeps a divider between whole percents (a canvas drag, an agent's
        // value) as it is while the panel shows: clamping always rounds it to the step each frame
        // and reports a change.
        let slider = egui::Slider::new(&mut pct, 0.0..=100.0).suffix("%").step_by(1.0).clamping(egui::SliderClamping::Edits);
        let r = widgets::slider(ui, slider).labelled_by(label.id);
        r.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), pct, tl!("Divider")));
        if cfg!(test) {
            ui.ctx().data_mut(|d| {
                d.insert_temp(egui::Id::unique("trace-split-slider-id"), r.id);
                d.insert_temp(egui::Id::unique("trace-split-slider-rect"), r.rect);
            });
        }
        let phase = if r.dragged() && !r.drag_stopped() {
            widgets::Live::Dragging
        } else if r.drag_stopped() || r.changed() {
            widgets::Live::Released
        } else {
            widgets::Live::Idle
        };
        if phase != widgets::Live::Idle {
            let split = (pct / 100.0).clamp(0.0, 1.0);
            crate::panels::live_run(app, "Image Trace View", "imageTrace.setView", json!({ "view": TraceView::Split.id(), "split": split }), phase);
        }
    });
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

/// The retained original image of a live trace, or the selected placed image itself.
fn source_image(n: &Node) -> Option<&vectorcraft_doc::ImageObject> {
    let source = if is_trace(n) { n.children()?.first()?.as_ref() } else { n };
    match &source.kind {
        NodeKind::Image(image) => Some(image),
        _ => None,
    }
}

fn sampler_binding(s: &crate::state::TraceIgnoreSampler) -> Value {
    json!({ "doc": s.doc, "target": s.target.0, "revision": s.revision })
}

fn sampler_valid(app: &VectorcraftApp, s: &crate::state::TraceIgnoreSampler) -> bool {
    let Some(doc) = app.session.active() else { return false };
    doc.uid == s.doc
        && doc.revision == s.revision
        && find_target(app).object == Some((s.doc, s.target))
        && doc.doc.node(s.target).and_then(source_image).is_some_and(|image| doc.doc.images.contains_key(&image.key))
}

fn sampler_tool_active(app: &VectorcraftApp, s: &crate::state::TraceIgnoreSampler) -> bool {
    app.session.tool_id() == "eyedropper" && app.session.tool_options()["imageTraceIgnoreColor"] == sampler_binding(s)
}

/// Clear only this temporary mode. An explicit tool choice keeps the tool the user chose.
pub(crate) fn cancel_ignore_sampler(app: &mut VectorcraftApp, restore: bool) {
    let Some(s) = app.ui.trace_ignore_sampler.take() else { return };
    // This UI session owns these transient fields even while Cmd/Ctrl parks the eyedropper
    // behind a borrowed selection tool. The named setter also updates that stored lender.
    let options = serde_json::Map::from_iter([("imageTraceIgnoreColor".into(), Value::Null), ("stop".into(), Value::Null)]);
    app.session.set_tool_options(Some("eyedropper"), &options);
    if restore && app.session.tool_id() == "eyedropper" {
        app.return_to_tool(&s.back);
    }
    if let Some(ctx) = &app.ui_ctx {
        ctx.request_repaint();
    }
}

/// Presets and restored panel settings can turn Ignore Color off without its checkbox.
fn cancel_disabled_ignore_sampler(app: &mut VectorcraftApp, params: &Value) {
    if params["ignoreWhite"] != true {
        cancel_ignore_sampler(app, true);
    }
}

/// Also runs when the panel is closed: document changes and tool changes end the pick.
pub(crate) fn refresh_ignore_sampler(app: &mut VectorcraftApp) -> bool {
    let Some(s) = app.ui.trace_ignore_sampler.as_ref() else { return false };
    if !sampler_valid(app, s) {
        cancel_ignore_sampler(app, true);
    } else if !sampler_tool_active(app, s) {
        cancel_ignore_sampler(app, false);
    } else {
        return false;
    }
    true
}

/// A stale sampler's click must not become a paint/selection click in the restored tool.
pub(crate) fn discard_stale_ignore_click(app: &mut VectorcraftApp) -> bool {
    let Some(s) = app.ui.trace_ignore_sampler.as_ref() else { return false };
    if sampler_valid(app, s) && sampler_tool_active(app, s) {
        return false;
    }
    let restore = app.session.tool_id() == "eyedropper";
    cancel_ignore_sampler(app, restore);
    true
}

fn start_ignore_sampler(app: &mut VectorcraftApp, ctx: &egui::Context) {
    if app.ui.trace_ignore_sampler.is_some() {
        cancel_ignore_sampler(app, true);
        return;
    }
    let Some((doc, target)) = target(app, ctx).object else { return };
    let Some(revision) = app.session.active().map(|d| d.revision) else { return };
    let s = crate::state::TraceIgnoreSampler { doc, target, revision, back: app.session.tool_id().to_string() };
    if !sampler_valid(app, &s) {
        return;
    }
    if let Err(e) = app.try_select_tool("eyedropper") {
        app.status(e);
        return;
    }
    if !sampler_valid(app, &s) {
        app.return_to_tool(&s.back);
        return;
    }
    app.session.set_tool_option("imageTraceIgnoreColor", &sampler_binding(&s));
    app.session.set_tool_option("stop", &json!(s.back.as_str()));
    // Bind the reply to this panel's state, also when it is drawn without the full app frame.
    app.ui_ctx = Some(ctx.clone());
    app.ui.trace_ignore_sampler = Some(s);
    if let Some(ctx) = &app.ui_ctx {
        ctx.request_repaint();
    }
}

/// Match Image Trace's input bytes, not the current canvas result or the working-space colour.
fn sampled_ignore_color(app: &VectorcraftApp, s: &crate::state::TraceIgnoreSampler, p: &Value) -> Option<String> {
    let point = vectorcraft_geom::Point::new(p["point"][0].as_f64()?, p["point"][1].as_f64()?);
    if !point.x.is_finite() || !point.y.is_finite() {
        return None;
    }
    let doc = &app.session.active()?.doc;
    let image = source_image(doc.node(s.target)?)?;
    if image.xf.determinant().abs() < 1e-12 {
        return None;
    }
    let pixel = image.xf.inverse() * point;
    if !pixel.x.is_finite()
        || !pixel.y.is_finite()
        || pixel.x < 0.0
        || pixel.y < 0.0
        || pixel.x >= f64::from(image.width.max(1))
        || pixel.y >= f64::from(image.height.max(1))
    {
        return None;
    }
    let [r, g, b, alpha] = doc.images.get(&image.key)?.sample_object(pixel.x, pixel.y, (image.width, image.height), 1)?;
    (alpha != 0).then(|| format!("#{r:02x}{g:02x}{b:02x}"))
}

/// A miss keeps the pick armed; only a valid source color, Escape or invalid binding ends it.
pub(crate) fn apply_ignore_color_sample(app: &mut VectorcraftApp, p: &Value) {
    let Some(s) = app.ui.trace_ignore_sampler.as_ref() else { return };
    if p["binding"] != sampler_binding(s) {
        return;
    }
    if p["cancel"] == true {
        // Escape already carries its one deferred SwitchTool request.
        cancel_ignore_sampler(app, false);
        return;
    }
    if !sampler_valid(app, s) {
        cancel_ignore_sampler(app, true);
        return;
    }
    // Outside the source or over a transparent pixel there is no color to accept. Keep the
    // binding and previous tool so the next canvas click can still complete this pick.
    let Some(color) = sampled_ignore_color(app, s, p) else { return };
    let s = s.clone();
    // The canvas UI establishes this context before a real panel click can arm the sampler.
    let Some(ctx) = app.ui_ctx.clone() else { return };
    cancel_ignore_sampler(app, false);
    let all = presets(app, &ctx);
    let mut st: TraceUi = pstate(&ctx, "image-trace");
    let Target { is_trace, stored, .. } = target(app, &ctx);
    follow_preset_changes(app, &mut st, &all);
    let changes: Vec<&TracePresetChange> = app.session.trace_preset_changes_since(0).collect();
    sync_with_object(&mut st, stored, Some((s.doc, s.target)), &all, &changes);
    st.params["ignoreColor"] = json!(color);
    st.preset = "Custom".into();
    let preview = app.ui.image_trace_preview;
    if is_trace || !preview {
        st.preview_off = !preview;
    }
    if is_trace && preview {
        trace(app, &ctx, &mut st);
    }
    set_pstate(&ctx, "image-trace", st);
    ctx.request_repaint();
    // Complete the temporary tool without treating its return as a fresh explicit tool choice.
    // A miss above has no switch request and remains armed.
    app.return_to_tool(&s.back);
}

/// The selected Image Trace object's View row (Control bar, Properties).
pub(crate) fn view_row(app: &mut VectorcraftApp, ui: &mut Ui, current: TraceView, width: f32) {
    widgets::dim_label(ui, tl!("View:"));
    // No wider than what the label leaves of the row (#1358).
    if let Some(v) = view_dropdown(ui, current, width.min(ui.available_width())) {
        set_view(app, v);
    }
}

/// Trace the selected image, or trace the selected Image Trace object again, with `preset`.
fn trace_with(app: &mut VectorcraftApp, preset: &str) {
    // A failure is reported in the status bar. The panel stays as it was: the Image Trace
    // button (Properties, Control bar) opens it.
    if let Err(e) = crate::trace_runs::start(app, "imageTrace.make", json!({ "preset": preset }), false) {
        app.status(e);
    }
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

/// The presets to choose from, built-in then saved (`current` highlighted) → the one chosen. Listed only
/// while the menu is open.
/// A preset's name as shown: translated as a preset's name where the catalog has it so (the
/// Outline preset is not the Outline view), as any other string else.
pub(crate) fn preset_label(name: &str) -> &str {
    crate::i18n::tr_ctx(crate::i18n::current(), "imageTrace", name)
}

/// The buttons above the Image Trace panel's Preset dropdown, in panel order: each one's name and the
/// built-in preset a click chooses. The help page lists these options before Preset
/// (<https://helpx.adobe.com/illustrator/desktop/manage-objects/traces-mockups-symbols/image-trace-panel-options.html>);
/// which preset each one chooses is VectorCraft's own.
fn preset_buttons() -> [(&'static str, &'static str); 6] {
    [
        (preset_label("Auto Color"), "Auto Color"),
        (tl!("High Color"), "High Fidelity Photo"),
        (tl!("Low Color"), "Low Fidelity Photo"),
        (tl!("Grayscale"), "Shades of Gray"),
        (tl!("Black and White"), "Black and White Logo"),
        (preset_label("Outline"), "Outline"),
    ]
}

fn preset_list(app: &mut VectorcraftApp, ui: &mut Ui, current: &str) -> Option<String> {
    let mut chosen = None;
    let list = presets(app, ui.ctx());
    for (name, _, _, built_in) in list.iter() {
        // A saved preset's name is the user's, shown as it is.
        let label = if *built_in { preset_label(name) } else { name.as_str() };
        if ui.add(egui::Button::selectable(name == current, label)).clicked() {
            chosen = Some(name.clone());
        }
    }
    chosen
}

/// Manage Presets, next to the Image Trace panel's Preset dropdown: Save as New Preset… names the
/// panel's settings; Rename… and Delete act on the saved preset the dropdown shows (`saved`).
/// Built-in presets don't change. From the public documentation of saving tracing presets
/// (<https://web.archive.org/web/20260902004121/https://helpx.adobe.com/illustrator/desktop/manage-objects/traces-mockups-symbols/save-image-trace-presets.html>).
fn manage_presets(app: &mut VectorcraftApp, ui: &mut Ui, st: &TraceUi, saved: bool) {
    let run = if menu_item(ui, tl!("Save as New Preset…"), true, false) {
        Some(("ui.imageTracePresetDialog", json!({ "params": st.params })))
    } else if menu_item(ui, tl!("Rename…"), saved, false) {
        Some(("ui.imageTracePresetDialog", json!({ "rename": st.preset })))
    } else if menu_item(ui, tl!("Delete"), saved, false) {
        Some(("imageTrace.presets.delete", json!({ "name": st.preset })))
    } else {
        None
    };
    if let Some((id, p)) = run
        && let Err(e) = app.run(id, p)
    {
        app.ui.status = e;
    }
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
    if let Some(p) = egui::Popup::menu(&arrow).show(|ui| widgets::menu_scroll(ui, |ui| preset_list(app, ui, ""))).and_then(|r| r.inner) {
        trace_with(app, &p);
    }
}

/// The selected Image Trace object's preset (Control bar, Properties), traced with the preset
/// `traced_with`, shown as the panel shows it: choosing another traces it again with that preset.
pub(crate) fn preset_dropdown(app: &mut VectorcraftApp, ui: &mut Ui, traced_with: &str, width: f32) {
    let all = presets(app, ui.ctx());
    let params = selected_trace_params(app);
    let changes: Vec<&TracePresetChange> = app.session.trace_preset_changes_since(0).collect();
    let current = listed_preset(traced_with, &params, &all, &changes);
    // A saved preset's name is the user's, shown as it is.
    let shown = if app.session.trace_preset(&current).is_some() { current.as_str() } else { tl!(current.as_str()) };
    if let Some(p) = widgets::combo(ui, "trace-preset", shown, width, |ui| preset_list(app, ui, &current)) {
        trace_with(app, &p);
    }
}

/// The Control bar for one selected Image Trace object: its preset, view, the Image Trace panel
/// and Expand. Whether it is one. Each label with its menu, and each button, shows only while the row
/// has room for it and `reserve` after it, and the controls after one left out are left out too.
pub fn control_bar(app: &mut VectorcraftApp, ui: &mut Ui, reserve: f32) -> bool {
    let Some((preset, view)) = selected_trace(app) else { return false };
    let mut fit = crate::chrome::Fit::new(reserve);
    if fit.room(ui, &[widgets::dim_label_width(ui, tl!("Preset:")), 150.0]) {
        widgets::dim_label(ui, tl!("Preset:"));
        preset_dropdown(app, ui, &preset, 150.0);
    }
    if fit.room(ui, &[widgets::dim_label_width(ui, tl!("View:")), 190.0]) {
        view_row(app, ui, view, 190.0);
    }
    if fit.room(ui, &[24.0]) && widgets::icon_button(ui, "image", tl!("Image Trace"), false, 24.0).clicked() {
        open_settings(app);
    }
    if fit.room(ui, &[widgets::flat_button_fit_width(ui, tl!("Expand"), 64.0)]) && widgets::flat_button_fit(ui, tl!("Expand"), 64.0).clicked() {
        app.run("imageTrace.expand", json!({})).ok();
    }
    fit.separator(ui);
    true
}

fn trace(app: &mut VectorcraftApp, ctx: &egui::Context, st: &mut TraceUi) {
    // A group the active document does not have traces with All, as the panel shows.
    if color_group(app, &st.params).is_none() && !st.params["colorGroup"].is_null() {
        st.params["colorGroup"] = Value::Null;
    }
    // The settings are complete, so with a preset that no longer exists (a deleted saved one) they
    // are traced over Default, and the object shows Custom.
    let all = presets(app, ctx);
    let known = find_preset(&all, &st.preset).map(|p| p.0.as_str());
    let preset = known.filter(|_| st.preset != "Custom").unwrap_or("Default");
    let p = json!({ "preset": preset, "params": st.params, "view": st.view.id() });
    match crate::trace_runs::start(app, "imageTrace.make", p, true) {
        // Traced off the UI thread: the result comes to [`traced`].
        Ok(r) if r["background"] == true => {}
        Ok(r) => adopt(app, ctx, st, &r),
        Err(e) => app.ui.status = e,
    }
}

/// The panel takes trace `r`'s counts and the traced object's stored settings at once: Undo before
/// the next panel frame must still be recognized as a change, even when it returns to the previous
/// settings.
fn adopt(app: &VectorcraftApp, ctx: &egui::Context, st: &mut TraceUi, r: &Value) {
    st.info = Some((r["paths"].as_u64().unwrap_or(0), r["anchors"].as_u64().unwrap_or(0), r["colors"].as_u64().unwrap_or(0)));
    let target = target(app, ctx);
    if let Some(stored) = target.stored {
        st.synced = stored;
        st.object = target.object;
    }
}

/// A trace the panel started off the UI thread was applied with result `r` ([`crate::trace_runs`]).
pub(crate) fn traced(app: &VectorcraftApp, r: &Value) {
    let Some(ctx) = app.ui_ctx.clone() else { return };
    let mut st: TraceUi = pstate(&ctx, "image-trace");
    adopt(app, &ctx, &mut st, r);
    set_pstate(&ctx, "image-trace", st);
}

pub(crate) fn slider(ui: &mut Ui, label: &str, v: &mut f64, range: std::ops::RangeInclusive<f64>, suffix: &str) -> (bool, bool) {
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

/// The preset `name` an object was traced with, or, when no preset in `all` has that name, the name that the saved
/// preset renames in `changes` (oldest first) gave it since. None when the preset was deleted, or when more than one
/// preset had the name in this session (a preset made or renamed to it after the first one was renamed or deleted),
/// since the object may have been traced with any of them.
fn renamed_preset(name: &str, all: &[Preset], changes: &[&TracePresetChange]) -> Option<String> {
    let name = name.trim();
    if find_preset(all, name).is_some() {
        return Some(name.to_string());
    }
    // Each preset that had the name: its name now, or None once deleted.
    let mut had: Vec<Option<String>> = vec![Some(name.to_string())];
    let named = |had: &[Option<String>], n: &str| had.iter().flatten().any(|h| h.eq_ignore_ascii_case(n));
    for c in changes {
        if c.deleted {
            had.iter_mut().filter(|h| h.as_deref().is_some_and(|h| h.eq_ignore_ascii_case(&c.name))).for_each(|h| *h = None);
        } else if let Some(from) = c.renamed_from.as_deref() {
            if c.name.eq_ignore_ascii_case(name) && !named(&had, name) && !named(&had, from) {
                had.push(Some(c.name.clone()));
            }
            had.iter_mut().flatten().filter(|h| h.eq_ignore_ascii_case(from)).for_each(|h| h.clone_from(&c.name));
        } else if c.name.eq_ignore_ascii_case(name) && !named(&had, name) {
            had.push(Some(c.name.clone()));
        }
    }
    match had.as_slice() {
        [only] => only.clone(),
        _ => None,
    }
}

/// The name the preset `name` an object was traced with `params` shows under, with the presets `all` and the saved
/// preset changes `changes` of this session (oldest first): its listed name (`[sketched art]` is Sketched Art), the
/// new name of a saved preset renamed since, or Custom when that preset was deleted or its settings changed since.
fn listed_preset(name: &str, params: &Value, all: &[Preset], changes: &[&TracePresetChange]) -> String {
    if name == "Custom" {
        return name.to_string();
    }
    let found = renamed_preset(name, all, changes).and_then(|n| find_preset(all, &n)).filter(|p| p.3 || p.1 == *params);
    found.map_or_else(|| "Custom".into(), |p| p.0.clone())
}

/// Bring the panel's settings `st` in line with the selected object `object`, whose trace settings are `stored`,
/// with the presets `all` and the saved preset changes `changes` of this session (oldest first); whether the object
/// came back with the settings that waited for it.
fn sync_with_object(st: &mut TraceUi, stored: Option<Value>, object: Option<(u64, NodeId)>, all: &[Preset], changes: &[&TracePresetChange]) -> bool {
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
            // The preset shows under its listed name (`[sketched art]` is Sketched Art). An object
            // traced with a saved preset that was renamed since shows the new name; one traced with a
            // saved preset that was deleted since, or whose settings changed since, shows Custom.
            st.preset = listed_preset(&st.preset, &st.params, all, changes);
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
    restored
}

/// Bring the panel's preset in `st`, and the preset of each setting that waits for an object no longer selected, in
/// line with the saved presets made, changed, renamed or deleted since the last frame (Manage Presets, or
/// imageTrace.presets.save and .delete), as the commands recorded them, with the presets `all`.
fn follow_preset_changes(app: &VectorcraftApp, st: &mut TraceUi, all: &[Preset]) {
    let revision = app.session.trace_presets_revision();
    if let Some(seen) = st.revision.filter(|seen| *seen != revision) {
        let changes: Vec<&TracePresetChange> = app.session.trace_preset_changes_since(seen).collect();
        follow_preset(&mut st.preset, &st.params, &changes, all);
        for (preset, params, _) in st.drafts.values_mut() {
            follow_preset(preset, params, &changes, all);
        }
    }
    st.revision = Some(revision);
}

/// Bring `preset`, shown with the settings `params`, in line with the saved preset changes `changes`, with the
/// presets `all`: it follows its own preset's new name, becomes a preset saved with its settings, and becomes Custom
/// when its preset no longer exists or its preset's settings changed away from `params`.
fn follow_preset(preset: &mut String, params: &Value, changes: &[&TracePresetChange], all: &[Preset]) {
    // The presets saved meanwhile, under the names later renames gave them.
    let mut saved: Vec<String> = vec![];
    let mut followed = false;
    for c in changes {
        match c.renamed_from.as_deref() {
            Some(from) => {
                if *preset == from {
                    *preset = c.name.clone();
                    followed = true;
                }
                for n in saved.iter_mut().filter(|n| n.as_str() == from) {
                    *n = c.name.clone();
                }
            }
            None if !c.deleted => saved.push(c.name.clone()),
            None => {}
        }
    }
    let as_shown = saved.iter().rev().find(|n| all.iter().any(|p| !p.3 && p.0 == **n && p.1 == *params));
    if let Some(name) = as_shown.filter(|_| !followed) {
        *preset = name.clone();
    }
    // A save or rename of the preset can also change its settings.
    let changed_away = (followed || saved.contains(preset)) && all.iter().any(|p| !p.3 && p.0 == *preset && p.1 != *params);
    if changed_away || !all.iter().any(|p| p.0 == *preset) {
        *preset = "Custom".into();
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let all = presets(app, ui.ctx());
    let mut st: TraceUi = pstate(ui.ctx(), "image-trace");
    if st.params.is_null() {
        st.preset = "Default".into();
        st.params = all.first().map(|p| p.1.clone()).unwrap_or_default();
    }
    let Target { object, is_trace, is_image, stored, any_trace } = target(app, ui.ctx());
    // The saved preset changes since the last frame apply to the panel's settings before the selected object's
    // settings, which `sync_with_object` resolves against every change, are adopted.
    follow_preset_changes(app, &mut st, &all);
    let changes: Vec<&TracePresetChange> = app.session.trace_preset_changes_since(0).collect();
    let restored = sync_with_object(&mut st, stored, object, &all, &changes);
    let mut retrace = false;

    // A preset button chooses its preset as the Preset dropdown does; the button of the panel's
    // preset shows as selected.
    ui.horizontal_wrapped(|ui| {
        for (label, name) in preset_buttons() {
            let selected = st.preset == name;
            if cfg!(test) {
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique(("trace-preset-button", name)), selected));
            }
            if ui.add(egui::Button::selectable(selected, label)).on_hover_text(preset_label(name)).clicked()
                && let Some(p) = all.iter().find(|p| p.0 == name)
            {
                st.preset = name.to_string();
                st.params = p.1.clone();
                retrace = true;
            }
        }
    });
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Preset:"));
        let manage = 24.0;
        let width = (ui.available_width() - manage - ui.spacing().item_spacing.x).min(170.0);
        let is_saved = all.iter().any(|p| !p.3 && p.0 == st.preset);
        let shown = if is_saved { st.preset.as_str() } else { preset_label(&st.preset) };
        let current = st.preset.clone();
        if let Some(name) = widgets::combo(ui, "it-preset", shown, width, |ui| preset_list(app, ui, &current))
            && let Some(p) = all.iter().find(|p| p.0 == name)
        {
            st.preset = name;
            st.params = p.1.clone();
            retrace = true;
        }
        let r = widgets::icon_button(ui, "ellipsis", tl!("Manage Presets"), false, manage);
        if cfg!(test) {
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique("trace-manage-presets"), r.rect));
        }
        egui::Popup::menu(&r).show(|ui| manage_presets(app, ui, &st, is_saved));
    });
    // The preset's enhanced version (with Shapes and Auto Grouping) or its legacy one.
    if let Some((_, legacy, better, _)) = all.iter().find(|p| p.0 == st.preset && !p.2.is_null()) {
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
    split_row(app, ui);
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
                if let Some(k) =
                    widgets::combo(ui, "it-library", &swatches::library_label(app, &key), 170.0, |ui| swatches::library_list(app, ui, &key))
                {
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
                    if let Some(g) = widgets::combo(ui, "it-color-group", &label, 170.0, |ui| color_group_list(app, ui, group.as_deref())) {
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
                if on {
                    cancel_ignore_sampler(app, true);
                }
                st.params["ignoreWhite"] = json!(!on);
                st.preset = "Custom".into();
                retrace = true;
            }
            if on {
                let mut rgb = ignore_color(&st.params);
                let picker = ui.color_edit_button_srgb(&mut rgb);
                if picker.clicked() {
                    cancel_ignore_sampler(app, true);
                }
                if picker.changed() {
                    st.params["ignoreColor"] = json!(format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]));
                    st.preset = "Custom".into();
                    retrace = true;
                }
                if widgets::icon_button_enabled(
                    ui,
                    "pipette",
                    tl!("Eyedropper: click the source image to sample the ignored color"),
                    app.ui.trace_ignore_sampler.is_some(),
                    is_trace || is_image,
                    22.0,
                )
                .clicked()
                {
                    start_ignore_sampler(app, ui.ctx());
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
            trace(app, ui.ctx(), &mut st);
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
    cancel_disabled_ignore_sampler(app, &st.params);
    if retrace && is_trace && preview {
        trace(app, ui.ctx(), &mut st);
    }
    set_pstate(ui.ctx(), "image-trace", st);
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let any_trace = target(app, ui.ctx()).any_trace;
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
