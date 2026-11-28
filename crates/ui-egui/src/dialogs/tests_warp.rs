//! Effect > Warp uses the existing modal and live-effect commands, without changing envelope art. The dialogs are the
//! hosted ones: values go in through the host's `set_json_field`, as `ui.dialog.set` sets them, and a frame is the
//! app's dialog pump.
use craft::ui::dialog::{DialogHost, DialogId};
use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_engine::Session;

use crate::VectorcraftApp;

pub(super) use crate::testing::windowed;

fn app() -> (VectorcraftApp, NodeId) {
    let mut app = windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 50, "y": 60, "width": 120, "height": 80})).unwrap()["id"].as_u64().unwrap();
    (app, NodeId(id))
}

fn ctx_of(app: &VectorcraftApp) -> egui::Context {
    app.ui_ctx.clone().expect("a window")
}

pub(super) fn host<R>(app: &VectorcraftApp, f: impl FnOnce(&mut DialogHost) -> R) -> R {
    app.dialogs().expect("a window").with(f)
}

/// One frame of the app's dialog pump.
pub(super) fn pump(app: &mut VectorcraftApp) {
    let ctx = ctx_of(app);
    super::host::pump(app, &ctx);
}

/// One frame of the window: the host shows the dialogs and closes the answered ones, then the app's dialog pump runs.
fn frame(app: &mut VectorcraftApp) {
    let ctx = ctx_of(app);
    crate::testing::app_frame(app, &ctx, Default::default());
}

/// The ids of the open dialogs, the top one last.
pub(super) fn open_ids(app: &VectorcraftApp) -> Vec<DialogId> {
    host(app, |h| h.open_dialogs()).into_iter().map(|d| d.id).collect()
}

/// The values the dialog `id` reports for a script.
pub(super) fn json_fields(app: &VectorcraftApp, id: DialogId) -> Value {
    host(app, |h| h.open_dialogs()).into_iter().find(|d| d.id == id).and_then(|d| d.fields).expect("an open dialog with fields")
}

/// Sets a field of the dialog `id` as `ui.dialog.set` does.
pub(super) fn set(app: &VectorcraftApp, id: DialogId, field: &str, value: Value) {
    let ctx = ctx_of(app);
    host(app, |h| h.set_json_field(&ctx, id, field, value)).unwrap_or_else(|e| panic!("{field}: {e}"));
}

/// Answers the dialog `id` with OK: the held answer is filed, the app's pump runs it, and the next confirm closes the
/// dialog with it.
pub(super) fn accept(app: &mut VectorcraftApp, id: DialogId) {
    let ctx = ctx_of(app);
    host(app, |h| h.confirm_by_id(&ctx, id));
    pump(app);
    host(app, |h| h.confirm_by_id(&ctx, id));
    pump(app);
}

/// Cancels the dialog `id`; the pump rolls back what it previewed.
pub(super) fn dismiss(app: &mut VectorcraftApp, id: DialogId) {
    let ctx = ctx_of(app);
    host(app, |h| h.cancel_by_id(&ctx, id));
    pump(app);
}

fn effect_dialog(app: &VectorcraftApp) -> DialogId {
    app.effect_dialogs.effect.as_ref().map(|open| open.id).expect("an open Effect dialog")
}

/// Answers the Edit or Add New prompt the Effect menu asks.
fn answer_prompt(app: &mut VectorcraftApp, choice: &str) {
    let prompt = *open_ids(app).last().expect("the Edit or Add New prompt");
    set(app, prompt, "choice", json!(choice));
    // The choice is picked, then answered with OK, as `ui.dialog.set` then `ui.dialog.confirm` do.
    let ctx = ctx_of(app);
    host(app, |h| h.confirm_by_id(&ctx, prompt));
    frame(app);
    frame(app);
}

