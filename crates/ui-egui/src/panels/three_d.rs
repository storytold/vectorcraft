//! Window › 3D and Materials: edits the selection's 3D effect in place. The object types (Plane,
//! Extrude, Revolve, Inflate) apply a 3D effect to a selection without one (`effect.apply`) or
//! change the applied one (`effect.setParams` with `threeDType`). Below them are the 3D effect
//! dialogs' own Object, Rotation and Lighting controls; each change runs `effect.setParams` as one
//! undo step, and a drag previews and commits once, when the pointer is released.

use egui::Ui;
use serde_json::{Map, Value, json};
use vectorcraft_doc::{Effect, Node};
use vectorcraft_effects::{THREE_D_TYPES, is_three_d};

use super::{live_run, pstate, set_pstate};
use crate::VectorcraftApp;
use crate::state::Dialog;
use crate::widgets::{self, Live, dim_label};

/// The panel's id (`window.panel`).
pub const ID: &str = "threeD";

/// The name of the undo step a change makes, as the effect dialogs' commits name it.
const LABEL: &str = "Effect Options";

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

/// Commits the drag in progress, when the panel has one.
fn finish(app: &mut VectorcraftApp, ctx: &egui::Context) {
    if !pstate::<Map<String, Value>>(ctx, PENDING).is_empty() {
        app.session.commit_interaction().ok();
        app.sync_views();
    }
    set_pstate(ctx, PENDING, Map::<String, Value>::new());
    set_pstate(ctx, PLAN, None::<Plan>);
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
/// pointer is down (`dragging`) and commits them as one undo step when it is released. Settings
/// changed earlier in the same drag are sent again with them, to the objects the drag started
/// with, since each preview starts again from the document as it was before the drag. A drag
/// whose selection changes is committed first, and the change starts a new one.
pub(crate) fn send(app: &mut VectorcraftApp, ctx: &egui::Context, changed: Map<String, Value>, dragging: bool) {
    if changed.is_empty() {
        // A drag that ended without a change in its last frame is committed now.
        if !dragging {
            finish(app, ctx);
        }
        return;
    }
    let selection: Vec<u64> = app.session.active().map(|st| st.selection.objects.iter().map(|id| id.0).collect()).unwrap_or_default();
    let mut held: Option<Plan> = pstate(ctx, PLAN);
    if held.as_ref().is_some_and(|p| p.selection != selection) {
        finish(app, ctx);
        held = None;
    }
    let plan = held.unwrap_or_else(|| plan(app));
    let mut pending: Map<String, Value> = pstate(ctx, PENDING);
    pending.extend(changed);
    let steps: Vec<Value> = plan
        .targets
        .iter()
        .filter_map(|(id, t, same_lights)| {
            let takes = vectorcraft_effects::default_params(&t.effect).unwrap_or(Value::Null);
            let params: Map<String, Value> = pending
                .iter()
                .filter(|(k, _)| takes.get(k.as_str()).is_some() && (k.as_str() != "lights" || *same_lights))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            (!params.is_empty())
                .then(|| json!({"command": "effect.setParams", "params": {"ids": [id], "index": t.index, "item": t.item, "params": params}}))
        })
        .collect();
    let phase = if dragging { Live::Dragging } else { Live::Released };
    match steps.as_slice() {
        [] => {}
        [step] => live_run(app, LABEL, "effect.setParams", step["params"].clone(), phase),
        _ => live_run(app, LABEL, "command.batch", json!({"label": LABEL, "commands": steps}), phase),
    }
    if dragging {
        set_pstate(ctx, PENDING, pending);
        set_pstate(ctx, PLAN, Some(plan));
    } else {
        set_pstate(ctx, PENDING, Map::<String, Value>::new());
        set_pstate(ctx, PLAN, None::<Plan>);
    }
}

/// Panel state key of the [`Plan`] of the drag in progress.
const PLAN: &str = "three-d-plan";
/// Panel state key of the pass the panel was last drawn in.
const DRAWN: &str = "three-d-drawn";

/// Commits the panel's drag in progress when the panel was not drawn in this pass (it was closed
/// or hidden during the drag), so the drag is one undo step and no preview stays open into a later
/// pass. The app runs this at the end of every pass, after the panels.
pub(crate) fn end_stale_drag(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let drawn: Option<u64> = pstate(ctx, DRAWN);
    if drawn.is_some_and(|pass| pass != root_pass(ctx)) {
        finish(app, ctx);
        set_pstate(ctx, DRAWN, None::<u64>);
    }
}

/// The main window's pass in progress. A floating window draws inside it and counts its own
/// passes, so the panel's mark uses this one wherever the panel draws.
fn root_pass(ctx: &egui::Context) -> u64 {
    ctx.cumulative_pass_nr_for(egui::ViewportId::ROOT)
}

/// Panel state key of the settings changed during the drag in progress.
const PENDING: &str = "three-d-pending";
/// Panel state key of the controls' own state (the open tab, the selected light).
const UI_STATE: &str = "three-d-ui";

/// The 3D and Materials panel.
pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    set_pstate(&ctx, DRAWN, Some(root_pass(&ctx)));
    let target = target(app);
    let selected = app.session.active().is_some_and(|st| !st.selection.objects.is_empty());
    let current = target.as_ref().and_then(|t| type_of(&t.effect));
    let labels = [tl!("Plane"), tl!("Extrude"), tl!("Revolve"), tl!("Inflate")];
    let at = current.and_then(|kind| THREE_D_TYPES.iter().position(|(k, _)| *k == kind)).unwrap_or(usize::MAX);
    let picked = ui.add_enabled_ui(selected, |ui| widgets::tab_bar(ui, &labels, at)).inner;
    if let Some((kind, _)) = picked.and_then(|i| THREE_D_TYPES.get(i))
        && let Err(e) = set_type(app, kind)
    {
        app.ui.status = e;
    }
    ui.add_space(8.0);
    let Some(t) = target else {
        // The selection changed during a drag: the drag ends with what it previewed.
        finish(app, &ctx);
        dim_label(ui, tl!("Select objects, then pick an object type to give them a 3D effect."));
        return;
    };
    // The applied effect's settings in the dialog's form, with the controls' own state.
    let mut fields = vectorcraft_effects::merged_params(&t.effect, &t.params).as_object().cloned().unwrap_or_default();
    mark_mixed(app, &mut fields);
    fields.extend(pstate::<Map<String, Value>>(&ctx, UI_STATE));
    fields.insert("__effect".into(), json!(t.effect));
    fields.insert("__panel".into(), json!(true));
    let mut d = Dialog { kind: ID.into(), fields };
    let before = crate::dialogs::form::params(&d);
    crate::dialogs::revolve::controls(app, ui, &mut d);
    let state: Map<String, Value> = d
        .fields
        .iter()
        .filter(|(k, _)| k.starts_with("__") && !matches!(k.as_str(), "__effect" | "__panel"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    set_pstate(&ctx, UI_STATE, state);
    let after = crate::dialogs::form::params(&d);
    let changed = changed_settings(&before, &after);
    let dragging = ctx.input(|i| i.pointer.any_down());
    send(app, &ctx, changed, dragging);
}

/// Sets to null each setting in `fields` (the first selected object's 3D effect) that the 3D effect
/// of another selected object has with another value. The controls then show it at its default
/// (a checkbox shows a dash) with its reset dot filled, and its reset dot, a menu choice, a checkbox click, or a number other
/// than the default shown is a change sent to every effect. The light list (`lights`, where null
/// is the single light) is left as it is.
pub(crate) fn mark_mixed(app: &VectorcraftApp, fields: &mut Map<String, Value>) {
    let Some(st) = app.session.active() else { return };
    for other in st.selection.objects.iter().skip(1).filter_map(|id| find(st.doc.node(*id)?)) {
        let other = vectorcraft_effects::merged_params(&other.effect, &other.params);
        for (key, value) in fields.iter_mut().filter(|(k, _)| k.as_str() != "lights") {
            if other.get(key.as_str()).is_some_and(|v| v != value) {
                *value = Value::Null;
            }
        }
    }
}

/// The settings whose values differ between `before` and `after`, with their values in `after`.
pub(crate) fn changed_settings(before: &Value, after: &Value) -> Map<String, Value> {
    let Some(after) = after.as_object() else { return Map::new() };
    after.iter().filter(|(k, v)| before.get(k.as_str()) != Some(*v)).map(|(k, v)| (k.clone(), v.clone())).collect()
}

#[cfg(test)]
#[path = "tests_three_d.rs"]
mod tests;
