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

// PLAN: assumed import paths of the framework items below: `craft::ui::primitives::IconId`,
// `craft::ui::runtime::commands::{CommandState, CommandStates}`, `craft::ui::menu::PanelRow`,
// `craft::ui::patterns::presets::*`, `crate::edits` (widgets-w13) and `crate::chrome::rank` (chrome-w16).
use craft::behavior::{IconId, Metrics, Proposal, ScrollPos, Sense};
use craft::keymap::CommandRef;
use craft::ui::buttons::{Button, IconButton, SplitButton, SplitMenu, ToggleButton};
use craft::ui::choice::Checkbox;
use craft::ui::contract::{
    Bind, BindMut, Common, Component, Container, ContainerProps, Cx, Entry, Props as _, Role, Scroll, Show as _, ShowContainer as _, Sizing,
};
use craft::ui::display::{Divider, Shade, Text};
use craft::ui::form::{Field, SliderField, SliderFieldLayout};
use craft::ui::layout::BodyLayout;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::menu::PanelRow;
use craft::ui::patterns::presets::{
    NameCheck, NamedPresetEvent, NamedPresetRow, NamedPresetStyle, PickerField, PresetError, PresetPicker, PresetRow, PresetSource, PresetState,
};
use craft::ui::pickers::Select;
use craft::ui::runtime::commands::{CommandState, CommandStates};
use craft::ui::runtime::env::Env;
use craft::ui::sections::Section;
use craft::ui::style::{ComponentStyle, StyleCx};
use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_doc::{Node, NodeId, NodeKind, TraceView};
use vectorcraft_engine::cmd::TracePresetChange;

use super::first_node;
use crate::VectorcraftApp;
use crate::chrome::rank;
use crate::edits::{self, EditPath};

pub(crate) mod rows;
use rows::{RowsData, RowsHost, RowsOutput, Sampler};

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

/// `params`' color group when the active document has it, found as `imageTrace.make` finds it (so
/// named exactly, else ignoring case); `None` is All. A group of another document, or a deleted
/// one, is All.
fn color_group<'a>(app: &VectorcraftApp, params: &'a Value) -> Option<&'a str> {
    rows::known_group(&rows::swatch_groups(app), params)
}

/// An object's open document, content generation and node id.
type ObjectKey = (u64, u64, NodeId);

/// Panel state: the preset name, the current parameters, the view and the last result's counts.
#[derive(Clone, Debug, Default)]
pub(crate) struct TraceUi {
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
    /// The document contents and object `synced` came from.
    object: Option<ObjectKey>,
    /// Preview was off at the panel's last frame.
    preview_off: bool,
    /// The settings that wait for objects no longer selected, by document contents and object: the preset,
    /// the parameters, and the object's stored settings they were changed from.
    drafts: std::collections::HashMap<ObjectKey, (String, Value, Value)>,
}

/// The Image Trace panel's run-only state: what it shows, and the view settings of its preset picker.
#[derive(Clone, Debug, Default)]
pub(crate) struct ImageTraceUi {
    pub(crate) shown: TraceUi,
    pub(crate) picker: PresetState<String>,
}

impl ImageTraceUi {
    /// Reset to Default: the settings go back to the Default preset's. Preview and Advanced are the
    /// user's preferences and stay.
    pub(crate) fn reset(&mut self) {
        self.shown = TraceUi::default();
    }
}

/// A preset: its name, its parameters, those of its enhanced version (null if it has none), and
/// whether it is built in (else the user saved it).
pub(crate) type Preset = (String, Value, Value, bool);

/// The presets as `imageTrace.presets` listed them at a revision of the saved presets, and the rows
/// and select entries made from them.
// PLAN: the contract's `revision` field is the key of the memo `app.canvas.trace_presets`; it is not kept twice.
#[derive(Debug)]
pub(crate) struct PresetList {
    pub(crate) presets: Vec<Preset>,
    pub(crate) rows: Vec<PresetRow<String>>,
    pub(crate) entries: Vec<Entry<String>>,
}

