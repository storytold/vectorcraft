//! Data Recovery: `file.recovery.save/list/restore/discard`, copies removed on save, revert and
//! close, complex documents skipped, each app's own area (another running app's copies are never
//! offered: locks, heartbeats, announcements; a paused app repairs its copies), and the folder
//! store with real file locks.

use std::sync::Arc;

use serde_json::json;

use super::*;
use crate::cmd::recovery::{FolderStore, MemoryStore, RecoveryStore};

/// A session keeping its copies in `store`, with one modified document.
fn session_in(store: &Arc<MemoryStore>) -> Session {
    let mut s = Session::new();
    s.recovery.set_store(store.clone());
    s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 20})).unwrap();
    s
}

/// A session keeping its copies in memory (the store is returned to look inside), with one
/// modified document.
fn session() -> (Session, Arc<MemoryStore>) {
    let store = Arc::new(MemoryStore::default());
    (session_in(&store), store)
}

/// The copies in the store, without their areas.
fn copies(store: &dyn RecoveryStore) -> Vec<String> {
    let mut v: Vec<String> =
        store.list().unwrap().iter().filter_map(|n| n.strip_suffix(".vectorcraft")).map(|n| n.split_once('/').unwrap().1.to_string()).collect();
    v.sort();
    v
}

/// The titles of the copies `file.recovery.list` offers after a crash (neither open here nor
/// kept by a running app).
fn offered(s: &mut Session) -> Vec<String> {
    let l = s.execute("file.recovery.list", &json!({})).unwrap();
    l["copies"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["open"] == false && c["running"] == false)
        .map(|c| c["title"].as_str().unwrap().to_string())
        .collect()
}

/// A crash: a new session over the same store (the copies stay behind; dropping the old session
/// releases its lock, as the system does for a process that ends).
fn relaunch(store: &Arc<MemoryStore>) -> Session {
    let mut s = Session::new();
    s.recovery.set_store(store.clone());
    s
}

fn restored(r: serde_json::Value) -> usize {
    r["restored"].as_array().unwrap().len()
}

#[test]
fn recovery_save_writes_modified_documents_once() {
    let (mut s, store) = session();
    // A clean second document gets no copy.
    s.execute("file.new", &json!({"width": 50, "height": 50})).unwrap();
    let r = s.execute("file.recovery.save", &json!({})).unwrap();
    assert_eq!(r["saved"].as_array().unwrap().len(), 1, "{r}");
    let area = s.recovery.own_area().unwrap().to_string();
    assert_eq!(r["saved"][0]["file"], format!("{area}/Untitled-1-1"));
    assert_eq!(store.list().unwrap(), [format!("{area}/Untitled-1-1.json"), format!("{area}/Untitled-1-1.vectorcraft")]);
    let doc = store.read(&format!("{area}/Untitled-1-1.vectorcraft")).unwrap();
    assert_eq!(vectorcraft_format::load(&doc).unwrap().node_count(), s.documents()[0].doc.node_count());
    // Unchanged since: nothing written again.
    let again = s.execute("file.recovery.save", &json!({})).unwrap();
    assert!(again["saved"].as_array().unwrap().is_empty());
    assert_eq!(again["skipped"][0]["reason"], "unchanged since its last recovery copy");
    // Changed: the same copy is rewritten.
    s.execute("document.activate", &json!({"index": 0})).unwrap();
    s.execute("shape.ellipse", &json!({"x": 100, "y": 10, "width": 20, "height": 20})).unwrap();
    let r = s.execute("file.recovery.save", &json!({})).unwrap();
    assert_eq!(r["saved"][0]["file"], format!("{area}/Untitled-1-1"));
    assert_eq!(copies(store.as_ref()), ["Untitled-1-1"]);
    // The list shows it as the copy of an open document, not one to offer.
    let l = s.execute("file.recovery.list", &json!({})).unwrap();
    assert_eq!((l["copies"][0]["title"].as_str(), l["copies"][0]["open"].as_bool()), (Some("Untitled-1"), Some(true)));
    assert_eq!(l["location"], "memory");
    assert!(offered(&mut s).is_empty());
}

