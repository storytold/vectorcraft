//! Window › 3D and Materials: edits the selection's 3D effect in place. The object types (Plane,
//! Extrude, Revolve, Inflate) apply a 3D effect to a selection without one (`effect.apply`) or
//! change the applied one (`effect.setParams` with `threeDType`). Below them are the 3D effect
//! dialogs' own Object, Rotation and Lighting controls and the panel's Materials tab
//! ([`materials`]); each change runs `effect.setParams` as one undo step, and a drag previews and
//! commits once, when the pointer is released. While the Materials tab is open, the artboard shows
//! a handle on each mapped graphic, which drags it over its face.

use egui::Ui;
use serde_json::{Map, Value, json};
use vectorcraft_doc::{Effect, Node};
use vectorcraft_effects::{BEVEL_NAME, BEVEL_PROFILE, CLASSIC_EXTRUDE, CLASSIC_REVOLVE, EXTRUDE, INFLATE, THREE_D_TYPES, is_three_d};

use super::{live_run, pstate, set_pstate};
use crate::VectorcraftApp;
use crate::dialogs::three_d_materials as materials;
use crate::state::Dialog;
use crate::theme::Tokens;
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
        // The Graphics stay at hand, so art can be added before the 3D object is picked.
        ui.add_space(10.0);
        if let Some(action) = materials::library(app, ui, false) {
            run_action(app, &ctx, action);
        }
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
    let edit_bevels = d.fields.remove("__editBevels").is_some();
    let action = d.fields.remove(materials::ACTION).and_then(|v| materials::Action::from_value(&v));
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
    // Bevel Shape's Edit Bevels…: the bevels file opens as a document of its own.
    if edit_bevels {
        finish(app, &ctx);
        if let Err(e) = app.run("threeD.bevels.edit", json!({})) {
            app.ui.status = e;
        }
        return;
    }
    if quick_actions_shown(&d) {
        ui.add_space(10.0);
        let t = Tokens::get(ui.ctx());
        ui.label(egui::RichText::new(tl!("Quick Actions")).font(crate::theme::semibold(13.0)).color(t.text));
        let tip = tl!("Replace the selected 3D objects with the edges of their faces, as stroked paths.");
        if ui.button(tl!("Expand as Wireframes")).on_hover_text(tip).clicked() {
            finish(app, &ctx);
            if let Err(e) = expand_as_wireframes(app) {
                app.ui.status = e;
            }
        }
    }
    if let Some(action) = action {
        run_action(app, &ctx, action);
    }
}

/// Runs what a click in the Materials tab's Graphics asked for: maps the graphic onto the selected
/// objects, its row then selected, or keeps art as a new graphic. An error goes to the status bar.
fn run_action(app: &mut VectorcraftApp, ctx: &egui::Context, action: materials::Action) {
    finish(app, ctx);
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
                set_selected_graphic(ctx, count.saturating_sub(1));
            }
        }
        Err(e) => app.ui.status = e,
    }
}

/// Makes the mapped graphic at `index` the one the Materials tab edits.
fn set_selected_graphic(ctx: &egui::Context, index: usize) {
    let mut state: Map<String, Value> = pstate(ctx, UI_STATE);
    state.insert(materials::SELECTED.into(), json!(index));
    set_pstate(ctx, UI_STATE, state);
}

/// Is the panel showing its Materials tab? It was drawn in this pass or the last one, so a canvas
/// drawn before the panel in the same pass still sees it.
fn materials_open(ctx: &egui::Context) -> bool {
    let drawn: Option<u64> = pstate(ctx, DRAWN);
    let state: Map<String, Value> = pstate(ctx, UI_STATE);
    drawn.is_some_and(|pass| pass + 1 >= root_pass(ctx))
        && state.get("__tab").and_then(Value::as_str) == Some("materials")
        && state.get("__gizmo").and_then(Value::as_str) != Some("light")
}

/// What [`graphic_handles`] gives: the object's frame, its graphics, and each seen graphic's index
/// and page point.
type Handles = (vectorcraft_effects::GraphicFrame, Vec<vectorcraft_effects::MappedGraphic>, Vec<(usize, vectorcraft_geom::Point)>);

