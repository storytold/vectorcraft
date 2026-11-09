//! The raster editor is selected by a person in a native picker, never by a preference command
//! or a generic dialog field. Keep both the saved choice and the dialog's draft opaque.

use serde::{Deserialize, Serialize};

use crate::{FilePick, VectorcraftApp, state::Dialog};

/// Opaque saved choice. Public so hosts can construct `UiState` with a struct update; its path
/// has no public setter and is never read from the generic preference or dialog-field maps.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Choice {
    path: Option<String>,
}

#[derive(Default)]
pub(crate) struct Draft {
    choice: Choice,
    generation: u64,
}

pub(crate) fn open(app: &mut VectorcraftApp) {
    app.original_editor.generation = app.original_editor.generation.wrapping_add(1);
    app.original_editor.choice = app.ui.original_editor.clone();
}

pub(crate) fn commit(app: &mut VectorcraftApp) {
    app.ui.original_editor = app.original_editor.choice.clone();
}

pub(crate) fn reset(app: &mut VectorcraftApp) {
    app.original_editor.choice = Choice::default();
    // Discard an outstanding pick when Reset was pressed while its dialog was open.
    app.original_editor.generation = app.original_editor.generation.wrapping_add(1);
}

pub(crate) fn cancel(app: &mut VectorcraftApp) {
    open(app);
}

/// A chosen app may have been removed since Preferences saved it. Only absence falls through;
/// validation and launch failures are errors, as they are for PhotoCraft.
pub(crate) fn try_open(app: &mut VectorcraftApp, path: &str) -> Result<bool, String> {
    let Some(editor) = app.ui.original_editor.path.as_deref() else { return Ok(false) };
    let Some(open) = app.services.open_in_application.as_mut() else { return Ok(false) };
    open(editor, path)
}

fn pick(app: &mut VectorcraftApp, d: &mut Dialog) -> Result<(), String> {
    app.refuse_automation("preferences.chooseOriginalEditor")?;
    let generation = app.original_editor.generation;
    crate::picks::in_app_dialog(app, d, move |app, _| {
        if generation != app.original_editor.generation {
            return Ok(());
        }
        let filters = if cfg!(target_os = "macos") {
            vec![FilePick::filter("Applications", &["app"])]
        } else if cfg!(windows) {
            vec![FilePick::filter("Applications", &["exe"])]
        } else {
            Vec::new()
        };
        let Some(path) = crate::picks::open(app, &FilePick { filters, ..Default::default() }) else { return Ok(()) };
        app.services.check_application.as_ref().ok_or("no application chooser here")?(&path)?;
        app.original_editor.choice.path = Some(path);
        Ok(())
    })
}