impl PresetList {
    fn new(presets: Vec<Preset>) -> Self {
        // A saved preset's name is the user's, shown as it is.
        let label = |p: &Preset| if p.3 { preset_label(&p.0).to_string() } else { p.0.clone() };
        let rows = presets.iter().map(|p| PresetRow::item(p.0.clone(), label(p), 0)).collect();
        let entries = presets.iter().map(|p| Entry::new(p.0.clone(), label(p))).collect();
        PresetList { presets, rows, entries }
    }
}

/// Every preset, built-in then saved. `imageTrace.presets` runs once for each revision of the saved
/// presets ([`vectorcraft_engine::Session::trace_presets_revision`]); the list is kept in
/// `app.canvas`, which the panel, the Control bar, Properties and Edit → Tracing Presets share.
pub(crate) fn presets(app: &mut VectorcraftApp) -> Arc<PresetList> {
    let revision = app.session.trace_presets_revision();
    if let Some((key, list)) = &app.canvas.trace_presets
        && *key == revision
    {
        return list.clone();
    }
    let v = app.session.execute("imageTrace.presets", &json!({})).unwrap_or_default();
    let presets: Vec<Preset> = v["presets"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|p| (p["name"].as_str().unwrap_or("").to_string(), p["params"].clone(), p["enhanced"].clone(), p["builtIn"] != false))
                .collect()
        })
        .unwrap_or_default();
    let list = Arc::new(PresetList::new(presets));
    app.canvas.trace_presets = Some((revision, list.clone()));
    list
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
pub(crate) struct Target {
    /// Present even with nothing selected, so Revert can retire that document's old drafts.
    document: Option<(u64, u64)>,
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

impl Target {
    fn object_key(&self) -> Option<ObjectKey> {
        let (uid, node) = self.object?;
        let (_, reverts) = self.document?;
        Some((uid, reverts, node))
    }
}

/// Whether any selected object is an Image Trace object.
fn any_trace_selected(app: &VectorcraftApp) -> bool {
    app.session.active().is_some_and(|st| st.selection.objects.iter().any(|id| st.doc.node(*id).is_some_and(is_trace)))
}

/// The image or Image Trace object `imageTrace.make` takes from the selected roots, else the
/// first selected non-traceable object's metadata. Release and Expand keep their own selection scope.
fn find_target(app: &VectorcraftApp) -> Target {
    let Some(st) = app.session.active() else { return Target::default() };
    let document = Some((st.uid, st.reverts));
    let any_trace = any_trace_selected(app);
    let node = vectorcraft_engine::cmd::image_trace_target(&app.session, &Value::Null)
        .ok()
        .and_then(|id| st.doc.node(id))
        .or_else(|| first_node(app).filter(|n| !is_trace(n) && !matches!(n.kind, NodeKind::Image(_))));
    let Some(n) = node else { return Target { document, any_trace, ..Target::default() } };
    Target {
        document,
        object: Some((st.uid, n.id)),
        is_trace: is_trace(n),
        is_image: matches!(n.kind, NodeKind::Image(_)),
        stored: n.extras().trace.as_deref().cloned(),
        any_trace,
    }
}

/// [`find_target`], looked up once for each document and revision of it: the panel reads it on
/// every frame.
pub(crate) fn target(app: &mut VectorcraftApp) -> Arc<Target> {
    let Some(st) = app.session.active() else { return Arc::default() };
    let key = (st.uid, st.revision);
    if let Some((k, t)) = &app.canvas.trace_target
        && *k == key
    {
        return t.clone();
    }
    let t = Arc::new(find_target(app));
    app.canvas.trace_target = Some((key, t.clone()));
    t
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
    // The canvas sets the divider to a ten-thousandth: a hundredth of a percent.
    let pct = (split * 10_000.0).round() / 100.0;
    let edit = Field::new(tl!("Divider"))
        .show(ui, |ui, _| {
            SliderField::new(tl!("Divider"), Bind::given(pct), 0.0..=100.0).layout(SliderFieldLayout::Inline).step(1.0).unit("%").show(ui).edit
        })
        .inner;
    let Some(edit) = edit else { return };
    // Only an edit runs the command, so a divider between whole percents (a canvas drag, an agent's
    // value) stays exact while the panel shows it.
    edits::apply_edits(app, &edit, EditPath::Preview, |app, pct, phase| {
        let split = (pct / 100.0).clamp(0.0, 1.0);
        crate::panels::live_run(app, "Image Trace View", "imageTrace.setView", json!({ "view": TraceView::Split.id(), "split": split }), phase);
    });
}

/// The View select showing `current` → the view the user picked this pass.
fn view_pick(ui: &mut Ui, current: TraceView) -> Option<TraceView> {
    let entries: Vec<Entry<String>> = TraceView::ALL.iter().map(|v| Entry::new(v.id().to_string(), view_label(*v))).collect();
    let picked = Field::new(tl!("View"))
        .show(ui, |ui, _| Select::new(tl!("View"), Bind::given(current.id().to_string()), &entries).show(ui).events.committed().cloned())
        .inner
        .flatten()?;
    TraceView::ALL.iter().copied().find(|v| v.id() == picked)
}

/// Show the selected Image Trace objects with `view` (no new trace).
fn set_view(app: &mut VectorcraftApp, view: TraceView) {
    if let Err(e) = app.run("imageTrace.setView", json!({ "view": view.id() })) {
        app.ui.status = e;
    }
}

/// Run `id` with `params`; a failure goes to the status line.
fn run_reporting(app: &mut VectorcraftApp, id: &str, params: Value) {
    if let Err(e) = app.run(id, params) {
        app.status(e);
    }
}

/// While `down`, the selected Image Trace objects show their source image on the canvas, whatever
/// their View. The document and the undo history stay as they are. The panel's eye button next to
/// View calls it; per the public documentation of the panel, that control is pressed and held to
/// view the source image
/// (<https://web.archive.org/web/20250831090623/https://helpx.adobe.com/illustrator/using/image-trace.html>).
fn hold_source(app: &mut VectorcraftApp, down: bool) {
    let ids = app
        .session
        .active()
        .filter(|_| down)
        .map(|st| st.selection.objects.iter().copied().filter(|id| st.doc.node(*id).is_some_and(is_trace)).collect::<Vec<_>>())
        .filter(|ids| !ids.is_empty())
        .map(std::sync::Arc::new);
    if ids != app.ui.trace_hold_source {
        app.ui.trace_hold_source = ids;
        if let Some(ctx) = &app.ui_ctx {
            ctx.request_repaint();
        }
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

/// Also runs when the panel is closed: document changes and tool changes end the pick. Whether it ended it.
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

fn start_ignore_sampler(app: &mut VectorcraftApp) {
    if app.ui.trace_ignore_sampler.is_some() {
        cancel_ignore_sampler(app, true);
        return;
    }
    let Some((doc, target)) = target(app).object else { return };
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
    cancel_ignore_sampler(app, false);
    let list = presets(app);
    let t = target(app);
    let mut st = std::mem::take(&mut app.ui.image_trace_panel.shown);
    follow_preset_changes(app, &mut st, &list.presets);
    let changes: Vec<&TracePresetChange> = app.session.trace_preset_changes_since(0).collect();
    sync_with_object(&mut st, &t, &list.presets, &changes);
    st.params["ignoreColor"] = json!(color);
    st.preset = "Custom".into();
    let preview = app.ui.image_trace_preview;
    if t.is_trace || !preview {
        st.preview_off = !preview;
    }
    if t.is_trace && preview {
        trace(app, &mut st);
    }
    app.ui.image_trace_panel.shown = st;
    if let Some(ctx) = &app.ui_ctx {
        ctx.request_repaint();
    }
    // Complete the temporary tool without treating its return as a fresh explicit tool choice.
    // A miss above has no switch request and remains armed.
    app.return_to_tool(&s.back);
}

/// The selected Image Trace object's View row (Control bar, Properties).
pub(crate) fn view_row(app: &mut VectorcraftApp, ui: &mut Ui, view: TraceView) {
    if let Some(v) = view_pick(ui, view) {
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

/// The Image Trace button for a selected image (Control bar, Properties): its main part traces it
/// with the Default preset, its arrow lists the presets to trace it with.
pub(crate) fn trace_button(app: &mut VectorcraftApp, ui: &mut Ui) {
    let list = presets(app);
    let trace = CommandRef::with_params("imageTrace.make", json!({ "preset": "Default" }));
    let chosen = SplitButton::command(trace, SplitMenu::Entries(&list.entries))
        .label(tl!("Image Trace"))
        .arrow_label(tl!("Tracing Presets"))
        .tooltip(tl!("Trace the image with the Default preset"))
        .show(ui)
        .chosen;
    if let Some(name) = chosen {
        trace_with(app, &name);
    }
}

/// The selected Image Trace object's preset (Control bar, Properties), traced with the preset
/// `traced_with`, shown as the panel shows it: choosing another traces it again with that preset.
pub(crate) fn preset_dropdown(app: &mut VectorcraftApp, ui: &mut Ui, traced_with: &str) {
    let list = presets(app);
    let params = selected_trace_params(app);
    let changes: Vec<&TracePresetChange> = app.session.trace_preset_changes_since(0).collect();
    let current = listed_preset(traced_with, &params, &list.presets, &changes);
    let picked = Select::new(tl!("Preset"), Bind::given(current), &list.entries).placeholder(tl!("Custom")).show(ui).events.committed().cloned();
    if let Some(name) = picked {
        trace_with(app, &name);
    }
}

/// The Control bar for one selected Image Trace object: its preset, view, the Image Trace panel
/// and Expand, each a group that leaves the bar whole when it is short of room → whether it is one.
pub(crate) fn control_bar(app: &mut VectorcraftApp, ui: &mut Ui) -> bool {
    let Some((preset, view)) = selected_trace(app) else { return false };
    Divider::vertical().show(ui);
    Row::new().fit_rank(rank::GROUP).show(ui, |ui, _| {
        Field::new(tl!("Preset")).show(ui, |ui, _| preset_dropdown(app, ui, &preset));
    });
    Row::new().fit_rank(rank::GROUP).show(ui, |ui, _| view_row(app, ui, view));
    Row::new().fit_rank(rank::GROUP).show(ui, |ui, _| {
        if IconButton::new(IconId::new("image")).label(tl!("Image Trace")).tooltip(tl!("Image Trace")).show(ui).clicked() {
            open_settings(app);
        }
    });
    Row::new().fit_rank(rank::GROUP).show(ui, |ui, _| {
        if Button::new(tl!("Expand")).show(ui).clicked() {
            run_reporting(app, "imageTrace.expand", json!({}));
        }
    });
    true
}

fn trace(app: &mut VectorcraftApp, st: &mut TraceUi) {
    // A group the active document does not have traces with All, as the panel shows.
    if color_group(app, &st.params).is_none() && !st.params["colorGroup"].is_null() {
        st.params["colorGroup"] = Value::Null;
    }
    // The settings are complete, so with a preset that no longer exists (a deleted saved one) they
    // are traced over Default, and the object shows Custom.
    let list = presets(app);
    let known = find_preset(&list.presets, &st.preset).map(|p| p.0.as_str());
    let preset = known.filter(|_| st.preset != "Custom").unwrap_or("Default");
    let p = json!({ "preset": preset, "params": st.params, "view": st.view.id() });
    match crate::trace_runs::start(app, "imageTrace.make", p, true) {
        // Traced off the UI thread: the result comes to [`traced`].
        Ok(r) if r["background"] == true => {}
        Ok(r) => adopt(st, &target(app), &r),
        Err(e) => app.ui.status = e,
    }
}

/// The panel takes trace `r`'s counts and the traced object's stored settings `target` at once: Undo before
/// the next panel frame must still be recognized as a change, even when it returns to the previous
/// settings.
fn adopt(st: &mut TraceUi, target: &Target, r: &Value) {
    discard_retired_drafts(st, target.document);
    st.info = Some((r["paths"].as_u64().unwrap_or(0), r["anchors"].as_u64().unwrap_or(0), r["colors"].as_u64().unwrap_or(0)));
    if let Some(stored) = &target.stored {
        st.synced = stored.clone();
        st.object = target.object_key();
    }
}

/// A trace the panel started off the UI thread was applied with result `r` ([`crate::trace_runs`]).
pub(crate) fn traced(app: &mut VectorcraftApp, r: &Value) {
    let t = target(app);
    adopt(&mut app.ui.image_trace_panel.shown, &t, r);
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

/// Drop only drafts for content that Revert replaced in the active document.
fn discard_retired_drafts(st: &mut TraceUi, document: Option<(u64, u64)>) {
    let Some((uid, reverts)) = document else { return };
    st.drafts.retain(|(doc, generation, _), _| *doc != uid || *generation == reverts);
    if st.object.is_some_and(|(doc, generation, _)| doc == uid && generation != reverts) {
        st.object = None;
        st.synced = Value::Null;
    }
}

/// Bring the panel's settings `st` in line with the selected object's stored settings,
/// with the presets `all` and the saved preset changes `changes` of this session (oldest first); whether the object
/// came back with the settings that waited for it.
fn sync_with_object(st: &mut TraceUi, target: &Target, all: &[Preset], changes: &[&TracePresetChange]) -> bool {
    discard_retired_drafts(st, target.document);
    let (stored, object) = (target.stored.clone(), target.object_key());
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

/// Make `name` the panel's preset with its settings → whether the list has it.
fn choose_preset(st: &mut TraceUi, all: &[Preset], name: &str) -> bool {
    let Some(p) = all.iter().find(|p| p.0 == name) else { return false };
    st.preset = name.to_string();
    st.params = p.1.clone();
    true
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let list = presets(app);
    let t = target(app);
    // The panel's state is taken out while the frame runs, so the rows borrow it apart from `app`.
    let mut st = std::mem::take(&mut app.ui.image_trace_panel.shown);
    if st.params.is_null() {
        st.preset = "Default".into();
        st.params = list.presets.first().map(|p| p.1.clone()).unwrap_or_default();
    }
    // The saved preset changes since the last frame apply to the panel's settings before the selected object's
    // settings, which `sync_with_object` resolves against every change, are adopted.
    follow_preset_changes(app, &mut st, &list.presets);
    let restored = {
        let changes: Vec<&TracePresetChange> = app.session.trace_preset_changes_since(0).collect();
        sync_with_object(&mut st, &t, &list.presets, &changes)
    };
    let preview = app.ui.image_trace_preview;
    let mut actions = (false, false, false);
    let stack = Stack::new().label(tl!("Image Trace")).fill().scroll(Scroll::vertical());
    let env = Env::of(ui);
    let settings_row = env.tokens.size.control.md + env.tokens.space.gap.row;
    let footer = Stack::new().id_salt("trace-footer-measure").measure_in(ui, Proposal::Width(ui.available_width()), |ui, _| {
        show_footer(ui, preview, &t);
    });
    let compact_offset = ui.scope_id().with("trace-compact-offset");
    let settings = if ui.available_height() >= footer.y + settings_row {
        if !ui.is_sizing_pass() {
            ui.ctx().data_mut(|data| data.remove::<ScrollPos>(compact_offset));
        }
        stack.footer(|ui, _| actions = show_footer(ui, preview, &t)).show(ui, |ui, _| show_settings(app, ui, &mut st, &list, &t))
    } else {
        // When the footer moves into the scroll area, start with its actions in view.
        let before = ui.ctx().data(|data| data.get_temp(compact_offset)).unwrap_or_default();
        let mut offset = before;
        let shown = CompactSettings::new(Bind::bound(&mut offset)).show(ui, |ui, _| {
            actions = show_actions(ui, preview, &t);
            Divider::horizontal().show(ui);
            show_settings(app, ui, &mut st, &list, &t)
        });
        if offset != before && !ui.is_sizing_pass() {
            ui.ctx().data_mut(|data| data.insert_temp(compact_offset, offset));
        }
        shown
    };
    let (mut retrace, held) = settings.inner.unwrap_or_default();
    let (preview_toggled, trace_clicked, expand_clicked) = actions;
    if !ui.is_sizing_pass() {
        hold_source(app, t.any_trace && held);
    }
    if preview_toggled {
        run_reporting(app, "imageTrace.preview", json!({}));
    }
    if trace_clicked {
        trace(app, &mut st);
    }
    if expand_clicked {
        run_reporting(app, "imageTrace.expand", json!({}));
    }
    // Preview turned on (the checkbox, `imageTrace.preview` or `ui.set`): the selected object is
    // traced with the settings that waited in the panel. With no Image Trace object selected, it is
    // traced when it is selected again. An object whose settings waited while another object was
    // selected is traced when it is selected again with Preview on.
    let preview = app.ui.image_trace_preview;
    retrace |= preview && (st.preview_off || restored) && t.is_trace && st.params != st.synced["params"];
    if t.is_trace || !preview {
        st.preview_off = !preview;
    }
    cancel_disabled_ignore_sampler(app, &st.params);
    if retrace && t.is_trace && preview {
        trace(app, &mut st);
    }
    app.ui.image_trace_panel.shown = st;
}

/// The very short layout needs a bound offset so returning to it shows the actions first.
#[derive(craft::ui::contract::Component, craft::ui::contract::Props)]
#[component(container, unregistered)]
struct CompactSettings<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    offset: Bind<'a, ScrollPos>,
}

impl<'a> CompactSettings<'a> {
    fn new(offset: Bind<'a, ScrollPos>) -> Self {
        let props = ContainerProps::new().sizing(Sizing::Fill).layout(BodyLayout::stack()).scroll(Scroll::vertical());
        Self { common: Common::new(), props, offset }.label(tl!("Image Trace"))
    }
}

#[derive(Clone, Debug, PartialEq, ComponentStyle)]
struct CompactSettingsStyle {
    #[style(metrics)]
    metrics: Metrics,
}

impl Component for CompactSettings<'_> {
    const ROLE: Role = Role::GROUP;
    type Style = CompactSettingsStyle;
    type Output = egui::Response;

    fn style(_: &StyleCx<'_>, _: CompactSettingsStyleKey) -> CompactSettingsStyle {
        CompactSettingsStyle { metrics: Metrics::new() }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn scroll(&mut self) -> Option<BindMut<'_, ScrollPos>> {
        Some(self.offset.reborrow())
    }

    fn finish(self, cx: Cx<'_, Self>) -> egui::Response {
        cx.response.clone()
    }
}

impl Container for CompactSettings<'_> {}

fn show_footer(ui: &mut Ui, preview: bool, target: &Target) -> (bool, bool, bool) {
    Divider::horizontal().show(ui);
    show_actions(ui, preview, target)
}

fn show_actions(ui: &mut Ui, preview: bool, target: &Target) -> (bool, bool, bool) {
    let mut value = preview;
    let mut trace_clicked = false;
    let mut expand_clicked = false;
    Row::new().wrap(true).show(ui, |ui, _| {
        Checkbox::new(Bind::bound(&mut value)).label(tl!("Preview")).show(ui);
        let can_trace = target.is_trace || target.is_image;
        let mut trace_button = Button::new(tl!("Trace")).enabled(can_trace);
        if !can_trace {
            trace_button = trace_button.tooltip(tl!("Select an image to trace"));
        }
        trace_clicked = trace_button.show(ui).clicked();
        expand_clicked = Button::new(tl!("Expand")).enabled(target.any_trace).show(ui).clicked();
    });
    (value != preview, trace_clicked, expand_clicked)
}

fn show_settings(app: &mut VectorcraftApp, ui: &mut Ui, st: &mut TraceUi, list: &PresetList, t: &Target) -> (bool, bool) {
    let mut retrace = false;

    // A preset button chooses its preset as the Preset row does; the button of the panel's
    // preset shows as on.
    Row::new().wrap(true).show(ui, |ui, _| {
        for (label, name) in preset_buttons() {
            let was = st.preset == name;
            let mut on = was;
            ToggleButton::new(label, Bind::bound(&mut on)).tooltip(preset_label(name)).show(ui);
            if on && !was {
                retrace |= choose_preset(st, &list.presets, name);
            }
        }
    });
    let mut value = list.presets.iter().any(|p| p.0 == st.preset).then(|| st.preset.clone());
    let session = &app.session;
    let source = PresetSource::listed(&list.rows);
    let taken = |name: &str| session.trace_preset_taken(name, None).then_some(PresetError::NameTaken);
    let preset_row = NamedPresetRow::new(
        tl!("Preset"),
        PresetPicker::new(tl!("Preset"), Bind::bound(&mut value), &source, Bind::bound(&mut app.ui.image_trace_panel.picker))
            .field(PickerField::Name),
    )
    .style(NamedPresetStyle::Menu)
    .built_in(|name: &String| list.presets.iter().any(|p| p.3 && p.0 == *name))
    .check_name(NameCheck::of_host(&taken))
    .suggest_name(session.new_trace_preset_name())
    .show(ui);
    for event in preset_row.events {
        match event {
            NamedPresetEvent::Apply(name) => retrace |= choose_preset(st, &list.presets, &name),
            NamedPresetEvent::Create { name, .. } => run_reporting(app, "imageTrace.presets.save", json!({ "name": name, "params": st.params })),
            NamedPresetEvent::Rename { key, text, .. } => run_reporting(app, "imageTrace.presets.save", json!({ "name": key, "newName": text })),
            NamedPresetEvent::Delete(name) => run_reporting(app, "imageTrace.presets.delete", json!({ "name": name })),
            _ => {}
        }
    }
    // The preset's enhanced version (with Shapes and Auto Grouping) or its legacy one.
    if let Some((_, legacy, better, _)) = list.presets.iter().find(|p| p.0 == st.preset && !p.2.is_null()) {
        let on = *better == st.params;
        let mut value = on;
        Checkbox::new(Bind::bound(&mut value)).label(tl!("Enhanced")).show(ui);
        if value != on {
            st.params = if on { legacy.clone() } else { better.clone() };
            retrace = true;
        }
    }
    let (mut picked, mut held) = (None, false);
    Row::new().show(ui, |ui, _| {
        picked = view_pick(ui, st.view);
        let tip = tl!("Press and hold to view the source image");
        held = IconButton::new(IconId::new("eye"))
            .label(tip)
            .tooltip(tip)
            .selected(app.ui.trace_hold_source.is_some())
            .enabled(t.any_trace)
            .show(ui)
            .held;
    });
    if let Some(v) = picked {
        st.view = v;
        if t.any_trace {
            set_view(app, v);
        }
    }
    split_row(app, ui);

    let data = RowsData::of(app);
    let main = rows::main_rows(ui, &mut st.params, &data, RowsHost::Panel);
    let sampler = Sampler { enabled: t.is_trace || t.is_image, armed: app.ui.trace_ignore_sampler.is_some() };
    let advanced = Section::new(tl!("Advanced"))
        .open(Bind::bound(&mut app.ui.image_trace_advanced))
        .show(ui, |ui, _| rows::advanced_rows(ui, &mut st.params, &data, RowsHost::Panel, Some(sampler)))
        .inner
        .unwrap_or_default();
    // A setting is settled when its pick, check or drag ends. Switching to Custom while a slider
    // still drags would remove the preset's Enhanced row from under it.
    for RowsOutput { committed, .. } in [main, advanced] {
        if committed {
            st.preset = "Custom".into();
            retrace = true;
        }
    }
    if advanced.sampler_toggled {
        start_ignore_sampler(app);
    }
    if advanced.color_opened {
        cancel_ignore_sampler(app, true);
    }

    if let Some((p, a, c)) = st.info {
        Text::new(crate::i18n::fmt(
            tl!("Paths: {paths}    Anchors: {anchors}    Colors: {colors}"),
            &[("paths", &p.to_string()), ("anchors", &a.to_string()), ("colors", &c.to_string())],
        ))
        .shade(Shade::Muted)
        .show(ui);
    }
    (retrace, held)
}

/// The panel menu's rows: Release and Expand act on the selected Image Trace objects.
pub(crate) fn menu_rows(_app: &VectorcraftApp) -> Vec<PanelRow> {
    vec![
        PanelRow::Command(CommandRef::new("imageTrace.release")),
        PanelRow::Command(CommandRef::new("imageTrace.expand")),
        PanelRow::Separator,
        PanelRow::Command(CommandRef::new("ui.imageTrace.reset")),
    ]
}

/// Release and Expand need an Image Trace object in the selection: `trace_targets` refuses any other.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    let enabled = any_trace_selected(app);
    for id in ["imageTrace.release", "imageTrace.expand"] {
        let command = CommandRef::new(id);
        let state = states.get(&command).cloned().unwrap_or_else(CommandState::new).enabled(enabled);
        states.set(command, state);
    }
}

/// The panel's own menu commands.
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    match id {
        "ui.imageTrace.reset" => {
            app.ui.image_trace_panel.reset();
            Some(Ok(Value::Null))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests;
