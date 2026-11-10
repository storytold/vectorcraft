//! Software updates, background check at startup, and startup prompt modal.

use serde_json::Value;

use crate::VectorcraftApp;

#[cfg(not(target_arch = "wasm32"))]
use craft_update::updater::{Action, Status, Updater};
#[cfg(not(target_arch = "wasm32"))]
use craft_update::{CheckResultDialog, CheckResultResponse};

pub(crate) struct Runtime {
    #[cfg(not(target_arch = "wasm32"))]
    updater: Updater,
    /// The desktop app turns the check at start on; tests and the web build leave it off.
    #[cfg(not(target_arch = "wasm32"))]
    startup_check: bool,
    #[cfg(not(target_arch = "wasm32"))]
    check_requested: bool,
    /// The context of the last frame, to wake the UI when a check is queued from a menu callback.
    #[cfg(not(target_arch = "wasm32"))]
    repaint: Option<egui::Context>,
    /// Whether an update dialog held the input in the last frame.
    #[cfg(not(target_arch = "wasm32"))]
    was_blocking: bool,
    /// The web build has no updater; its window only says so.
    #[cfg(target_arch = "wasm32")]
    pub(crate) manual_window_open: bool,
    #[cfg(target_arch = "wasm32")]
    pub(crate) status_message: Option<String>,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            updater: Updater::new(&crate::UPDATE_APP, env!("CARGO_PKG_VERSION")),
            #[cfg(not(target_arch = "wasm32"))]
            startup_check: false,
            #[cfg(not(target_arch = "wasm32"))]
            check_requested: false,
            #[cfg(not(target_arch = "wasm32"))]
            repaint: None,
            #[cfg(not(target_arch = "wasm32"))]
            was_blocking: false,
            #[cfg(target_arch = "wasm32")]
            manual_window_open: false,
            #[cfg(target_arch = "wasm32")]
            status_message: None,
        }
    }
}

impl VectorcraftApp {
    /// Whether the app asks for a newer release when it starts (off until the desktop app turns it on).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_startup_update_check(&mut self, on: bool) {
        self.updates_rt.startup_check = on;
    }

    /// Starts the install helper of an update that is ready, so it installs once the process has
    /// exited (does nothing when no update is ready). Call it in `on_exit` when the process ends
    /// there, since the app is then never dropped.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn start_pending_update(&mut self) {
        self.updates_rt.updater.start_pending_update();
    }
}

pub(crate) fn check(app: &mut VectorcraftApp) -> Result<Value, String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        app.updates_rt.check_requested = true;
        if let Some(ctx) = &app.updates_rt.repaint {
            ctx.request_repaint();
        }
        Ok(Value::Null)
    }
    #[cfg(target_arch = "wasm32")]
    {
        app.updates_rt.manual_window_open = true;
        app.updates_rt.status_message = Some("Update check is not supported in the web browser.".into());
        Ok(Value::Null)
    }
}

pub(crate) fn tick(app: &mut VectorcraftApp, ctx: &egui::Context) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let rt = &mut app.updates_rt;
        rt.repaint = Some(ctx.clone());
        if std::mem::take(&mut rt.check_requested) {
            rt.updater.check_now(ctx);
        } else if rt.startup_check {
            rt.updater.check_at_start(ctx);
        }
        rt.updater.tick(ctx);
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (app, ctx);
}

/// Whether the check result window shows: for every status but idle (a requested check, and the
/// first start after an update, which reports that it installed or why it failed), and never
/// over the startup prompt.
#[cfg(not(target_arch = "wasm32"))]
fn result_window_shows(status: &Status, prompt_open: bool) -> bool {
    !prompt_open && !matches!(status, Status::Idle)
}

/// Whether the update prompt or the modal result window is open: keys, tool keys and clipboard
/// events then stay out of the documents behind it, and the app's other windows take no input.
pub(crate) fn blocks_input(app: &VectorcraftApp) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let updater = &app.updates_rt.updater;
        holds_keyboard(updater.status(), updater.prompt_open())
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = app;
        false
    }
}