/// Selecting other art drops the Puppet Warp pins, which a copy never holds: no new copy is written.
#[test]
fn dropping_puppet_warp_pins_writes_no_new_copy() {
    let (mut s, _) = session();
    let id = s.doc().unwrap().selection.objects[0];
    s.execute("object.puppetWarp", &json!({"id": id.0, "pins": [[35, 20]], "moved": [[35, 20]], "rest": true})).unwrap();
    assert!(s.doc().unwrap().doc.puppet.is_some());
    let r = s.execute("file.recovery.save", &json!({})).unwrap();
    assert_eq!(r["saved"].as_array().unwrap().len(), 1, "{r}");
    s.execute("select.none", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.puppet.is_none());
    let again = s.execute("file.recovery.save", &json!({})).unwrap();
    assert!(again["saved"].as_array().unwrap().is_empty(), "{again}");
    assert_eq!(again["skipped"][0]["reason"], "unchanged since its last recovery copy");
}

#[test]
fn a_crash_leaves_copies_that_restore_modified_and_titled() {
    let (mut s, store) = session();
    s.doc_mut().unwrap().path = Some("/art/Poster.svg".into());
    s.doc_mut().unwrap().format = "svg";
    s.execute("file.recovery.save", &json!({})).unwrap();
    let nodes = s.doc().unwrap().doc.node_count();
    let crashed = s.recovery.own_area().unwrap().to_string();
    drop(s);

    let mut s = relaunch(&store);
    let l = s.execute("file.recovery.list", &json!({})).unwrap();
    let copy = &l["copies"][0];
    assert_eq!(copy["file"], format!("{crashed}/Poster-1"));
    assert_eq!((copy["path"].as_str(), copy["open"].as_bool(), copy["running"].as_bool()), (Some("/art/Poster.svg"), Some(false), Some(false)));
    let r = s.execute("file.recovery.restore", &json!({})).unwrap();
    assert_eq!(r["restored"][0]["title"], "Poster [Recovered]");
    let st = s.doc().unwrap();
    assert_eq!(st.title(), "Poster [Recovered]");
    assert!(st.is_dirty() && st.recovered, "restored documents are unsaved");
    assert_eq!((st.path.as_deref(), st.format, st.doc.node_count()), (Some("/art/Poster.svg"), "svg", nodes));
    // The copy moved into this session's area; the crashed one's area is gone.
    let mine = s.recovery.own_area().unwrap().to_string();
    assert_eq!(st.recovery.as_ref().unwrap().file, format!("{mine}/Poster-1"));
    assert_eq!(store.areas().unwrap(), std::slice::from_ref(&mine));
    // Save asks for a path (suggesting the original file) rather than overwriting it.
    let saved = s.execute("document.save", &json!({})).unwrap();
    assert!(saved.get("dataBase64").is_some(), "no path: {saved}");
    assert_eq!(saved["name"], "Poster.svg");
    // Restored once: nothing is left to restore.
    assert_eq!(restored(s.execute("file.recovery.restore", &json!({})).unwrap()), 0);
    assert!(s.execute("file.recovery.restore", &json!({"file": format!("{mine}/Poster-1")})).is_err(), "an open document's copy");
}

#[test]
fn a_restored_copy_still_refuses_to_replace_the_file_an_import_left_things_out_of() {
    let (mut s, store) = session();
    s.doc_mut().unwrap().path = Some("/art/Poster.eps".into());
    s.doc_mut().unwrap().imported_from = Some("/art/Poster.eps".into());
    s.doc_mut().unwrap().import_losses = vec!["hidden layers have art this can't read".into()];
    s.execute("file.recovery.save", &json!({})).unwrap();
    drop(s);

    let mut s = relaunch(&store);
    s.execute("file.recovery.restore", &json!({})).unwrap();
    let st = s.doc().unwrap();
    assert_eq!(st.imported_from.as_deref(), Some("/art/Poster.eps"));
    assert_eq!(st.import_losses, ["hidden layers have art this can't read"]);
}

