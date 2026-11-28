//! The progress window of the work that runs in the background: a Background Export (artboard by
//! artboard, file by file for Export for Screens, as a share for an exporter plug-in that reports
//! its progress, busy for one artboard and for formats made in one go), a Background Save, a
//! heavy plug-in filter, an Image Trace, a file being opened, a file being read for Place and
//! Retype's search of the installed fonts. Each
//! job table answers [`Shown`] for its first job the window shows; [`show`] draws the first
//! table's window once its job has run [`DELAY`], with Cancel and, where the job can go on without
//! it, Continue in Background.
//!
//! It waits while another dialog is open and for the first job's [`DELAY`], so a quick job doesn't
//! flash it: never two modals at once.

use std::time::Duration;

use craft::ui::dialog::{JobAnswer, JobProgress};
use serde_json::json;

use crate::VectorcraftApp;

/// Seconds a job runs before the window opens.
pub const DELAY: f64 = 0.4;

/// What a job table shows of its first job in the progress window.
pub(crate) struct Shown {
    /// The window's title.
    pub title: String,
    /// What the job is doing now.
    pub step: String,
    /// How far it has got; `None` for work that reports no steps (an indeterminate bar).
    pub fraction: Option<f32>,
    /// The step count line ("Artboard 2 of 5").
    pub counts: Option<String>,
    /// The line naming the jobs of its kind that wait.
    pub queued: Option<String>,
    /// [`now`] when the job started.
    pub started: f64,
    /// The command Cancel runs.
    pub cancel: &'static str,
    /// The command Continue in Background runs; `None` for a job that can't go on without the window.
    pub hide: Option<&'static str>,
}

/// Seconds on the clock the jobs record their start with: the app's clock, which a test can hold
/// still.
pub(crate) fn now(app: &VectorcraftApp) -> f64 {
    app.now()
}

/// The window of the first table that has a job it shows. Opening and placing wait for the tables
/// before them, as the windows never showed together.
fn first(app: &VectorcraftApp) -> Option<Shown> {
    crate::background::shown(app)
        .or_else(|| crate::plugin_runs::shown(app))
        .or_else(|| crate::trace_runs::shown(app))
        .or_else(|| crate::opening::shown(app))
        .or_else(|| crate::placing::shown(app))
        .or_else(|| crate::retype_runs::shown(app))
}

