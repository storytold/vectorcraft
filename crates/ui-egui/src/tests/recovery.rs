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

/// A 40×30 PNG of one color.
fn png(rgb: [u8; 3]) -> Vec<u8> {
    let mut out = vec![];
    let [r, g, b] = rgb;
    image::RgbaImage::from_pixel(40, 30, image::Rgba([r, g, b, 255])).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// The copies a crash left of `n` documents, each with an image linked to a file in `dir`. The
/// files are deleted and the images' bytes are gone from the store, so each copy restores with a
/// warning about its image and a missing linked file. → the store.
fn crashed_with_lost_images(dir: &std::path::Path, n: u8) -> Arc<MemoryStore> {
    let store = Arc::new(MemoryStore::default());
    let mut app = VectorcraftApp::new(Session::new(), Services { recovery_store: Some(store.clone()), ..Default::default() });
    app.services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
    for k in 0..n {
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let p = dir.join(format!("{k}.png"));
        std::fs::write(&p, png([200, k * 100, 0])).unwrap();
        app.run("file.place", json!({"path": p.to_string_lossy()})).unwrap();
    }
    app.run("file.recovery.save", json!({})).unwrap();
    drop(app);
    for k in 0..n {
        std::fs::remove_file(dir.join(format!("{k}.png"))).unwrap();
    }
    for name in store.list().unwrap().into_iter().filter(|n| n.ends_with(".blob")) {
        store.remove(&name).unwrap();
    }
    store
}

/// The app launched after the crash, with the Recover Documents dialog open → the app and the
/// titles the restored documents get, in the dialog's order.
fn launched(store: &Arc<MemoryStore>) -> (VectorcraftApp, Vec<String>) {
    let mut app = VectorcraftApp::new(Session::new(), Services { recovery_store: Some(store.clone()), ..Default::default() });
    recovery::frame(&mut app, 0.0);
    let d = app.ui.dialog.as_ref().expect("the Recover Documents dialog");
    let titles = d.fields["copies"].as_array().unwrap().iter().map(|c| format!("{} [Recovered]", c["title"].as_str().unwrap())).collect();
    (app, titles)
}

/// Is `note` the warning about a restored document's image that couldn't be read?
fn lost_image(note: &str) -> bool {
    note.starts_with("the image “") && note.contains("” of the recovered document couldn't be read")
}

/// Restore keeps the warnings each restored document opened with: the status bar names the first
/// after the count, and its hover lists them all, each named with its document. The questions
/// about the missing linked files come one document at a time, with that document current.
#[test]
fn restore_keeps_each_documents_notes_and_asks_about_its_links_in_turn() {
    let dir = scratch("restore-notes");
    let store = crashed_with_lost_images(&dir, 2);
    let (mut app, titles) = launched(&store);
    dialogs::confirm(&mut app).unwrap();
    let (status, notes) = app.ui.status_notes.clone().expect("the notes are kept");
    assert_eq!(status, app.ui.status);
    assert_eq!(notes.len(), 2, "{notes:?}");
    for (note, title) in notes.iter().zip(&titles) {
        let lost = note.strip_prefix(&format!("{title}: ")).unwrap_or_default();
        assert!(lost_image(lost), "{note}");
    }
    assert_eq!(status, format!("Restored 2 recovered documents: save them to keep them. Opened with 2 notes: {}", notes[0]));
    // The first document's missing file is asked about first, and the second's waits.
    for title in &titles {
        let d = app.ui.dialog.clone().expect("the missing file's question");
        assert_eq!(d.kind, dialogs::missing_links::KIND);
        assert_eq!(&app.session.doc().unwrap().title(), title);
        app.ui.dialog.as_mut().unwrap().fields.insert("discard".into(), json!(true));
        dialogs::confirm(&mut app).unwrap();
    }
    assert!(app.ui.dialog.is_none());
    assert_eq!(app.ui.status, status, "answering the questions keeps the line");
    let _ = std::fs::remove_dir_all(&dir);
}

/// With one document restored, its notes are not named with its title.
#[test]
fn restoring_one_document_shows_its_notes_after_the_count() {
    let dir = scratch("restore-one");
    let store = crashed_with_lost_images(&dir, 1);
    let (mut app, _) = launched(&store);
    dialogs::confirm(&mut app).unwrap();
    let (status, notes) = app.ui.status_notes.clone().expect("the notes are kept");
    assert!(notes.len() == 1 && lost_image(&notes[0]), "{notes:?}");
    assert_eq!(status, format!("Restored 1 recovered document: save it to keep it. Opened with 1 note: {}", notes[0]));
    assert_eq!(app.ui.status, status);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A copy that can't be restored after another was: the status bar shows the error, and its hover
/// still lists the count and the notes of the document that was restored.
#[test]
fn a_failed_restore_keeps_the_notes_of_the_documents_restored_before_it() {
    let dir = scratch("restore-partial");
    let store = crashed_with_lost_images(&dir, 2);
    let (mut app, _) = launched(&store);
    let second = app.ui.dialog.as_ref().unwrap().fields["copies"][1]["file"].as_str().unwrap().to_string();
    store.write(&format!("{second}.vectorcraft"), b"not a document").unwrap();
    let e = dialogs::confirm(&mut app).unwrap_err();
    assert_eq!(app.session.documents().len(), 1);
    assert_eq!(app.ui.status, e);
    let (status, notes) = app.ui.status_notes.clone().expect("the notes are kept");
    assert_eq!(status, e);
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert_eq!(notes[0], "Restored 1 recovered document: save it to keep it");
    assert!(lost_image(&notes[1]), "{}", notes[1]);
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(dialogs::missing_links::KIND), "its missing file is asked about");
    let _ = std::fs::remove_dir_all(&dir);
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

/// The end of a failed recovery round's status line: how to use another folder.
const TO_DO: &str = "; to use another folder, set Preferences → File Handling → Data Recovery → Folder (recoveryFolder)";

/// An app keeping its copies in `folder` (the folder the app sets) or the one the preference
/// names, with a modified document, a recovery round every 2 minutes and the clock started.
fn app_in_folder(folder: &str, services: Services) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.session.recovery.set_default_folder(Some(folder.to_string()));
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 20})).unwrap();
    app.session.prefs.autosave_interval = 2;
    recovery::tick(&mut app, 0.0);
    app
}