#[test]
fn another_running_apps_copies_are_never_offered() {
    let store = Arc::new(MemoryStore::default());
    let mut a = session_in(&store);
    a.execute("file.recovery.save", &json!({})).unwrap();
    let a_file = format!("{}/Untitled-1-1", a.recovery.own_area().unwrap());
    // B runs at the same time: A's copy is listed as running, never offered or taken.
    let mut b = session_in(&store);
    b.execute("file.recovery.save", &json!({})).unwrap();
    let l = b.execute("file.recovery.list", &json!({})).unwrap();
    let rows = l["copies"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().any(|c| c["file"] == a_file.as_str() && c["running"] == true));
    assert!(offered(&mut b).is_empty());
    assert_eq!(restored(b.execute("file.recovery.restore", &json!({})).unwrap()), 0);
    assert!(b.execute("file.recovery.discard", &json!({})).unwrap()["discarded"].as_array().unwrap().is_empty());
    let e = b.execute("file.recovery.discard", &json!({"file": a_file})).unwrap_err().to_string();
    assert!(e.contains("running"), "{e}");
    assert_eq!(copies(store.as_ref()).len(), 2, "nothing deleted");
    // A quits normally: its area goes, and B still has nothing to offer.
    a.execute("file.close", &json!({})).unwrap();
    cmd::recovery::forget_all(&mut a);
    assert!(offered(&mut b).is_empty());
    assert_eq!(copies(store.as_ref()), ["Untitled-1-1"], "B's own copy");
}

#[test]
fn a_crashed_apps_copies_are_offered_once_its_lock_is_released() {
    let store = Arc::new(MemoryStore::default());
    let mut a = session_in(&store);
    a.execute("file.recovery.save", &json!({})).unwrap();
    let mut b = relaunch(&store);
    assert!(offered(&mut b).is_empty(), "A still runs");
    drop(a);
    assert_eq!(offered(&mut b), ["Untitled-1"]);
    // Restored by B, the copy is B's: C, launched at the same time, can't take it.
    let mut c = relaunch(&store);
    assert_eq!(restored(b.execute("file.recovery.restore", &json!({})).unwrap()), 1);
    assert!(offered(&mut c).is_empty(), "B owns the copy now");
    assert_eq!(restored(c.execute("file.recovery.restore", &json!({})).unwrap()), 0);
}

#[test]
fn heartbeats_hold_areas_where_the_store_has_no_locks() {
    let store = Arc::new(MemoryStore::without_locks());
    store.set_now(10_000);
    let mut a = session_in(&store);
    a.execute("file.recovery.save", &json!({})).unwrap();
    let mut b = relaunch(&store);
    b.prefs.autosave_interval = 2;
    // Three intervals (6 minutes) without a heartbeat: until then A counts as running.
    store.set_now(10_000 + 359);
    assert!(offered(&mut b).is_empty());
    // A beats (as the app does every minute): fresh again.
    cmd::recovery::heartbeat(&mut a);
    store.set_now(10_000 + 359 + 300);
    assert!(offered(&mut b).is_empty());
    assert_eq!(restored(b.execute("file.recovery.restore", &json!({})).unwrap()), 0);
    // A stops beating (it crashed): its copies are offered once three intervals have passed.
    drop(a);
    store.set_now(10_000 + 359 + 361);
    assert_eq!(offered(&mut b), ["Untitled-1"]);
    let mut c = relaunch(&store);
    // B restores the copies one by one (as the dialog does): its own heartbeat on A's area doesn't
    // stop it, and C sees the area taken.
    let file = b.execute("file.recovery.list", &json!({})).unwrap()["copies"][0]["file"].as_str().unwrap().to_string();
    b.execute("file.recovery.restore", &json!({"file": file})).unwrap();
    assert!(offered(&mut c).is_empty());
    assert_eq!(b.doc().unwrap().title(), "Untitled-1 [Recovered]");
    // Never under three minutes, whatever the interval.
    let store = Arc::new(MemoryStore::without_locks());
    let mut a = session_in(&store);
    a.execute("file.recovery.save", &json!({})).unwrap();
    drop(a);
    let mut b = relaunch(&store);
    b.prefs.autosave_interval = 1;
    store.set_now(1_000_000 + 179);
    assert!(offered(&mut b).is_empty());
    store.set_now(1_000_000 + 181);
    assert_eq!(offered(&mut b), ["Untitled-1"]);
}