/// The handles of the graphics mapped onto the first selected object, while the Materials tab is
/// open: each graphic's index, its face and where its center lands on the page, for the graphics
/// whose face the viewer sees, and none while a geometry effect follows a selected object's 3D effect.
/// With them, the object's [`vectorcraft_effects::GraphicFrame`] and the
/// graphics.
fn graphic_handles(app: &VectorcraftApp, ctx: &egui::Context) -> Option<Handles> {
    if !materials_open(ctx) {
        return None;
    }
    let st = app.session.active()?;
    let n = st.selection.objects.first().and_then(|id| st.doc.node(*id))?;
    let t = find(n)?;
    // The frame of the effect whose graphics are listed: none while that effect is hidden.
    let frame = vectorcraft_effects::graphic_frame_at(Some(&st.doc), n, t.item, t.index)?;
    // A geometry effect after the 3D effect of any selected object moves or bends its drawn
    // graphics away from their places in the frame: no handles, so a drag does not put a graphic
    // away from the pointer.
    let reshaped = |n: &Node| find(n).is_some_and(|t| vectorcraft_effects::graphics_reshaped_after(n, t.item, t.index));
    if st.selection.objects.iter().filter_map(|id| st.doc.node(*id)).any(reshaped) {
        return None;
    }
    let list = vectorcraft_effects::mapped_graphics(&vectorcraft_effects::merged_params(&t.effect, &t.params));
    // Selected objects with different graphics show no handles: a drag sends the whole list.
    let graphics_of = |t: Target| vectorcraft_effects::mapped_graphics(&vectorcraft_effects::merged_params(&t.effect, &t.params));
    if st.selection.objects.iter().skip(1).filter_map(|id| find(st.doc.node(*id)?)).any(|o| graphics_of(o) != list) {
        return None;
    }
    let handles = list.iter().enumerate().filter_map(|(i, g)| frame.handle(g).filter(|(_, seen)| *seen).map(|(p, _)| (i, p))).collect();
    Some((frame, list, handles))
}

/// Temp-memory key of a drag of a graphic's handle on the artboard.
fn handle_drag_id() -> egui::Id {
    egui::Id::unique("three-d-graphic-handle-drag")
}

/// A drag of graphic `index`'s handle: its face, the page offset from the handle to the point
/// where it was grabbed, and the document and selected objects it was grabbed for.
#[derive(Clone, Debug)]
struct HandleDrag {
    index: usize,
    face: vectorcraft_effects::PlanarFace,
    grab: vectorcraft_geom::Vec2,
    owner: (u64, Vec<u64>),
}

/// The active document's uid and its selected objects' ids: what a handle drag moves graphics for.
fn drag_owner(app: &VectorcraftApp) -> (u64, Vec<u64>) {
    app.session.active().map(|st| (st.uid, st.selection.objects.iter().map(|id| id.0).collect())).unwrap_or_default()
}

/// The offset, rounded to 0.1 pt, that a handle drag to offset (`x`, `y`) gives graphic `g`. `None`
/// when the graphic stays where it is: the pointer is where the handle was grabbed, or the rounded
/// offset is the graphic's own, so a click on a handle records no undo step.
fn moved_to(g: &vectorcraft_effects::MappedGraphic, x: f64, y: f64) -> Option<(f64, f64)> {
    let rounded = ((x * 10.0).round() / 10.0, (y * 10.0).round() / 10.0);
    let still = (x - g.x).abs() < 1e-6 && (y - g.y).abs() < 1e-6;
    (!still && rounded != (g.x, g.y)).then_some(rounded)
}

/// How near a press must be to a graphic's handle to grab it, in screen points.
const HANDLE_REACH: f32 = 8.0;

