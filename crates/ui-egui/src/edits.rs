//! Applying the edits a framework component reports to the document, one undo step per gesture.

use craft::behavior::{CancelReason, EditEvent, GestureId};
use craft::ui::contract::EditResponse;

use crate::VectorcraftApp;
use crate::panels::Live;

/// How a gesture reaches the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditPath {
    /// Every value is a real command run; an undo scope makes the gesture one step.
    Scope,
    /// A value while the gesture runs is an engine preview, the commit one step; the closure picks
    /// the command by value, so the engine's own interaction names the step.
    Preview,
}

/// Applies the events of `response` in order through `apply`, which gets each value with the phase
/// it arrives in (`Live::Dragging` for a preview, `Live::Released` for a run).
pub(crate) fn apply_edits<V: Clone>(
    app: &mut VectorcraftApp,
    response: &EditResponse<V>,
    path: EditPath,
    mut apply: impl FnMut(&mut VectorcraftApp, &V, Live),
) {
    for event in response.events.iter() {
        apply_event(app, event, &path, &mut apply);
    }
}

/// Applies one event. Only the events of the pass that received the input are applied, so a rerun
/// pass gives none and nothing runs twice.
pub(crate) fn apply_event<V: Clone>(
    app: &mut VectorcraftApp,
    event: &EditEvent<V>,
    path: &EditPath,
    apply: &mut impl FnMut(&mut VectorcraftApp, &V, Live),
) {
    let scoped = *path == EditPath::Scope;
    match event {
        EditEvent::Begin { id, .. } => {
            if scoped && let Some(scope) = app.session.begin_undo_scope() {
                app.edit_scopes.insert(*id, scope);
            }
        }
        EditEvent::Update { value, .. } => apply(app, value, if scoped { Live::Released } else { Live::Dragging }),
        EditEvent::Commit { id, after, .. } => {
            apply(app, after, Live::Released);
            if scoped && let Some(scope) = app.edit_scopes.remove(id) {
                app.session.end_undo_scope(scope, false);
            }
        }
        EditEvent::Cancel { id, reason, .. } => {
            if scoped {
                if let Some(scope) = app.edit_scopes.remove(id) {
                    // A document that moved under the gesture keeps the gesture's edits: a revert
                    // would take back the change too.
                    app.session.end_undo_scope(scope, !reason.document_moved());
                }
            } else {
                end_preview(app, *reason);
            }
        }
        EditEvent::Set { after, .. } => apply(app, after, Live::Released),
        _ => {}
    }
}

/// Ends the undo scopes of the gestures whose component went away while they ran
/// (`FrameOutput::removed_gestures`), whose `Cancel` never comes: what they applied stays, as one
/// step, as a removed preview keeps what it showed.
pub(crate) fn end_removed(app: &mut VectorcraftApp, removed: &[GestureId]) {
    for id in removed {
        if let Some(scope) = app.edit_scopes.remove(id) {
            app.session.end_undo_scope(scope, false);
        }
    }
}

/// Ends a preview that did not commit: rolled back, except where the document moved or the
/// component went away, which keep what was previewed.
fn end_preview(app: &mut VectorcraftApp, reason: CancelReason) {
    if !app.session.in_interaction() {
        return;
    }
    let keep = reason.document_moved() || reason == CancelReason::Removed;
    let ended = if keep { app.session.commit_interaction() } else { app.session.cancel_interaction() };
    if let Err(e) = ended {
        app.ui.status = e.to_string();
    }
    app.sync_views();
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::sync::Arc;

    use craft::behavior::Mixed;
    use serde_json::json;
    use vectorcraft_engine::Session;

    use super::*;
    use crate::Services;

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Services::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app
    }

    fn steps(app: &VectorcraftApp) -> usize {
        app.session.active().map_or(0, |st| st.history.undo.len())
    }

    /// A rectangle at `x`: one command run, or one preview.
    fn rectangle(app: &mut VectorcraftApp, x: &f64, phase: Live) {
        crate::panels::live_run(app, "Rectangle", "shape.rectangle", json!({"x": x, "y": 0, "width": 5, "height": 5}), phase);
    }

    fn begin(id: u64) -> EditEvent<f64> {
        EditEvent::begin(GestureId::new(id), Mixed::Same(0.0), Arc::from("Test"), None)
    }
    fn update(id: u64, v: f64) -> EditEvent<f64> {
        EditEvent::update(GestureId::new(id), v)
    }
    fn commit(id: u64, v: f64) -> EditEvent<f64> {
        EditEvent::commit(GestureId::new(id), Mixed::Same(0.0), v)
    }
    fn cancel(id: u64, reason: CancelReason) -> EditEvent<f64> {
        EditEvent::cancel(GestureId::new(id), reason, None)
    }

    fn feed(app: &mut VectorcraftApp, path: EditPath, events: &[EditEvent<f64>]) {
        for event in events {
            apply_event(app, event, &path, &mut |app, v, phase| {
                if path == EditPath::Scope {
                    app.run("shape.rectangle", json!({"x": v, "y": 0, "width": 5, "height": 5})).ok();
                } else {
                    rectangle(app, v, phase);
                }
            });
        }
    }

    #[test]
    fn a_gesture_is_one_undo_step_on_either_path() {
        // Scope: three runs, one step.
        let mut a = app();
        let base = steps(&a);
        feed(&mut a, EditPath::Scope, &[begin(1), update(1, 1.0), update(1, 2.0), commit(1, 3.0)]);
        assert_eq!(steps(&a), base + 1, "a scope joins its runs");

        // Scope, cancelled: reverted; with the document moved, kept.
        let mut a = app();
        let base = steps(&a);
        feed(&mut a, EditPath::Scope, &[begin(1), update(1, 1.0), cancel(1, CancelReason::Escape)]);
        assert_eq!(steps(&a), base, "an Escape reverts");
        feed(&mut a, EditPath::Scope, &[begin(2), update(2, 1.0), cancel(2, CancelReason::UndoDuringGesture)]);
        assert_eq!(steps(&a), base + 1, "the edits stay when the document moved");

        // Preview: an interaction while it runs, one step at the commit.
        let mut a = app();
        let base = steps(&a);
        feed(&mut a, EditPath::Preview, &[begin(1), update(1, 1.0), update(1, 2.0)]);
        assert!(a.session.in_interaction());
        assert_eq!(steps(&a), base, "a preview is not a step");
        feed(&mut a, EditPath::Preview, &[commit(1, 3.0)]);
        assert!(!a.session.in_interaction());
        assert_eq!(steps(&a), base + 1);

        // Preview, cancelled: restored.
        feed(&mut a, EditPath::Preview, &[begin(2), update(2, 1.0), cancel(2, CancelReason::Escape)]);
        assert!(!a.session.in_interaction());
        assert_eq!(steps(&a), base + 1, "a cancelled preview leaves no step");

        // No document: a value on the preview path still runs once, at release.
        let mut none = VectorcraftApp::new(Session::new(), Services::default());
        let phases = Cell::new(Vec::new());
        let set = EditEvent::set(Mixed::Same(0.0), 1.0, Arc::from("Test"), None);
        apply_event(&mut none, &set, &EditPath::Preview, &mut |_, _, phase| {
            let mut seen = phases.take();
            seen.push(phase);
            phases.set(seen);
        });
        assert_eq!(phases.take(), [Live::Released]);
    }
}