/// Draws the progress window of the first job that has one, once it has run [`DELAY`] seconds and
/// no other dialog is open, and runs the command of the button pressed.
pub(crate) fn show(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    let Some(shown) = first(app) else { return };
    let age = now(app) - shown.started;
    if age < DELAY {
        // Nothing else wakes the frame that opens the window.
        ui.ctx().request_repaint_after(Duration::from_secs_f64(DELAY - age));
        return;
    }
    if app.any_dialog_open() {
        return;
    }
    let Shown { title, step, fraction, counts, queued, cancel, hide, .. } = shown;
    let mut window = JobProgress::new(title, step).fraction(fraction).can_hide(hide.is_some());
    if let Some(counts) = counts {
        window = window.counts(counts);
    }
    if let Some(queued) = queued {
        window = window.queued(queued);
    }
    let id = match window.show(ui).answer {
        Some(JobAnswer::Cancel) => cancel,
        Some(JobAnswer::Hide) => match hide {
            Some(hide) => hide,
            None => return,
        },
        _ => return,
    };
    if let Err(e) = app.run(id, json!({})) {
        app.status(e);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;
    use vectorcraft_engine::Session;
    use vectorcraft_engine::cmd::fileio::progress::Progress;

    use super::DELAY;
    use crate::{Services, VectorcraftApp, background};

    /// An app whose background writer waits until `go` is set. Its clock stands still: a job is only
    /// old enough for its window after [`age`].
    fn app(go: Arc<std::sync::atomic::AtomicBool>) -> VectorcraftApp {
        let services = Services {
            write_shared: Some(Arc::new(move |_: &str, _: &[u8]| {
                while !go.load(std::sync::atomic::Ordering::SeqCst) {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = VectorcraftApp::new(Session::new(), services);
        crate::tests_background::threaded(&mut app);
        app.now_override = Some(0.0);
        app
    }

    /// Makes every job old enough for its window.
    fn age(app: &mut VectorcraftApp) {
        for job in &mut app.background.jobs {
            job.started -= 2.0 * DELAY;
        }
    }

    /// Draws one frame of the dialog → the texts it drew.
    fn frame(app: &mut VectorcraftApp, ctx: &egui::Context) -> String {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::show(app, ui));
        out.textures_delta.clear();
        let mut texts = String::new();
        for shape in out.shapes {
            if let egui::epaint::Shape::Text(t) = shape.shape {
                texts.push_str(t.galley.text());
                texts.push('\n');
            }
        }
        texts
    }

    #[test]
    fn opens_after_the_delay_and_cancel_closes_it() {
        let go = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut app = app(go.clone());
        let p = Progress::new();
        let r = background::run_with_progress(
            &mut app,
            true,
            "Exporting a.png".into(),
            None,
            Some(p.clone()),
            |w| w("/a.png", b"x").map(|()| json!({})),
            |_, r| r,
        );
        assert_eq!(r.unwrap()["background"], true);
        let ctx = crate::testing::ui_context();
        assert!(!frame(&mut app, &ctx).contains("Exporting a.png"), "not before the delay");
        age(&mut app);
        frame(&mut app, &ctx); // a new window measures itself unseen first
        let shown = frame(&mut app, &ctx);
        assert!(shown.contains("Exporting a.png") && shown.contains("Cancel") && shown.contains("Continue in Background"), "{shown}");
        app.run("file.cancelExport", json!({})).unwrap();
        assert!(p.cancelled());
        assert_eq!(app.ui.status, crate::io::EXPORT_CANCELLED);
        assert!(!frame(&mut app, &ctx).contains("Exporting a.png"), "a cancelled export isn't shown");
        go.store(true, std::sync::atomic::Ordering::SeqCst);
        background::wait_all(&mut app);
    }

    #[test]
    fn continue_in_background_hides_it_and_the_export_goes_on() {
        let go = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut app = app(go.clone());
        let p = Progress::new();
        background::run_with_progress(
            &mut app,
            true,
            "Exporting b.png".into(),
            None,
            Some(p.clone()),
            |w| w("/b.png", b"x").map(|()| json!({})),
            |_, r| r,
        )
        .unwrap();
        let ctx = crate::testing::ui_context();
        age(&mut app);
        frame(&mut app, &ctx);
        assert!(frame(&mut app, &ctx).contains("Exporting b.png"));
        assert_eq!(app.run("file.exportInBackground", json!({})).unwrap()["hidden"], 1);
        assert!(!frame(&mut app, &ctx).contains("Exporting b.png"));
        assert!(!p.cancelled());
        go.store(true, std::sync::atomic::Ordering::SeqCst);
        background::wait_all(&mut app);
        assert!(app.background.jobs.is_empty());
    }

    /// A Background Save shows in the window after the delay, with how many more saves wait;
    /// Continue in Background hides it and the saves go on. With Background Save off the save is
    /// written at once and no window shows.
    #[test]
    fn a_background_save_shows_in_the_window_and_continue_hides_it() {
        let go = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut app = app(go.clone());
        for name in ["a", "b"] {
            app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
            let r = app.run("file.save", json!({"path": format!("/docs/{name}.vectorcraft")})).unwrap();
            assert_eq!(r["background"], true, "{r}");
        }
        let ctx = crate::testing::ui_context();
        assert!(!frame(&mut app, &ctx).contains("Saving a.vectorcraft"), "not before the delay");
        age(&mut app);
        frame(&mut app, &ctx);
        let shown = frame(&mut app, &ctx);
        assert!(shown.contains("Saving a.vectorcraft…") && shown.contains("waiting"), "{shown}");
        assert!(shown.contains("Cancel") && shown.contains("Continue in Background") && !shown.contains("export"), "{shown}");
        assert_eq!(app.run("file.saveInBackground", json!({})).unwrap()["hidden"], 2);
        assert!(!frame(&mut app, &ctx).contains("Saving"), "hidden saves aren't shown");
        assert!(crate::menus::enabled(&app, "file.cancelSave"), "they can still be canceled");
        go.store(true, std::sync::atomic::Ordering::SeqCst);
        background::wait_all(&mut app);
        assert!(app.background.jobs.is_empty());
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/docs/b.vectorcraft"));
        app.session.execute("prefs.set", &json!({"values": {"backgroundSave": false}})).unwrap();
        app.services.write = Some(Box::new(|_: &str, _: &[u8]| Ok(())));
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        let r = app.run("file.save", json!({"path": "/docs/c.vectorcraft"}));
        assert!(r.as_ref().is_ok_and(|r| r.get("background").is_none()), "{r:?}");
        assert!(app.background.jobs.is_empty() && !frame(&mut app, &ctx).contains("Saving"));
    }
}
