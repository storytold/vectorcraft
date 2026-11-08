//! The system clipboard is checked for Paste off the UI thread where the host installs a probe
//! (Linux): an X11 clipboard owner that never answers used to block the frame loop for seconds, so
//! the window stopped drawing and could not even be closed. The in-line check (no probe: the web,
//! Windows, macOS) is covered by `tests_clipboard` and `tests_sysclip`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::clipboard::Flavour;

use crate::tests_sysclip::{copy_rect, run};
use crate::{Services, SystemClipboard, VectorcraftApp, menus};

/// What the clipboard handles saw: the thread each `has()` ran on, and how many handles are alive.
#[derive(Default)]
struct Recorder {
    threads: Mutex<Vec<Option<String>>>,
    alive: AtomicUsize,
}

impl Recorder {
    fn looks(&self) -> usize {
        self.threads.lock().unwrap().len()
    }
}

/// A system clipboard whose `has()` takes `delay` (an owner slow to answer) and answers `verdict`,
/// or panics without one (a bug in the platform code).
struct Slow {
    rec: Arc<Recorder>,
    delay: Duration,
    verdict: Option<bool>,
}

impl Slow {
    fn boxed(rec: &Arc<Recorder>, delay: Duration, verdict: Option<bool>) -> Box<dyn SystemClipboard> {
        rec.alive.fetch_add(1, Ordering::SeqCst);
        Box::new(Self { rec: Arc::clone(rec), delay, verdict })
    }
}

impl Drop for Slow {
    fn drop(&mut self) {
        self.rec.alive.fetch_sub(1, Ordering::SeqCst);
    }
}

impl SystemClipboard for Slow {
    fn write(&mut self, _flavours: &[Flavour]) -> Result<(), String> {
        Ok(())
    }
    fn holds_ours(&mut self) -> bool {
        false
    }
    fn read(&mut self, _mimes: &[&'static str]) -> Option<Flavour> {
        None
    }
    fn has(&mut self, _mimes: &[&'static str]) -> bool {
        self.rec.threads.lock().unwrap().push(std::thread::current().name().map(str::to_owned));
        std::thread::sleep(self.delay);
        self.verdict.expect("the clipboard failed")
    }
}

/// An app whose two clipboard handles record into `rec`: the UI's says `inline`, the probe's
/// `probe`, each after `delay`.
fn app(rec: &Arc<Recorder>, delay: Duration, inline: bool, probe: Option<bool>) -> VectorcraftApp {
    let make = {
        let rec = Arc::clone(rec);
        move || Slow::boxed(&rec, delay, probe)
    };
    let services =
        Services { system_clipboard: Some(Slow::boxed(rec, delay, Some(inline))), clipboard_probe: Some(Box::new(make)), ..Default::default() };
    VectorcraftApp::new(Session::new(), services)
}

/// One headless frame of app logic in the test's one context (a new one would restart the fonts),
/// half a second after the last: past the in-line check's throttle, so it would run every frame.
fn frame(app: &mut VectorcraftApp) {
    thread_local! {
        static CTX: egui::Context = egui::Context::default();
        static NOW: std::cell::Cell<f64> = const { std::cell::Cell::new(0.0) };
    }
    let time = NOW.with(|t| {
        t.set(t.get() + 0.5);
        t.get()
    });
    let input = egui::RawInput { time: Some(time), ..Default::default() };
    CTX.with(|ctx| ctx.run_ui(input, |ui| app.logic(ui.ctx())).textures_delta.clear());
}

/// Frames for `wait`, or until `done` holds; whether it does.
fn frames_until(app: &mut VectorcraftApp, wait: Duration, done: impl Fn(&VectorcraftApp) -> bool) -> bool {
    let end = Instant::now() + wait;
    loop {
        frame(app);
        if done(app) || Instant::now() > end {
            return done(app);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_slow_clipboard_is_read_off_the_ui_thread() {
    let rec = Arc::new(Recorder::default());
    let mut app = app(&rec, Duration::from_secs(1), false, Some(true));
    // The first frames install the UI fonts. Without a document Paste is off whatever the clipboard
    // holds: nobody looks.
    assert!(!frames_until(&mut app, Duration::from_millis(100), |_| rec.looks() > 0));
    run(&mut app, "file.new", json!({"width": 100, "height": 100}));
    frame(&mut app);
    let t0 = Instant::now();
    frame(&mut app);
    assert!(t0.elapsed() < Duration::from_millis(300), "the frame waited on the clipboard: {:?}", t0.elapsed());
    // The probe's verdict (the UI's own handle would say no) enables Paste once it arrives.
    assert!(frames_until(&mut app, Duration::from_secs(60), |app| app.system_paste), "the probe's verdict never arrived");
    assert!(menus::enabled(&app, "edit.paste"));
    let threads = rec.threads.lock().unwrap().clone();
    assert!(threads.iter().all(|n| n.as_deref() == Some("vectorcraft-clip")), "checked on: {threads:?}");
}

#[test]
fn the_probe_rests_after_a_copy_and_stops_with_the_app() {
    let rec = Arc::new(Recorder::default());
    let mut app = app(&rec, Duration::ZERO, false, Some(true));
    run(&mut app, "file.new", json!({"width": 100, "height": 100}));
    assert!(frames_until(&mut app, Duration::from_secs(60), |app| app.system_paste));
    // Something copied in the app enables Paste alone: the clipboard isn't read any more.
    copy_rect(&mut app);
    frames_until(&mut app, Duration::from_millis(50), |_| false);
    let looks = rec.looks();
    frames_until(&mut app, Duration::from_millis(600), |_| false);
    assert_eq!(rec.looks(), looks);
    assert!(!app.system_paste && menus::enabled(&app, "edit.paste"));
    // Quitting waits until the probe's thread has let go of its handle: on X11 the last handle
    // alive hands what the app copied to the clipboard manager.
    assert_eq!(rec.alive.load(Ordering::SeqCst), 2);
    drop(app);
    assert_eq!(rec.alive.load(Ordering::SeqCst), 0, "the probe's thread kept its clipboard");
}

#[test]
fn without_its_thread_the_check_runs_in_line_again() {
    let rec = Arc::new(Recorder::default());
    // The probe's clipboard panics: its thread ends.
    let mut app = app(&rec, Duration::ZERO, true, None);
    run(&mut app, "file.new", json!({"width": 100, "height": 100}));
    assert!(frames_until(&mut app, Duration::from_secs(60), |app| app.system_paste), "no fallback");
    assert!(app.clipboard_probe.is_none());
    let threads = rec.threads.lock().unwrap().clone();
    assert_eq!(threads.first().cloned().flatten().as_deref(), Some("vectorcraft-clip"), "{threads:?}");
}
