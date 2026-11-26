//! Window › 3D and Materials: edits the selection's 3D effect in place. The object types (Plane,
//! Extrude, Revolve, Inflate) apply a 3D effect to a selection without one (`effect.apply`) or
//! change the applied one (`effect.setParams` with `threeDType`). Below them are the 3D effect
//! dialogs' own Object, Rotation and Lighting rows ([`crate::dialogs::revolve::params`]) and the
//! panel's Materials page ([`materials`]); each change runs `effect.setParams` as one undo step,
//! and a drag previews and commits once, when the gesture ends. While the Materials page is open,
//! the artboard shows a handle on each mapped graphic ([`handles`]), which drags it over its face.
//! At the bottom, Export 3D Object adds the selected objects that have a 3D effect to Asset Export
//! as 3D assets (`assets.add3D`) and shows that panel with their assets selected.

use std::borrow::Cow;
use std::sync::{Arc, RwLock};

use craft::behavior::EditEvents;
use craft::behavior::{CancelReason, EditEvent, GestureId, SelectionChange, keyed};
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, CollectionEvent, Entry, Props as _, Show as _, ShowCollection as _, Trigger, clicked};
use craft::ui::display::{Shade, Text};
use craft::ui::menu::Menu;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamRows};
use craft::ui::tabs::{Tab, TabKind, TabStrip};
use egui::Ui;
use serde_json::{Map, Value, json};
use vectorcraft_doc::{Effect, Node, NodeId, NodeKind};
use vectorcraft_effects::{BEVEL_NAME, BEVEL_PROFILE, CLASSIC_EXTRUDE, CLASSIC_REVOLVE, EXTRUDE, INFLATE, THREE_D_TYPES, is_three_d};

use super::{Live, live_run};
use crate::VectorcraftApp;
use crate::dialogs::form;
use crate::dialogs::revolve::{self, Place};
use crate::dialogs::three_d as host;
use crate::dialogs::three_d_materials as materials;

pub(crate) mod handles;
pub(crate) use handles::FrameKey;

/// The panel's id (`window.panel`).
pub const ID: &str = "threeD";

/// The name of the undo step a change makes, as the effect dialogs' commits name it.
const LABEL: &str = "Effect Options";

/// The field that holds the page the panel shows.
const PAGE: &str = "__page";
/// The field that holds the selected light.
const LIGHT: &str = "__light";
/// The field that holds Bevel Shape's Edit Bevels… request.
const EDIT_BEVELS: &str = "__editBevels";

/// The panel's view state and the edit in progress, kept on the app between passes.
#[derive(Default)]
pub(crate) struct Model {
    /// The page the rows show (`rotation`, `materials`, `lighting`); empty until one is picked.
    page: String,
    /// The selected light of the Lighting page.
    light: usize,
    /// What the gesture in progress goes to, taken when it began.
    plan: Option<Plan>,
    /// The gesture whose preview is open.
    gesture: Option<GestureId>,
    /// The settings changed during the gesture, sent again with each preview.
    pending: Map<String, Value>,
    /// The graphics frame the handles last built, with what it was built for.
    frames: Option<(FrameKey, Arc<Option<vectorcraft_effects::GraphicFrame>>)>,
    /// The mapped graphic the Materials page edits.
    graphic: usize,
    /// The settings as they were when the gesture began.
    before: Value,
}

/// The selection's 3D effect: the appearance item whose effects hold it (`None`: the object's
/// own effects), its index there, and its id and parameters.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Target {
    pub item: Option<usize>,
    pub index: usize,
    pub effect: String,
    pub params: Value,
}

/// The 3D effect of the first selected object: its own effects first, then its fills' and
/// strokes'. `None` without a selection or without a 3D effect.
pub(crate) fn target(app: &VectorcraftApp) -> Option<Target> {
    let st = app.session.active()?;
    let n = st.selection.objects.first().and_then(|id| st.doc.node(*id))?;
    find(n)
}

fn find(n: &Node) -> Option<Target> {
    let in_list = |fx: &[Effect], item: Option<usize>| {
        fx.iter().position(|e| is_three_d(&e.id)).map(|index| Target { item, index, effect: fx[index].id.clone(), params: fx[index].params.clone() })
    };
    in_list(&n.appearance.effects, None).or_else(|| n.appearance.items.iter().enumerate().find_map(|(k, item)| in_list(item.effects(), Some(k))))
}

