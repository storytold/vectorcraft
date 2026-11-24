//! Software updates: the check at startup, the startup prompt and the check-result dialog, both
//! on the dialog host.

use serde_json::Value;

use crate::VectorcraftApp;

use craft::ui::dialog::{Alert, HostedAlert};
#[cfg(not(target_arch = "wasm32"))]
use craft::update::stack_dialogs::{CheckChoice, UpdateCheckDialog, UpdatePromptDialog};
#[cfg(not(target_arch = "wasm32"))]
use craft::update::updater::{Action, Status, Updater};

/// Whether this build checks for, downloads and installs VectorCraft's releases itself. A nightly
/// does not: ArtCraft Launcher manages its updates, and the updater keeps its records, downloads
/// and logs in the stable app's folders (`%LOCALAPPDATA%\VectorCraft` on Windows), which the
/// profile fence refuses.
#[cfg(not(target_arch = "wasm32"))]
const fn app_updates_allowed_for(identity: crate::os_window::LaunchIdentity) -> bool {
    crate::os_window::stable_services_for(identity)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct Runtime {
    /// This build's updater: absent in a nightly, which never checks, downloads or installs.
    updater: Option<Updater>,
    /// The desktop app turns the check at start on; tests leave it off.
    startup_check: bool,
    /// Whether a modal update dialog was open in the last pass.
    was_modal: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl Runtime {
    /// The runtime `identity` uses: a nightly keeps no updater at all, so no check, download,
    /// install or record can start from it.
    fn for_identity(identity: crate::os_window::LaunchIdentity) -> Self {
        let updater = app_updates_allowed_for(identity).then(|| Updater::new(&crate::UPDATE_APP, env!("CARGO_PKG_VERSION")));
        Self { updater, startup_check: false, was_modal: false }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for Runtime {
    fn default() -> Self {
        Self::for_identity(crate::os_window::identity())
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
        let Some(updater) = self.updates_rt.updater.as_mut() else { return };
        updater.start_pending_update();
    }
}

/// The result dialog of a check; its status comes from [`pump`].
#[cfg(not(target_arch = "wasm32"))]
fn check_dialog() -> UpdateCheckDialog {
    UpdateCheckDialog::new(env!("CARGO_PKG_VERSION"))
}

pub(crate) fn check(app: &mut VectorcraftApp) -> Result<Value, String> {
    // PLAN: assumed `VectorcraftApp::dialogs() -> Option<handle>` whose `with(|host: &mut DialogHost| ..)` runs on the host (wg.S1).
    let dialogs = app.dialogs().ok_or("no window")?;
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Some(updater) = app.updates_rt.updater.as_mut() else {
            // A nightly's updates are the Launcher's: say so, rather than offer a stable release
            // that would install over the profile this build has to stay inside.
            dialogs.with(|h| {
                // One notice is enough: a second Check while it is open shows nothing new.
                if !h.is_open::<HostedAlert>() {
                    h.open(HostedAlert::new(Alert::new(
                        tl!("VectorCraft Updates").to_string(),
                        tl!("VectorCraft Nightly's updates come from ArtCraft Launcher.").to_string(),
                    )));
                }
            });
            return Ok(Value::Null);
        };
        // An open result dialog shows the new status itself.
        dialogs.with(|h| {
            let _ = h.open_unless_open(check_dialog());
        });
        updater.check_now();
    }
    #[cfg(target_arch = "wasm32")]
    dialogs.with(|h| {
        h.open(HostedAlert::new(Alert::new(
            tl!("VectorCraft Updates").to_string(),
            tl!("Update check is not supported in the web browser.").to_string(),
        )));
    });
    Ok(Value::Null)
}

/// Before the shell pass: the updater works, and puts its startup prompt on the host, which must
/// happen before `DialogHost::run`.
pub(crate) fn tick(app: &mut VectorcraftApp, ctx: &egui::Context) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        // A nightly keeps no updater: nothing here runs, so nothing checks, downloads or installs.
        if app.updates_rt.updater.is_none() {
            return;
        }
        if app.updates_rt.startup_check
            && let Some(updater) = app.updates_rt.updater.as_mut()
        {
            updater.check_at_start();
        }
        let Some(updater) = app.updates_rt.updater.as_mut() else { return };
        updater.tick(ctx);
        // An installation, and the close for a ready update, wait for unsaved and running work.
        let safe = safe_to_close(app);
        let Some(dialogs) = app.dialogs() else { return };
        let Some(updater) = app.updates_rt.updater.as_mut() else { return };
        match dialogs.with(|h| updater.run(h, safe)) {
            Action::OpenUrl(url) => app.open_url(&url),
            Action::Close => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            _ => {}
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (app, ctx);
}

/// Whether an update may install and close the app: no document holds unsaved work, no save,
/// export, file opening, plug-in filter or Image Trace is running, no action is playing, no
/// dialog (the update's own aside) or file dialog is open, and no tool is in the middle of a drag
/// or an edit it computes on another thread.
#[cfg(not(target_arch = "wasm32"))]
fn safe_to_close(app: &VectorcraftApp) -> bool {
    let session = &app.session;
    // The update's own dialogs would hold the install back forever.
    let others_open = app.dialogs().is_some_and(|d| {
        d.with(|h| h.open_dialogs().len() > usize::from(h.is_open::<UpdatePromptDialog>()) + usize::from(h.is_open::<UpdateCheckDialog>()))
    });
    !crate::unsaved::any_dirty(app)
        && app.background.jobs.is_empty()
        && !crate::opening::long_operation_busy(app)
        && app.ui.action_playing.is_none()
        && !others_open
        && !app.file_dialog_open()
        && !session.in_interaction()
        && !session.tool_busy()
        && !session.tool_editing()
}

/// After the shell pass: feeds the check dialog the updater's status and opens it for every
/// status but idle (a requested check, and the first start after an update, which reports that it
/// installed or why it failed), never over the startup prompt; carries out what it answered.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &egui::Context) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Some(dialogs) = app.dialogs() else { return };
        let Some(updater) = app.updates_rt.updater.as_mut() else { return };
        let status = updater.status().clone();
        let prompt_open = updater.prompt_open();
        let (answered, modal) = dialogs.with(|h| {
            let answered = h.take::<UpdateCheckDialog>();
            if answered.is_none() {
                if let Some(dialog) = h.get_mut::<UpdateCheckDialog>() {
                    dialog.set_status(status.clone());
                } else if !prompt_open && status != Status::Idle {
                    let mut dialog = check_dialog();
                    dialog.set_status(status.clone());
                    h.open(dialog);
                }
            }
            // The prompt is modal, and so is the result dialog for every status but `Updated`, a
            // notice that leaves the app usable behind it.
            (answered, prompt_open || (h.is_open::<UpdateCheckDialog>() && !matches!(status, Status::Updated(_))))
        });
        if let Some(result) = answered {
            match result.ok() {
                Some(CheckChoice::CheckAgain | CheckChoice::CheckNow) => updater.check_now(),
                Some(CheckChoice::RestartToUpdate) => updater.restart_to_update(),
                // Closing the dialog dismisses the outcome it showed.
                Some(CheckChoice::Close) | None => updater.dismiss_status(),
                // The app never offers a release itself, so Update and Download do not arrive.
                Some(_) => {}
            }
        }
        // A check at startup can open the prompt while a floating document's window covers it.
        if modal && !app.updates_rt.was_modal {
            crate::doc_windows::focus_main_window(app, ctx);
        }
        app.updates_rt.was_modal = modal;
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (app, ctx);
}

/// Opens the update prompt for a test in another module.
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) fn open_prompt_for_test(app: &mut VectorcraftApp) {
    tests::offer(app);
}

/// Closes the update prompt and the result dialog for a test in another module.
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) fn close_dialogs_for_test(app: &mut VectorcraftApp) {
    app.updates_rt.updater = Some(tests::isolated_updater(env!("CARGO_PKG_VERSION"), "closed"));
    // The dialogs are the host's: a new updater does not take them down.
    if let (Some(ctx), Some(dialogs)) = (app.ui_ctx.clone(), app.dialogs()) {
        dialogs.with(|h| {
            h.cancel::<UpdatePromptDialog>(&ctx);
            h.cancel::<UpdateCheckDialog>(&ctx);
        });
    }
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
            .with_paths(craft::update::Paths::under(&root))
            .with_pins_path(root.join("pins.json"))
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn offer(app: &mut VectorcraftApp) {
        let mut updater = isolated_updater(env!("CARGO_PKG_VERSION"), "prompt");
        offer_and_wait(&mut updater, release_99());
        app.updates_rt.updater = Some(updater);
    }

    /// The release the tests offer.
    #[cfg(not(target_arch = "wasm32"))]
    fn release_99() -> craft::update::Release {
        craft::update::Release {
            tag: "v99.0.0".into(),
            version: craft::update::parse_version("99.0.0"),
            name: "VectorCraft 99.0.0".into(),
            html_url: "https://github.com/storytold/vectorcraft/releases/tag/v99.0.0".into(),
            ..Default::default()
        }
    }

    /// Offers `release` and returns once its prompt is open. The updater reads the installed
    /// files for an offer on a thread of its own, which asks the context the updater keeps for a
    /// repaint after it answers; the next tick opens the prompt. The observer sees every repaint
    /// request, so each immediate one is followed by a tick until the prompt opens. The updater
    /// must not have kept another context, which its thread would ask instead. A copy the
    /// launcher manages ignores the offer and fails the test.
    #[cfg(not(target_arch = "wasm32"))]
    fn offer_and_wait(updater: &mut Updater, release: craft::update::Release) {
        let ctx = crate::testing::ui_context();
        updater.tick(&ctx);
        updater.offer(release);
        // `offer` reads the launcher's choice; a managed copy starts no inspection.
        assert!(updater.managed().is_none(), "the launcher manages this copy, so it ignored the offer");
        // The offer's thread only answers; `tick` polls for it, as each frame does.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !updater.prompt_open() {
            assert!(std::time::Instant::now() < deadline, "the offer's prompt never opened");
            std::thread::sleep(std::time::Duration::from_millis(5));
            updater.tick(&ctx);
        }
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

    /// A nightly keeps no updater: its check points at ArtCraft Launcher, and no frame or exit
    /// hook starts a check, a download or an install.
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn a_nightly_never_checks_downloads_or_installs_an_update() {
        use crate::os_window::LaunchIdentity;
        assert!(app_updates_allowed_for(LaunchIdentity::Stable));
        assert!(!app_updates_allowed_for(LaunchIdentity::NightlyV1));
        assert!(Runtime::for_identity(LaunchIdentity::NightlyV1).updater.is_none());
        assert!(Runtime::for_identity(LaunchIdentity::Stable).updater.is_some());

        let mut app = app_with_saved_object();
        let ctx = crate::testing::ui_context();
        app.updates_rt.updater = None;
        // Neither a frame nor the exit hook does anything at all.
        tick(&mut app, &ctx);
        pump(&mut app, &ctx);
        app.start_pending_update();
        let opened = app.dialogs().unwrap().with(|h| h.open_dialogs().len());
        assert_eq!(opened, 0, "a nightly opens no update dialog on its own");
        // Asked to check, it says who manages its updates instead of asking about a release.
        check(&mut app).unwrap();
        assert!(app.dialogs().unwrap().with(|h| h.is_open::<HostedAlert>()), "the check names ArtCraft Launcher");
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn unsaved_changes_dialogs_and_running_work_hold_an_update_back() {
        use craft::ui::dialog::{Alert, HostedAlert};

        let mut app = app_with_saved_object();
        let ctx = crate::unsaved::ready_for_test(&mut app);
        assert!(safe_to_close(&app));

        app.run("shape.rectangle", json!({"x": 70, "y": 10, "width": 50, "height": 50})).unwrap();
        assert!(!safe_to_close(&app), "unsaved changes");
        app.run("file.save", json!({"path": "/docs/saved.vectorcraft"})).unwrap();
        assert!(safe_to_close(&app));

        app.dialogs().unwrap().with(|h| {
            h.open(HostedAlert::new(Alert::new("About", "A dialog")));
        });
        assert!(!safe_to_close(&app), "a dialog");
        app.dialogs().unwrap().with(|h| h.cancel::<HostedAlert>(&ctx));
        assert!(safe_to_close(&app));

        let picker = crate::tests_picks::scripted(&mut app);
        // The command goes on with the answer: until then the dialog is open.
        app.run("file.open", json!({})).unwrap();
        assert!(app.file_dialog_open());
        assert!(!safe_to_close(&app), "a file dialog");
        picker.cancel();
        crate::tests_picks::poll(&mut app, &ctx);
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

        crate::opening::start(&mut app, "/art/Wait.vectorcraft", vectorcraft_format::save(&vectorcraft_doc::Document::new(10.0, 10.0), false));
        assert!(!safe_to_close(&app), "a file opening");
        app.opening.pending.clear();
        assert!(safe_to_close(&app));

        use crate::panels::actions::tests_support::{action, push_set};
        use crate::panels::actions::{PlaybackSpeed, play};
        let step = ("object.move", json!({"dx": 1, "dy": 0}));
        app.ui.actions = craft::automation::actions::ActionList::new();
        push_set(&mut app, "Test", vec![action("Move", &[step.clone(), step])]);
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
