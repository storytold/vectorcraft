//! Data Recovery in the app: the timer (driven by a fake clock), copies written in the background,
//! the Recover Documents dialog at launch, and quitting leaving no copies behind.

use std::sync::Arc;

use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::recovery::{MemoryStore, RecoveryStore};

use crate::{Services, VectorcraftApp, background, dialogs, recovery, theme};

/// An app keeping its copies in `store`, with a modified document.
fn app_with(store: &Arc<MemoryStore>, services: Services) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Services { recovery_store: Some(store.clone()), ..services });
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 20})).unwrap();
    app
}

fn copies(store: &MemoryStore) -> usize {
    store.list().unwrap().iter().filter(|n| n.ends_with(".vectorcraft")).count()
}

/// One headless frame of the dialog layer.
fn frame(app: &mut VectorcraftApp) {
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| dialogs::show(app, ui.ctx()));
    out.textures_delta.clear();
}

#[test]
fn the_timer_writes_modified_documents_every_interval() {
    let store = Arc::new(MemoryStore::default());
    let mut app = app_with(&store, Services::default());
    app.session.prefs.autosave_interval = 2;
    // The first tick starts the clock; nothing before the interval has passed.
    assert_eq!(recovery::tick(&mut app, 10.0), 0);
    assert_eq!(recovery::tick(&mut app, 129.0), 0);
    assert_eq!(copies(&store), 0);
    assert_eq!(recovery::tick(&mut app, 130.0), 1);
    assert_eq!(copies(&store), 1);
    assert_eq!(app.ui.status, "Saved recovery data for Untitled-1");
    // Unchanged: the next round writes nothing.
    assert_eq!(recovery::tick(&mut app, 250.0), 0);
    app.run("shape.ellipse", json!({"x": 100, "y": 10, "width": 20, "height": 20})).unwrap();
    assert_eq!(recovery::tick(&mut app, 300.0), 0, "the interval runs from the last round");
    assert_eq!(recovery::tick(&mut app, 370.0), 1);
    assert_eq!(copies(&store), 1, "the same copy, rewritten");
    // Turned off: no copies, and the clock starts again when turned back on.
    app.session.prefs.autosave_recovery = false;
    app.run("shape.ellipse", json!({"x": 150, "y": 10, "width": 20, "height": 20})).unwrap();
    assert_eq!(recovery::tick(&mut app, 1000.0), 0);
    app.session.prefs.autosave_recovery = true;
    assert_eq!(recovery::tick(&mut app, 1001.0), 0);
    assert_eq!(recovery::tick(&mut app, 1121.0), 1);
    // A clock that goes back restarts the interval instead of writing.
    app.run("shape.ellipse", json!({"x": 10, "y": 50, "width": 20, "height": 20})).unwrap();
    assert_eq!(recovery::tick(&mut app, 5.0), 0);
    assert_eq!(recovery::tick(&mut app, 125.0), 1);
}

#[test]
fn copies_are_written_in_the_background_and_go_on_save() {
    let store = Arc::new(MemoryStore::default());
    let services = Services {
        write: Some(Box::new(|_: &str, _: &[u8]| Ok(()))),
        write_shared: Some(Arc::new(|_: &str, _: &[u8]| Ok(()))),
        ..Default::default()
    };
    let mut app = app_with(&store, services);
    recovery::tick(&mut app, 0.0);
    assert_eq!(recovery::tick(&mut app, 120.0), 1);
    assert_eq!(app.background.jobs.len(), 1, "on the worker");
    assert!(app.ui.status.starts_with("Saving recovery data for Untitled-1"), "{}", app.ui.status);
    background::wait_all(&mut app);
    assert_eq!(copies(&store), 1);
    assert!(app.session.doc().unwrap().recovery.is_some());
    // Saving (in the background too) removes it.
    app.run("file.save", json!({"path": "/docs/a.vectorcraft"})).unwrap();
    background::wait_all(&mut app);
    assert_eq!(copies(&store), 0);
}