/// The panel's object type of effect `id` (`plane`, `extrude`, `revolve` or `inflate`).
pub(crate) fn type_of(id: &str) -> Option<&'static str> {
    THREE_D_TYPES.iter().find(|(_, effect)| *effect == id).map(|(kind, _)| *kind)
}

/// Picks object type `kind` for each selected object: changes its 3D effect into that type, or
/// applies that type's effect to it when it has none, all as one undo step. An object whose 3D
/// effect already has that type is left as it is (`effect.setParams` records nothing for it).
pub(crate) fn set_type(app: &mut VectorcraftApp, kind: &str) -> Result<(), String> {
    let Some(&(_, effect)) = THREE_D_TYPES.iter().find(|(k, _)| *k == kind) else {
        let key = "threeDType";
        return Err(format!("`{key}` must be one of {}", THREE_D_TYPES.map(|(k, _)| k).join(", ")));
    };
    let steps: Vec<(&str, Value)> = app.session.active().map_or_else(Vec::new, |st| {
        st.selection
            .objects
            .iter()
            .filter_map(|id| st.doc.node(*id).map(|n| (id.0, find(n))))
            .map(|(id, t)| match t {
                Some(t) => ("effect.setParams", json!({"ids": [id], "index": t.index, "item": t.item, "threeDType": kind})),
                None => ("effect.apply", json!({"ids": [id], "effect": effect, "item": null})),
            })
            .collect()
    });
    match steps.as_slice() {
        // Without a selection, `effect.apply` says why nothing changed.
        [] => app.run("effect.apply", json!({"effect": effect, "item": null})).map(|_| ()),
        [(id, p)] => app.run(id, p.clone()).map(|_| ()),
        _ => {
            app.run("edit.undoGroup.begin", json!({"label": LABEL}))?;
            for (id, p) in &steps {
                if let Err(e) = app.run(id, p.clone()) {
                    app.run("edit.undoGroup.end", json!({"cancel": true})).ok();
                    return Err(e);
                }
            }
            app.run("edit.undoGroup.end", json!({})).map(|_| ())
        }
    }
}

/// Commits the gesture in progress, when the panel has one.
fn finish(app: &mut VectorcraftApp) {
    let m = &mut app.three_d;
    let previewed = !m.pending.is_empty();
    m.pending.clear();
    m.plan = None;
    m.gesture = None;
    m.before = Value::Null;
    if previewed {
        app.session.commit_interaction().ok();
        app.sync_views();
    }
}

/// Rolls back what the gesture in progress previewed.
fn rollback(app: &mut VectorcraftApp) {
    let m = &mut app.three_d;
    let previewed = !m.pending.is_empty();
    m.pending.clear();
    m.plan = None;
    m.gesture = None;
    m.before = Value::Null;
    if previewed {
        app.session.cancel_interaction().ok();
        app.sync_views();
    }
}

/// What a change goes to, taken when it starts (before any preview): the selected objects' ids,
/// and each selected object with a 3D effect, the effect, and whether its list of lights matches
/// the first one's (a light list edited in the panel replaces only those).
#[derive(Clone, Debug)]
struct Plan {
    selection: Vec<u64>,
    targets: Vec<(u64, Target, bool)>,
}

fn plan(app: &VectorcraftApp) -> Plan {
    let Some(st) = app.session.active() else { return Plan { selection: vec![], targets: vec![] } };
    let selection: Vec<u64> = st.selection.objects.iter().map(|id| id.0).collect();
    let found: Vec<(u64, Target)> = st.selection.objects.iter().filter_map(|id| Some((id.0, find(st.doc.node(*id)?)?))).collect();
    let lights = |t: &Target| vectorcraft_effects::merged_params(&t.effect, &t.params).get("lights").cloned();
    let first = found.first().map(|(_, t)| lights(t));
    let targets = found
        .into_iter()
        .map(|(id, t)| {
            let same = first.as_ref() == Some(&lights(&t));
            (id, t, same)
        })
        .collect();
    Plan { selection, targets }
}

