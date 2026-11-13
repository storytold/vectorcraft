//! Data Recovery: `file.recovery.save/list/restore/discard`, copies removed on save, revert and
//! close, complex documents skipped, each app's own area (another running app's copies are never
//! offered: locks, heartbeats, announcements; a paused app repairs its copies), and the folder
//! store with real file locks.

use std::sync::Arc;

use serde_json::json;

use crate::cmd::recovery::{FolderStore, MemoryStore, RecoveryStore};
use crate::*;

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
fn native_recovery_copies_leave_out_the_pdf() {
    let (mut s, store) = session();
    let r = s.execute("file.recovery.save", &json!({})).unwrap();
    let file = format!("{}.vectorcraft", r["saved"][0]["file"].as_str().unwrap());
    let bytes = store.read(&file).unwrap();
    assert!(vectorcraft_format::pdf_content(&bytes).is_none());
    assert_eq!(vectorcraft_format::load(&bytes).unwrap().node_count(), s.doc().unwrap().doc.node_count());
}

const RECOVERY_FONT_FAMILY: &str = "Recovery Carried Font Regression";
const RECOVERY_FONT_BYTES: &[u8] = include_bytes!("../../../../assets/fonts/Inter-Regular.ttf");

fn carried_font_copy() -> (Arc<MemoryStore>, vectorcraft_doc::NodeId) {
    use vectorcraft_doc::{CharStyle, FILE_FONT_MIME, FileFont, FileFonts, ImageBlob, Node, NodeKind, TextObject};

    let (mut s, store) = session();
    let doc = Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    doc.images.insert("recovered-font".into(), ImageBlob::new(FILE_FONT_MIME, RECOVERY_FONT_BYTES.to_vec()));
    // An uninstalled family ensures the fixture must be read from the recovered resource.
    let style = CharStyle { font_family: RECOVERY_FONT_FAMILY.into(), size: 20.0, ..Default::default() };
    let mut text = TextObject::point(kurbo::Point::new(10.0, 40.0), "x", style);
    text.file_fonts = Some(Box::new(FileFonts::new(
        vec![FileFont { family: RECOVERY_FONT_FAMILY.into(), style: "Regular".into(), key: "recovered-font".into() }],
        "x",
    )));
    let id = doc.alloc_id();
    doc.insert(Some(doc.layers[0].id), usize::MAX, Node::new(id, NodeKind::Text(Box::new(text)))).unwrap();
    let saved = s.execute("file.recovery.save", &json!({})).unwrap();
    let file = saved["saved"][0]["file"].as_str().unwrap();
    let lean = vectorcraft_format::load(&store.read(&format!("{file}.vectorcraft")).unwrap()).unwrap();
    assert!(lean.images.is_empty(), "the font travels in the recovery store, not the document entry");
    drop(s);
    (store, id)
}

#[test]
fn recovered_carried_fonts_bind_after_their_resources_are_restored() {
    use vectorcraft_doc::NodeKind;
    use vectorcraft_text::FontDb;

    let (store, id) = carried_font_copy();
    let mut s = relaunch(&store);
    let reply = s.execute("file.recovery.restore", &json!({})).unwrap();
    assert_eq!(reply["restored"].as_array().unwrap().len(), 1, "{reply}");
    assert!(reply["failed"].as_array().unwrap().is_empty(), "{reply}");
    assert!(reply["restored"][0]["warnings"].as_array().unwrap().is_empty(), "{reply}");
    let doc = &s.doc().unwrap().doc;
    assert_eq!(doc.images["recovered-font"].bytes.as_slice(), RECOVERY_FONT_BYTES);
    let NodeKind::Text(text) = &doc.node(id).unwrap().kind else { panic!("text") };
    let db = FontDb::global();
    let laid = vectorcraft_text::layout(db, text);
    assert_eq!(laid.glyphs.len(), 1);
    let face = db.face_by_id(laid.glyphs[0].font_id).unwrap();
    assert_eq!(face.file_data(), RECOVERY_FONT_BYTES, "layout uses the recovered font's exact bytes");
}

#[test]
fn recovered_missing_or_corrupt_carried_fonts_report_the_actual_refusal() {
    for missing in [true, false] {
        let (store, id) = carried_font_copy();
        let blob = store.list().unwrap().into_iter().find(|name| name.ends_with(".blob")).unwrap();
        if missing {
            store.remove(&blob).unwrap();
        } else {
            store.write(&blob, b"not a font").unwrap();
        }
        let mut s = relaunch(&store);
        let reply = s.execute("file.recovery.restore", &json!({})).unwrap();
        assert_eq!(reply["restored"].as_array().unwrap().len(), 1, "{reply}");
        assert!(reply["failed"].as_array().unwrap().is_empty(), "{reply}");
        let warnings = reply["restored"][0]["warnings"].as_array().unwrap();
        assert!(
            warnings.iter().any(|note| note.as_str().is_some_and(|note| note.contains(RECOVERY_FONT_FAMILY) && note.contains("substitute"))),
            "missing={missing}: {reply}"
        );
        let vectorcraft_doc::NodeKind::Text(text) = &s.doc().unwrap().doc.node(id).unwrap().kind else { panic!("text") };
        assert!(vectorcraft_text::FontDb::global().file_faces(text).is_empty(), "missing={missing}");
    }
}

#[test]
fn restored_recovery_copies_keep_the_original_pdf_preference() {
    let dir = crate::tests_links::Folder::new("recovered-pdf-preference");
    for pdf in [false, true] {
        let (mut s, store) = session();
        s.execute("file.saveAs", &json!({"path": dir.file(&format!("original-{pdf}.vectorcraft")), "pdfCompatible": pdf})).unwrap();
        s.execute("shape.ellipse", &json!({"x": 80, "y": 20, "width": 20, "height": 20})).unwrap();
        let copied = s.execute("file.recovery.save", &json!({})).unwrap();
        let file = format!("{}.vectorcraft", copied["saved"][0]["file"].as_str().unwrap());
        assert!(vectorcraft_format::pdf_content(&store.read(&file).unwrap()).is_none());
        drop(s);
        let mut s = relaunch(&store);
        assert_eq!(restored(s.execute("file.recovery.restore", &json!({})).unwrap()), 1);
        assert_eq!(s.doc().unwrap().save_options.get("pdfCompatible"), Some(&json!(pdf)));
        let saved = s.execute("document.save", &json!({})).unwrap();
        let bytes = vectorcraft_format::base64_decode(saved["dataBase64"].as_str().unwrap()).unwrap();
        assert_eq!(vectorcraft_format::pdf_content(&bytes).is_some(), pdf);
    }
}