/// Issue #367: a tab whose timers were paused (a background tab) long enough for its heartbeat to
/// go stale, while another tab discards the copies it takes for crash leftovers.
#[test]
fn a_paused_app_writes_its_copies_again_and_is_running_again() {
    let store = Arc::new(MemoryStore::without_locks());
    let mut a = session_in(&store);
    a.execute("file.recovery.save", &json!({})).unwrap();
    let a_file = format!("{}/Untitled-1-1", a.recovery.own_area().unwrap());
    let mut b = relaunch(&store);
    b.prefs.autosave_interval = 1;
    assert!(b.execute("file.recovery.discard", &json!({})).unwrap()["discarded"].as_array().unwrap().is_empty());
    // A is paused past three minutes: with heartbeats alone, B can't tell it from a crash.
    store.set_now(1_000_000 + 186);
    assert_eq!(b.execute("file.recovery.discard", &json!({})).unwrap()["discarded"], json!([a_file.clone()]));
    assert!(copies(store.as_ref()).is_empty());
    // A resumes: its heartbeat finds the copy gone and the next save writes it again, unchanged
    // document or not.
    store.set_now(1_000_000 + 200);
    assert_eq!(cmd::recovery::heartbeat(&mut a), [a.doc().unwrap().uid]);
    assert!(a.doc().unwrap().recovery.is_none());
    let r = a.execute("file.recovery.save", &json!({})).unwrap();
    assert_eq!(r["saved"][0]["file"], a_file.as_str(), "{r}");
    assert_eq!(copies(store.as_ref()), ["Untitled-1-1"]);
    assert!(cmd::recovery::heartbeat(&mut a).is_empty(), "nothing missing any more");
    // A's heartbeat is newer than the one B wrote taking the area over: A runs again for B.
    let l = b.execute("file.recovery.list", &json!({})).unwrap();
    assert_eq!((l["copies"][0]["file"].as_str(), l["copies"][0]["running"].as_bool()), (Some(a_file.as_str()), Some(true)));
    assert!(b.execute("file.recovery.discard", &json!({})).unwrap()["discarded"].as_array().unwrap().is_empty());
    assert!(b.execute("file.recovery.discard", &json!({"file": a_file})).is_err());
    // A recovery save alone repairs too (no heartbeat in between).
    store.set_now(1_000_000 + 600);
    let mut c = relaunch(&store);
    c.prefs.autosave_interval = 1;
    c.execute("file.recovery.discard", &json!({})).unwrap();
    assert!(copies(store.as_ref()).is_empty());
    assert_eq!(a.execute("file.recovery.save", &json!({})).unwrap()["saved"].as_array().unwrap().len(), 1);
    assert_eq!(copies(store.as_ref()), ["Untitled-1-1"]);
}

/// Issue #367 where the store can tell a paused app from a gone one (the browser's Web Locks): the
/// paused app's copies are never taken, however old its heartbeat.
#[test]
fn announced_areas_stay_held_while_their_app_is_paused() {
    let store = Arc::new(MemoryStore::announcing());
    let mut a = session_in(&store);
    a.execute("file.recovery.save", &json!({})).unwrap();
    let a_file = format!("{}/Untitled-1-1", a.recovery.own_area().unwrap());
    store.set_now(1_000_000 + 3600);
    let mut b = relaunch(&store);
    assert!(offered(&mut b).is_empty());
    assert!(b.execute("file.recovery.discard", &json!({})).unwrap()["discarded"].as_array().unwrap().is_empty());
    assert_eq!(restored(b.execute("file.recovery.restore", &json!({})).unwrap()), 0);
    assert!(b.execute("file.recovery.discard", &json!({"file": a_file})).unwrap_err().to_string().contains("running"));
    assert_eq!(copies(store.as_ref()), ["Untitled-1-1"]);
    // A crashes: the browser releases its lock, and its copies are offered once its heartbeat is
    // stale too.
    drop(a);
    assert_eq!(offered(&mut b), ["Untitled-1"]);
    assert_eq!(restored(b.execute("file.recovery.restore", &json!({})).unwrap()), 1);
    // B's area is announced in turn.
    let mut c = relaunch(&store);
    store.set_now(1_000_000 + 7200);
    assert!(offered(&mut c).is_empty());
}

#[test]
fn an_app_that_leaves_has_its_copies_offered_at_once() {
    for store in [MemoryStore::without_locks(), MemoryStore::announcing(), MemoryStore::default()] {
        let store = Arc::new(store);
        let mut a = session_in(&store);
        a.execute("file.recovery.save", &json!({})).unwrap();
        let mut b = relaunch(&store);
        assert!(offered(&mut b).is_empty(), "A still runs");
        // A can't go on (its page lost its graphics): its copies stay, for the next app at once.
        cmd::recovery::leave(&mut a);
        assert!(a.doc().unwrap().recovery.is_none());
        assert_eq!(copies(store.as_ref()), ["Untitled-1-1"]);
        assert_eq!(offered(&mut b), ["Untitled-1"]);
        assert_eq!(restored(b.execute("file.recovery.restore", &json!({})).unwrap()), 1);
    }
}