pub(crate) fn show(ui: &mut egui::Ui, app: &mut VectorcraftApp, d: &mut Dialog) {
    if app.services.check_application.is_none() || app.services.open_in_application.is_none() {
        return;
    }
    ui.add_enabled_ui(!d.bool("editOriginalSystemDefaults"), |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(tl!("Edit Original with:"));
            let name = app.original_editor.choice.path.as_deref().and_then(|p| std::path::Path::new(p).file_name());
            ui.label(name.map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "PhotoCraft".into()));
            if ui.button(tl!("Choose…")).clicked()
                && let Err(e) = pick(app, d)
            {
                app.status(e);
            }
            if app.original_editor.choice.path.is_some() && ui.button("PhotoCraft").clicked() {
                reset(app);
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Services,
        control::{ControlRequest, Outcome},
        prefs_dialog,
    };
    use serde_json::{Value, json};
    use std::{cell::RefCell, rc::Rc};

    fn app() -> VectorcraftApp {
        let services = Services {
            check_application: Some(Box::new(|p| if p == "/Apps/Bad" { Err("not an application".into()) } else { Ok(()) })),
            open_in_application: Some(Box::new(|_, _| Ok(true))),
            ..Default::default()
        };
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), services);
        prefs_dialog::open(&mut app, Some("File Handling"));
        app
    }

    fn choose(app: &mut VectorcraftApp, path: Option<&str>) -> Result<(), String> {
        let path = path.map(str::to_owned);
        app.services.pick_open = Some(Box::new(move |_| path.clone()));
        let mut d = app.ui.dialog.take().unwrap();
        let r = pick(app, &mut d);
        app.ui.dialog = Some(d);
        r
    }

    fn request(app: &mut VectorcraftApp, method: &str, params: Value) -> Value {
        let (req, _) = ControlRequest::new(method, params);
        let Outcome::Done(r) = crate::control::handle(app, &egui::Context::default(), &req) else { panic!("deferred") };
        r
    }

    #[test]
    fn native_choice_commits_persists_cancels_and_resets() {
        let mut app = app();
        choose(&mut app, Some("/Apps/My Editor.app")).unwrap();
        assert!(app.ui.original_editor.path.is_none(), "a draft until OK");
        prefs_dialog::confirm(&mut app).unwrap();
        let saved = serde_json::to_vec(&app.ui).unwrap();
        let restored: crate::UiState = serde_json::from_slice(&saved).unwrap();
        assert_eq!(restored.original_editor.path.as_deref(), Some("/Apps/My Editor.app"));
        assert!(app.session.prefs.to_json().get("original_editor").is_none());
        assert!(app.take_prefs_changed(), "OK asks the host to persist the choice");
        prefs_dialog::open(&mut app, Some("File Handling"));
        choose(&mut app, None).unwrap();
        assert_eq!(app.original_editor.choice.path, restored.original_editor.path);
        choose(&mut app, Some("/Apps/Other Editor.app")).unwrap();
        crate::dialogs::cancel(&mut app);
        prefs_dialog::open(&mut app, Some("File Handling"));
        assert_eq!(app.original_editor.choice.path, restored.original_editor.path);
        assert!(choose(&mut app, Some("/Apps/Bad")).is_err());
        assert_eq!(app.original_editor.choice.path, restored.original_editor.path);
        reset(&mut app);
        prefs_dialog::confirm(&mut app).unwrap();
        assert!(app.ui.original_editor.path.is_none());
        let legacy: crate::UiState = serde_json::from_str("{}").unwrap();
        assert!(legacy.original_editor.path.is_none());
    }

    #[test]
    fn original_editor_routing_and_errors() {
        let mut app = app();
        choose(&mut app, Some("/Apps/My Editor.app")).unwrap();
        prefs_dialog::confirm(&mut app).unwrap();
        let calls = Rc::new(RefCell::new(Vec::new()));
        let c = calls.clone();
        app.services.open_file = Some(Box::new(move |_| {
            c.borrow_mut().push("system");
            Ok(())
        }));
        let c = calls.clone();
        app.services.open_in_photocraft = Some(Box::new(move |_| {
            c.borrow_mut().push("photocraft");
            Ok(true)
        }));
        let c = calls.clone();
        app.services.open_in_application = Some(Box::new(move |editor, path| {
            assert_eq!(editor, "/Apps/My Editor.app");
            assert_eq!(path, "/Art/a b.png");
            c.borrow_mut().push("chosen");
            Ok(true)
        }));
        crate::io::open_original(&mut app, "/Art/a b.png").unwrap();
        app.session.prefs.edit_original_system_defaults = false;
        crate::io::open_original(&mut app, "/Art/a b.png").unwrap();
        crate::io::open_original(&mut app, "/Art/plan.emf").unwrap();
        app.services.open_in_application = Some(Box::new(|_, _| Ok(false)));
        crate::io::open_original(&mut app, "/Art/a b.png").unwrap();
        app.services.open_in_photocraft = Some(Box::new(|_| Ok(false)));
        crate::io::open_original(&mut app, "/Art/a b.png").unwrap();
        app.services.open_in_application = Some(Box::new(|_, _| Err("launch failed".into())));
        assert_eq!(crate::io::open_original(&mut app, "/Art/a b.png").unwrap_err(), "launch failed");
        assert_eq!(*calls.borrow(), ["system", "chosen", "system", "photocraft", "system"]);
    }

    #[test]
    fn automation_cannot_inject_an_original_editor() {
        let mut app = app();
        for key in ["editOriginalApp", "original_editor", "__originalEditor", "__chosenApp"] {
            let r = request(&mut app, "engine.execute", json!({"command": "prefs.set", "params": {"key": key, "value": "/Apps/Bad"}}));
            assert_eq!(r["ok"], false, "{r}");
            let r = request(&mut app, "engine.execute", json!({"command": "prefs.set", "params": {"values": {key: "/Apps/Bad"}}}));
            assert_eq!(r["ok"], false, "{r}");
            // __ fields are accepted by generic dialogs, but they cannot become a trusted choice.
            let _ = request(&mut app, "ui.dialog.set", json!({"field": key, "value": "/Apps/Bad"}));
        }
        let r = request(&mut app, "engine.execute", json!({"command": "prefs.setValues", "params": {"values": {"editOriginalApp": "/Apps/Bad"}}}));
        assert_eq!(r["ok"], false, "{r}");
        let _ = request(&mut app, "ui.set", json!({"original_editor": {"path": "/Apps/Bad"}}));
        let r = request(&mut app, "ui.dialog.confirm", json!({}));
        assert_eq!(r["ok"], true, "{r}");
        assert!(app.ui.original_editor.path.is_none());
        prefs_dialog::open(&mut app, Some("File Handling"));
        app.ui.answering_control = true;
        assert!(choose(&mut app, Some("/Apps/Bad")).is_err());
        app.ui.answering_control = false;
        app.synthetic_frame = true;
        assert!(choose(&mut app, Some("/Apps/Bad")).is_err());
        assert!(app.original_editor.choice.path.is_none());
    }

    #[test]
    fn original_editor_async_picker_validates_and_commits_only_on_ok() {
        let mut app = app();
        let (tx, rx) = std::sync::mpsc::channel();
        let mut rx = Some(rx);
        app.services.start_pick = Some(Box::new(move |_| rx.take()));
        choose(&mut app, None).unwrap();
        tx.send(vec!["/Apps/Async Editor.app".into()]).unwrap();
        crate::picks::poll(&mut app, &egui::Context::default());
        assert_eq!(app.original_editor.choice.path.as_deref(), Some("/Apps/Async Editor.app"));
        assert!(app.ui.original_editor.path.is_none());
        prefs_dialog::confirm(&mut app).unwrap();
        assert_eq!(app.ui.original_editor.path.as_deref(), Some("/Apps/Async Editor.app"));
    }

    #[test]
    fn original_editor_picker_answer_does_not_survive_reopening_preferences() {
        let mut app = app();
        let (tx, rx) = std::sync::mpsc::channel();
        let mut rx = Some(rx);
        app.services.start_pick = Some(Box::new(move |_| rx.take()));
        choose(&mut app, None).unwrap();
        crate::dialogs::cancel(&mut app);
        prefs_dialog::open(&mut app, Some("File Handling"));
        tx.send(vec!["/Apps/Stale.app".into()]).unwrap();
        crate::picks::poll(&mut app, &egui::Context::default());
        assert!(app.original_editor.choice.path.is_none());
    }

    #[test]
    fn original_editor_controls_are_desktop_only_and_draw_the_choice() {
        let mut app = app();
        let text = crate::tests_labels::painted_text(&mut app, |a, ui| prefs_dialog::show(a, ui.ctx()));
        assert!(text.contains("Edit Original with:") && text.contains("PhotoCraft"), "{text}");
        app.services.check_application = None;
        let text = crate::tests_labels::painted_text(&mut app, |a, ui| prefs_dialog::show(a, ui.ctx()));
        assert!(!text.contains("Edit Original with:"), "{text}");
    }
}