#[test]
fn recovered_unsaved_documents_keep_inheriting_the_current_pdf_preference() {
    let dir = crate::tests_links::Folder::new("recovered-inherited-pdf-choice");
    let (mut s, store) = session();
    s.execute("file.recovery.save", &json!({})).unwrap();
    assert!(s.doc().unwrap().save_options.get("pdfCompatible").is_none());
    s.execute("file.new", &json!({"width": 10, "height": 10})).unwrap();
    s.execute("file.saveAs", &json!({"path":dir.file("other.vectorcraft"),"pdfCompatible":false})).unwrap();
    assert!(s.execute("file.recovery.save", &json!({})).unwrap()["saved"].as_array().unwrap().is_empty());
    let latest_preferences = s.prefs.clone();
    drop(s);
    let mut s = relaunch(&store);
    // The host loads its latest preferences before offering recovery.
    s.prefs = latest_preferences;
    assert_eq!(restored(s.execute("file.recovery.restore", &json!({})).unwrap()), 1);
    assert!(s.doc().unwrap().save_options.get("pdfCompatible").is_none());
    let saved = s.execute("document.save", &json!({})).unwrap();
    let bytes = vectorcraft_format::base64_decode(saved["dataBase64"].as_str().unwrap()).unwrap();
    assert!(vectorcraft_format::pdf_content(&bytes).is_none());
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

/// A threaded frame whose first paragraph continues from the frame before is restored without
/// that paragraph's first-line indent.
#[test]
fn a_restored_copy_keeps_continued_paragraphs_in_threaded_frames() {
    let store = Arc::new(MemoryStore::default());
    let mut s = relaunch(&store);
    s.execute("file.new", &json!({"width": 600, "height": 300})).unwrap();
    let story = "The quick brown fox jumps over the lazy dog and keeps running far beyond the first frame of this story. ".repeat(4);
    let a =
        s.execute("text.create", &json!({"x": 10, "y": 10, "text": story, "area": {"width": 150, "height": 60}})).unwrap()["id"].as_u64().unwrap();
    let b = s.execute("shape.rectangle", &json!({"x": 200, "y": 10, "width": 150, "height": 200})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    s.execute("text.thread.create", &json!({})).unwrap();
    s.execute("text.setFormat", &json!({"ids": [a, b], "leftIndent": 0, "firstLineIndent": 24})).unwrap();
    let continues = |s: &Session| match &s.doc().unwrap().doc.node(vectorcraft_doc::NodeId(b)).unwrap().kind {
        vectorcraft_doc::NodeKind::Text(t) => t.first_para_continues,
        _ => panic!("not text"),
    };
    assert!(continues(&s));
    s.execute("file.recovery.save", &json!({})).unwrap();
    drop(s);

    let mut s = relaunch(&store);
    assert_eq!(restored(s.execute("file.recovery.restore", &json!({})).unwrap()), 1);
    assert!(continues(&s), "the second frame continues the first frame's paragraph");
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

/// An entry that is a link (the copy's metadata names it) is not followed out of the store.
#[cfg(unix)]
#[test]
fn the_folder_store_reads_no_linked_entries() {
    let base = std::env::temp_dir().join(format!("vc-recovery-links-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let store = FolderStore::new(&base);
    store.write("1-1/Untitled-1-1.vectorcraft", b"copy").unwrap();
    std::fs::write(base.join("outside"), b"private").unwrap();
    std::os::unix::fs::symlink(base.join("outside"), base.join("1-1").join("img")).unwrap();
    assert_eq!(store.read("1-1/Untitled-1-1.vectorcraft").unwrap(), b"copy");
    assert!(store.read("1-1/img").is_err());
    let _ = std::fs::remove_dir_all(&base);
}

/// A file where a folder of the store's path should be: the error names that file, not the area
/// folder or the copy below it, for a write, a lock, a read, a listing, a removal and a recovery
/// save.
#[cfg(unix)]
#[test]
fn folder_errors_name_a_file_in_the_way() {
    let base = std::env::temp_dir().join(format!("vc-recovery-file-in-way-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let file = base.join("file");
    std::fs::write(&file, b"").unwrap();
    let folder = file.join("recovery");
    let store = FolderStore::new(&folder);
    let mut s = Session::new();
    s.prefs.recovery_folder = folder.to_string_lossy().to_string();
    s.execute("file.new", &json!({"width": 50, "height": 50})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap();
    let errors = [
        ("write", store.write("1-1/Untitled-1-1.vectorcraft", b"copy").err()),
        ("lock", store.lock("1-2").err()),
        ("read", store.read("1-1/Untitled-1-1.vectorcraft").err()),
        ("list", store.list().err()),
        ("remove", store.remove("1-1/Untitled-1-1.vectorcraft").err()),
        ("save", s.execute("file.recovery.save", &json!({})).err().map(|e| e.to_string())),
    ];
    let _ = std::fs::remove_dir_all(&base);
    let named = format!("{} is not a folder: ", file.display());
    let wrong: Vec<_> = errors.iter().filter(|(_, e)| !e.as_deref().is_some_and(|e| e.contains(&named))).collect();
    assert!(wrong.is_empty(), "{wrong:?}");
}

/// A removal that finds the copy missing because a file is on the way to it (how Windows reports a
/// file in the way) names that file. A copy that is just not there still counts as removed.
#[test]
fn a_removal_reported_missing_names_a_file_in_the_way() {
    let base = std::env::temp_dir().join(format!("vc-recovery-remove-in-way-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let file = base.join("file");
    std::fs::write(&file, b"").unwrap();
    let missing = || std::io::Error::from(std::io::ErrorKind::NotFound);
    let behind = FolderStore::removed(&file.join("recovery").join("1-1").join("Untitled-1-1.vectorcraft"), Err(missing()));
    let gone = FolderStore::removed(&base.join("recovery").join("1-1").join("Untitled-1-1.vectorcraft"), Err(missing()));
    let _ = std::fs::remove_dir_all(&base);
    let named = format!("{} is not a folder: ", file.display());
    assert!(behind.as_ref().is_err_and(|e| e.starts_with(&named)), "{behind:?}");
    assert_eq!(gone, Ok(()));
}

/// A folder of the store's path that is missing and can't be made: a write or a lock names it as
/// the folder it couldn't make, and a read names it as missing. An area folder that can't be made
/// in a store folder that is there is named the same way.
#[cfg(unix)]
#[test]
fn folder_errors_name_a_missing_folder_that_cannot_be_made() {
    use std::os::unix::fs::PermissionsExt as _;
    let base = std::env::temp_dir().join(format!("vc-recovery-missing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let closed = base.join("closed");
    std::fs::create_dir_all(&closed).unwrap();
    std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o500)).unwrap();
    let gone = closed.join("gone");
    let store = FolderStore::new(gone.join("recovery"));
    let unmade = format!("couldn't make {}: ", gone.display());
    let area = FolderStore::new(&closed).write("1-1/Untitled-1-1.vectorcraft", b"copy").err();
    let errors = [
        ("write", store.write("1-1/Untitled-1-1.vectorcraft", b"copy").err(), unmade.clone()),
        ("lock", store.lock("1-2").err(), unmade),
        ("read", store.read("1-1/Untitled-1-1.vectorcraft").err(), format!("{} is missing", gone.display())),
        ("area", area, format!("couldn't make {}: ", closed.join("1-1").display())),
    ];
    std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o700)).unwrap();
    let _ = std::fs::remove_dir_all(&base);
    // An account that can write anywhere (root) makes the folders, so the test has nothing to check.
    if errors[0].1.is_none() {
        return;
    }
    let wrong: Vec<_> = errors.iter().filter(|(_, e, named)| !e.as_deref().is_some_and(|e| e.starts_with(named.as_str()))).collect();
    assert!(wrong.is_empty(), "{wrong:?}");
}

/// When every folder on the way is there, an error names the folder it happened on.
#[cfg(unix)]
#[test]
fn folder_errors_name_the_folder_when_every_folder_is_there() {
    use std::os::unix::fs::PermissionsExt as _;
    let base = std::env::temp_dir().join(format!("vc-recovery-unreadable-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let unreadable = base.join("unreadable");
    std::fs::create_dir_all(&unreadable).unwrap();
    std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o300)).unwrap();
    let listed = FolderStore::new(&unreadable).list();
    std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let _ = std::fs::remove_dir_all(&base);
    // An account that can read any folder (root) lists it, so the test has nothing to check.
    let Err(err) = listed else { return };
    assert!(err.starts_with(&format!("{}: ", unreadable.display())), "{err}");
}

/// A store in memory that notes each removal of an area (its lock file and folder, then its empty
/// folder) and whether the area was still held then.
#[derive(Default)]
struct Tidying {
    inner: MemoryStore,
    removals: std::sync::Mutex<Vec<(&'static str, bool)>>,
}

impl Tidying {
    fn note(&self, what: &'static str, area: &str) {
        let held = matches!(self.inner.lock(area), Ok(cmd::recovery::Lock::Busy));
        self.removals.lock().unwrap().push((what, held));
    }
}

impl RecoveryStore for Tidying {
    fn list(&self) -> std::result::Result<Vec<String>, String> {
        self.inner.list()
    }
    fn read(&self, name: &str) -> std::result::Result<Vec<u8>, String> {
        self.inner.read(name)
    }
    fn write(&self, name: &str, bytes: &[u8]) -> std::result::Result<(), String> {
        self.inner.write(name, bytes)
    }
    fn remove(&self, name: &str) -> std::result::Result<(), String> {
        self.inner.remove(name)
    }
    fn location(&self) -> String {
        self.inner.location()
    }
    fn lock(&self, area: &str) -> std::result::Result<cmd::recovery::Lock, String> {
        self.inner.lock(area)
    }
    fn remove_area(&self, area: &str) {
        self.note("area", area);
    }
    fn remove_empty_area(&self, area: &str) {
        self.note("empty folder", area);
    }
    fn now(&self) -> Option<i64> {
        self.inner.now()
    }
}

/// An app tidying an area away (a crashed app's, or its own when it quits) removes it while it
/// still holds it, so another app taking a new area of that name can't lock it in between. Once it
/// let the area go, it removes the folder again if that is empty.
#[test]
fn areas_are_removed_while_still_held() {
    let store = Arc::new(Tidying::default());
    let app = || {
        let mut s = Session::new();
        s.recovery.set_store(store.clone());
        s.execute("file.new", &json!({"width": 50, "height": 50})).unwrap();
        s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap();
        s
    };
    let mut a = app();
    a.execute("file.recovery.save", &json!({})).unwrap();
    drop(a);
    let mut b = app();
    assert_eq!(b.execute("file.recovery.discard", &json!({})).unwrap()["discarded"].as_array().unwrap().len(), 1);
    b.execute("file.recovery.save", &json!({})).unwrap();
    b.execute("file.close", &json!({})).unwrap();
    cmd::recovery::forget_all(&mut b);
    let tidy = [("area", true), ("empty folder", false)];
    assert_eq!(*store.removals.lock().unwrap(), [tidy, tidy].concat(), "discard's tidy, then quitting");
}

/// Another app tidied an empty area away between this app making it and locking it: the area is
/// busy, not held and not an error.
#[test]
fn an_area_tidied_away_before_it_is_locked_is_busy() {
    use cmd::recovery::Lock;
    let base = std::env::temp_dir().join(format!("vc-recovery-tidied-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let store = FolderStore::new(&base);
    drop(store.lock("5-1").unwrap());
    let area = base.join("5-1");
    let path = area.join(".lock");
    let open = |create: bool| std::fs::OpenOptions::new().read(true).write(true).create(create).truncate(false).open(&path).unwrap();
    // The lock file was opened, then removed before it was locked.
    let opened = open(false);
    std::fs::remove_file(&path).unwrap();
    assert!(matches!(FolderStore::take(opened, &path), Ok(Lock::Busy)));
    // The folder went before it was checked or its lock file opened.
    std::fs::remove_dir(&area).unwrap();
    assert!(matches!(FolderStore::lock_area(&area), Ok(Lock::Busy)));
    // A lock file made again under the same name is not the one opened, also where it keeps the
    // removed one's creation time (NTFS does within 15 seconds).
    #[cfg(any(unix, windows))]
    {
        std::fs::create_dir_all(&area).unwrap();
        let opened = open(true);
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, b"").unwrap();
        assert!(matches!(FolderStore::take(opened, &path), Ok(Lock::Busy)));
        assert!(matches!(FolderStore::take(open(false), &path), Ok(Lock::Held(_))));
    }
    let _ = std::fs::remove_dir_all(&base);
}

/// Opening an area's lock file failed. When the folder went after it was checked, or the lock file
/// is there but can't be opened (a removed lock file that another app still has open, on file
/// systems that keep it until it is closed), the area is busy. A lock file that can't be made is an
/// error.
#[test]
fn a_lock_file_that_is_gone_or_cannot_be_opened_is_busy() {
    use cmd::recovery::Lock;
    use std::io::ErrorKind::{NotFound, PermissionDenied};
    let base = std::env::temp_dir().join(format!("vc-recovery-unopened-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let path = base.join(".lock");
    let failed = |kind: std::io::ErrorKind| FolderStore::open_failed(kind.into(), &path);
    assert!(matches!(failed(NotFound), Ok(Lock::Busy)));
    assert!(failed(PermissionDenied).is_err());
    std::fs::write(&path, b"").unwrap();
    assert!(matches!(failed(PermissionDenied), Ok(Lock::Busy)));
    #[cfg(unix)]
    if !rustix::process::geteuid().is_root() {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = |p: &std::path::Path, m: u32| std::fs::set_permissions(p, std::fs::Permissions::from_mode(m)).unwrap();
        // A lock file that is there but can't be opened.
        let area = base.join("5-1");
        std::fs::create_dir(&area).unwrap();
        std::fs::write(area.join(".lock"), b"").unwrap();
        mode(&area.join(".lock"), 0o400);
        assert!(matches!(FolderStore::lock_area(&area), Ok(Lock::Busy)));
        // A folder in which no lock file can be made.
        let area = base.join("5-2");
        std::fs::create_dir(&area).unwrap();
        mode(&area, 0o500);
        let r = FolderStore::lock_area(&area);
        mode(&area, 0o700);
        assert!(r.is_err());
    }
    let _ = std::fs::remove_dir_all(&base);
}

/// A folder store in which another app tidies the first area away as soon as it is made.
struct TidiedFirst {
    inner: FolderStore,
    dir: std::path::PathBuf,
    first: std::sync::atomic::AtomicBool,
}

impl RecoveryStore for TidiedFirst {
    fn list(&self) -> std::result::Result<Vec<String>, String> {
        self.inner.list()
    }
    fn read(&self, name: &str) -> std::result::Result<Vec<u8>, String> {
        self.inner.read(name)
    }
    fn write(&self, name: &str, bytes: &[u8]) -> std::result::Result<(), String> {
        self.inner.write(name, bytes)
    }
    fn remove(&self, name: &str) -> std::result::Result<(), String> {
        self.inner.remove(name)
    }
    fn location(&self) -> String {
        self.inner.location()
    }
    fn areas(&self) -> std::result::Result<Vec<String>, String> {
        self.inner.areas()
    }
    fn lock(&self, area: &str) -> std::result::Result<cmd::recovery::Lock, String> {
        if !self.first.swap(false, std::sync::atomic::Ordering::SeqCst) {
            return self.inner.lock(area);
        }
        let dir = self.dir.join(area);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::remove_dir(&dir).unwrap();
        FolderStore::lock_area(&dir)
    }
    fn remove_area(&self, area: &str) {
        self.inner.remove_area(area);
    }
}

#[test]
fn a_recovery_save_takes_the_next_area_when_the_first_is_tidied_away() {
    let dir = std::env::temp_dir().join(format!("vc-recovery-next-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Arc::new(TidiedFirst { inner: FolderStore::new(&dir), dir: dir.clone(), first: true.into() });
    let mut s = Session::new();
    s.recovery.set_store(store.clone());
    s.execute("file.new", &json!({"width": 50, "height": 50})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap();
    let r = s.execute("file.recovery.save", &json!({})).unwrap();
    let area = s.recovery.own_area().unwrap().to_string();
    assert!(area.ends_with("-2"), "{area}");
    assert_eq!(r["saved"][0]["file"], format!("{area}/Untitled-1-1").as_str(), "{r}");
    assert!(dir.join(&area).join("Untitled-1-1.vectorcraft").is_file());
    drop(s);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A session keeping its copies in folder `dir`, with one modified document whose copy is written.
fn saved_in(dir: &std::path::Path) -> (Session, String) {
    let mut s = Session::new();
    s.prefs.recovery_folder = dir.to_string_lossy().to_string();
    s.execute("file.new", &json!({"width": 50, "height": 50})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap();
    s.execute("file.recovery.save", &json!({})).unwrap();
    let area = s.recovery.own_area().unwrap().to_string();
    (s, area)
}

/// A new app over folder `dir`, with nothing open.
#[cfg(unix)]
fn other_app(dir: &std::path::Path) -> Session {
    let mut s = Session::new();
    s.prefs.recovery_folder = dir.to_string_lossy().to_string();
    s
}

/// The area's lock file was removed while its app runs: the next recovery save locks the area
/// again, so another app neither offers nor discards its copy.
#[cfg(unix)]
#[test]
fn a_recovery_save_locks_the_area_again_when_its_lock_file_was_removed() {
    use cmd::recovery::Lock;
    let dir = std::env::temp_dir().join(format!("vc-recovery-relock-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (mut a, area) = saved_in(&dir);
    std::fs::remove_file(dir.join(&area).join(".lock")).unwrap();
    a.execute("file.recovery.save", &json!({})).unwrap();
    assert_eq!(a.recovery.own_area(), Some(area.as_str()));
    let mut b = other_app(&dir);
    assert!(offered(&mut b).is_empty());
    assert!(b.execute("file.recovery.discard", &json!({})).unwrap()["discarded"].as_array().unwrap().is_empty());
    assert!(dir.join(&area).join("Untitled-1-1.vectorcraft").is_file());
    assert!(matches!(FolderStore::new(&dir).lock(&area), Ok(Lock::Busy)));
    drop(a);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The whole area was removed while its app runs: the next recovery save makes it again, locked,
/// with the copy written again.
#[cfg(unix)]
#[test]
fn a_recovery_save_makes_the_area_again_when_it_was_removed() {
    use cmd::recovery::Lock;
    let dir = std::env::temp_dir().join(format!("vc-recovery-remade-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (mut a, area) = saved_in(&dir);
    std::fs::remove_dir_all(dir.join(&area)).unwrap();
    let r = a.execute("file.recovery.save", &json!({})).unwrap();
    assert_eq!(r["saved"][0]["file"], format!("{area}/Untitled-1-1").as_str(), "{r}");
    assert!(dir.join(&area).join("Untitled-1-1.vectorcraft").is_file());
    assert!(matches!(FolderStore::new(&dir).lock(&area), Ok(Lock::Busy)));
    assert!(offered(&mut other_app(&dir)).is_empty());
    drop(a);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Another app locked the area after its lock file was removed: the next heartbeat takes a new
/// area, and the document is to be copied again there. Its copy in the old area stays until then.
#[cfg(unix)]
#[test]
fn an_area_another_app_locked_meanwhile_is_left_for_a_new_one() {
    use cmd::recovery::Lock;
    let dir = std::env::temp_dir().join(format!("vc-recovery-moved-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (mut a, area) = saved_in(&dir);
    std::fs::remove_file(dir.join(&area).join(".lock")).unwrap();
    let store = FolderStore::new(&dir);
    let Ok(Lock::Held(other)) = store.lock(&area) else { panic!("the other app locks the area") };
    let uid = a.doc().unwrap().uid;
    assert_eq!(cmd::recovery::heartbeat(&mut a), [uid]);
    let mine = a.recovery.own_area().unwrap().to_string();
    assert_ne!(mine, area);
    assert!(a.doc().unwrap().recovery.is_none());
    assert!(dir.join(&area).join("Untitled-1-1.vectorcraft").is_file(), "kept until the new copy is written");
    let r = a.execute("file.recovery.save", &json!({})).unwrap();
    assert_eq!(r["saved"][0]["file"], format!("{mine}/Untitled-1-1").as_str(), "{r}");
    assert!(!dir.join(&area).join("Untitled-1-1.vectorcraft").exists(), "the copy left there goes");
    assert!(matches!(store.lock(&mine), Ok(Lock::Busy)));
    drop(other);
    assert!(offered(&mut other_app(&dir)).is_empty());
    drop(a);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A copy written in the background into the area the session left meanwhile is not the
/// document's copy: it stays only until the document is copied again in the new area.
#[cfg(unix)]
#[test]
fn a_copy_written_into_an_area_left_meanwhile_goes_once_copied_again() {
    use cmd::recovery::Lock;
    let dir = std::env::temp_dir().join(format!("vc-recovery-inflight-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (mut a, area) = saved_in(&dir);
    a.execute("shape.ellipse", &json!({"x": 10, "y": 10, "width": 5, "height": 5})).unwrap();
    let store: Arc<dyn RecoveryStore> = Arc::new(FolderStore::new(&dir));
    let (jobs, _) = cmd::recovery::jobs(&mut a, &store).unwrap();
    assert_eq!(jobs.len(), 1);
    std::fs::remove_file(dir.join(&area).join(".lock")).unwrap();
    let Ok(Lock::Held(other)) = store.lock(&area) else { panic!("the other app locks the area") };
    cmd::recovery::heartbeat(&mut a);
    for j in jobs {
        j.write().unwrap();
        j.finish(&mut a);
    }
    assert!(a.doc().unwrap().recovery.is_none());
    assert!(dir.join(&area).join("Untitled-1-1.vectorcraft").is_file());
    let uid = a.doc().unwrap().uid;
    assert_eq!(cmd::recovery::heartbeat(&mut a), [uid], "the next heartbeat has it copied again");
    let r = a.execute("file.recovery.save", &json!({})).unwrap();
    assert_eq!(r["saved"].as_array().unwrap().len(), 1, "{r}");
    assert!(!dir.join(&area).join("Untitled-1-1.vectorcraft").exists());
    assert!(cmd::recovery::heartbeat(&mut a).is_empty());
    drop(other);
    assert!(offered(&mut other_app(&dir)).is_empty());
    drop(a);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Another app locked the area meanwhile and the document is unchanged since its copy: the next
/// recovery save still writes the copy, in the new area.
#[cfg(unix)]
#[test]
fn a_recovery_save_copies_unchanged_documents_into_the_new_area() {
    use cmd::recovery::Lock;
    let dir = std::env::temp_dir().join(format!("vc-recovery-unchanged-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (mut a, area) = saved_in(&dir);
    std::fs::remove_file(dir.join(&area).join(".lock")).unwrap();
    let Ok(Lock::Held(other)) = FolderStore::new(&dir).lock(&area) else { panic!("the other app locks the area") };
    let r = a.execute("file.recovery.save", &json!({})).unwrap();
    let mine = a.recovery.own_area().unwrap().to_string();
    assert_ne!(mine, area);
    assert_eq!(r["saved"][0]["file"], format!("{mine}/Untitled-1-1").as_str(), "{r}");
    assert!(dir.join(&mine).join("Untitled-1-1.vectorcraft").is_file());
    assert!(!dir.join(&area).join("Untitled-1-1.vectorcraft").exists());
    drop(other);
    drop(a);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A copy left in an area the session had to leave goes at once when its document is saved,
/// reverted or closed, and every one goes when the app quits with nothing unsaved.
#[cfg(unix)]
#[test]
fn copies_left_behind_go_when_their_document_is_saved_reverted_or_closed() {
    use cmd::recovery::Lock;
    let base = std::env::temp_dir().join(format!("vc-recovery-leftgo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let dir = base.join("recovery");
    let path = base.join("doc.vectorcraft").to_string_lossy().to_string();
    let left = |area: &str| std::fs::read_dir(dir.join(area)).unwrap().flatten().any(|e| e.path().extension().is_some_and(|x| x == "vectorcraft"));
    for how in ["save", "revert", "close", "quit"] {
        let (mut a, _) = saved_in(&dir);
        a.execute("document.save", &json!({"path": path})).unwrap();
        a.execute("shape.ellipse", &json!({"x": 10, "y": 10, "width": 5, "height": 5})).unwrap();
        a.execute("file.recovery.save", &json!({})).unwrap();
        let area = a.recovery.own_area().unwrap().to_string();
        std::fs::remove_file(dir.join(&area).join(".lock")).unwrap();
        let Ok(Lock::Held(other)) = FolderStore::new(&dir).lock(&area) else { panic!("the other app locks the area") };
        cmd::recovery::heartbeat(&mut a);
        assert!(left(&area), "{how}");
        match how {
            "save" => a.execute("document.save", &json!({"path": path})).map(drop).unwrap(),
            "revert" => a.execute("file.revert", &json!({})).map(drop).unwrap(),
            "close" => a.execute("file.close", &json!({})).map(drop).unwrap(),
            _ => cmd::recovery::forget_all(&mut a),
        }
        assert!(!left(&area), "{how}");
        drop(other);
        drop(a);
    }
    let _ = std::fs::remove_dir_all(&base);
}

/// The lock file was removed: the next heartbeat locks the area again, before any recovery save.
#[cfg(unix)]
#[test]
fn a_heartbeat_locks_the_area_again_when_its_lock_file_was_removed() {
    use cmd::recovery::Lock;
    let dir = std::env::temp_dir().join(format!("vc-recovery-beatlock-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (mut a, area) = saved_in(&dir);
    std::fs::remove_file(dir.join(&area).join(".lock")).unwrap();
    assert!(cmd::recovery::heartbeat(&mut a).is_empty(), "the copy is still there");
    assert_eq!(a.recovery.own_area(), Some(area.as_str()));
    assert!(matches!(FolderStore::new(&dir).lock(&area), Ok(Lock::Busy)));
    assert!(offered(&mut other_app(&dir)).is_empty());
    drop(a);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The lock file was removed and can't be made again (the area folder can't be written to now):
/// the session takes a new area and writes its copy there.
#[cfg(unix)]
#[test]
fn an_area_whose_lock_file_cannot_be_made_again_is_left_for_a_new_one() {
    use std::os::unix::fs::PermissionsExt as _;

    use cmd::recovery::Lock;
    // Permissions don't stop the superuser.
    if rustix::process::geteuid().is_root() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("vc-recovery-readonly-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (mut a, area) = saved_in(&dir);
    std::fs::remove_file(dir.join(&area).join(".lock")).unwrap();
    std::fs::set_permissions(dir.join(&area), std::fs::Permissions::from_mode(0o500)).unwrap();
    let r = a.execute("file.recovery.save", &json!({})).unwrap();
    let mine = a.recovery.own_area().unwrap().to_string();
    assert_ne!(mine, area);
    assert_eq!(r["saved"][0]["file"], format!("{mine}/Untitled-1-1").as_str(), "{r}");
    assert!(matches!(FolderStore::new(&dir).lock(&mine), Ok(Lock::Busy)));
    std::fs::set_permissions(dir.join(&area), std::fs::Permissions::from_mode(0o700)).unwrap();
    drop(a);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A copy left behind that can't be removed when its document is saved (its area can't be written
/// to now) stays the session's own: it isn't offered here, and a later heartbeat removes it.
#[cfg(unix)]
#[test]
fn a_copy_left_behind_that_cannot_be_removed_yet_stays_the_sessions_own() {
    use std::os::unix::fs::PermissionsExt as _;

    use cmd::recovery::Lock;
    // Permissions don't stop the superuser.
    if rustix::process::geteuid().is_root() {
        return;
    }
    let base = std::env::temp_dir().join(format!("vc-recovery-leftstuck-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let dir = base.join("recovery");
    let path = base.join("doc.vectorcraft").to_string_lossy().to_string();
    let (mut a, area) = saved_in(&dir);
    std::fs::remove_file(dir.join(&area).join(".lock")).unwrap();
    let Ok(Lock::Held(other)) = FolderStore::new(&dir).lock(&area) else { panic!("the other app locks the area") };
    cmd::recovery::heartbeat(&mut a);
    std::fs::set_permissions(dir.join(&area), std::fs::Permissions::from_mode(0o500)).unwrap();
    a.execute("document.save", &json!({"path": path})).unwrap();
    std::fs::set_permissions(dir.join(&area), std::fs::Permissions::from_mode(0o700)).unwrap();
    let copy = dir.join(&area).join("Untitled-1-1.vectorcraft");
    assert!(copy.is_file());
    drop(other);
    assert!(offered(&mut a).is_empty());
    cmd::recovery::heartbeat(&mut a);
    assert!(!copy.exists());
    drop(a);
    let _ = std::fs::remove_dir_all(&base);
}

/// A copy left in an area the session had to leave is still the session's own after the app that
/// locked that area is gone: the session neither offers nor discards it.
#[cfg(unix)]
#[test]
fn a_copy_left_behind_is_not_offered_by_its_own_app() {
    use cmd::recovery::Lock;
    let dir = std::env::temp_dir().join(format!("vc-recovery-leftmine-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (mut a, area) = saved_in(&dir);
    std::fs::remove_file(dir.join(&area).join(".lock")).unwrap();
    let Ok(Lock::Held(other)) = FolderStore::new(&dir).lock(&area) else { panic!("the other app locks the area") };
    cmd::recovery::heartbeat(&mut a);
    drop(other);
    assert!(offered(&mut a).is_empty());
    let r = a.execute("file.recovery.discard", &json!({})).unwrap();
    assert!(r["discarded"].as_array().unwrap().is_empty(), "{r}");
    assert!(a.execute("file.recovery.discard", &json!({"file": format!("{area}/Untitled-1-1")})).is_err());
    assert!(dir.join(&area).join("Untitled-1-1.vectorcraft").is_file());
    drop(a);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A copy left in an area the session had to leave is not another app's to offer, restore or
/// discard while the session runs, even after the app that locked that area let it go: an entry in
/// the session's new area says it keeps it. Once the copy is written again in the new area, the
/// entry goes; after a crash before that, the copy is offered.
#[cfg(unix)]
#[test]
fn a_copy_left_behind_is_not_offered_by_another_app_while_its_app_runs() {
    use cmd::recovery::Lock;
    let dir = std::env::temp_dir().join(format!("vc-recovery-leftother-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let left_behind = |area: &str| dir.join(area).join("Untitled-1-1.vectorcraft");
    let leave = |a: &mut Session, area: &str| {
        std::fs::remove_file(dir.join(area).join(".lock")).unwrap();
        let Ok(Lock::Held(other)) = FolderStore::new(&dir).lock(area) else { panic!("the other app locks the area") };
        cmd::recovery::heartbeat(a);
        drop(other);
        a.recovery.own_area().unwrap().to_string()
    };
    let (mut a, area) = saved_in(&dir);
    let mine = leave(&mut a, &area);
    assert_ne!(mine, area);
    assert!(dir.join(&mine).join(format!("keeps-{area}")).is_file());
    // Removed meanwhile: the next heartbeat writes it again.
    std::fs::remove_file(dir.join(&mine).join(format!("keeps-{area}"))).unwrap();
    cmd::recovery::heartbeat(&mut a);
    assert!(dir.join(&mine).join(format!("keeps-{area}")).is_file());
    let mut b = other_app(&dir);
    assert!(offered(&mut b).is_empty());
    assert_eq!(restored(b.execute("file.recovery.restore", &json!({})).unwrap()), 0);
    assert!(b.execute("file.recovery.discard", &json!({})).unwrap()["discarded"].as_array().unwrap().is_empty());
    assert!(b.execute("file.recovery.discard", &json!({"file": format!("{area}/Untitled-1-1")})).is_err());
    assert!(left_behind(&area).is_file());
    // Written again in the new area: the copy left behind and the entry go.
    a.execute("file.recovery.save", &json!({})).unwrap();
    assert!(!left_behind(&area).exists());
    assert!(!dir.join(&mine).join(format!("keeps-{area}")).exists());
    assert!(offered(&mut b).is_empty());
    drop(a);
    let _ = std::fs::remove_dir_all(&dir);
    // A crash before the copy is written again: the copy left behind is offered.
    let (mut a, area) = saved_in(&dir);
    leave(&mut a, &area);
    drop(a);
    assert_eq!(offered(&mut other_app(&dir)).len(), 1);
    assert!(left_behind(&area).is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A store in memory whose areas named `foreign-…` can't be locked by this account (as another
/// account's folder in a shared recovery folder).
#[derive(Default)]
struct ForeignAreas(MemoryStore);

impl RecoveryStore for ForeignAreas {
    fn list(&self) -> std::result::Result<Vec<String>, String> {
        self.0.list()
    }
    fn read(&self, name: &str) -> std::result::Result<Vec<u8>, String> {
        self.0.read(name)
    }
    fn write(&self, name: &str, bytes: &[u8]) -> std::result::Result<(), String> {
        self.0.write(name, bytes)
    }
    fn remove(&self, name: &str) -> std::result::Result<(), String> {
        self.0.remove(name)
    }
    fn location(&self) -> String {
        self.0.location()
    }
    fn lock(&self, area: &str) -> std::result::Result<cmd::recovery::Lock, String> {
        if area.starts_with("foreign-") {
            return Err(format!("{area}: belongs to another account"));
        }
        self.0.lock(area)
    }
    fn now(&self) -> Option<i64> {
        self.0.now()
    }
}

/// An entry `<area>/keeps-<left area>` in an area this account can't lock keeps nothing: a crashed
/// app's copies in the left area are still offered.
#[test]
fn a_keeps_entry_in_an_area_that_cannot_be_locked_keeps_nothing() {
    let store = Arc::new(ForeignAreas::default());
    let app = || {
        let mut s = Session::new();
        s.recovery.set_store(store.clone());
        s
    };
    let mut a = app();
    a.execute("file.new", &json!({"width": 50, "height": 50})).unwrap();
    a.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap();
    a.execute("file.recovery.save", &json!({})).unwrap();
    let area = a.recovery.own_area().unwrap().to_string();
    drop(a);
    store.write(&format!("foreign-1/keeps-{area}"), b"").unwrap();
    let mut b = app();
    assert_eq!(offered(&mut b).len(), 1);
    assert_eq!(restored(b.execute("file.recovery.restore", &json!({})).unwrap()), 1);
}

/// An entry `<area>/keeps-<left area>` in a running app's area keeps a crashed app's copies in the
/// left area until that app is gone too.
#[test]
fn a_keeps_entry_in_a_running_apps_area_keeps_the_copies() {
    let store = Arc::new(MemoryStore::default());
    let mut a = session_in(&store);
    a.execute("file.recovery.save", &json!({})).unwrap();
    let area = a.recovery.own_area().unwrap().to_string();
    drop(a);
    let mut c = session_in(&store);
    c.execute("file.recovery.save", &json!({})).unwrap();
    let keeper = c.recovery.own_area().unwrap().to_string();
    store.write(&format!("{keeper}/keeps-{area}"), b"").unwrap();
    let mut b = relaunch(&store);
    assert!(offered(&mut b).is_empty());
    assert_eq!(restored(b.execute("file.recovery.restore", &json!({})).unwrap()), 0);
    drop(c);
    assert_eq!(offered(&mut b).len(), 2);
}

/// An entry `<area>/keeps-<left area>` in a crashed app's area keeps nothing while restore or
/// discard holds that area: both crashed apps' copies are restored, and discarded.
#[test]
fn a_keeps_entry_in_a_crashed_apps_area_held_by_the_same_command_keeps_nothing() {
    let crashed = |store: &Arc<MemoryStore>| {
        let mut s = session_in(store);
        s.execute("file.recovery.save", &json!({})).unwrap();
        s.recovery.own_area().unwrap().to_string()
    };
    for cmd in ["file.recovery.restore", "file.recovery.discard"] {
        let store = Arc::new(MemoryStore::default());
        let a = crashed(&store);
        let c = crashed(&store);
        assert_ne!(a, c);
        // The area that sorts first, which the command holds first, keeps the other one's copies.
        let (first, second) = if a < c { (a, c) } else { (c, a) };
        store.write(&format!("{first}/keeps-{second}"), b"").unwrap();
        let mut b = relaunch(&store);
        assert_eq!(offered(&mut b).len(), 2);
        let r = b.execute(cmd, &json!({})).unwrap();
        let done = if cmd == "file.recovery.restore" { restored(r) } else { r["discarded"].as_array().unwrap().len() };
        assert_eq!(done, 2, "{cmd}");
    }
}

/// An entry `<area>/keeps-<left area>` in a folder whose lock file can't be opened keeps nothing.
#[cfg(unix)]
#[test]
fn a_keeps_entry_in_a_folder_whose_lock_cannot_be_checked_keeps_nothing() {
    let dir = std::env::temp_dir().join(format!("vc-recovery-keepsunlocked-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (a, area) = saved_in(&dir);
    drop(a);
    std::fs::create_dir_all(dir.join("keeper").join(".lock")).unwrap();
    std::fs::write(dir.join("keeper").join(format!("keeps-{area}")), b"").unwrap();
    assert_eq!(offered(&mut other_app(&dir)).len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A complex document (Data Recovery off for those) gets no new copy after the session left its
/// area: it keeps the copy it left there, and the heartbeat asks for it only once.
#[cfg(unix)]
#[test]
fn a_complex_documents_copy_left_behind_is_asked_for_once() {
    use cmd::recovery::Lock;
    let dir = std::env::temp_dir().join(format!("vc-recovery-leftcomplex-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut a = other_app(&dir);
    let mut d = vectorcraft_doc::Document::new(1000.0, 1000.0);
    let layer = d.layers[0].id;
    for i in 0..cmd::recovery::COMPLEX_OBJECTS {
        let id = d.alloc_id();
        let r = vectorcraft_geom::Rect::new((i % 200) as f64, (i / 200) as f64, (i % 200) as f64 + 1.0, (i / 200) as f64 + 1.0);
        let n = vectorcraft_doc::Node::path(id, vectorcraft_geom::shapes::rectangle(r), vectorcraft_doc::Appearance::default_art());
        d.insert(Some(layer), i, n).unwrap();
    }
    a.add_document(d, None);
    a.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap();
    let r = a.execute("file.recovery.save", &json!({})).unwrap();
    let copy = r["saved"][0]["file"].as_str().unwrap().to_string();
    let area = a.recovery.own_area().unwrap().to_string();
    a.prefs.recovery_off_for_complex = true;
    std::fs::remove_file(dir.join(&area).join(".lock")).unwrap();
    let Ok(Lock::Held(other)) = FolderStore::new(&dir).lock(&area) else { panic!("the other app locks the area") };
    let uid = a.doc().unwrap().uid;
    assert_eq!(cmd::recovery::heartbeat(&mut a), [uid]);
    let r = a.execute("file.recovery.save", &json!({})).unwrap();
    assert!(r["saved"].as_array().unwrap().is_empty(), "{r}");
    assert!(cmd::recovery::heartbeat(&mut a).is_empty(), "asked for once");
    assert!(dir.join(format!("{copy}.vectorcraft")).is_file(), "its copy stays");
    drop(other);
    drop(a);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The preference named another folder while a copy was written in the background: the copy goes
/// from the folder it was written to, and the document keeps its copy in the new folder.
#[test]
fn a_copy_finished_after_the_folder_changed_goes_from_its_own_folder() {
    let base = std::env::temp_dir().join(format!("vc-recovery-refolder-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let (old, new) = (base.join("old"), base.join("new"));
    let (mut a, area) = saved_in(&old);
    a.execute("shape.ellipse", &json!({"x": 10, "y": 10, "width": 5, "height": 5})).unwrap();
    let store: Arc<dyn RecoveryStore> = Arc::new(FolderStore::new(&old));
    let (jobs, _) = cmd::recovery::jobs(&mut a, &store).unwrap();
    a.prefs.recovery_folder = new.to_string_lossy().to_string();
    a.execute("file.recovery.save", &json!({})).unwrap();
    let mine = a.doc().unwrap().recovery.as_ref().unwrap().file.clone();
    for j in jobs {
        j.write().unwrap();
        j.finish(&mut a);
    }
    assert!(!old.join(&area).join("Untitled-1-1.vectorcraft").exists());
    assert_eq!(a.doc().unwrap().recovery.as_ref().unwrap().file, mine);
    assert!(new.join(format!("{mine}.vectorcraft")).is_file());
    drop(a);
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

/// Image keys sharing their bytes share one entry: it is written once when saved, and read and
/// written once when restored.
#[test]
fn images_sharing_an_entry_are_written_once() {
    let store = Arc::new(Counting::default());
    let mut s = Session::new();
    s.recovery.set_store(store.clone());
    s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    place_png(&mut s, "red.png", crate::tests_links::RED);
    let doc = Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    let blob = doc.images.values().next().unwrap().clone();
    for i in 0..50 {
        doc.images.insert(format!("copy-{i}"), blob.clone());
    }
    s.execute("file.recovery.save", &json!({})).unwrap();
    assert_eq!(store.images_written().len(), 1);
    let images = images_of(&s);
    drop(s);
    let mut s = Session::new();
    s.recovery.set_store(store.clone());
    assert_eq!(restored(s.execute("file.recovery.restore", &json!({})).unwrap()), 1);
    assert_eq!(images_of(&s), images);
    assert_eq!(store.images_written().len(), 1, "restored into this session's area once");
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

/// Under automation read roots, `file.recovery.save`, `file.recovery.list` and
/// `file.recovery.restore` name a document's file and the recovery folder only inside them.
#[test]
fn recovery_replies_leave_out_paths_outside_the_read_root() {
    use crate::file_access::{AutomationRoots, confine};
    use serde_json::Value;
    let base = crate::tests_links::Folder::new("recovery-read-root");
    let (inside, outside) = (base.0.join("in"), base.0.join("out"));
    std::fs::create_dir_all(&inside).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let store: Arc<dyn RecoveryStore> = Arc::new(FolderStore::new(outside.join("recovery")));
    let location = outside.join("recovery").to_string_lossy().to_string();
    let kept = inside.join("kept.vectorcraft").to_string_lossy().to_string();
    let hidden = outside.join("hidden.vectorcraft").to_string_lossy().to_string();
    let mut s = Session::new();
    s.recovery.set_store(store.clone());
    for path in [&kept, &hidden] {
        s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
        s.execute("file.saveAs", &json!({"path": path})).unwrap();
        s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 20})).unwrap();
    }
    let roots = AutomationRoots::new(&[&inside], &[]).unwrap();
    let saved = confine(roots.as_ref(), || s.execute("file.recovery.save", &json!({}))).unwrap();
    assert_eq!(saved["location"], Value::Null, "{saved}");
    s.execute("shape.ellipse", &json!({"x": 80, "y": 20, "width": 20, "height": 20})).unwrap();
    assert_eq!(s.execute("file.recovery.save", &json!({})).unwrap()["location"], json!(location), "the person sees the folder");

    let sorted = |v: &Value, key: &str| -> Vec<String> {
        let mut out: Vec<String> = v[key].as_array().unwrap().iter().map(|c| c["path"].to_string()).collect();
        out.sort();
        out
    };
    let both = |first: Value| {
        let mut out = vec![first.to_string(), json!(kept).to_string()];
        out.sort();
        out
    };
    assert_eq!(sorted(&s.execute("file.recovery.list", &json!({})).unwrap(), "copies"), both(json!(hidden)));
    let listed = confine(roots.as_ref(), || s.execute("file.recovery.list", &json!({}))).unwrap();
    assert_eq!((sorted(&listed, "copies"), &listed["location"]), (both(Value::Null), &Value::Null), "{listed}");
    let outside_text = json!(outside.to_string_lossy()).to_string();
    assert!(!listed.to_string().contains(outside_text.trim_matches('"')), "no path outside the root: {listed}");
    // A relative recovery folder is checked as the file system reads it; a store that is no folder
    // keeps its name.
    let mut relative = Session::new();
    relative.recovery.set_store(Arc::new(FolderStore::new("vc-recovery-relative-folder")));
    let l = confine(roots.as_ref(), || relative.execute("file.recovery.list", &json!({}))).unwrap();
    assert_eq!(l["location"], Value::Null, "{l}");
    let (mut memory, _) = session();
    let l = confine(roots.as_ref(), || memory.execute("file.recovery.list", &json!({}))).unwrap();
    assert_eq!(l["location"], "memory", "{l}");

    // After a crash, the copies restored.
    drop(s);
    let mut s = Session::new();
    s.recovery.set_store(store.clone());
    let r = confine(roots.as_ref(), || s.execute("file.recovery.restore", &json!({}))).unwrap();
    assert_eq!(sorted(&r, "restored"), both(Value::Null), "{r}");
    let mut opened: Vec<Option<String>> = s.documents().iter().map(|d| d.path.clone()).collect();
    opened.sort();
    let mut files = vec![Some(hidden), Some(kept)];
    files.sort();
    assert_eq!(opened, files, "the documents keep their files");
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