#[test]
fn launching_after_a_crash_offers_the_copies() {
    let store = Arc::new(MemoryStore::default());
    let mut crashed = app_with(&store, Services::default());
    crashed.run("file.recovery.save", json!({})).unwrap();
    drop(crashed);

    // Restore.
    let mut app = VectorcraftApp::new(Session::new(), Services { recovery_store: Some(store.clone()), ..Default::default() });
    recovery::frame(&mut app, 0.0);
    let d = app.ui.dialog.clone().expect("the Recover Documents dialog");
    assert_eq!((d.kind.as_str(), d.fields["copies"][0]["title"].as_str()), (recovery::KIND, Some("Untitled-1")));
    frame(&mut app);
    assert!(app.ui.dialog.is_some(), "drawing keeps it open");
    dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    let st = app.session.doc().unwrap();
    assert_eq!(st.title(), "Untitled-1 [Recovered]");
    assert!(st.is_dirty());
    assert_eq!(app.views.len(), 1);
    // Asked once per launch.
    recovery::frame(&mut app, 1.0);
    assert!(app.ui.dialog.is_none());
    // Closing it discards its copy; nothing is offered next time.
    app.run("file.close", json!({})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("discard".into(), json!(true));
    dialogs::confirm(&mut app).unwrap();
    assert_eq!(copies(&store), 0);
    let mut next = VectorcraftApp::new(Session::new(), Services { recovery_store: Some(store.clone()), ..Default::default() });
    recovery::frame(&mut next, 0.0);
    assert!(next.ui.dialog.is_none());
}

#[test]
fn discard_at_launch_deletes_the_copies_and_cancel_keeps_them() {
    let store = Arc::new(MemoryStore::default());
    let mut crashed = app_with(&store, Services::default());
    crashed.run("file.recovery.save", json!({})).unwrap();
    drop(crashed);
    // Cancel: asked again next launch.
    let mut app = VectorcraftApp::new(Session::new(), Services { recovery_store: Some(store.clone()), ..Default::default() });
    assert!(recovery::offer(&mut app));
    dialogs::cancel(&mut app);
    assert_eq!(copies(&store), 1);
    // Discard (as agents do: set discard, confirm).
    let mut app = VectorcraftApp::new(Session::new(), Services { recovery_store: Some(store.clone()), ..Default::default() });
    assert!(recovery::offer(&mut app));
    app.ui.dialog.as_mut().unwrap().fields.insert("discard".into(), json!(true));
    dialogs::confirm(&mut app).unwrap();
    assert_eq!(copies(&store), 0);
    assert!(app.session.documents().is_empty());
}

#[test]
fn quitting_with_nothing_unsaved_leaves_no_copies() {
    let store = Arc::new(MemoryStore::default());
    let mut app = app_with(&store, Services::default());
    app.run("file.recovery.save", json!({})).unwrap();
    assert_eq!(copies(&store), 1);
    // Undone back to the saved state, then quit: the copy goes.
    app.run("edit.undo", json!({})).unwrap();
    app.run("app.quit", json!({})).unwrap();
    assert_eq!(app.ui.status, "quit");
    assert_eq!(copies(&store), 0);
}

#[test]
fn a_second_app_never_asks_about_a_running_apps_copies() {
    let store = Arc::new(MemoryStore::default());
    let mut a = app_with(&store, Services::default());
    a.run("file.recovery.save", json!({})).unwrap();
    // B launches while A runs: no Recover Documents dialog, A's copy untouched.
    let mut b = VectorcraftApp::new(Session::new(), Services { recovery_store: Some(store.clone()), ..Default::default() });
    recovery::frame(&mut b, 0.0);
    assert!(b.ui.dialog.is_none());
    assert_eq!(copies(&store), 1);
    // A crashes: the next launch asks.
    drop(a);
    let mut c = VectorcraftApp::new(Session::new(), Services { recovery_store: Some(store.clone()), ..Default::default() });
    recovery::frame(&mut c, 0.0);
    assert_eq!(c.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(recovery::KIND));
}

#[test]
fn the_timer_asks_for_frames_while_idle() {
    let store = Arc::new(MemoryStore::without_locks());
    let mut app = app_with(&store, Services::default());
    app.session.prefs.autosave_interval = 2;
    // The interval and the heartbeat each want a frame on time, even with no input.
    let wait = recovery::frame(&mut app, 0.0).unwrap();
    assert!((1.0..=60.0).contains(&wait), "{wait}");
    app.session.prefs.autosave_recovery = false;
    assert!(recovery::frame(&mut app, 1.0).is_none());
}

/// Issue #367: a tab whose timers were paused (in the background) long enough for another tab to
/// take it for gone and discard its copy writes the copy again on its first heartbeat after it
/// resumes, even with the timer off (the copy was written by hand).
#[test]
fn a_resumed_tab_writes_again_the_copies_another_tab_discarded() {
    let store = Arc::new(MemoryStore::without_locks());
    let mut a = app_with(&store, Services::default());
    a.session.prefs.autosave_recovery = false;
    a.run("file.recovery.save", json!({})).unwrap();
    recovery::frame(&mut a, 0.0);
    store.set_now(1_000_000 + 400);
    let mut b = VectorcraftApp::new(Session::new(), Services { recovery_store: Some(store.clone()), ..Default::default() });
    b.run("file.recovery.discard", json!({})).unwrap();
    assert_eq!(copies(&store), 0);
    store.set_now(1_000_000 + 410);
    recovery::frame(&mut a, 30.0);
    assert_eq!(copies(&store), 0, "the store is looked at with the heartbeat, not every frame");
    recovery::frame(&mut a, 410.0);
    assert_eq!(copies(&store), 1);
    assert_eq!(a.ui.status, "Saved recovery data for Untitled-1");
}