/// The artboard's handles of the mapped graphics while the Materials tab is open: a press on one
/// grabs it, and dragging moves that graphic over its face, previewed while the pointer is down and
/// committed as one undo step when it is released. Returns whether the handles took the pointer.
pub(crate) fn graphic_handles_input(app: &mut VectorcraftApp, ui: &egui::Ui, resp: &egui::Response, rect: egui::Rect) -> bool {
    let ctx = ui.ctx().clone();
    let xf = &crate::canvas::Xf::at(rect, app.view().unwrap_or(&crate::state::View::default()), app.canvas_ui_zoom);
    let pointer = ui.input(|i| i.pointer.interact_pos());
    if let Some(drag) = ctx.data(|m| m.get_temp::<HandleDrag>(handle_drag_id())) {
        if drag_owner(app) != drag.owner {
            // The selection or the document changed during the drag: commit the preview and end
            // the drag, so its grab offset does not move another object's graphic.
            send(app, &ctx, Map::new(), false);
            ctx.data_mut(|m| m.remove::<HandleDrag>(handle_drag_id()));
            return true;
        }
        let down = ui.input(|i| i.pointer.primary_down());
        if let (Some(p), Some((frame, mut list, _))) = (pointer, graphic_handles(app, &ctx))
            && let Some((x, y)) = frame.offset_at(drag.face, xf.to_doc(p) - drag.grab)
            && let Some(g) = list.get_mut(drag.index)
            && let Some((x, y)) = moved_to(g, x, y)
        {
            (g.x, g.y) = (x, y);
            let graphics = Value::Array(list.iter().map(vectorcraft_effects::MappedGraphic::to_value).collect());
            let mut changed = Map::new();
            changed.insert(vectorcraft_effects::GRAPHICS.into(), graphics);
            send(app, &ctx, changed, down);
        } else if !down {
            send(app, &ctx, Map::new(), false);
        }
        if !down {
            ctx.data_mut(|m| m.remove::<HandleDrag>(handle_drag_id()));
        }
        return true;
    }
    if !(resp.hovered() && ui.input(|i| i.pointer.primary_pressed())) {
        return false;
    }
    // A press with Space held, or with the Hand, Zoom or Rotate View tool, pans, zooms or turns the
    // view, as on any other part of the canvas.
    let space = ui.input(|i| i.key_down(egui::Key::Space));
    if space || matches!(app.session.tool_id(), "hand" | "zoom" | "rotateView") {
        return false;
    }
    let (Some(p), Some((_, list, handles))) = (pointer, graphic_handles(app, &ctx)) else { return false };
    let nearest = handles
        .iter()
        .map(|(i, at)| (*i, *at, xf.to_screen(*at).distance(p)))
        .filter(|(.., d)| *d <= HANDLE_REACH)
        .min_by(|a, b| a.2.total_cmp(&b.2));
    let Some((index, at, _)) = nearest else { return false };
    let Some(face) = list.get(index).map(|g| g.face) else { return false };
    let owner = drag_owner(app);
    ctx.data_mut(|m| m.insert_temp(handle_drag_id(), HandleDrag { index, face, grab: xf.to_doc(p) - at, owner }));
    set_selected_graphic(&ctx, index);
    true
}

/// Draws the mapped graphics' handles on the artboard while the Materials tab is open: a blue dot
/// at each graphic's center, larger for the one the tab edits.
pub(crate) fn paint_graphic_handles(app: &VectorcraftApp, painter: &egui::Painter, xf: &crate::canvas::Xf) {
    let ctx = painter.ctx();
    let Some((_, _, handles)) = graphic_handles(app, ctx) else { return };
    let state: Map<String, Value> = pstate(ctx, UI_STATE);
    let selected = state.get(materials::SELECTED).and_then(Value::as_u64).map(|i| i as usize);
    let t = crate::theme::Tokens::get(ctx);
    for (i, at) in handles {
        let r = if Some(i) == selected { 6.0 } else { 4.5 };
        let c = xf.to_screen(at);
        painter.circle_filled(c, r, t.accent);
        painter.circle_stroke(c, r, egui::Stroke::new(1.5, egui::Color32::WHITE));
    }
}

/// Does the panel show its Quick Actions? On the Object tab of the effects that have one, and on
/// the first tab of the others (Revolve, Rotate), never on Lighting.
fn quick_actions_shown(d: &Dialog) -> bool {
    let effect = d.str("__effect");
    let object_tab = [EXTRUDE, INFLATE, CLASSIC_EXTRUDE, CLASSIC_REVOLVE].contains(&effect.as_str());
    !crate::dialogs::revolve_gizmo::lighting(d) && (!object_tab || d.str("__tab") != "rotation")
}

/// Quick Actions › Expand as Wireframes: each selected object with a 3D effect becomes a group of
/// the edges of its faces (`effect.expandAsWireframes`), one undo step.
pub(crate) fn expand_as_wireframes(app: &mut VectorcraftApp) -> Result<(), String> {
    app.run("effect.expandAsWireframes", json!({})).map(|_| ())
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
