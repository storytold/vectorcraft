//! The live preview of a dialog that previews its command on the canvas: the dialog's values run as one interaction
//! that Cancel rolls back and OK keeps as one undo step.

use serde_json::Value;

use crate::VectorcraftApp;

/// What a dialog's live preview has shown: the command and parameters it last ran inside the interaction.
#[derive(Clone, Default)]
pub(crate) struct LivePreview {
    previewed: Option<(String, Value)>,
}

impl LivePreview {
    /// While `on`, runs `cmd` with `params` on the interaction's snapshot whenever they differ from the last preview
    /// or no interaction is running; `label` names the undo step. Returns the command's answer when this call ran a
    /// preview, and `None` when nothing ran: the same values again (a pass reruns nothing), a preview that could not
    /// begin, or `on` off, which rolls a shown preview back. A refusal's text is written for the user, without the
    /// command's id.
    pub(crate) fn run(&mut self, app: &mut VectorcraftApp, on: bool, label: &str, cmd: &str, params: &Value) -> Option<Result<Value, String>> {
        if !on {
            if self.previewed.take().is_some() {
                let _ = app.session.cancel_interaction();
            }
            return None;
        }
        let same = self.previewed.as_ref().is_some_and(|(c, p)| c == cmd && p == params);
        if same && app.session.in_interaction() {
            return None;
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
        Some(answer)
    }

    /// OK: keeps the previewed `cmd` as one undo step (or runs it when no preview runs); the command's warning shows
    /// in the status bar either way. On an error the dialog stays open.
    pub(crate) fn commit(self, app: &mut VectorcraftApp, cmd: &str, params: Value) -> Result<Value, String> {
        if !app.session.in_interaction() {
            return app.run(cmd, params);
        }
        let v = app.session.preview(cmd, &params).map_err(|e| e.to_string())?;
        let r = app.session.commit_interaction().map(|_| Value::Null).map_err(|e| e.to_string());
        if r.is_ok()
            && let Some(w) = v.get("warning").and_then(Value::as_str)
        {
            app.status(w.to_string());
        }
        r
    }

    /// Cancel: rolls a shown preview back.
    pub(crate) fn cancel(self, app: &mut VectorcraftApp) {
        if self.previewed.is_some() {
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
