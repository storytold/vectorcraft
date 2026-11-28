//! The live preview of a dialog that previews its command on the canvas: the dialog's values run as one interaction
//! that Cancel rolls back and OK keeps as one undo step.

use std::sync::{Arc, Weak};

use serde_json::Value;
use vectorcraft_doc::Selection;

use crate::VectorcraftApp;

/// What a dialog's live preview has shown: the command and parameters it last ran inside the interaction, and which
/// interaction that was.
#[derive(Clone, Default)]
pub(crate) struct LivePreview {
    previewed: Option<(String, Value)>,
    owner: Option<Owner>,
}

/// The interaction a preview ran in: its document's `uid` and `reverts`, and the selection snapshot the interaction
/// began with, which no other interaction shares.
#[derive(Clone)]
struct Owner {
    document: (u64, u64),
    selection: Weak<Selection>,
}

impl LivePreview {
    /// Whether the active document's interaction is the one this preview ran in.
    fn owns(&self, app: &VectorcraftApp) -> bool {
        let Some(owner) = &self.owner else { return false };
        app.session.active().is_some_and(|state| {
            (state.uid, state.reverts) == owner.document
                && state.interaction.as_ref().is_some_and(|interaction| owner.selection.as_ptr() == Arc::as_ptr(&interaction.selection))
        })
    }

    /// Forgets a preview whose interaction has ended (an undo, a switch of document) or was never this one's.
    fn forget_if_gone(&mut self, app: &VectorcraftApp) {
        if !self.owns(app) {
            self.previewed = None;
            self.owner = None;
        }
    }

    /// While `on`, runs `cmd` with `params` on the interaction's snapshot whenever they differ from the last preview
    /// or its interaction has ended; `label` names the undo step. Returns the command's answer when this call ran a
    /// preview, and `None` when nothing ran: the same values again (a pass reruns nothing), a preview that could not
    /// begin, another edit in progress, or `on` off, which rolls a shown preview back. Typing in progress is kept as
    /// its own step first. A refusal's text is written for the user, without the command's id.
    pub(crate) fn run(&mut self, app: &mut VectorcraftApp, on: bool, label: &str, cmd: &str, params: &Value) -> Option<Result<Value, String>> {
        self.forget_if_gone(app);
        if !on {
            if self.previewed.take().is_some() {
                let _ = app.session.cancel_interaction();
            }
            self.owner = None;
            return None;
        }
        let same = self.previewed.as_ref().is_some_and(|(c, p)| c == cmd && p == params);
        if same {
            return None;
        }
        // Another dialog's, a panel's or a tool's edit in progress gets no preview of this one.
        if self.owner.is_none() && app.session.in_interaction() {
            if let Err(e) = app.keep_typing() {
                app.status(e);
                return None;
            }
            if app.session.in_interaction() {
                return None;
            }
        }
        // A failure to begin (typing that can't be kept) shows, and nothing previews on the interaction still in
        // progress.
        if let Err(e) = app.begin_interaction(label) {
            app.status(e);
            return None;
        }
        let answer = match app.session.preview(cmd, params) {
            Ok(v) => Ok(v),
            Err(e) => {
                app.status(e.to_string());
                Err(match e {
                    vectorcraft_engine::EngineError::BadParams { msg, .. } => msg,
                    other => other.to_string(),
                })
            }
        };
        self.previewed = Some((cmd.to_string(), params.clone()));
        self.owner = app.session.active().and_then(|state| {
            let interaction = state.interaction.as_ref()?;
            Some(Owner { document: (state.uid, state.reverts), selection: Arc::downgrade(&interaction.selection) })
        });
        Some(answer)
    }

    /// Forgets the values last previewed and keeps the interaction they ran in, so the next [`Self::run`] previews
    /// them again inside that interaction and returns the command's answer, a refusal included.
    pub(crate) fn rerun_next(&mut self) {
        self.previewed = None;
    }