/// Sends the settings `changed` to the 3D effect of each selected object (only the settings its
/// effect takes; objects without a 3D effect are left as they are): previews them while the
/// gesture runs (`Live::Dragging`) and commits them as one undo step when it ends. Settings
/// changed earlier in the same gesture are sent again with them, to the objects the gesture
/// started with, since each preview starts again from the document as it was before the gesture.
/// A gesture whose selection changes is committed first, and the change starts a new one.
pub(crate) fn send(app: &mut VectorcraftApp, changed: Map<String, Value>, phase: Live) {
    let dragging = matches!(phase, Live::Dragging);
    if changed.is_empty() {
        // A gesture that ended without a change in its last pass is committed now.
        if !dragging {
            finish(app);
        }
        return;
    }
    let selection: Vec<u64> = app.session.active().map(|st| st.selection.objects.iter().map(|id| id.0).collect()).unwrap_or_default();
    if app.three_d.plan.as_ref().is_some_and(|p| p.selection != selection) {
        finish(app);
    }
    let held = app.three_d.plan.take().unwrap_or_else(|| plan(app));
    let mut pending = std::mem::take(&mut app.three_d.pending);
    pending.extend(changed);
    let steps: Vec<Value> = held
        .targets
        .iter()
        .filter_map(|(id, t, same_lights)| {
            let takes = vectorcraft_effects::default_params(&t.effect).unwrap_or(Value::Null);
            // A custom bevel's name and points go with `bevel` to the effects that take one.
            let custom_bevel = |k: &str| (k == BEVEL_NAME || k == BEVEL_PROFILE) && takes.get("bevel").is_some();
            let params: Map<String, Value> = pending
                .iter()
                .filter(|(k, _)| (takes.get(k.as_str()).is_some() || custom_bevel(k)) && (k.as_str() != "lights" || *same_lights))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            (!params.is_empty())
                .then(|| json!({"command": "effect.setParams", "params": {"ids": [id], "index": t.index, "item": t.item, "params": params}}))
        })
        .collect();
    match steps.as_slice() {
        [] => {}
        [step] => live_run(app, LABEL, "effect.setParams", step["params"].clone(), phase),
        _ => live_run(app, LABEL, "command.batch", json!({"label": LABEL, "commands": steps}), phase),
    }
    if dragging {
        app.three_d.pending = pending;
        app.three_d.plan = Some(held);
    }
}

/// Applies the edits of one pass of the panel's rows: `Begin` takes the plan and the settings as
/// they were (`input`, the values the rows were shown with), `Update` previews, `Commit` and a
/// one-step `Set` commit. A cancel rolls back, except the ones that keep what was previewed: the
/// document moved under the gesture, or the panel was removed (the gesture commits as one step).
pub(crate) fn apply(app: &mut VectorcraftApp, params: &[Param], input: &DialogFields, events: &EditEvents<DialogFields>) {
    let settings = |fields: &DialogFields| Value::Object(form::json_of(params, fields));
    // The one-step changes of a pass (several key presses in one frame) are one undo step, to where the last left them.
    let mut set = None;
    for event in events {
        match event {
            EditEvent::Begin { id, .. } => {
                // A gesture still running (its selection changed under it) ends with what it previewed.
                finish(app);
                app.three_d.gesture = Some(*id);
                app.three_d.before = settings(input);
                app.three_d.plan = Some(plan(app));
            }
            EditEvent::Update { value, .. } => {
                let changed = changed_settings(&app.three_d.before, &settings(value));
                send(app, changed, Live::Dragging);
            }
            EditEvent::Commit { after, .. } => {
                let changed = changed_settings(&app.three_d.before, &settings(after));
                send(app, changed, Live::Released);
            }
            EditEvent::Set { after, .. } => set = Some(after),
            EditEvent::Cancel { reason, .. } => {
                if matches!(reason, CancelReason::Removed) || reason.document_moved() {
                    send(app, Map::new(), Live::Released);
                } else {
                    rollback(app);
                }
            }
            _ => {}
        }
    }
    if let Some(after) = set {
        send(app, changed_settings(&settings(input), &settings(after)), Live::Released);
    }
}