#[test]
fn warp_options_style_orientation_clicks_preview_and_cancel_preserve_redo() {
    let (mut app, id) = app();
    app.run("object.move", json!({"dx": 10, "dy": 0})).unwrap();
    app.run("edit.undo", json!({})).unwrap();
    app.session.doc_mut().unwrap().mark_saved();
    let before = app.session.doc().unwrap().doc.clone();
    let selection = app.session.doc().unwrap().selection.clone();
    let undo = app.session.doc().unwrap().history.undo.len();
    let redo = app.session.doc().unwrap().history.redo.len();
    assert_eq!(redo, 1);
    app.run("effect.dialog", json!({"effect": "warp.arc", "item": null})).unwrap();
    let dialog = effect_dialog(&app);
    set(&app, dialog, "style", json!("arcLower"));
    set(&app, dialog, "styleChosen", json!(true));
    set(&app, dialog, "orientation", json!("vertical"));
    for (field, value) in [("bend", -30.0), ("horizontal", 20.0), ("vertical", -15.0)] {
        set(&app, dialog, field, json!(value));
    }
    frame(&mut app);
    let state = app.session.doc().unwrap();
    let effects = &state.doc.node(id).unwrap().appearance.effects;
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].id, "warp.arcLower");
    assert_eq!(effects[0].params["orientation"], "vertical");
    assert_eq!(effects[0].params["bend"], -30.0);
    assert_eq!(effects[0].params["horizontal"], 20.0);
    assert_eq!(effects[0].params["vertical"], -15.0);
    assert_eq!(state.history.undo.len(), undo);
    dismiss(&mut app, dialog);
    let state = app.session.doc().unwrap();
    assert_eq!(state.doc, before);
    assert_eq!(state.selection, selection);
    assert_eq!((state.history.undo.len(), state.history.redo.len()), (undo, redo));
    assert!(!state.is_dirty());
    assert!(!app.session.in_interaction() && open_ids(&app).is_empty());
    app.run("edit.redo", json!({})).unwrap();
    assert_ne!(app.session.doc().unwrap().doc, before, "the original Redo remains usable");
}

#[test]
fn warp_options_existing_style_edit_keeps_stack_target_visibility_and_history() {
    for item in [Value::Null, json!(1)] {
        let (mut app, id) = app();
        for effect in ["path.offsetPath", "warp.arc", "stylize.roundCorners"] {
            app.run("effect.apply", json!({"effect": effect, "item": item})).unwrap();
        }
        app.run("effect.setParams", json!({"index": 1, "item": item, "visible": false, "params": {"bend": -25.0}})).unwrap();
        app.session.doc_mut().unwrap().mark_saved();
        let before = app.session.doc().unwrap().doc.clone();
        let selection = app.session.doc().unwrap().selection.clone();
        let undo = app.session.doc().unwrap().history.undo.len();
        let last_effect = app.last_effect.clone();
        app.run("effect.dialog", json!({"effect": "warp.arc", "index": 1, "item": item})).unwrap();
        let dialog = effect_dialog(&app);
        assert_eq!(json_fields(&app, dialog)["bend"], -25.0);
        set(&app, dialog, "style", json!("arcLower"));
        set(&app, dialog, "styleChosen", json!(true));
        set(&app, dialog, "bend", json!(35.0));
        frame(&mut app);
        accept(&mut app, dialog);
        let state = app.session.doc().unwrap();
        let target = item.as_u64().map(|n| n as usize);
        let old = before.node(id).unwrap().appearance.effects_at(target).unwrap();
        let new = state.doc.node(id).unwrap().appearance.effects_at(target).unwrap();
        assert_eq!(new.len(), old.len());
        assert_eq!(new[0], old[0]);
        assert_eq!(new[2], old[2]);
        assert_eq!(new[1].id, "warp.arcLower");
        assert_eq!(new[1].params["bend"], 35.0);
        assert!(!new[1].visible);
        assert_eq!(state.selection, selection);
        assert_eq!(state.history.undo.len(), undo + 1);
        assert_eq!(app.last_effect, last_effect, "editing does not replace Last Effect");
        let after = state.doc.clone();
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc, before);
        assert!(!app.session.doc().unwrap().is_dirty());
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc, after);
    }
}