/// The prompt is modal, and so is the result window for every status but [`Status::Updated`], a
/// notice that leaves the app usable behind it.
#[cfg(not(target_arch = "wasm32"))]
fn holds_keyboard(status: &Status, prompt_open: bool) -> bool {
    prompt_open || !matches!(status, Status::Idle | Status::Updated(_))
}

/// Whether an update may install and close the app: no document holds unsaved work, no save,
/// export, file opening, plug-in filter or Image Trace is running, no action is playing, no dialog
/// or file dialog is open, and no tool is in the middle of a drag or an edit it computes on
/// another thread.
#[cfg(not(target_arch = "wasm32"))]
fn safe_to_close(app: &VectorcraftApp) -> bool {
    let session = &app.session;
    !crate::unsaved::any_dirty(app)
        && app.background.jobs.is_empty()
        && !crate::opening::long_operation_busy(app)
        && app.ui.action_playing.is_none()
        && app.ui.dialog.is_none()
        && !app.file_dialog_open()
        && !session.in_interaction()
        && !session.tool_busy()
        && !session.tool_editing()
}

pub(crate) fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        // An installation, and the close for a ready update, wait for unsaved and running work.
        let safe = safe_to_close(app);
        match app.updates_rt.updater.show_if(ctx, safe) {
            Action::OpenUrl(url) => app.open_url(&url),
            Action::Close => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Action::None => {}
        }
        // An available update goes to the prompt above, which replaces the result dialog.
        let rt = &mut app.updates_rt;
        if result_window_shows(rt.updater.status(), rt.updater.prompt_open()) {
            match CheckResultDialog::new(&crate::UPDATE_APP, env!("CARGO_PKG_VERSION")).show(ctx, rt.updater.status()) {
                CheckResultResponse::None => {}
                // Closing the dialog dismisses the outcome it showed.
                CheckResultResponse::Close => rt.updater.dismiss_status(),
                CheckResultResponse::CheckAgain => rt.updater.check_now(ctx),
                CheckResultResponse::RestartToUpdate => rt.updater.restart_to_update(),
            }
        }
        // A check at startup can open the prompt while a floating document's window covers it.
        let blocking = blocks_input(app);
        if blocking && !app.updates_rt.was_blocking {
            crate::doc_windows::focus_main_window(app, ctx);
        }
        app.updates_rt.was_blocking = blocking;
    }

    #[cfg(target_arch = "wasm32")]
    if app.updates_rt.manual_window_open {
        let mut open = true;
        egui::Window::new(tl!("VectorCraft Updates"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                if let Some(msg) = &app.updates_rt.status_message {
                    ui.label(msg);
                }
                ui.add_space(8.0);
                if ui.button(tl!("OK")).clicked() {
                    app.updates_rt.manual_window_open = false;
                }
            });
        if !open {
            app.updates_rt.manual_window_open = false;
        }
    }
}

/// Opens the update prompt for a test in another module.
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) fn open_prompt_for_test(app: &mut VectorcraftApp) {
    tests::offer(app);
}

/// Opens the result window of a running check for a test in another module.
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) fn open_result_window_for_test(app: &mut VectorcraftApp, ctx: &egui::Context) {
    app.updates_rt.updater = tests::isolated_updater(env!("CARGO_PKG_VERSION"), "result");
    app.updates_rt.updater.check_now(ctx);
}