/// The 3D and Materials panel.
pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let target = target(app);
    let selected = app.session.active().is_some_and(|st| !st.selection.objects.is_empty());
    let current = target.as_ref().and_then(|t| type_of(&t.effect));
    let labels = [tl!("Plane"), tl!("Extrude"), tl!("Revolve"), tl!("Inflate")];
    let tabs: Vec<(&'static str, &'static str)> = THREE_D_TYPES.iter().map(|(kind, _)| *kind).zip(labels).collect();
    let source = keyed(&tabs[..], |tab: &(&'static str, &'static str)| tab.0).with_text(|tab| Cow::Borrowed(tab.1));
    let mut strip = TabStrip::new(tl!("Object type")).kind(TabKind::Text).enabled(selected);
    if let Some(kind) = current {
        strip = strip.shown(kind);
    }
    let out = strip.show_items(ui, &source, &mut |key, _| Tab::new(tabs.iter().find(|tab| tab.0 == *key).map_or("", |tab| tab.1)));
    for event in out.response.events {
        if let CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } = event
            && let Some(kind) = keys.last()
            && let Err(e) = set_type(app, kind)
        {
            app.ui.status = e;
        }
    }
    let Some(t) = target else {
        // The selection changed during a gesture: it ends with what it previewed.
        finish(app);
        Text::new(tl!("Select objects, then pick an object type to give them a 3D effect.")).shade(Shade::Faint).show(ui);
        // The Graphics stay at hand, so art can be added before the 3D object is picked.
        let data = Arc::new(RwLock::new(host::data(app, None, &Map::new())));
        let params = [materials::row(data)];
        let out = ParamRows::new(tl!("Graphics"), &params, Bind::given(DialogFields::new())).show(ui);
        if let Some(action) = out.values.as_ref().and_then(action_of) {
            run_action(app, action);
        }
        export_button(app, ui);
        return;
    };
    if takes_render(&t.effect) {
        render_settings(app, ui, &t);
    }
    let (params, fields) = rows(app, &t);
    let out = ParamRows::new(tl!("3D and Materials"), &params, Bind::given(fields.clone())).show(ui);
    apply(app, &params, &fields, &out.events);
    let mut edit_bevels = false;
    let mut action = None;
    if let Some(values) = &out.values {
        if let Some(page) = values.text(PAGE) {
            app.three_d.page = page.to_string();
        }
        if let Some(light) = values.int(LIGHT) {
            app.three_d.light = usize::try_from(light).unwrap_or(0);
        }
        if let Some(graphic) = values.int(materials::SELECTED) {
            app.three_d.graphic = usize::try_from(graphic).unwrap_or(0);
        }
        edit_bevels = values.contains(EDIT_BEVELS);
        action = action_of(values);
    }
    // Bevel Shape's Edit Bevels…: the bevels file opens as a document of its own.
    if edit_bevels {
        finish(app);
        if let Err(e) = app.run("threeD.bevels.edit", json!({})) {
            app.ui.status = e;
        }
        return;
    }
    if quick_actions_shown(&t.effect, &app.three_d.page) {
        Text::new(tl!("Quick Actions")).show(ui);
        let tip = tl!("Replace the selected 3D objects with the edges of their faces, as stroked paths.");
        if clicked(&Button::new(tl!("Expand as Wireframes")).tooltip(tip).show(ui)) {
            finish(app);
            if let Err(e) = expand_as_wireframes(app) {
                app.ui.status = e;
            }
        }
    }
    if let Some(action) = action {
        run_action(app, action);
    }
    export_button(app, ui);
}

/// The panel's rows for the applied effect `t` and the values they show: the effect's settings over the panel's view
/// state, the settings another selected effect has with another value marked mixed.
fn rows(app: &VectorcraftApp, t: &Target) -> (Vec<Param>, DialogFields) {
    let merged = vectorcraft_effects::merged_params(&t.effect, &t.params).as_object().cloned().unwrap_or_default();
    let data = host::data(app, Some(&t.effect), &merged);
    let mut json = merged;
    json.insert(PAGE.into(), json!(app.three_d.page));
    json.insert(LIGHT.into(), json!(app.three_d.light));
    json.insert(materials::SELECTED.into(), json!(app.three_d.graphic));
    // The rows depend on the values (the page, which settings the selected effects disagree on) and the values' types
    // on the rows, so the first pass over the bare values only fixes the rows.
    let mut bare = form::fields_of(&[], &json);
    mark_mixed(app, &mut bare);
    let params = revolve::params(&data, &bare, Place::Panel);
    let mut fields = form::fields_of(&params, &json);
    mark_mixed(app, &mut fields);
    (params, fields)
}

