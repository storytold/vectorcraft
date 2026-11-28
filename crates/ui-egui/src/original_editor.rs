//! The raster editor is selected by a person in a native picker, never by a preference command
//! or a generic dialog field. Keep both the saved choice and the dialog's draft opaque.

use std::path::Path;

use craft::host::{Host, PickId, PickRequest};
use craft::ui::buttons::Button;
use craft::ui::contract::{Props as _, Show as _, ShowContainer as _};
use craft::ui::display::Text;
use craft::ui::layout_components::Row;
use serde::{Deserialize, Serialize};

use crate::VectorcraftApp;

/// Opaque saved choice. Public so hosts can construct `UiState` with a struct update; its path
/// has no public setter and is never read from the generic preference or dialog-field maps.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Choice {
    path: Option<String>,
}

#[derive(Default)]
pub(crate) struct Draft {
    choice: Choice,
    /// The picker Choose… opened; its answer is taken by the row, and a reopened Preferences forgets it.
    pick: Option<PickId>,
}

// PLAN: assumed app.ui_ctx readable here (the window's context).
fn host(app: &VectorcraftApp) -> Option<std::sync::Arc<Host>> {
    app.ui_ctx.as_ref().and_then(Host::try_of_ctx)
}

/// Drops a pick still open, so its answer never reaches the draft.
fn forget(app: &mut VectorcraftApp) {
    if let Some(ticket) = app.original_editor.pick.take()
        && let Some(host) = host(app)
    {
        host.files().forget(ticket);
    }
}

pub(crate) fn open(app: &mut VectorcraftApp) {
    forget(app);
    app.original_editor.choice = app.ui.original_editor.clone();
}

/// Saves the dialog's choice; returns whether it differs from the saved one.
pub(crate) fn commit(app: &mut VectorcraftApp) -> bool {
    let changed = app.ui.original_editor != app.original_editor.choice;
    app.ui.original_editor = app.original_editor.choice.clone();
    changed
}

pub(crate) fn reset(app: &mut VectorcraftApp) {
    forget(app);
    app.original_editor.choice = Choice::default();
}

/// `prefs.reset`: the saved choice goes back to the default, and the next Preferences shows it.
pub(crate) fn reset_saved(app: &mut VectorcraftApp) {
    app.ui.original_editor = Choice::default();
    reset(app);
}

pub(crate) fn cancel(app: &mut VectorcraftApp) {
    open(app);
}

/// A chosen app may have been removed since Preferences saved it. Only absence falls through;
/// validation and launch failures are errors, as they are for PhotoCraft.
pub(crate) fn try_open(app: &mut VectorcraftApp, path: &str) -> Result<bool, String> {
    let Some(editor) = app.ui.original_editor.path.as_deref() else { return Ok(false) };
    launch(editor, path)
}

#[cfg(not(target_arch = "wasm32"))]
fn launch(editor: &str, path: &str) -> Result<bool, String> {
    match craft::platform::open::open_with(Path::new(editor), Path::new(path)) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(target_arch = "wasm32")]
fn launch(_: &str, _: &str) -> Result<bool, String> {
    Ok(false)
}

fn pick(app: &mut VectorcraftApp) -> Result<(), String> {
    app.refuse_automation("preferences.chooseOriginalEditor")?;
    let host = host(app).ok_or("no file picker here")?;
    forget(app);
    let request = PickRequest::open("originalEditor");
    let request = if cfg!(target_os = "macos") {
        request.filter("Applications", &["app"])
    } else if cfg!(windows) {
        request.filter("Applications", &["exe"])
    } else {
        request
    };
    app.original_editor.pick = Some(host.files().pick(request));
    Ok(())
}

/// The answer of the picker Choose… opened, once it has come: a draft until Preferences' OK.
fn take_picked(app: &mut VectorcraftApp, host: &Host) -> Result<(), String> {
    let Some(ticket) = app.original_editor.pick else { return Ok(()) };
    let Some(picked) = host.files().take_answer(ticket) else { return Ok(()) };
    app.original_editor.pick = None;
    let Some(path) = picked.paths().into_iter().next() else { return Ok(()) };
    if !craft::platform::open::is_application(&path) {
        return Err(tl!("not an application").into());
    }
    app.original_editor.choice.path = Some(path.to_string_lossy().into_owned());
    Ok(())
}