/// A new Warp Options dialog opens on the sliders confirmed last time (#1760 follow-up).
#[test]
fn warp_dialog_remembers_last_confirmed_sliders() {
    let (mut app, _) = app();
    app.run("effect.dialog", json!({"effect": "warp.arc", "item": null})).unwrap();
    let first = effect_dialog(&app);
    for (field, value) in [("bend", json!(-40.0)), ("horizontal", json!(12.0)), ("vertical", json!(-8.0)), ("orientation", json!("vertical"))] {
        set(&app, first, field, value);
    }
    accept(&mut app, first);
    frame(&mut app);
    app.run("effect.dialog", json!({"effect": "warp.flag", "item": null})).unwrap();
    let fields = json_fields(&app, effect_dialog(&app));
    assert_eq!(fields["style"], "flag");
    assert_eq!(fields["bend"], -40.0);
    assert_eq!(fields["horizontal"], 12.0);
    assert_eq!(fields["vertical"], -8.0);
    assert_eq!(fields["orientation"], "vertical");
}

/// The edit the Warp dialog test makes: the Bend field, the style, or none.
fn make_warp_edit(app: &mut VectorcraftApp, dialog: DialogId, edit: &str) {
    match edit {
        "bend" => set(app, dialog, "bend", json!(25.0)),
        "style" => {
            set(app, dialog, "style", json!("arcLower"));
            set(app, dialog, "styleChosen", json!(true));
        }
        _ => {}
    }
    frame(app);
}

/// OK or Cancel was answered: the document, the history and Redo as each leaves them.
fn finish_warp_dialog(
    app: &mut VectorcraftApp,
    approved: bool,
    (before, expected): (&std::sync::Arc<vectorcraft_doc::Document>, &std::sync::Arc<vectorcraft_doc::Document>),
    (undo, redo): (usize, usize),
) {
    let state = app.session.doc().unwrap();
    // OK with nothing changed records no step, as Cancel does.
    if approved && expected != before {
        // Whole-document equality also checks both targets, sibling effects and
        // the first visible / second hidden Warp. Existing OK history stays one step.
        assert_eq!(state.doc.as_ref(), expected.as_ref());
        assert_eq!(state.history.undo.len(), undo + 1);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc.as_ref(), before.as_ref());
        assert!(!app.session.doc().unwrap().is_dirty());
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc.as_ref(), expected.as_ref());
    } else {
        assert_eq!(state.doc.as_ref(), before.as_ref());
        assert_eq!((state.history.undo.len(), state.history.redo.len()), (undo, redo));
        assert!(!state.is_dirty());
        app.run("edit.redo", json!({})).unwrap();
        assert_ne!(app.session.doc().unwrap().doc.as_ref(), before.as_ref(), "Cancel, or OK with nothing changed, keeps the original Redo usable");
    }
}

