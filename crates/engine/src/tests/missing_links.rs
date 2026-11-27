//! Canvas marker state and command API, independent of artwork and link update preferences.

use serde_json::json;

use crate::tests_links::{Folder, RED, place, png, session, write};
use crate::*;

fn marked(s: &mut Session) -> Vec<u64> {
    s.execute("links.marked", &json!({})).unwrap()["images"].as_array().unwrap().iter().map(|r| r["id"].as_u64().unwrap()).collect()
}

#[test]
fn missing_link_toggle_is_transient_per_document_and_validated() {
    let mut s = session();
    let before = s.doc().unwrap().doc.clone();
    let undo = s.doc().unwrap().history.undo.len();
    assert!(marked(&mut s).is_empty());
    assert_eq!(s.execute("view.showMissingLinks", &json!({"on": true})).unwrap(), json!({"on": true}));
    for p in [json!({"on": "false"}), json!({"on": 1}), json!({"on": []})] {
        assert!(s.execute("view.showMissingLinks", &p).is_err());
        assert!(s.doc().unwrap().show_missing_links);
    }
    assert!(std::sync::Arc::ptr_eq(&before, &s.doc().unwrap().doc));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    assert!(!s.doc().unwrap().is_dirty());
    s.execute("file.new", &json!({})).unwrap();
    assert!(!s.doc().unwrap().show_missing_links);
    s.execute("document.activate", &json!({"index": 0})).unwrap();
    assert!(s.doc().unwrap().show_missing_links);
    assert_eq!(s.execute("view.showMissingLinks", &json!({})).unwrap(), json!({"on": false}));
    assert!(s.doc().unwrap().missing_link_markers.is_none());
}

#[test]
fn missing_link_markers_follow_files_relink_update_and_embed() {
    let dir = Folder::new("marker-lifecycle");
    let path = dir.file("art.png");
    let replacement = dir.file("replacement.png");
    let bytes = png(40, 40, RED);
    write(&path, &bytes);
    write(&replacement, &bytes);
    let mut s = session();
    let id = place(&mut s, &path);
    s.execute("view.showMissingLinks", &json!({"on": true})).unwrap();
    assert!(marked(&mut s).is_empty());
    std::fs::remove_file(&path).unwrap();
    assert_eq!(marked(&mut s), vec![id.0]);
    let doc = s.doc().unwrap().doc.clone();
    let selection = s.doc().unwrap().selection.clone();
    let undo = s.doc().unwrap().history.undo.len();
    let saved = vectorcraft_format::save_file(&doc);
    let pixels = vectorcraft_render::Renderer::new().render_region(&doc, vectorcraft_geom::Rect::new(0.0, 0.0, 400.0, 300.0), 1.0, true).pixels;
    s.execute("view.showMissingLinks", &json!({"on": false})).unwrap();
    assert!(marked(&mut s).is_empty());
    s.execute("view.showMissingLinks", &json!({"on": true})).unwrap();
    assert_eq!(marked(&mut s), vec![id.0]);
    let st = s.doc().unwrap();
    assert_eq!(st.selection, selection);
    assert_eq!(st.history.undo.len(), undo);
    assert_eq!(vectorcraft_format::save_file(&st.doc), saved);
    assert_eq!(
        vectorcraft_render::Renderer::new().render_region(&st.doc, vectorcraft_geom::Rect::new(0.0, 0.0, 400.0, 300.0), 1.0, true).pixels,
        pixels
    );
    s.execute("links.relink", &json!({"ids": [id.0], "path": replacement})).unwrap();
    assert!(s.doc().unwrap().marked_missing_links().is_empty());
    assert!(marked(&mut s).is_empty());
    std::fs::remove_file(&replacement).unwrap();
    assert_eq!(marked(&mut s), vec![id.0]);
    write(&replacement, &bytes);
    s.execute("links.update", &json!({"ids": [id.0]})).unwrap();
    assert!(s.doc().unwrap().marked_missing_links().is_empty());
    assert!(marked(&mut s).is_empty());
    s.execute("links.embed", &json!({"ids": [id.0]})).unwrap();
    std::fs::remove_file(&replacement).unwrap();
    assert!(marked(&mut s).is_empty());
}

#[test]
fn missing_link_worker_refreshes_without_the_panel_and_ignores_old_art() {
    for mode in ["automatically", "askWhenModified", "manually"] {
        let dir = Folder::new(&format!("marker-watch-{mode}"));
        let path = dir.file("art.png");
        let bytes = png(20, 20, RED);
        write(&path, &bytes);
        let mut s = session();
        s.prefs.update_links = mode.into();
        let id = place(&mut s, &path);
        s.execute("view.showMissingLinks", &json!({"on": true})).unwrap();
        let undo = s.doc().unwrap().history.undo.len();
        std::fs::remove_file(&path).unwrap();
        s.start_link_scan();
        s.wait_link_scan();
        s.poll_link_scan().unwrap().unwrap();
        assert_eq!(s.doc().unwrap().missing_link_markers.as_ref().unwrap().1, vec![id]);
        write(&path, &bytes);
        s.start_link_scan();
        s.wait_link_scan();
        s.poll_link_scan().unwrap().unwrap();
        assert!(s.doc().unwrap().missing_link_markers.as_ref().unwrap().1.is_empty());
        assert_eq!(s.doc().unwrap().history.undo.len(), undo);
        std::fs::remove_file(&path).unwrap();
        s.start_link_scan();
        s.wait_link_scan();
        let seen = s.doc().unwrap().missing_link_markers.as_ref().unwrap().0.clone();
        s.execute("shape.rectangle", &json!({"x": 1, "y": 1, "width": 3, "height": 4})).unwrap();
        s.poll_link_scan().unwrap().unwrap();
        assert!(std::sync::Arc::ptr_eq(&seen, &s.doc().unwrap().missing_link_markers.as_ref().unwrap().0));
        assert!(s.doc().unwrap().marked_missing_links().is_empty());
        s.start_link_scan();
        s.wait_link_scan();
        // Switching the toggle off while a look is pending must not resurrect its markers.
        s.execute("view.showMissingLinks", &json!({"on": false})).unwrap();
        s.poll_link_scan().unwrap().unwrap();
        assert!(s.doc().unwrap().missing_link_markers.is_none());
    }
}