    /// OK: keeps the previewed `cmd` as one undo step, or, when its preview is not running, keeps typing in progress
    /// and runs `cmd` as a step of its own; the command's warning, or a plug-in run's notice, shows in the status bar
    /// either way. Another edit in progress is left as it is and OK is refused. On an error the dialog stays open.
    pub(crate) fn commit(self, app: &mut VectorcraftApp, cmd: &str, params: Value) -> Result<Value, String> {
        if !self.owns(app) {
            if app.session.in_interaction() {
                app.keep_typing()?;
            }
            if app.session.in_interaction() {
                return Err(tl!("Finish the active edit first.").to_owned());
            }
            return app.run(cmd, params);
        }
        let result = app.session.preview(cmd, &params).and_then(|answer| {
            app.session.commit_interaction()?;
            Ok(answer)
        });
        let answer = match result {
            Ok(answer) => answer,
            Err(error) => {
                self.cancel(app);
                return Err(error.to_string());
            }
        };
        if let Some(warning) = answer.get("warning").and_then(Value::as_str) {
            app.status(warning.to_string());
        }
        crate::plugin_runs::show_notice(app, cmd, &answer);
        Ok(Value::Null)
    }

    /// Cancel: rolls a shown preview back, and leaves any other edit in progress as it is.
    pub(crate) fn cancel(self, app: &mut VectorcraftApp) {
        if self.previewed.is_some() && self.owns(app) {
            let _ = app.session.cancel_interaction();
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_engine::{Session, ViewInfo};
    use vectorcraft_tools::{PointerEvent, PointerKind};

    use super::*;

    /// An app with Real-time Drawing off and " world" typed after "Hello", not yet applied → the
    /// type's id.
    fn typing(app: &mut crate::VectorcraftApp) -> u64 {
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        app.run("prefs.set", json!({"key": "realTimeDrawing", "value": false})).unwrap();
        let id = app.run("text.create", json!({"x": 100, "y": 100, "text": "Hello", "size": 20})).unwrap()["id"].as_u64().unwrap();
        let v = ViewInfo::default();
        app.session.select_tool("type", v).unwrap();
        let b = app.session.active().unwrap().doc.bounds_of(&[vectorcraft_doc::NodeId(id)], false).unwrap();
        for k in [PointerKind::Down, PointerKind::Up] {
            app.session.pointer(&PointerEvent::new(k, b.x1 - 1.0, b.center().y), v).unwrap();
        }
        app.session.tool_text(" world", v).unwrap();
        id
    }

    fn plain(app: &crate::VectorcraftApp, id: u64) -> String {
        match &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
            vectorcraft_doc::NodeKind::Text(t) => t.plain_text(),
            _ => panic!("not text"),
        }
    }

    #[test]
    fn refused_confirmation_restores_the_art_and_ends_the_preview() {
        let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
        let original = app.session.doc().unwrap().doc.clone();
        let undo = app.session.doc().unwrap().history.undo.len();

        let mut preview = LivePreview::default();
        let fill = json!({"color": "#00a000"});
        assert!(matches!(preview.run(&mut app, true, "Fill Color", "paint.setFill", &fill), Some(Ok(_))));
        assert_ne!(app.session.doc().unwrap().doc, original);
        assert!(preview.commit(&mut app, "paint.setFill", json!({"color": true})).is_err());
        assert!(!app.session.in_interaction(), "the refused edit left no open interaction");
        assert_eq!(app.session.doc().unwrap().doc, original);
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);

        let mut next = LivePreview::default();
        assert!(matches!(next.run(&mut app, true, "Next Fill", "paint.setFill", &fill), Some(Ok(_))));
        next.commit(&mut app, "paint.setFill", fill).unwrap();
        let history = &app.session.doc().unwrap().history;
        assert_eq!(history.undo.len(), undo + 1);
        assert_eq!(history.undo.last().unwrap().label, "Next Fill");
    }