#[test]
fn warp_options_mixed_styles_change_only_after_explicit_style_choice() {
    for item in [Value::Null, json!(1)] {
        for edit in ["none", "bend", "style"] {
            for approved in [false, true] {
                let (mut app, first) = app();
                for effect in ["path.offsetPath", "warp.arc", "stylize.roundCorners"] {
                    app.run("effect.apply", json!({"effect": effect, "item": item})).unwrap();
                }
                let second =
                    NodeId(app.run("shape.rectangle", json!({"x": 230, "y": 60, "width": 120, "height": 80})).unwrap()["id"].as_u64().unwrap());
                for effect in ["path.offsetPath", "warp.flag", "stylize.roundCorners"] {
                    app.run("effect.apply", json!({"effect": effect, "item": item})).unwrap();
                }
                app.run("effect.setParams", json!({"index": 1, "item": item, "visible": false})).unwrap();
                app.run("select.set", json!({"ids": [first.0, second.0]})).unwrap();
                app.run("object.move", json!({"dx": 5, "dy": 0})).unwrap();
                app.run("edit.undo", json!({})).unwrap();
                app.session.doc_mut().unwrap().mark_saved();
                let before = app.session.doc().unwrap().doc.clone();
                let selection = app.session.doc().unwrap().selection.clone();
                let undo = app.session.doc().unwrap().history.undo.len();
                let redo = app.session.doc().unwrap().history.redo.len();
                assert_eq!(redo, 1);
                let last_effect = app.last_effect.clone();
                let mut expected = before.clone();
                for id in [first, second] {
                    let node = std::sync::Arc::make_mut(&mut expected).node_mut(id).unwrap();
                    let effects = node.appearance.effects_mut(item.as_u64().map(|i| i as usize)).unwrap();
                    match edit {
                        // Whole numbers are stored as JSON integers.
                        "bend" => effects[1].params["bend"] = json!(25),
                        "style" => effects[1].id = "warp.arcLower".into(),
                        _ => {}
                    }
                }
                // Ordinary Effect menu route: first object's Arc is already present, then Edit.
                let asked = app.run("effect.dialog", json!({"effect": "warp.arc", "item": item})).unwrap();
                assert_eq!(asked["pending"], "effectExists");
                answer_prompt(&mut app, "proceed");
                let dialog = effect_dialog(&app);
                assert_eq!(app.session.doc().unwrap().doc, before, "opening must not broadcast the first target's style");
                make_warp_edit(&mut app, dialog, edit);
                assert_eq!(app.session.doc().unwrap().doc, expected, "edit={edit}, item={item}");
                assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);
                set(&app, dialog, "preview", json!(false));
                frame(&mut app);
                assert_eq!(app.session.doc().unwrap().doc, before);
                set(&app, dialog, "preview", json!(true));
                frame(&mut app);
                assert_eq!(app.session.doc().unwrap().doc, expected, "Style intent survives Preview off/on");
                if approved {
                    accept(&mut app, dialog);
                } else {
                    dismiss(&mut app, dialog);
                }
                assert!(open_ids(&app).is_empty() && !app.session.in_interaction());
                let state = app.session.doc().unwrap();
                assert_eq!(state.selection, selection);
                assert_eq!(app.last_effect, last_effect);
                finish_warp_dialog(&mut app, approved, (&before, &expected), (undo, redo));
            }
        }
    }
}

/// Warp Options editing the Warps of two objects, one new and one saved without a map version,
/// changes the Bend of both and keeps each one's own `version`.
#[test]
fn warp_options_edit_keeps_each_targets_map_version() {
    let (mut app, first) = app();
    app.run("effect.apply", json!({"effect": "warp.arc"})).unwrap();
    let second = NodeId(app.run("shape.rectangle", json!({"x": 230, "y": 60, "width": 120, "height": 80})).unwrap()["id"].as_u64().unwrap());
    app.run("effect.apply", json!({"effect": "warp.arc"})).unwrap();
    // As a file saved before the version existed gives it.
    let doc = std::sync::Arc::make_mut(&mut app.session.doc_mut().unwrap().doc);
    if let Some(Value::Object(p)) = doc.node_mut(second).and_then(|n| n.appearance.effects.first_mut()).map(|e| &mut e.params) {
        p.remove("version");
    }
    app.run("select.set", json!({"ids": [first.0, second.0]})).unwrap();
    let asked = app.run("effect.dialog", json!({"effect": "warp.arc"})).unwrap();
    assert_eq!(asked["pending"], "effectExists");
    answer_prompt(&mut app, "proceed");
    let dialog = effect_dialog(&app);
    make_warp_edit(&mut app, dialog, "bend");
    accept(&mut app, dialog);
    assert!(open_ids(&app).is_empty());
    let doc = app.session.doc().unwrap().doc.clone();
    let params = |id: NodeId| doc.node(id).unwrap().appearance.effects[0].params.clone();
    // Whole numbers are stored as JSON integers: Bend compares by value.
    assert_eq!((params(first)["bend"].as_f64(), &params(first)["version"]), (Some(25.0), &json!(vectorcraft_doc::live::WARP_NET)));
    assert_eq!(params(second)["bend"].as_f64(), Some(25.0));
    assert!(params(second).get("version").is_none(), "{:?}", params(second));
}