#[test]
fn turning_missing_links_on_returns_before_the_file_probe_and_scans_on_the_link_watcher() {
    use crate::cmd::links::probe_gate::{self, Gate};
    use std::time::Duration;
    let dir = Folder::new("marker-toggle-nonblocking");
    let path = dir.file("art.png");
    write(&path, &png(20, 20, RED));
    let mut s = session();
    let id = place(&mut s, &path);
    std::fs::remove_file(&path).unwrap();
    let stored = crate::tests_links::image(&s, id).link.unwrap().path;
    let gate = Gate::close(&stored);
    // The toggle returns while the probe of the (slow) linked file is held closed.
    assert_eq!(s.execute("view.showMissingLinks", &json!({"on": true})).unwrap(), json!({"on": true}));
    assert!(!gate.finished(), "the toggle waited for the file probe");
    assert_eq!(gate.entered(Duration::ZERO), None, "the toggle probed the file on the calling thread");
    assert!(s.doc().unwrap().missing_link_markers.is_none());
    assert!(s.doc().unwrap().marked_missing_links().is_empty());
    // The desktop app starts the watcher's look at once, and the probe runs there.
    assert!(s.missing_link_scan_due());
    s.start_link_scan();
    assert!(!s.missing_link_scan_due());
    assert_eq!(gate.entered(Duration::from_secs(10)).as_deref(), Some("link-watch"));
    assert!(s.poll_link_scan().is_none(), "the look ended before its probe was released");
    gate.open();
    s.wait_link_scan();
    s.poll_link_scan().unwrap().unwrap();
    probe_gate::remove(&stored);
    assert_eq!(s.doc().unwrap().marked_missing_links(), vec![id]);
    assert!(!s.missing_link_scan_due());
    // Turning it on again while on keeps the markers; off clears them.
    s.execute("view.showMissingLinks", &json!({"on": true})).unwrap();
    assert_eq!(s.doc().unwrap().marked_missing_links(), vec![id]);
    s.execute("view.showMissingLinks", &json!({"on": false})).unwrap();
    assert!(!s.missing_link_scan_due());
}

#[test]
fn missing_link_scan_for_a_document_without_links_ends_without_a_worker() {
    let mut s = session();
    s.execute("view.showMissingLinks", &json!({"on": true})).unwrap();
    assert!(s.missing_link_scan_due());
    s.start_link_scan();
    assert!(s.link_scan.is_none());
    assert!(!s.missing_link_scan_due());
}

#[test]
fn missing_link_hidden_when_the_overlay_turns_on_is_marked_once_shown() {
    let dir = Folder::new("marker-hidden-then-shown");
    let path = dir.file("art.png");
    write(&path, &png(20, 20, RED));
    // Found by the desktop's link watch worker, and in place as the web app does.
    for in_place in [false, true] {
        let mut s = session();
        let id = place(&mut s, &path);
        std::fs::remove_file(&path).unwrap();
        s.execute("object.hide", &json!({"ids": [id.0]})).unwrap();
        s.execute("view.showMissingLinks", &json!({"on": true})).unwrap();
        let mark = |s: &mut Session| {
            if in_place {
                s.mark_missing_links_in_place();
            } else {
                s.start_link_scan();
                s.wait_link_scan();
                s.poll_link_scan().unwrap().unwrap();
            }
        };
        mark(&mut s);
        assert!(s.doc().unwrap().marked_missing_links().is_empty(), "hidden art is not marked");
        s.execute("object.showAll", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().marked_missing_links(), vec![id], "shown art is marked without another scan");
        if in_place {
            // Artwork restored while the overlay is on is marked again.
            s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
            s.execute("edit.clear", &json!({})).unwrap();
            assert!(s.doc().unwrap().doc.node(id).is_none());
            s.execute("edit.undo", &json!({})).unwrap();
            assert!(s.mark_missing_links_in_place());
            assert_eq!(s.doc().unwrap().marked_missing_links(), vec![id]);
            assert!(!s.mark_missing_links_in_place(), "unchanged art is not looked at again");
        }
        write(&path, &png(20, 20, RED));
    }
}

#[test]
fn missing_link_markers_include_placed_documents_and_exclude_hidden_art() {
    let dir = Folder::new("marker-placed");
    let path = dir.file("placed.vectorcraft");
    write(&path, &vectorcraft_format::save_file(&vectorcraft_doc::Document::new(40.0, 20.0)));
    let mut s = session();
    let id = place(&mut s, &path);
    assert!(matches!(s.doc().unwrap().doc.node(id).unwrap().kind, vectorcraft_doc::NodeKind::PlacedDocument(_)));
    std::fs::remove_file(&path).unwrap();
    s.execute("view.showMissingLinks", &json!({"on": true})).unwrap();
    assert_eq!(marked(&mut s), vec![id.0]);
    s.execute("object.move", &json!({"ids": [id.0], "dx": 15, "dy": 10})).unwrap();
    assert_eq!(s.doc().unwrap().marked_missing_links(), vec![id]);
    s.execute("object.hide", &json!({"ids": [id.0]})).unwrap();
    assert!(s.doc().unwrap().marked_missing_links().is_empty());
    assert!(marked(&mut s).is_empty());
}
