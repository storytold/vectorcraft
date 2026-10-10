//! Linked files changing while their document is open (`link_watch`): `links.updateChanged`,
//! and the background look following Preferences → File Handling → Update Links.

use serde_json::{Value, json};

use super::*;
use crate::tests_links::{BLUE, Folder, RED, centre_colour, place, png, session};

/// Write `bytes` to `path` so its modification time moves even on coarse file systems.
fn save(path: &str, bytes: &[u8]) {
    std::thread::sleep(std::time::Duration::from_millis(30));
    std::fs::write(path, bytes).unwrap();
}

fn colour(s: &Session) -> [u8; 3] {
    centre_colour(&s.doc().unwrap().doc)
}

/// One background look, waited for.
fn look(s: &mut Session) -> Value {
    s.start_link_scan();
    let t0 = std::time::Instant::now();
    loop {
        if let Some(r) = s.poll_link_scan() {
            return r.unwrap();
        }
        assert!(t0.elapsed().as_secs() < 10, "the look never finished");
        std::thread::yield_now();
    }
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
