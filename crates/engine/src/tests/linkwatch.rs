//! Linked files changing while their document is open (`link_watch`): `links.updateChanged`,
//! and the background look following Preferences → File Handling → Update Links.

use std::sync::atomic::Ordering;

use serde_json::{Value, json};

use crate::tests_links::{BLUE, Folder, RED, centre_colour, place, png, session};
use crate::*;

/// Write `bytes` to `path`, dated 2 s after the file it replaces so its modification time moves
/// even on coarse file systems.
fn save(path: &str, bytes: &[u8]) {
    let stamp = std::fs::metadata(path).unwrap().modified().unwrap();
    std::fs::write(path, bytes).unwrap();
    let later = std::fs::FileTimes::new().set_modified(stamp + std::time::Duration::from_secs(2));
    std::fs::OpenOptions::new().write(true).open(path).unwrap().set_times(later).unwrap();
}

fn colour(s: &Session) -> [u8; 3] {
    centre_colour(&s.doc().unwrap().doc)
}

/// One background look, waited for.
fn look(s: &mut Session) -> Value {
    s.start_link_scan();
    s.wait_link_scan();
    s.poll_link_scan().expect("a look ran").unwrap()
}

#[test]
fn update_changed_reads_changed_files_again_once() {
    let dir = Folder::new("watch-update");
    let path = dir.file("art.png");
    std::fs::write(&path, png(40, 40, RED)).unwrap();
    let mut s = session();
    let id = place(&mut s, &path);
    // First look: remembered, nothing to read.
    assert_eq!(s.execute("links.updateChanged", &json!({})).unwrap()["updated"], json!([]));
    save(&path, &png(40, 40, BLUE));
    assert_eq!(s.execute("links.updateChanged", &json!({})).unwrap()["updated"], json!([id.0]));
    assert_eq!(colour(&s), BLUE);
    // Undoing it is respected: the link stays modified until the file changes again.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(colour(&s), RED);
    assert_eq!(s.execute("links.updateChanged", &json!({})).unwrap()["updated"], json!([]));
    assert_eq!(s.execute("links.check", &json!({})).unwrap()["modified"], 1);
}

#[test]
fn a_background_look_follows_update_links() {
    for (mode, key) in [("automatically", "updated"), ("askWhenModified", "ask"), ("manually", "modified")] {
        let dir = Folder::new(&format!("watch-{mode}"));
        let path = dir.file("art.png");
        std::fs::write(&path, png(40, 40, RED)).unwrap();
        let mut s = session();
        s.prefs.update_links = mode.into();
        let id = place(&mut s, &path);
        assert_eq!(look(&mut s), json!({"updated": [], "ask": [], "modified": []}), "{mode}");
        save(&path, &png(40, 40, BLUE));
        let r = look(&mut s);
        assert_eq!(r[key], json!([id.0]), "{mode}: {r}");
        assert_eq!(colour(&s), if mode == "automatically" { BLUE } else { RED }, "{mode}");
        // Acted on once: the next look finds nothing new.
        assert_eq!(look(&mut s), json!({"updated": [], "ask": [], "modified": []}), "{mode}");
        assert!(s.link_scan.is_none());
    }
}

#[test]
fn a_file_still_being_written_is_tried_again() {
    let dir = Folder::new("watch-partial");
    let path = dir.file("art.png");
    std::fs::write(&path, png(40, 40, RED)).unwrap();
    let mut s = session();
    s.prefs.update_links = "automatically".into();
    let id = place(&mut s, &path);
    look(&mut s);
    // Another app has only written the start of the file so far.
    let full = png(40, 40, BLUE);
    save(&path, &full[..16]);
    assert_eq!(look(&mut s)["updated"], json!([]));
    assert_eq!(colour(&s), RED, "the last good pixels stay");
    save(&path, &full);
    assert_eq!(look(&mut s)["updated"], json!([id.0]));
    assert_eq!(colour(&s), BLUE);
}

#[test]
fn a_look_whose_worker_ended_without_stamps_does_not_stop_later_looks() {
    let dir = Folder::new("watch-unwound");
    let path = dir.file("art.png");
    std::fs::write(&path, png(40, 40, RED)).unwrap();
    let mut s = session();
    s.prefs.update_links = "automatically".into();
    let id = place(&mut s, &path);
    look(&mut s);
    let scan = link_watch::LinkScan::default();
    s.link_scan = Some(scan.clone());
    // Still running: the look stays in flight.
    assert!(s.poll_link_scan().is_none());
    assert!(s.link_scan.is_some(), "a look still running stays");
    let worker = std::thread::spawn(move || {
        let _ended = link_watch::Ended(scan);
        panic!("the look's worker unwinds");
    });
    assert!(worker.join().is_err());
    assert!(s.poll_link_scan().is_none(), "there are no stamps to apply");
    assert!(s.link_scan.is_none(), "the look ended");
    save(&path, &png(40, 40, BLUE));
    assert_eq!(look(&mut s)["updated"], json!([id.0]));
    assert_eq!(colour(&s), BLUE);
}

#[test]
fn a_look_that_stores_its_stamps_between_the_polls_reads_keeps_them() {
    let dir = Folder::new("watch-stored");
    let path = dir.file("art.png");
    std::fs::write(&path, png(40, 40, RED)).unwrap();
    let mut s = session();
    s.prefs.update_links = "automatically".into();
    let id = place(&mut s, &path);
    look(&mut s);
    let uid = s.doc().unwrap().uid;
    let stamps: Vec<_> = link_watch::linked_files(&s.doc().unwrap().doc).into_iter().map(|(p, ids)| (p, ids, Some((1, 1)))).collect();
    let scan = link_watch::LinkScan::default();
    s.link_scan = Some(scan.clone());
    // The worker takes its turn after the slot is read and before the flag is, if the slot is free.
    let ended = link_watch::ended_without_stamps(&scan, |flag| {
        if let Ok(mut slot) = scan.0.try_lock() {
            *slot = Some((uid, stamps.clone()));
            flag.store(true, Ordering::SeqCst);
        }
        flag.load(Ordering::SeqCst)
    });
    assert!(!ended, "the worker waits for the poll, so the look has not ended without stamps");
    // The worker's turn comes once the poll has let go of the slot.
    let worker = std::thread::spawn({
        let scan = scan.clone();
        move || {
            let _ended = link_watch::Ended(scan.clone());
            *scan.0.lock().unwrap() = Some((uid, stamps));
        }
    });
    worker.join().unwrap();
    assert_eq!(s.poll_link_scan().expect("the stamps are applied").unwrap()["updated"], json!([id.0]));
    assert!(s.link_scan.is_none());
}

#[test]
fn a_change_during_a_drag_waits_for_the_drag_to_end() {
    let dir = Folder::new("watch-drag");
    let path = dir.file("art.png");
    std::fs::write(&path, png(40, 40, RED)).unwrap();
    let mut s = session();
    s.prefs.update_links = "automatically".into();
    let id = place(&mut s, &path);
    look(&mut s);
    s.begin_interaction("Move").unwrap();
    save(&path, &png(40, 40, BLUE));
    // The preview would throw the update away, so it is not applied (nor its stamp kept).
    assert_eq!(look(&mut s), json!({"updated": [], "ask": [], "modified": []}));
    s.cancel_interaction().unwrap();
    assert_eq!(look(&mut s)["updated"], json!([id.0]));
    assert_eq!(colour(&s), BLUE);
}