/// The File Handling row: the chosen editor, Choose… and PhotoCraft. It draws only where the platform opens files with an
/// application; `enabled` is off while Use System Defaults for Edit Original is on.
pub(crate) fn show(app: &mut VectorcraftApp, ui: &mut egui::Ui, enabled: bool) {
    if !craft::platform::open::CAN_OPEN_WITH {
        return;
    }
    if let Some(host) = Host::try_of_ctx(ui.ctx())
        && let Err(e) = take_picked(app, &host)
    {
        app.status(e);
    }
    let chosen = app.original_editor.choice.path.clone();
    let (mut choose, mut default) = (false, false);
    Row::new().wrap(true).show(ui, |ui, _| {
        Text::new(tl!("Edit Original with:")).show(ui);
        let name = chosen.as_deref().and_then(|p| Path::new(p).file_name());
        Text::new(name.map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "PhotoCraft".into())).show(ui);
        choose = Button::new(tl!("Choose…")).enabled(enabled).show(ui).clicked();
        default = chosen.is_some() && Button::new("PhotoCraft").enabled(enabled).show(ui).clicked();
    });
    if choose && let Err(e) = pick(app) {
        app.status(e);
    }
    if default {
        reset(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        prefs_dialog,
        tests::picks::{Picker, scripted},
    };
    use craft::automation::{ControlRequest, Reply};
    use craft::ui::dialog::PromptChoice;
    use serde_json::{Value, json};
    use std::{cell::RefCell, rc::Rc};

    fn app() -> (VectorcraftApp, Picker) {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let picker = scripted(&mut app);
        prefs_dialog::open(&mut app, Some("File Handling"));
        (app, picker)
    }

    // PLAN: assumed `app.ui_ctx` readable from tests (the window's context, which holds the scripted host).
    fn ctx(app: &VectorcraftApp) -> egui::Context {
        app.ui_ctx.clone().expect("a window")
    }

    /// An application the platform accepts: an `.app` folder, an `.exe` file, or an executable file.
    fn editor(name: &str) -> String {
        let dir = std::env::temp_dir().join(format!("vectorcraft-editor-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = if cfg!(target_os = "macos") {
            let p = dir.join("Editor.app");
            std::fs::create_dir_all(&p).unwrap();
            p
        } else {
            let p = dir.join(if cfg!(windows) { "Editor.exe" } else { "editor" });
            std::fs::write(&p, b"").unwrap();
            #[cfg(unix)]
            std::fs::set_permissions(&p, <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o755)).unwrap();
            p
        };
        path.to_string_lossy().into_owned()
    }

    /// An existing file that is not an application.
    fn not_an_application(name: &str) -> String {
        let p = std::env::temp_dir().join(format!("vectorcraft-editor-{name}-{}.txt", std::process::id()));
        std::fs::write(&p, b"text").unwrap();
        p.to_string_lossy().into_owned()
    }

    /// One pass of the File Handling row.
    fn row(app: &mut VectorcraftApp) {
        let ctx = ctx(app);
        ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui, true)).textures_delta.clear();
    }

    /// Choose… answered with `path` (cancelled for none), then the pass that takes the answer.
    fn choose(app: &mut VectorcraftApp, picker: &Picker, path: Option<&str>) -> Result<(), String> {
        pick(app)?;
        match path {
            Some(path) => picker.answer(&[path]),
            None => picker.cancel(),
        }
        row(app);
        Ok(())
    }

    // PLAN: assumed `crate::control::handle(app, &ctx, &req) -> Reply` with `Reply::Done(Value)` (shell-w12).
    fn request(app: &mut VectorcraftApp, method: &str, params: Value) -> Value {
        let (req, _) = ControlRequest::new(method, params);
        let ctx = ctx(app);
        let Reply::Done(r) = crate::control::handle(app, &ctx, &req) else { panic!("deferred") };
        r
    }

    #[test]
    fn native_choice_commits_persists_cancels_and_resets() {
        let (mut app, picker) = app();
        let (mine, other, bad) = (editor("mine"), editor("other"), not_an_application("bad"));
        choose(&mut app, &picker, Some(&mine)).unwrap();
        assert!(app.ui.original_editor.path.is_none(), "a draft until OK");
        prefs_dialog::confirm(&mut app).unwrap();
        let saved = serde_json::to_vec(&app.ui).unwrap();
        let restored: crate::UiState = serde_json::from_slice(&saved).unwrap();
        assert_eq!(restored.original_editor.path.as_deref(), Some(mine.as_str()));
        assert!(app.session.prefs.to_json().get("original_editor").is_none());
        assert!(app.take_prefs_changed(), "OK asks the host to persist the choice");
        prefs_dialog::open(&mut app, Some("File Handling"));
        choose(&mut app, &picker, None).unwrap();
        assert_eq!(app.original_editor.choice.path, restored.original_editor.path);
        choose(&mut app, &picker, Some(&other)).unwrap();
        cancel(&mut app);
        prefs_dialog::open(&mut app, Some("File Handling"));
        assert_eq!(app.original_editor.choice.path, restored.original_editor.path);
        choose(&mut app, &picker, Some(&bad)).unwrap();
        assert_eq!(app.original_editor.choice.path, restored.original_editor.path, "not an application");
        reset(&mut app);
        prefs_dialog::confirm(&mut app).unwrap();
        assert!(app.ui.original_editor.path.is_none());
        let legacy: crate::UiState = serde_json::from_str("{}").unwrap();
        assert!(legacy.original_editor.path.is_none());
    }

    #[test]
    fn original_editor_reset_confirmation_preserves_cancel_and_defers_persistence() {
        let (mut app, picker) = app();
        let mine = editor("reset");
        choose(&mut app, &picker, Some(&mine)).unwrap();
        prefs_dialog::confirm(&mut app).unwrap();
        prefs_dialog::open(&mut app, Some("File Handling"));
        prefs_dialog::ask_reset(&mut app);
        prefs_dialog::answer_reset(&mut app, PromptChoice::Cancel);
        assert_eq!(app.original_editor.choice.path.as_deref(), Some(mine.as_str()));
        prefs_dialog::ask_reset(&mut app);
        prefs_dialog::answer_reset(&mut app, PromptChoice::Proceed);
        assert!(app.original_editor.choice.path.is_none());
        assert_eq!(app.ui.original_editor.path.as_deref(), Some(mine.as_str()));
        cancel(&mut app);
        prefs_dialog::open(&mut app, Some("File Handling"));
        assert_eq!(app.original_editor.choice.path.as_deref(), Some(mine.as_str()));
        prefs_dialog::ask_reset(&mut app);
        prefs_dialog::answer_reset(&mut app, PromptChoice::Proceed);
        prefs_dialog::confirm(&mut app).unwrap();
        assert!(app.ui.original_editor.path.is_none());
    }

    /// `prefs.reset` of every category or of File Handling, alone or as a batch step, resets the
    /// saved choice; a reset of another category keeps it.
    #[test]
    fn prefs_reset_resets_the_saved_choice_with_file_handling() {
        let (mut app, picker) = app();
        prefs_dialog::cancel(&mut app);
        let mine = editor("prefs-reset");
        let chosen = |app: &mut VectorcraftApp| {
            prefs_dialog::open(app, Some("File Handling"));
            choose(app, &picker, Some(&mine)).unwrap();
            prefs_dialog::confirm(app).unwrap();
            assert!(app.ui.original_editor.path.is_some());
        };
        chosen(&mut app);
        app.run("prefs.reset", json!({"category": "General"})).unwrap();
        assert!(app.ui.original_editor.path.is_some(), "General keeps the choice");
        app.run("prefs.reset", json!({"category": "File Handling"})).unwrap();
        assert!(app.ui.original_editor.path.is_none(), "File Handling resets it");
        chosen(&mut app);
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("command.batch", json!({"commands": [{"command": "prefs.reset", "params": {}}]})).unwrap();
        assert!(app.ui.original_editor.path.is_none(), "every category, as a batch step");
        prefs_dialog::open(&mut app, Some("File Handling"));
        assert!(app.original_editor.choice.path.is_none(), "Preferences shows the default");
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn original_editor_routing_and_errors() {
        let (mut app, _picker) = app();
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
        // The chosen application is gone: only absence falls through.
        app.original_editor.choice.path = Some(std::env::temp_dir().join("vectorcraft-no-such-editor").to_string_lossy().into_owned());
        commit(&mut app);
        crate::io::open_original(&mut app, "/Art/a b.png").unwrap();
        app.session.prefs.edit_original_system_defaults = false;
        crate::io::open_original(&mut app, "/Art/a b.png").unwrap();
        crate::io::open_original(&mut app, "/Art/plan.emf").unwrap();
        app.services.open_in_photocraft = Some(Box::new(|_| Ok(false)));
        crate::io::open_original(&mut app, "/Art/a b.png").unwrap();
        assert_eq!(*calls.borrow(), ["system", "photocraft", "system", "system"]);
        // A chosen file that is not an application is an error, not a fall through.
        app.original_editor.choice.path = Some(not_an_application("routing"));
        commit(&mut app);
        assert!(crate::io::open_original(&mut app, "/Art/a b.png").is_err());
    }

    #[test]
    fn automation_cannot_inject_an_original_editor() {
        let (mut app, picker) = app();
        let bad = editor("injected");
        for key in ["editOriginalApp", "original_editor", "__originalEditor", "__chosenApp"] {
            let r = request(&mut app, "engine.execute", json!({"command": "prefs.set", "params": {"key": key, "value": bad}}));
            assert_eq!(r["ok"], false, "{r}");
            let r = request(&mut app, "engine.execute", json!({"command": "prefs.set", "params": {"values": {key: bad}}}));
            assert_eq!(r["ok"], false, "{r}");
            let _ = request(&mut app, "ui.dialog.set", json!({"field": key, "value": bad}));
        }
        let r = request(&mut app, "engine.execute", json!({"command": "prefs.setValues", "params": {"values": {"editOriginalApp": bad}}}));
        assert_eq!(r["ok"], false, "{r}");
        let _ = request(&mut app, "ui.set", json!({"original_editor": {"path": bad}}));
        let r = request(&mut app, "ui.dialog.confirm", json!({}));
        assert_eq!(r["ok"], true, "{r}");
        assert!(app.ui.original_editor.path.is_none());
        prefs_dialog::open(&mut app, Some("File Handling"));
        app.ui.answering_control = true;
        assert!(choose(&mut app, &picker, Some(&bad)).is_err());
        app.ui.answering_control = false;
        // A pass that carries injected input acts for automation too.
        let ctx = ctx(&app);
        craft::ui::runtime::windows::WindowHost::of(&ctx).inject_key(egui::ViewportId::ROOT, craft::keymap::Key::A, Default::default(), false);
        ctx.run_ui(egui::RawInput::default(), |_| assert!(pick(&mut app).is_err())).textures_delta.clear();
        assert!(app.original_editor.choice.path.is_none());
    }

    #[test]
    fn original_editor_async_picker_validates_and_commits_only_on_ok() {
        let (mut app, picker) = app();
        let late = editor("late");
        pick(&mut app).unwrap();
        picker.answer(&[&late]);
        row(&mut app);
        assert_eq!(app.original_editor.choice.path.as_deref(), Some(late.as_str()));
        assert!(app.ui.original_editor.path.is_none());
        prefs_dialog::confirm(&mut app).unwrap();
        assert_eq!(app.ui.original_editor.path.as_deref(), Some(late.as_str()));
    }

    #[test]
    fn original_editor_picker_answer_does_not_survive_reopening_preferences() {
        let (mut app, picker) = app();
        let stale = editor("stale");
        pick(&mut app).unwrap();
        cancel(&mut app);
        prefs_dialog::open(&mut app, Some("File Handling"));
        picker.answer(&[&stale]);
        row(&mut app);
        assert!(app.original_editor.choice.path.is_none());
    }
}