/// What the Materials row asked for in `fields`, if anything.
fn action_of(fields: &DialogFields) -> Option<materials::Action> {
    match fields.get(materials::ACTION)? {
        FieldValue::Json(v) => materials::Action::from_value(v),
        _ => None,
    }
}

/// The Export 3D Object button, available while a selected object has a 3D effect that
/// `assets.add3D` exports ([`can_export_3d_shown`]). An error goes to the status bar.
fn export_button(app: &mut VectorcraftApp, ui: &mut Ui) {
    let any = can_export_3d_shown(app);
    if clicked(&Button::new(tl!("Export 3D Object")).enabled(any).show(ui)) {
        finish(app);
        if let Err(e) = export_3d_object(app) {
            app.ui.status = e;
        }
    }
}

/// Does a selected object have a 3D effect that `assets.add3D` exports
/// ([`vectorcraft_effects::exports_three_d`])? A path of a compound path counts as the compound
/// path, which holds the effects.
pub(crate) fn can_export_3d(app: &VectorcraftApp) -> bool {
    let Some(st) = app.session.active() else { return false };
    let compound = |id: NodeId| st.doc.parent_of(id).filter(|p| st.doc.node(*p).is_some_and(|n| matches!(n.kind, NodeKind::Compound { .. })));
    st.selection.objects.iter().any(|id| st.doc.node(compound(*id).unwrap_or(*id)).is_some_and(vectorcraft_effects::exports_three_d))
}

/// [`can_export_3d`], worked out once per (document uid, revision)
/// ([`crate::caches::CanvasCache::three_d_exportable`]): the panel reads it every frame, and Select
/// All can select hundreds of thousands of objects.
pub(crate) fn can_export_3d_shown(app: &mut VectorcraftApp) -> bool {
    let Some(st) = app.session.active() else { return false };
    let key = (st.uid, st.revision);
    if let Some((k, any)) = app.canvas.three_d_exportable
        && k == key
    {
        return any;
    }
    let any = can_export_3d(app);
    app.canvas.three_d_exportable = Some((key, any));
    any
}

/// Export 3D Object: adds the selected objects that have a 3D effect to Asset Export as 3D assets
/// (`assets.add3D` on the selection, one undo step, none when each is an asset already), selects
/// their assets there and shows the Asset Export panel. The command leaves out the selected
/// objects without a 3D effect and those inside another selected 3D object.
pub(crate) fn export_3d_object(app: &mut VectorcraftApp) -> Result<(), String> {
    use super::asset_export;
    asset_export::add_3d(app, None)?;
    if !crate::panel_docking::visible(app, asset_export::ID) {
        app.run("window.panel", json!({"panel": asset_export::ID}))?;
    }
    Ok(())
}

/// Does 3D effect `id` take render settings (`render`)? Extrude & Bevel, Revolve, Inflate and Rotate
/// do; the 3D (Classic) effects have a Wireframe surface of their own.
fn takes_render(id: &str) -> bool {
    vectorcraft_effects::default_params(id).is_some_and(|d| d.get(vectorcraft_effects::RENDER).is_some())
}

/// Is target `t`'s 3D effect drawn as a wireframe (`render.mode` is `wireframe`)?
pub(crate) fn wireframe_on(t: &Target) -> bool {
    let p = vectorcraft_effects::merged_params(&t.effect, &t.params);
    p.get(vectorcraft_effects::RENDER).and_then(|r| r.get("mode")).and_then(Value::as_str) == Some("wireframe")
}

/// The Render Settings menu at the top of the panel: Wireframe, checked while the first selected
/// object's 3D effect draws the edges of its faces. Choosing it turns wireframe on or off for every
/// selected object's 3D effect ([`set_wireframe`]).
fn render_settings(app: &mut VectorcraftApp, ui: &mut Ui, t: &Target) {
    let on = wireframe_on(t);
    let opener = Button::new(tl!("Render Settings")).show(ui);
    let entries = [Entry::new((), tl!("Wireframe")).checked(on)];
    if Menu::new(tl!("Render Settings")).values().opened_by(&opener, Trigger::Press).pick(ui, &entries).picked.is_some() {
        set_wireframe(app, !on);
    }
}