#[test]
fn folder_areas_are_locked_by_running_apps() {
    let dir = std::env::temp_dir().join(format!("vc-recovery-locks-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let folder = dir.to_string_lossy().to_string();
    let app = |dirty: bool| {
        let mut s = Session::new();
        s.prefs.recovery_folder = folder.clone();
        s.execute("file.new", &json!({"width": 50, "height": 50})).unwrap();
        if dirty {
            s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap();
        }
        s
    };
    let mut a = app(true);
    a.execute("file.recovery.save", &json!({})).unwrap();
    let area = a.recovery.own_area().unwrap().to_string();
    assert!(dir.join(&area).join(".lock").is_file() && dir.join(&area).join("Untitled-1-1.vectorcraft").is_file());
    // The lock file is held: another app (another handle) can't take it.
    let store = FolderStore::new(&dir);
    assert!(matches!(store.lock(&area), Ok(cmd::recovery::Lock::Busy)));
    let mut b = app(false);
    assert!(offered(&mut b).is_empty());
    assert!(b.execute("file.recovery.discard", &json!({})).unwrap()["discarded"].as_array().unwrap().is_empty());
    assert!(dir.join(&area).join("Untitled-1-1.vectorcraft").is_file());
    // A crashes (its handle closes): B offers its copy, and restoring it tidies A's folder away.
    drop(a);
    assert_eq!(offered(&mut b), ["Untitled-1"]);
    b.execute("file.recovery.restore", &json!({})).unwrap();
    assert!(!dir.join(&area).exists(), "the crashed app's folder is removed");
    let mine = b.recovery.own_area().unwrap().to_string();
    assert!(dir.join(&mine).join("Untitled-1-1.vectorcraft").is_file());
    // An empty folder a crash left (no copies) is tidied by the next list.
    std::fs::create_dir_all(dir.join("123-1")).unwrap();
    std::fs::write(dir.join("123-1").join(".lock"), b"").unwrap();
    b.execute("file.recovery.list", &json!({})).unwrap();
    assert!(!dir.join("123-1").exists());
    // B quits with nothing unsaved: its folder goes too.
    b.execute("file.close", &json!({})).unwrap();
    b.execute("file.close", &json!({})).unwrap();
    cmd::recovery::forget_all(&mut b);
    assert!(!dir.join(&mine).exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The copies are unsaved documents: on Unix the folders the store makes are owner-only, and a
/// store folder that already exists (one picked in Preferences) keeps its mode.
#[cfg(unix)]
#[test]
fn the_folder_store_makes_owner_only_folders() {
    use std::os::unix::fs::PermissionsExt as _;
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    let base = std::env::temp_dir().join(format!("vc-recovery-modes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    // A store folder that doesn't exist yet is made with the first copy, owner-only like its area.
    let new = base.join("new");
    let store = FolderStore::new(&new);
    store.write("1-1/Untitled-1-1.vectorcraft", b"copy").unwrap();
    assert_eq!(mode(&new), 0o700);
    assert_eq!(mode(&new.join("1-1")), 0o700);
    assert!(matches!(store.lock("1-2"), Ok(cmd::recovery::Lock::Held(_))));
    assert_eq!(mode(&new.join("1-2")), 0o700);
    // A folder that already exists keeps its mode; the area made inside it is owner-only.
    let picked = base.join("picked");
    std::fs::create_dir_all(&picked).unwrap();
    std::fs::set_permissions(&picked, std::fs::Permissions::from_mode(0o755)).unwrap();
    FolderStore::new(&picked).write("2-1/Untitled-1-1.vectorcraft", b"copy").unwrap();
    assert_eq!(mode(&picked), 0o755);
    assert_eq!(mode(&picked.join("2-1")), 0o700);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn saving_reverting_or_closing_removes_the_copy() {
    let dir = std::env::temp_dir().join(format!("vc-recovery-save-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("doc.vectorcraft").to_string_lossy().to_string();
    // Save to a file.
    let (mut s, store) = session();
    s.execute("file.recovery.save", &json!({})).unwrap();
    assert_eq!(copies(store.as_ref()).len(), 1);
    s.execute("document.save", &json!({"path": path})).unwrap();
    assert!(copies(store.as_ref()).is_empty(), "saved: the copy goes");
    assert!(s.doc().unwrap().recovery.is_none());
    // Revert.
    s.execute("shape.ellipse", &json!({"x": 100, "y": 10, "width": 20, "height": 20})).unwrap();
    s.execute("file.recovery.save", &json!({})).unwrap();
    assert_eq!(copies(store.as_ref()).len(), 1);
    s.execute("file.revert", &json!({})).unwrap();
    assert!(copies(store.as_ref()).is_empty(), "reverted: the copy goes");
    // Close (discarding the changes).
    s.execute("shape.ellipse", &json!({"x": 100, "y": 10, "width": 20, "height": 20})).unwrap();
    s.execute("file.recovery.save", &json!({})).unwrap();
    s.execute("file.close", &json!({})).unwrap();
    assert!(copies(store.as_ref()).is_empty(), "closed: the copy goes");
    // A restored document's copy goes once it is saved.
    let (mut s, store) = session();
    s.execute("file.recovery.save", &json!({})).unwrap();
    drop(s);
    let mut s2 = relaunch(&store);
    assert_eq!(restored(s2.execute("file.recovery.restore", &json!({})).unwrap()), 1);
    s2.execute("document.save", &json!({"path": path})).unwrap();
    assert!(copies(store.as_ref()).is_empty());
    let st = s2.doc().unwrap();
    assert!(!st.recovered && !st.is_dirty());
    assert_eq!(st.title(), "doc.vectorcraft");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn undoing_back_to_the_saved_state_drops_the_copy() {
    let (mut s, store) = session();
    s.execute("file.recovery.save", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(!s.doc().unwrap().is_dirty());
    let r = s.execute("file.recovery.save", &json!({})).unwrap();
    assert!(r["saved"].as_array().unwrap().is_empty() && r["skipped"].as_array().unwrap().is_empty(), "{r}");
    assert!(copies(store.as_ref()).is_empty());
}

#[test]
fn complex_documents_are_skipped_when_the_preference_says() {
    let store = Arc::new(MemoryStore::default());
    let mut s = Session::new();
    s.recovery.set_store(store.clone());
    let mut d = vectorcraft_doc::Document::new(1000.0, 1000.0);
    let layer = d.layers[0].id;
    for i in 0..cmd::recovery::COMPLEX_OBJECTS {
        let id = d.alloc_id();
        let r = vectorcraft_geom::Rect::new((i % 200) as f64, (i / 200) as f64, (i % 200) as f64 + 1.0, (i / 200) as f64 + 1.0);
        let n = vectorcraft_doc::Node::path(id, vectorcraft_geom::shapes::rectangle(r), vectorcraft_doc::Appearance::default_art());
        d.insert(Some(layer), i, n).unwrap();
    }
    s.add_document(d, None);
    // One more object makes it complex (and modified).
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap();
    s.prefs.recovery_off_for_complex = true;
    let r = s.execute("file.recovery.save", &json!({})).unwrap();
    assert!(r["saved"].as_array().unwrap().is_empty());
    assert!(r["skipped"][0]["reason"].as_str().unwrap().starts_with("complex document"), "{r}");
    assert!(store.list().unwrap().is_empty(), "no copy, no area");
    s.prefs.recovery_off_for_complex = false;
    assert_eq!(s.execute("file.recovery.save", &json!({})).unwrap()["saved"].as_array().unwrap().len(), 1);
}

#[test]
fn discard_deletes_copies_left_behind_only() {
    let (mut s, store) = session();
    s.execute("file.recovery.save", &json!({})).unwrap();
    let crashed = s.recovery.own_area().unwrap().to_string();
    drop(s);
    let mut s = relaunch(&store);
    s.execute("file.new", &json!({"width": 50, "height": 50})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap();
    s.execute("file.recovery.save", &json!({})).unwrap();
    assert_eq!(copies(store.as_ref()).len(), 2);
    assert!(s.execute("file.recovery.discard", &json!({"file": "nope"})).is_err());
    // The open document's copy stays.
    let r = s.execute("file.recovery.discard", &json!({})).unwrap();
    assert_eq!(r["discarded"], json!([format!("{crashed}/Untitled-1-1")]));
    let mine = s.recovery.own_area().unwrap();
    assert_eq!(store.list().unwrap(), [format!("{mine}/Untitled-1-1.json"), format!("{mine}/Untitled-1-1.vectorcraft")]);
}

#[test]
fn without_a_store_the_commands_say_where_to_set_one() {
    let mut s = Session::new();
    let e = s.execute("file.recovery.list", &json!({})).unwrap_err().to_string();
    assert!(e.contains("recoveryFolder"), "{e}");
    // The preference names a folder: copies go there.
    let dir = std::env::temp_dir().join(format!("vc-recovery-folder-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    s.prefs.recovery_folder = dir.to_string_lossy().to_string();
    assert!(s.execute("file.recovery.list", &json!({})).unwrap()["copies"].as_array().unwrap().is_empty(), "no folder yet: no copies");
    s.execute("file.new", &json!({"width": 50, "height": 50})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap();
    s.execute("file.recovery.save", &json!({})).unwrap();
    let area = dir.join(s.recovery.own_area().unwrap());
    assert!(area.join("Untitled-1-1.vectorcraft").is_file() && area.join("Untitled-1-1.json").is_file());
    s.execute("file.close", &json!({})).unwrap();
    assert!(!area.join("Untitled-1-1.vectorcraft").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A store in memory that notes the entries written to it.
#[derive(Default)]
struct Counting {
    inner: MemoryStore,
    written: std::sync::Mutex<Vec<(String, usize)>>,
}

impl Counting {
    /// The image entries written since the last call, with their sizes.
    fn images_written(&self) -> Vec<(String, usize)> {
        std::mem::take(&mut *self.written.lock().unwrap()).into_iter().filter(|(n, _)| n.ends_with(".blob")).collect()
    }

    fn images(&self) -> Vec<String> {
        self.inner.list().unwrap().into_iter().filter(|n| n.ends_with(".blob")).collect()
    }
}

impl RecoveryStore for Counting {
    fn list(&self) -> std::result::Result<Vec<String>, String> {
        self.inner.list()
    }
    fn read(&self, name: &str) -> std::result::Result<Vec<u8>, String> {
        self.inner.read(name)
    }
    fn write(&self, name: &str, bytes: &[u8]) -> std::result::Result<(), String> {
        self.written.lock().unwrap().push((name.to_string(), bytes.len()));
        self.inner.write(name, bytes)
    }
    fn remove(&self, name: &str) -> std::result::Result<(), String> {
        self.inner.remove(name)
    }
    fn location(&self) -> String {
        self.inner.location()
    }
    fn lock(&self, area: &str) -> std::result::Result<crate::cmd::recovery::Lock, String> {
        self.inner.lock(area)
    }
    fn now(&self) -> Option<i64> {
        self.inner.now()
    }
}

fn place_png(s: &mut Session, name: &str, rgb: [u8; 3]) -> Vec<u8> {
    let png = crate::tests_links::png(64, 48, rgb);
    s.execute("file.place", &json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(&png), "link": false})).unwrap();
    png
}

/// The document's images: their bytes by key.
fn images_of(s: &Session) -> std::collections::BTreeMap<String, Vec<u8>> {
    s.doc().unwrap().doc.images.iter().map(|(k, b)| (k.clone(), b.bytes.to_vec())).collect()
}

#[test]
fn images_are_kept_apart_and_written_again_only_when_they_change() {
    let store = Arc::new(Counting::default());
    let mut s = Session::new();
    s.recovery.set_store(store.clone());
    s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    let red = place_png(&mut s, "red.png", crate::tests_links::RED);
    let r = s.execute("file.recovery.save", &json!({})).unwrap();
    assert_eq!(r["saved"][0]["imageBytes"], red.len(), "{r}");
    assert_eq!(store.images_written().len(), 1);
    let area = s.recovery.own_area().unwrap().to_string();
    // The document entry doesn't hold the image.
    let doc = store.read(&format!("{area}/Untitled-1-1.vectorcraft")).unwrap();
    assert!(vectorcraft_format::load(&doc).unwrap().images.is_empty());
    // An edit that leaves the image alone: the copy is written again, its image isn't.
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 20})).unwrap();
    let r = s.execute("file.recovery.save", &json!({})).unwrap();
    assert_eq!((r["saved"].as_array().unwrap().len(), r["saved"][0]["imageBytes"].as_u64()), (1, Some(0)), "{r}");
    assert!(store.images_written().is_empty(), "the image isn't written again");
    // A new image: only it is written.
    let blue = place_png(&mut s, "blue.png", crate::tests_links::BLUE);
    let r = s.execute("file.recovery.save", &json!({})).unwrap();
    assert_eq!(r["saved"][0]["imageBytes"], blue.len());
    assert_eq!(store.images_written().iter().map(|(_, n)| *n).collect::<Vec<_>>(), [blue.len()]);
    assert_eq!(store.images().len(), 2);
    let images = images_of(&s);
    assert_eq!(images.len(), 2);

    // A crash: the copy restores with its images.
    drop(s);
    let mut s = Session::new();
    s.recovery.set_store(store.clone());
    assert_eq!(restored(s.execute("file.recovery.restore", &json!({})).unwrap()), 1);
    assert_eq!(images_of(&s), images);
    let mine = s.recovery.own_area().unwrap().to_string();
    assert!(store.images().iter().all(|n| n.starts_with(&format!("{mine}/"))), "the images moved with the copy, the old area went");
    assert_eq!(store.images().len(), 2);
    // Its copy and another document's: once the first is closed, the next copy sweeps its images.
    s.execute("file.new", &json!({"width": 50, "height": 50})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 1, "y": 1, "width": 5, "height": 5})).unwrap();
    s.execute("file.close", &json!({"index": 0})).unwrap();
    s.execute("file.recovery.save", &json!({})).unwrap();
    assert!(store.images().is_empty(), "{:?}", store.list());
}

#[test]
fn copies_from_before_images_were_kept_apart_still_restore() {
    let store = Arc::new(MemoryStore::default());
    // The document entry is a whole native file, images inside; the details say nothing of them.
    let mut s = session_in(&store);
    place_png(&mut s, "red.png", crate::tests_links::RED);
    let d = vectorcraft_doc::Document::clone(&s.doc().unwrap().doc);
    let images = images_of(&s);
    drop(s);
    store.write("1-1/Old-1.vectorcraft", &vectorcraft_format::save_file(&d)).unwrap();
    store.write("1-1/Old-1.json", br#"{"title": "Old", "format": "vectorcraft", "saved": 5}"#).unwrap();
    let mut s = relaunch(&store);
    assert_eq!(restored(s.execute("file.recovery.restore", &json!({})).unwrap()), 1);
    assert_eq!(images_of(&s), images);
    assert_eq!(s.doc().unwrap().title(), "Untitled-1 [Recovered]");
}

/// A store whose app dies right after writing a copy's document entry: later writes fail, so the
/// copy's details stay as the last interval left them.
struct DiesAfterDocument {
    inner: Arc<MemoryStore>,
    dead: std::sync::atomic::AtomicBool,
}

impl RecoveryStore for DiesAfterDocument {
    fn list(&self) -> std::result::Result<Vec<String>, String> {
        self.inner.list()
    }
    fn read(&self, name: &str) -> std::result::Result<Vec<u8>, String> {
        self.inner.read(name)
    }
    fn write(&self, name: &str, bytes: &[u8]) -> std::result::Result<(), String> {
        if self.dead.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("the app is gone".into());
        }
        self.inner.write(name, bytes)?;
        if name.ends_with(".vectorcraft") {
            self.dead.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(())
    }
    fn remove(&self, name: &str) -> std::result::Result<(), String> {
        self.inner.remove(name)
    }
    fn location(&self) -> String {
        self.inner.location()
    }
    fn lock(&self, area: &str) -> std::result::Result<crate::cmd::recovery::Lock, String> {
        self.inner.lock(area)
    }
    fn now(&self) -> Option<i64> {
        self.inner.now()
    }
}

/// A crash between writing a copy's document and its details restores the new document with
/// every image, the one placed since the last copy too (and the first copy likewise).
#[test]
fn a_crash_between_a_copys_document_and_its_details_keeps_every_image() {
    for first in [true, false] {
        let store = Arc::new(MemoryStore::default());
        let mut s = session_in(&store);
        if !first {
            place_png(&mut s, "red.png", crate::tests_links::RED);
            s.execute("file.recovery.save", &json!({})).unwrap();
        }
        place_png(&mut s, "blue.png", crate::tests_links::BLUE);
        let images = images_of(&s);
        s.recovery.set_store(Arc::new(DiesAfterDocument { inner: store.clone(), dead: Default::default() }));
        assert!(s.execute("file.recovery.save", &json!({})).is_err(), "the details weren't written");
        drop(s);
        let mut s = relaunch(&store);
        assert_eq!(restored(s.execute("file.recovery.restore", &json!({})).unwrap()), 1, "first copy: {first}");
        assert_eq!(images_of(&s), images, "first copy: {first}");
    }
}