/// Closes the update prompt and the result window for a test in another module.
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) fn close_dialogs_for_test(app: &mut VectorcraftApp) {
    app.updates_rt.updater = tests::isolated_updater(env!("CARGO_PKG_VERSION"), "closed");
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use vectorcraft_engine::Session;
    use vectorcraft_tools::{PointerEvent, PointerKind};

    /// The folder that holds `name`'s update records.
    #[cfg(not(target_arch = "wasm32"))]
    fn update_root(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("vectorcraft-updates-{}-{name}", std::process::id()))
    }

    /// An updater that reads its records and the launcher's pins only under [`update_root`].
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn isolated_updater(version: &str, name: &str) -> Updater {
        let root = update_root(name);
        Updater::new(&crate::UPDATE_APP, version)
            .with_source(std::sync::Arc::new(|| Ok(None)))
            .with_paths(craft_update::Paths::under(&root))
            .with_pins_path(root.join("pins.json"))
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn offer(app: &mut VectorcraftApp) {
        app.updates_rt.updater = isolated_updater(env!("CARGO_PKG_VERSION"), "prompt");
        app.updates_rt.updater.offer(craft_update::Release {
            tag: "v99.0.0".into(),
            version: craft_update::parse_version("99.0.0"),
            name: "VectorCraft 99.0.0".into(),
            html_url: "https://github.com/storytold/vectorcraft/releases/tag/v99.0.0".into(),
            ..Default::default()
        });
    }

    /// An app with one saved document holding a selected rectangle.
    #[cfg(not(target_arch = "wasm32"))]
    fn app_with_saved_object() -> VectorcraftApp {
        let services = crate::Services { write: Some(Box::new(|_: &str, _: &[u8]| Ok(()))), ..Default::default() };
        let mut app = VectorcraftApp::new(Session::new(), services);
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
        app.run("file.save", json!({"path": "/docs/saved.vectorcraft"})).unwrap();
        assert!(!crate::unsaved::any_dirty(&app));
        app
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn press(key: egui::Key) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }
    }

    /// One frame in the app's order: the shortcuts read the input, then the update dialogs draw.
    #[cfg(not(target_arch = "wasm32"))]
    fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) {
        let mut output = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            crate::shortcuts::handle(app, ui.ctx());
            show(app, ui.ctx());
        });
        output.textures_delta.clear();
    }

    /// A Copy, the Pen tool's key and Delete, in one frame each: none of them may act.
    #[cfg(not(target_arch = "wasm32"))]
    fn assert_keys_do_nothing(app: &mut VectorcraftApp, ctx: &egui::Context) {
        let tool = app.session.tool_id();
        let mut reached = Vec::new();
        frame(app, ctx, vec![egui::Event::Copy]);
        if !app.session.clipboard.is_empty() {
            reached.push("Copy");
        }
        let tool_key =
            egui::Event::Key { key: egui::Key::P, physical_key: Some(egui::Key::P), pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        frame(app, ctx, vec![tool_key, egui::Event::Text("p".into())]);
        if app.session.tool_id() != tool {
            reached.push("the tool key");
        }
        frame(app, ctx, vec![press(egui::Key::Delete)]);
        if app.session.doc().unwrap().selection.is_empty() {
            reached.push("Delete");
        }
        assert_eq!(reached, Vec::<&str>::new(), "these reached the document");
        assert!(!crate::unsaved::any_dirty(app), "a key edited the document");
    }

    #[test]
    fn default_runtime_is_idle() {
        let rt = Runtime::default();
        #[cfg(target_arch = "wasm32")]
        assert!(!rt.manual_window_open);
        #[cfg(not(target_arch = "wasm32"))]
        {
            assert!(!rt.startup_check);
            assert!(!rt.updater.prompt_open());
            assert_eq!(rt.updater.status(), &Status::Idle);
        }
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn result_window_shows_every_status_but_idle() {
        let shown = [
            Status::Checking,
            Status::UpToDate,
            Status::Failed("offline".into()),
            Status::Installing,
            Status::Ready,
            Status::Managed,
            Status::Updated("99.0.0".into()),
            Status::InstallFailed("locked".into()),
        ];
        for status in &shown {
            assert!(result_window_shows(status, false), "{status:?} shows");
            assert!(!result_window_shows(status, true), "{status:?} waits for the prompt");
        }
        assert!(!result_window_shows(&Status::Idle, false));
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn startup_prompt_shows_and_closes() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let ctx = egui::Context::default();
        let release = craft_update::Release {
            tag: "v99.0.0".into(),
            version: craft_update::parse_version("99.0.0"),
            name: "VectorCraft 99.0.0".into(),
            html_url: "https://github.com/storytold/vectorcraft/releases/tag/v99.0.0".into(),
            ..Default::default()
        };
        app.updates_rt.updater.offer(release);

        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            show(&mut app, ui.ctx());
        });
        output.textures_delta.clear();
        assert!(app.updates_rt.updater.prompt_open());
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn the_prompt_and_the_result_window_hold_the_keyboard_but_the_updated_notice_does_not() {
        for status in [
            Status::Checking,
            Status::UpToDate,
            Status::Failed("offline".into()),
            Status::Installing,
            Status::Ready,
            Status::Managed,
            Status::InstallFailed("locked".into()),
        ] {
            assert!(holds_keyboard(&status, false), "{status:?}");
        }
        assert!(!holds_keyboard(&Status::Idle, false));
        assert!(!holds_keyboard(&Status::Updated("99.0.0".into()), false));
        assert!(holds_keyboard(&Status::Idle, true), "the prompt");
    }

    /// The update prompt, drawn: keys stop at it, Escape closes it, and the next key acts again.
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn keys_do_not_reach_the_document_behind_the_update_prompt() {
        let mut app = app_with_saved_object();
        let ctx = egui::Context::default();
        offer(&mut app);
        frame(&mut app, &ctx, vec![]);
        assert_keys_do_nothing(&mut app, &ctx);
        assert!(app.updates_rt.updater.prompt_open());

        frame(&mut app, &ctx, vec![press(egui::Key::Escape)]);
        assert!(!app.updates_rt.updater.prompt_open(), "Escape closes the prompt");
        assert!(!app.session.doc().unwrap().selection.is_empty(), "Escape stops at the prompt");
        frame(&mut app, &ctx, vec![press(egui::Key::Delete)]);
        assert!(crate::unsaved::any_dirty(&app), "Delete acts once the prompt has closed");
    }

    /// The result window of a check that is still running, drawn: keys stop at it too.
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn keys_do_not_reach_the_document_behind_the_check_result_window() {
        let mut app = app_with_saved_object();
        let ctx = egui::Context::default();
        app.updates_rt.updater = isolated_updater("0.0.1", "checking");
        app.updates_rt.updater.check_now(&ctx);
        assert_eq!(app.updates_rt.updater.status(), &Status::Checking);
        frame(&mut app, &ctx, vec![]);
        assert_keys_do_nothing(&mut app, &ctx);
    }

    /// The notice of a finished update, drawn: the keys act behind it.
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn keys_reach_the_document_behind_the_updated_notice() {
        let mut app = app_with_saved_object();
        let ctx = egui::Context::default();
        let root = update_root("updated");
        let records = craft_update::Paths::under(&root).data;
        std::fs::create_dir_all(&records).unwrap();
        std::fs::write(records.join("self-update-done.txt"), "v99.0.0").unwrap();
        app.updates_rt.updater = isolated_updater("99.0.0", "updated");
        app.updates_rt.updater.check_at_start(&ctx);
        assert_eq!(app.updates_rt.updater.status(), &Status::Updated("99.0.0".into()));
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![press(egui::Key::Delete)]);
        let _ = std::fs::remove_dir_all(&root);
        assert!(crate::unsaved::any_dirty(&app), "Delete acts behind the notice");
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn unsaved_changes_dialogs_and_running_work_hold_an_update_back() {
        let mut app = app_with_saved_object();
        assert!(safe_to_close(&app));

        app.run("shape.rectangle", json!({"x": 70, "y": 10, "width": 50, "height": 50})).unwrap();
        assert!(!safe_to_close(&app), "unsaved changes");
        app.run("file.save", json!({"path": "/docs/saved.vectorcraft"})).unwrap();
        assert!(safe_to_close(&app));

        app.ui.dialog = Some(crate::state::Dialog::new("about", json!({})));
        assert!(!safe_to_close(&app), "a dialog");
        app.ui.dialog = None;

        let (shown, _) = crate::tests_picks::off_the_ui_thread(&mut app.services);
        // The command asks again with the answer: until then it reports that it has none.
        assert!(app.run("file.open", json!({})).is_err());
        assert!(app.file_dialog_open());
        assert!(!safe_to_close(&app), "a file dialog");
        shown.answer(&[]);
        crate::picks::poll(&mut app, &egui::Context::default());
        assert!(!app.file_dialog_open());
        assert!(safe_to_close(&app));

        app.services.write_shared = Some(std::sync::Arc::new(|_: &str, _: &[u8]| Ok(())));
        let (release, held) = std::sync::mpsc::channel::<()>();
        let work = move |_: crate::background::Writer| {
            let _ = held.recv();
            Ok(Value::Null)
        };
        // No label: a job the status bar does not list holds the close back too.
        crate::background::run(&mut app, true, String::new(), None, work, |_, r| r).unwrap();
        assert!(!safe_to_close(&app), "a background save");
        let _ = release.send(());
        crate::background::wait_all(&mut app);
        assert!(safe_to_close(&app));

        crate::opening::start(&mut app, "/art/Wait.vectorcraft", vectorcraft_format::save(&vectorcraft_doc::Document::new(10.0, 10.0), false))
            .unwrap();
        assert!(!safe_to_close(&app), "a file opening");
        app.opening.pending.clear();
        assert!(safe_to_close(&app));

        use crate::panels::actions::{Action, ActionSet, PlaybackSpeed, play};
        let step = ("object.move".to_owned(), json!({"dx": 1, "dy": 0}));
        app.ui.action_sets = vec![ActionSet::new("Test", vec![Action::new("Move", vec![step.clone(), step])])];
        app.ui.action_playback.speed = PlaybackSpeed::Step;
        assert_eq!(play(&mut app, 0, 0).unwrap()["pending"], json!(1));
        app.run("file.save", json!({"path": "/docs/saved.vectorcraft"})).unwrap();
        crate::background::wait_all(&mut app);
        assert!(!safe_to_close(&app), "an action playing");
        app.ui.action_playing = None;
        assert!(safe_to_close(&app));
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn a_gesture_or_a_tools_edit_holds_an_update_back() {
        let mut app = app_with_saved_object();
        let view = app.view_info();
        app.session.begin_interaction("Move").unwrap();
        assert!(!safe_to_close(&app), "an interaction");
        app.session.cancel_interaction().unwrap();
        assert!(safe_to_close(&app));

        app.select_tool("selection");
        app.session.pointer(&PointerEvent::new(PointerKind::Down, 150.0, 150.0), view).unwrap();
        app.session.pointer(&PointerEvent::new(PointerKind::Drag, 180.0, 180.0), view).unwrap();
        assert!(app.session.tool_busy() && !app.session.in_interaction());
        assert!(!safe_to_close(&app), "a drag with a tool");
        app.session.pointer(&PointerEvent::new(PointerKind::Up, 180.0, 180.0), view).unwrap();
        assert!(!app.session.tool_busy());
        assert!(safe_to_close(&app));

        // Enough segments for the merge to run on another thread.
        for k in 0..40 {
            app.run("shape.rectangle", json!({"x": 10 * k, "y": 100, "width": 20, "height": 20})).unwrap();
        }
        app.run("file.save", json!({"path": "/docs/saved.vectorcraft"})).unwrap();
        app.run("select.all", json!({})).unwrap();
        app.select_tool("shapeBuilder");
        app.session.pointer(&PointerEvent::new(PointerKind::Move, 15.0, 110.0), view).unwrap();
        app.session.tool_poll(view, true).unwrap();
        app.session.pointer(&PointerEvent::new(PointerKind::Down, 15.0, 110.0), view).unwrap();
        app.session.pointer(&PointerEvent::new(PointerKind::Up, 15.0, 110.0), view).unwrap();
        assert!(app.session.tool_editing() && !crate::unsaved::any_dirty(&app), "the click starts a merge on another thread");
        assert!(!safe_to_close(&app), "a tool's edit");
        crate::unsaved::finish_pending(&mut app);
    }
}