/// Render Settings › Wireframe: sets `render.mode` of each selected object's 3D effect to
/// `wireframe` (`on`) or `shaded`, as one undo step (`effect.setParams`). Objects whose effect does
/// not take render settings are left as they are.
pub(crate) fn set_wireframe(app: &mut VectorcraftApp, on: bool) {
    finish(app);
    let mut changed = Map::new();
    changed.insert(vectorcraft_effects::RENDER.into(), json!({"mode": if on { "wireframe" } else { "shaded" }}));
    send(app, changed, Live::Released);
}

/// Runs what a click in the Materials page's Graphics asked for: maps the graphic onto the selected
/// objects, its row then selected, or keeps art as a new graphic. An error goes to the status bar.
fn run_action(app: &mut VectorcraftApp, action: materials::Action) {
    finish(app);
    let result = match &action {
        materials::Action::Map(name) => app.run("threeD.mapGraphic", json!({"symbol": name})),
        materials::Action::Add(Some(ids)) => app.run("threeD.addGraphic", json!({"ids": ids})),
        materials::Action::Add(None) => app.run("threeD.addGraphic", json!({})),
    };
    match result {
        Ok(_) => {
            if let materials::Action::Map(_) = action
                && let Some(t) = target(app)
            {
                let count = vectorcraft_effects::mapped_graphics(&vectorcraft_effects::merged_params(&t.effect, &t.params)).len();
                set_selected_graphic(app, count.saturating_sub(1));
            }
        }
        Err(e) => app.ui.status = e,
    }
}

/// Makes the mapped graphic at `index` the one the Materials page edits.
fn set_selected_graphic(app: &mut VectorcraftApp, index: usize) {
    app.three_d.graphic = index;
}

/// Does the panel show its Quick Actions? On the Object page of the effects that have one, and on
/// the first page of the others (Revolve, Rotate), never on Lighting.
fn quick_actions_shown(effect: &str, page: &str) -> bool {
    let object_tab = [EXTRUDE, INFLATE, CLASSIC_EXTRUDE, CLASSIC_REVOLVE].contains(&effect);
    page != "lighting" && (!object_tab || page != "rotation")
}

/// Quick Actions › Expand as Wireframes: each selected object with a 3D effect becomes a group of
/// the edges of its faces (`effect.expandAsWireframes`), one undo step.
pub(crate) fn expand_as_wireframes(app: &mut VectorcraftApp) -> Result<(), String> {
    app.run("effect.expandAsWireframes", json!({})).map(|_| ())
}

/// Marks in `fields` (the first selected object's 3D effect) each setting that the 3D effect of
/// another selected object has with another value. The rows then show it blank (a checkbox shows a
/// dash), and an edit of it is a change sent to every effect. The light list (`lights`, where null
/// is the single light) is left as it is.
pub(crate) fn mark_mixed(app: &VectorcraftApp, fields: &mut DialogFields) {
    let Some(st) = app.session.active() else { return };
    let mut effects =
        st.selection.objects.iter().filter_map(|id| find(st.doc.node(*id)?)).map(|t| vectorcraft_effects::merged_params(&t.effect, &t.params));
    let Some(first) = effects.next() else { return };
    for other in effects {
        let differing: Vec<&String> = other
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(key, value)| key.as_str() != "lights" && first.get(key.as_str()).is_some_and(|v| v != *value))
            .map(|(key, _)| key)
            .collect();
        for key in differing {
            fields.set_mixed(key.as_str());
        }
    }
}

/// The settings whose values differ between `before` and `after`, with their values in `after`.
/// A change to a custom bevel (`bevel` `"custom"`, its name or its points) sends all three, so an
/// effect of a mixed selection that had none takes the whole bevel.
pub(crate) fn changed_settings(before: &Value, after: &Value) -> Map<String, Value> {
    let Some(after) = after.as_object() else { return Map::new() };
    let mut changed: Map<String, Value> =
        after.iter().filter(|(k, v)| before.get(k.as_str()) != Some(*v)).map(|(k, v)| (k.clone(), v.clone())).collect();
    let custom = ["bevel", BEVEL_NAME, BEVEL_PROFILE];
    if after.get("bevel").and_then(Value::as_str) == Some("custom") && custom.iter().any(|k| changed.contains_key(*k)) {
        for k in custom {
            if let Some(v) = after.get(k) {
                changed.insert(k.into(), v.clone());
            }
        }
    }
    changed
}

#[cfg(test)]
#[path = "tests_three_d.rs"]
mod tests;