    /// Once the preview's interaction has ended, another edit that begins in the same document is left alone: a new
    /// preview does not run inside it, Cancel does not roll it back, and OK is refused instead of keeping the dialog's
    /// command in that edit's place.
    #[test]
    fn another_edit_in_progress_is_not_previewed_canceled_or_committed() {
        let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
        let mut preview = LivePreview::default();
        assert!(matches!(preview.run(&mut app, true, "Fill Color", "paint.setFill", &json!({"color": "#00a000"})), Some(Ok(_))));
        app.session.cancel_interaction().unwrap();

        app.session.begin_interaction("Other edit").unwrap();
        app.session.preview("paint.setStroke", &json!({"color": "#0000ff"})).unwrap();
        let other = app.session.doc().unwrap().doc.clone();
        let undo = app.session.doc().unwrap().history.undo.len();
        let other_edit = |app: &crate::VectorcraftApp| app.session.active().and_then(|st| st.interaction.as_ref()).map(|it| it.label.clone());

        preview.clone().cancel(&mut app);
        assert_eq!(other_edit(&app).as_deref(), Some("Other edit"), "Cancel leaves the other edit running");
        assert!(preview.run(&mut app, true, "Fill Color", "paint.setFill", &json!({"color": "#a00000"})).is_none());
        assert_eq!(app.session.doc().unwrap().doc, other, "the other edit's preview stays");
        assert!(preview.commit(&mut app, "paint.setFill", json!({"color": "#a00000"})).is_err());
        assert_eq!(other_edit(&app).as_deref(), Some("Other edit"), "a refused OK leaves the other edit running");
        assert_eq!(app.session.doc().unwrap().doc, other);
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);
    }

    /// OK without a running preview keeps typing in progress as its own step and then runs the command, instead of
    /// running it on the snapshot from before the typing.
    #[test]
    fn ok_without_a_preview_keeps_the_typing() {
        let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
        let id = typing(&mut app);
        LivePreview::default().commit(&mut app, "paint.setFill", json!({"color": "#00a000"})).unwrap();
        assert_eq!(plain(&app, id), "Hello world");
        assert!(!app.session.in_interaction());
    }

    /// A dialog's preview while typing is in progress keeps the typing as its own step and starts
    /// from it; the status bar shows the warning of the typing's preview that keeping it applied.
    #[test]
    fn a_dialog_preview_keeps_the_typing_and_shows_its_warning() {
        let fill = json!({"color": "#00a000"});
        let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
        let id = typing(&mut app);
        assert_eq!(plain(&app, id), "Hello", "the typing waits");
        let steps = app.session.active().unwrap().history.undo.len();
        let answer = LivePreview::default().run(&mut app, true, "Fill Color", "paint.setFill", &fill);
        assert!(matches!(answer, Some(Ok(_))), "{answer:?}");
        assert_eq!(plain(&app, id), "Hello world");
        let st = app.session.active().unwrap();
        assert_eq!(st.history.undo.len(), steps + 1, "the typing is a step of its own");
        assert_eq!(st.interaction.as_ref().map(|it| it.label.as_str()), Some("Fill Color"));

        // The typing's deferred preview warns that type is skipped (as Bloat does with type selected).
        let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        let rect = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
        let text = app.run("text.create", json!({"x": 300, "y": 200, "text": "Hi"})).unwrap();
        app.run("select.set", json!({"ids": [rect["id"], text["id"]]})).unwrap();
        app.run("prefs.set", json!({"key": "realTimeDrawing", "value": false})).unwrap();
        app.session.begin_interaction("Typing").unwrap();
        let p = json!({"tool": "bloat", "points": [[290.0, 200.0]], "width": 200.0, "height": 200.0});
        app.session.doc_mut().unwrap().interaction.as_mut().unwrap().preview = Some(("object.liquify".into(), p));
        app.ui.status.clear();
        let answer = LivePreview::default().run(&mut app, true, "Fill Color", "paint.setFill", &fill);
        assert!(matches!(answer, Some(Ok(_))), "{answer:?}");
        assert!(app.ui.status.contains("type"), "{:?}", app.ui.status);
    }

    /// The font menus end the typing session before their preview: the typing Real-time Drawing
    /// off left unapplied is kept as a step of its own.
    #[test]
    fn ending_typing_for_the_font_menus_keeps_the_typing() {
        let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
        let id = typing(&mut app);
        let steps = app.session.active().unwrap().history.undo.len();
        crate::panels::character::end_typing(&mut app);
        assert_eq!(plain(&app, id), "Hello world");
        let st = app.session.active().unwrap();
        assert_eq!(st.history.undo.len(), steps + 1);
        assert!(st.interaction.is_none());
        assert_ne!(app.session.tool_options().get("typing").and_then(Value::as_bool), Some(true), "the typing session is over");
    }

    /// A Character panel change keeps the typing first and acts on the characters the Type tool
    /// selected in the typed text, not on a selection cut short to the text before the typing.
    #[test]
    fn a_character_change_acts_on_characters_selected_in_the_typing() {
        let mut app = crate::VectorcraftApp::new(Session::new(), Default::default());
        let id = typing(&mut app);
        app.session.set_tool_option("select", &json!({"start": 6, "end": 11}));
        crate::panels::character::range_style(&mut app, json!({"underline": true}));
        assert_eq!(plain(&app, id), "Hello world");
        let vectorcraft_doc::NodeKind::Text(t) = &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind else {
            panic!("not text")
        };
        let underlined: String = t.runs.iter().filter(|r| r.style.underline).map(|r| r.text.as_str()).collect();
        assert_eq!(underlined, "world");
    }
}