/// A fresh empty folder for test `name`.
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vc-ui-recovery-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A recovery round that can't start names the recovery folder in use, whether the preference or
/// the app set it, the error, and what to do.
#[test]
fn a_round_that_fails_names_the_folder_in_use_and_what_to_do() {
    let dir = scratch("round");
    let file = dir.join("not-a-folder");
    std::fs::write(&file, b"").unwrap();
    let shown = file.to_string_lossy().into_owned();
    let mut app = app_in_folder(&shown, Services::default());
    assert_eq!(recovery::tick(&mut app, 120.0), 0);
    let start = format!("Couldn't save recovery data: {shown} is not a folder: ");
    let end = format!(". Copies go to {shown}, the default recovery folder{TO_DO}");
    assert!(app.ui.status.starts_with(&start) && app.ui.status.ends_with(&end), "{}", app.ui.status);
    // A preference of spaces is no preference.
    app.session.prefs.recovery_folder = "  ".into();
    assert_eq!(recovery::tick(&mut app, 240.0), 0);
    assert!(app.ui.status.starts_with(&start) && app.ui.status.ends_with(&end), "{}", app.ui.status);
    app.session.prefs.recovery_folder = format!(" {shown} ");
    assert_eq!(recovery::tick(&mut app, 360.0), 0);
    let end = format!(". Copies go to {shown}, the folder set in Preferences{TO_DO}");
    assert!(app.ui.status.starts_with(&start) && app.ui.status.ends_with(&end), "{}", app.ui.status);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A copy that fails on the background worker is reported the same way, not as the job's own
/// "Saving recovery data for … failed" line.
#[test]
fn a_copy_that_fails_in_the_background_names_the_folder_and_what_to_do() {
    let dir = scratch("copy");
    let folder = dir.join("recovery");
    let shown = folder.to_string_lossy().into_owned();
    let services = Services { write_shared: Some(Arc::new(|_: &str, _: &[u8]| Ok(()))), ..Default::default() };
    let mut app = app_in_folder(&shown, services);
    assert_eq!(recovery::tick(&mut app, 120.0), 1);
    assert_eq!(app.background.jobs.len(), 1, "on the worker");
    background::wait_all(&mut app);
    assert_eq!(app.ui.status, "Saved recovery data for Untitled-1");
    // A folder with a file in it takes the copy's place, so the next copy can't be written.
    let name = app.session.doc().unwrap().recovery.as_ref().unwrap().file.clone();
    let copy = folder.join(format!("{name}.vectorcraft"));
    std::fs::remove_file(&copy).unwrap();
    std::fs::create_dir(&copy).unwrap();
    std::fs::write(copy.join("in-the-way"), b"").unwrap();
    app.run("shape.ellipse", json!({"x": 100, "y": 10, "width": 20, "height": 20})).unwrap();
    assert_eq!(recovery::tick(&mut app, 240.0), 1);
    // A folder chosen while the copy runs isn't the one that failed.
    app.session.prefs.recovery_folder = dir.join("chosen-later").to_string_lossy().into_owned();
    background::wait_all(&mut app);
    let end = format!(". Copies go to {shown}, the default recovery folder{TO_DO}");
    assert!(app.ui.status.starts_with("Couldn't save recovery data for Untitled-1: ") && app.ui.status.ends_with(&end), "{}", app.ui.status);
    // Kept for replies to automation clients, which hide it outside their read roots.
    assert_eq!(app.recovery.named, std::slice::from_ref(&shown));
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A store the host installed whose document copies can't be written (other entries, such as
/// locks, can).
struct Refusing(MemoryStore);

impl RecoveryStore for Refusing {
    fn list(&self) -> Result<Vec<String>, String> {
        self.0.list()
    }
    fn read(&self, name: &str) -> Result<Vec<u8>, String> {
        self.0.read(name)
    }
    fn write(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        if name.ends_with(".vectorcraft") { Err("the store is full".into()) } else { self.0.write(name, bytes) }
    }
    fn remove(&self, name: &str) -> Result<(), String> {
        self.0.remove(name)
    }
    fn location(&self) -> String {
        self.0.location()
    }
    fn lock(&self, area: &str) -> Result<vectorcraft_engine::cmd::recovery::Lock, String> {
        self.0.lock(area)
    }
    fn now(&self) -> Option<i64> {
        self.0.now()
    }
}

/// With a store the host installed there is no folder to name: a copy that fails shows the job's
/// own line with the document's title, both on the worker and when it is written right away.
#[test]
fn a_copy_that_fails_in_a_host_store_names_the_document() {
    let worker = Services { write_shared: Some(Arc::new(|_: &str, _: &[u8]| Ok(()))), ..Default::default() };
    for services in [worker, Services::default()] {
        let store: Arc<dyn RecoveryStore> = Arc::new(Refusing(MemoryStore::default()));
        let mut app = VectorcraftApp::new(Session::new(), Services { recovery_store: Some(store), ..services });
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 20})).unwrap();
        app.session.prefs.autosave_interval = 2;
        recovery::tick(&mut app, 0.0);
        assert_eq!(recovery::tick(&mut app, 120.0), 1);
        background::wait_all(&mut app);
        assert_eq!(app.ui.status, "Saving recovery data for Untitled-1 failed: the store is full");
    }
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
