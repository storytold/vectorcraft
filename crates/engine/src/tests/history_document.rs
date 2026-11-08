//! History panel › New Document from State (`edit.newDocumentFromState`).

use std::sync::Arc;

use serde_json::{Value, json};

use super::{rect, session};
use crate::tests_links::{Folder, RED, place, png, write};
use crate::*;

const CMD: &str = "edit.newDocumentFromState";

fn objects(s: &Session) -> usize {
    s.doc().unwrap().doc.layers[0].children().unwrap().len()
}

fn x(s: &Session, id: NodeId) -> f64 {
    s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().x0
}

fn steps(s: &Session) -> (usize, usize) {
    let h = &s.doc().unwrap().history;
    (h.undo.len(), h.redo.len())
}

/// Three rectangles, the last one undone: rows 0 to 3 hold 0 to 3 of them, the current one row 2.
fn three_rectangles_one_undone() -> Session {
    let mut s = session();
    for x in [10.0, 200.0, 400.0] {
        rect(&mut s, x, 10.0, 100.0, 50.0);
    }
    s.execute("edit.undo", &json!({})).unwrap();
    s
}

/// Each row gives the art of its state, the steps to redo included. An omitted or null index gives
/// the current state. Each new document is added and made active.
#[test]
fn a_new_document_holds_the_art_of_the_state() {
    let params = [
        (json!({"index": 0}), 0),
        (json!({"index": 1}), 1),
        (json!({"index": 2}), 2),
        (json!({"index": 3}), 3),
        (json!({}), 2),
        (json!({"index": null}), 2),
    ];
    for (p, art) in params {
        let mut s = three_rectangles_one_undone();
        let r = s.execute(CMD, &p).unwrap();
        assert_eq!((r["index"].as_u64(), s.active_index()), (Some(1), Some(1)), "{p}");
        assert_eq!(objects(&s), art, "{p}");
        assert_eq!(s.documents().len(), 2);
    }
}

/// The source's document, steps, selection and saved state stay as they are, for a modified source
/// and for one at its saved state.
#[test]
fn the_source_stays_as_it_is() {
    let mut s = three_rectangles_one_undone();
    let source = s.doc().unwrap().doc.clone();
    let selection = s.doc().unwrap().selection.clone();
    assert!(s.doc().unwrap().is_dirty());
    s.execute(CMD, &json!({"index": 0})).unwrap();
    let st = &s.documents()[0];
    assert!(Arc::ptr_eq(&st.doc, &source));
    assert_eq!((st.history.undo.len(), st.history.redo.len()), (2, 1));
    assert_eq!(st.selection, selection);
    assert!(st.is_dirty());

    let mut s = session();
    rect(&mut s, 10.0, 10.0, 100.0, 50.0);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(!s.doc().unwrap().is_dirty());
    s.execute(CMD, &json!({"index": 1})).unwrap();
    assert!(!s.documents()[0].is_dirty(), "the source stays at its saved state");
    assert!(s.doc().unwrap().is_dirty(), "the new document is unsaved");
}

/// An index that names no state, or is not a whole number, is an error and changes nothing: no
/// document, no step and no Untitled number is used up.
#[test]
fn a_bad_index_is_an_error_and_changes_nothing() {
    let mut s = three_rectangles_one_undone();
    let source = s.doc().unwrap().doc.clone();
    let untitled = s.peek_untitled();
    let bad: [Value; 8] = [json!(4), json!(99), json!(-1), json!(1.5), json!("1"), json!(true), json!([1]), json!(u64::MAX)];
    for index in bad {
        let e = s.execute(CMD, &json!({ "index": index })).unwrap_err();
        assert!(matches!(e, EngineError::BadParams { .. }), "{index}: {e}");
        assert!(e.to_string().contains(&format!("no history state {index}")), "{e}");
        assert_eq!(s.documents().len(), 1, "{index}");
        assert_eq!(steps(&s), (2, 1), "{index}");
        assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &source), "{index}");
        assert_eq!(s.peek_untitled(), untitled, "{index}");
    }
    let e = s.execute(CMD, &json!({"created": "today"})).unwrap_err();
    assert!(matches!(e, EngineError::BadParams { .. }), "{e}");
    assert_eq!(s.documents().len(), 1);
}

/// With no document the command is disabled.
#[test]
fn it_needs_a_document() {
    let mut s = Session::new();
    assert!(matches!(s.execute(CMD, &json!({})), Err(EngineError::Disabled(..))));
}

/// An edit in either document leaves the other as it is, and so does taking an artboard id.
#[test]
fn the_two_documents_are_independent() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 10.0, 100.0, 50.0);
    rect(&mut s, 200.0, 10.0, 100.0, 50.0);
    s.execute(CMD, &json!({"index": 1})).unwrap();
    s.execute("object.move", &json!({"ids": [a.0], "dx": 30, "dy": 0})).unwrap();
    rect(&mut s, 400.0, 10.0, 100.0, 50.0);
    assert_eq!((x(&s, a), objects(&s)), (40.0, 2));
    s.execute("document.activate", &json!({"index": 0})).unwrap();
    assert_eq!((x(&s, a), objects(&s)), (10.0, 2), "the source");
    s.execute("object.move", &json!({"ids": [a.0], "dx": 5, "dy": 0})).unwrap();
    assert_eq!(x(&s, a), 15.0);
    let source_board = s.doc().unwrap().doc.next_artboard_id();
    s.execute("document.activate", &json!({"index": 1})).unwrap();
    assert_eq!((x(&s, a), objects(&s)), (40.0, 2), "the new document");
    assert!(s.doc().unwrap().doc.retired_artboard_id.get() < source_board, "an artboard id the source took");
}

/// The new document starts with no steps, no selection, no path and the next untitled name, and is
/// unsaved. Undo and Redo then work in it alone, and undoing back to the start leaves it unsaved.
#[test]
fn undo_in_the_new_document() {
    let mut s = three_rectangles_one_undone();
    s.execute("select.all", &json!({})).unwrap();
    let r = s.execute(CMD, &json!({"index": 1})).unwrap();
    assert_eq!(r["title"], "Untitled-2");
    let st = s.doc().unwrap();
    assert_eq!(st.title(), "Untitled-2");
    assert_eq!(st.path, None);
    assert!(st.selection.objects.is_empty());
    assert!(st.is_dirty());
    let start = st.doc.clone();
    assert_eq!(steps(&s), (0, 0));
    assert!(matches!(s.execute("edit.undo", &json!({})), Err(EngineError::Disabled(..))), "the source's steps are not its own");
    rect(&mut s, 600.0, 10.0, 100.0, 50.0);
    assert_eq!(objects(&s), 2);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(objects(&s), 1);
    assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &start));
    assert!(s.doc().unwrap().is_dirty(), "still unsaved at the state it started from");
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(objects(&s), 2);
    assert_eq!(s.documents()[0].history.undo.len(), 2, "the source's steps");
}

/// A drag in progress is first kept as a step and its change goes into the new document. An index
/// names the state as listed before that, and a bad one leaves the drag going on.
#[test]
fn a_drag_in_progress_is_kept_first() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 10.0, 100.0, 50.0);
    s.begin_interaction("Move").unwrap();
    s.preview("object.move", &json!({"dx": 30, "dy": 0})).unwrap();
    assert!(s.execute(CMD, &json!({"index": 2})).is_err());
    assert!(s.in_interaction(), "a bad index ends nothing");
    assert_eq!(steps(&s), (1, 0));
    s.execute(CMD, &json!({"index": 0})).unwrap();
    assert_eq!(objects(&s), 0, "the row listed as 0");
    assert_eq!(x_of_source(&s, a), 40.0);
    assert_eq!(s.documents()[0].history.undo.len(), 2, "the drag is a step in the source");

    let mut s = session();
    let a = rect(&mut s, 10.0, 10.0, 100.0, 50.0);
    s.begin_interaction("Move").unwrap();
    s.preview("object.move", &json!({"dx": 30, "dy": 0})).unwrap();
    s.execute(CMD, &json!({})).unwrap();
    assert_eq!(x(&s, a), 40.0, "the current state holds the drag");
    assert_eq!(x_of_source(&s, a), 40.0);
    assert!(s.documents()[0].interaction.is_none());
}

fn x_of_source(s: &Session, id: NodeId) -> f64 {
    s.documents()[0].doc.node(id).unwrap().geometric_bounds().unwrap().x0
}

/// A state saved while editing a symbol gives a document out of that mode and without the
/// temporary editing layer. The source stays in the mode.
#[test]
fn a_new_document_leaves_the_editing_modes() {
    let mut s = session();
    rect(&mut s, 10.0, 10.0, 100.0, 50.0);
    s.execute("symbol.new", &json!({"name": "Box"})).unwrap();
    let layers = s.doc().unwrap().doc.layers.len();
    s.execute("symbol.edit", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.symbol_edit.is_some());
    assert_eq!(s.doc().unwrap().doc.layers.len(), layers + 1);
    s.execute(CMD, &json!({})).unwrap();
    let st = s.doc().unwrap();
    assert!(st.doc.symbol_edit.is_none());
    assert_eq!(st.doc.layers.len(), layers);
    assert!(s.documents()[0].doc.symbol_edit.is_some());
}

/// A state saved while editing a pattern gives a document out of that mode, without the editing
/// layer, and with the art drawn in the tile stored in the pattern, as Done stores it. The source
/// stays in the mode.
#[test]
fn a_new_document_finishes_pattern_editing() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    s.execute("select.set", &json!({"ids": [r.0]})).unwrap();
    let layers = s.doc().unwrap().doc.layers.len();
    s.execute("object.pattern.make", &json!({"name": "P"})).unwrap();
    rect(&mut s, 2.0, 2.0, 3.0, 3.0);
    assert_eq!(s.doc().unwrap().doc.layers.len(), layers + 1);
    s.execute(CMD, &json!({})).unwrap();
    assert_eq!(s.active_index(), Some(1), "the new document");
    let st = s.doc().unwrap();
    assert!(st.doc.pattern_edit.is_none());
    assert_eq!(st.doc.layers.len(), layers);
    assert_eq!(st.doc.pattern("P").map(|p| p.art.len()), Some(2));
    assert_eq!(st.isolation, None);
    assert!(s.documents()[0].doc.pattern_edit.is_some());
}

/// A linked image still finds its file from the new document, which has no folder of its own, and
/// its pixels come along.
#[test]
fn links_resolve_in_the_new_document() {
    let dir = Folder::new("history-document-links");
    let pic = dir.file("photo.png");
    write(&pic, &png(60, 30, RED));
    let mut s = session();
    let id = place(&mut s, &pic);
    let list = |s: &mut Session| s.execute("links.list", &json!({})).unwrap();
    assert_eq!(list(&mut s)["links"][0]["status"], "ok");
    s.execute(CMD, &json!({})).unwrap();
    assert_eq!(s.active_index(), Some(1), "the new document");
    let links = list(&mut s);
    assert_eq!((&links["links"][0]["status"], &links["links"][0]["path"]), (&json!("ok"), &json!(pic)), "{links}");
    let st = s.doc().unwrap();
    assert!(st.doc.node(id).is_some());
    assert_eq!(st.doc.images.len(), s.documents()[0].doc.images.len());
}

/// A linked file the source finds in its own folder, away from the link's path, is found from the
/// new document too: its link takes the path where the source finds the file.
#[test]
fn a_file_the_source_finds_in_its_folder_is_found_from_the_new_document() {
    let a = Folder::new("history-document-moved-a");
    let b = Folder::new("history-document-moved-b");
    let pic = a.file("photo.png");
    write(&pic, &png(60, 30, RED));
    let mut s = session();
    place(&mut s, &pic);
    s.execute("document.save", &json!({"path": b.file("poster.vectorcraft")})).unwrap();
    let moved = b.file("photo.png");
    std::fs::rename(&pic, &moved).unwrap();
    let check = |s: &mut Session| s.execute("links.check", &json!({})).unwrap();
    assert_eq!(check(&mut s)["links"][0]["status"], "ok", "the source finds it by name in its folder");
    s.execute(CMD, &json!({})).unwrap();
    assert_eq!(s.active_index(), Some(1), "the new document");
    let c = check(&mut s);
    assert_eq!((&c["links"][0]["status"], &c["links"][0]["path"]), (&json!("ok"), &json!(moved)), "{c}");
}

/// A linked file missing from the moved source keeps the place its relative path names from the
/// source's folder: the new document, saved beside the source, writes the same relative path.
#[test]
fn a_missing_file_keeps_its_relative_path_in_the_new_document() {
    let a = Folder::new("history-document-missing-a");
    let b = Folder::new("history-document-missing-b");
    let away = Folder::new("history-document-missing-away");
    write(&a.file("art/photo.png"), &png(60, 30, RED));
    let mut s = session();
    place(&mut s, &a.file("art/photo.png"));
    s.execute("document.save", &json!({"path": a.file("poster.vectorcraft")})).unwrap();
    std::fs::remove_dir_all(&b.0).unwrap();
    std::fs::rename(&a.0, &b.0).unwrap();
    std::fs::rename(b.0.join("art"), away.0.join("art")).unwrap();
    let r = s.execute("document.open", &json!({"path": b.file("poster.vectorcraft")})).unwrap();
    assert_eq!(r["missingLinks"][0]["name"], "photo.png", "{r}");
    s.execute(CMD, &json!({})).unwrap();
    let copy = b.file("copy.vectorcraft");
    s.execute("document.save", &json!({"path": copy})).unwrap();
    let saved: Value = serde_json::from_slice(&std::fs::read(&copy).unwrap()).unwrap();
    let link = &saved["document"]["layers"][0]["kind"]["children"][0]["kind"]["link"];
    assert_eq!(link["relative"].as_str(), Some("art/photo.png"), "{link}");
}

/// A linked file that comes back beside the document the source was read from, after the source
/// was saved in another folder, is found from the new document.
#[test]
fn a_file_back_beside_the_source_it_was_read_from_is_found_from_the_new_document() {
    let a = Folder::new("history-document-back-a");
    let b = Folder::new("history-document-back-b");
    let away = Folder::new("history-document-back-away");
    write(&a.file("art/photo.png"), &png(60, 30, RED));
    let mut s = session();
    place(&mut s, &a.file("art/photo.png"));
    s.execute("document.save", &json!({"path": a.file("poster.vectorcraft")})).unwrap();
    std::fs::remove_dir_all(&b.0).unwrap();
    std::fs::rename(&a.0, &b.0).unwrap();
    std::fs::rename(b.0.join("art"), away.0.join("art")).unwrap();
    s.execute("document.open", &json!({"path": b.file("poster.vectorcraft")})).unwrap();
    std::fs::create_dir_all(b.0.join("saved")).unwrap();
    s.execute("document.save", &json!({"path": b.file("saved/poster.vectorcraft")})).unwrap();
    std::fs::rename(away.0.join("art"), b.0.join("art")).unwrap();
    let source = s.active_index().unwrap();
    s.execute(CMD, &json!({})).unwrap();
    let c = s.execute("links.check", &json!({})).unwrap();
    let moved = b.file("art/photo.png");
    assert_eq!((&c["links"][0]["status"], &c["links"][0]["path"]), (&json!("ok"), &json!(moved)), "{c}");
    // A file beside the folder the source was last saved in is found too.
    std::fs::create_dir_all(b.0.join("saved/art")).unwrap();
    std::fs::rename(&moved, b.file("saved/art/photo.png")).unwrap();
    s.execute("document.activate", &json!({"index": source})).unwrap();
    s.execute(CMD, &json!({})).unwrap();
    let c = s.execute("links.check", &json!({})).unwrap();
    let saved = b.file("saved/art/photo.png");
    assert_eq!((&c["links"][0]["status"], &c["links"][0]["path"]), (&json!("ok"), &json!(saved)), "{c}");
    // Missing again when the new document is saved, the file keeps the place it was found.
    std::fs::rename(&saved, away.file("photo.png")).unwrap();
    std::fs::create_dir_all(b.0.join("new")).unwrap();
    let copy = b.file("new/copy.vectorcraft");
    s.execute("document.save", &json!({"path": copy})).unwrap();
    let file: Value = serde_json::from_slice(&std::fs::read(&copy).unwrap()).unwrap();
    let link = &file["document"]["layers"][0]["kind"]["children"][0]["kind"]["link"];
    assert_eq!(link["relative"].as_str(), Some("../saved/art/photo.png"), "{link}");
}

/// The same holds for a linked image only a symbol shows.
#[test]
fn a_file_a_symbol_shows_is_found_from_the_new_document() {
    let a = Folder::new("history-document-symbol-a");
    let b = Folder::new("history-document-symbol-b");
    let pic = a.file("photo.png");
    write(&pic, &png(60, 30, RED));
    let mut s = session();
    place(&mut s, &pic);
    s.execute("symbol.new", &json!({"name": "Photo"})).unwrap();
    s.execute("document.save", &json!({"path": b.file("poster.vectorcraft")})).unwrap();
    let moved = b.file("photo.png");
    std::fs::rename(&pic, &moved).unwrap();
    s.execute(CMD, &json!({})).unwrap();
    assert_eq!(s.active_index(), Some(1), "the new document");
    let d = &s.doc().unwrap().doc;
    assert!(!d.symbols.is_empty());
    let mut paths = vec![];
    d.visit_images(|_, im| paths.extend(im.link.as_ref().map(|l| l.path.clone())));
    assert!(!paths.is_empty() && paths.iter().all(|p| *p == moved), "{paths:?}");
}

/// Files are looked for only within the roots of the caller that opened the source, or of a
/// confined caller asking now: a file outside them is not linked from the new document.
#[test]
fn a_file_outside_the_callers_roots_is_not_looked_for() {
    let a = Folder::new("history-document-confined-a");
    let b = Folder::new("history-document-confined-b");
    let other = Folder::new("history-document-confined-other");
    let pic = a.file("photo.png");
    write(&pic, &png(60, 30, RED));
    let mut s = session();
    place(&mut s, &pic);
    s.execute("document.save", &json!({"path": b.file("poster.vectorcraft")})).unwrap();
    let moved = b.file("photo.png");
    std::fs::rename(&pic, &moved).unwrap();
    let narrow = crate::file_access::AutomationRoots::new(&[&other.0], &[]).unwrap().unwrap();
    let link = |s: &Session| {
        let mut paths = vec![];
        s.doc().unwrap().doc.visit_images(|_, im| paths.extend(im.link.as_ref().map(|l| l.path.clone())));
        paths
    };
    crate::file_access::confine(Some(&narrow), || s.execute(CMD, &json!({}))).unwrap();
    assert_eq!(s.active_index(), Some(1), "the new document");
    assert!(link(&s).iter().all(|p| *p != moved), "a confined caller: {:?}", link(&s));
    s.execute("document.activate", &json!({"index": 0})).unwrap();
    s.active_mut().unwrap().opened_by = Some(narrow);
    s.execute(CMD, &json!({})).unwrap();
    assert_eq!(s.active_index(), Some(2), "the second new document");
    let paths = link(&s);
    assert!(!paths.is_empty() && paths.iter().all(|p| *p != moved), "opened by a confined caller: {paths:?}");
}

/// What reading the source's file left out still guards that file from the new document, and the
/// new document is read within what the caller that opened the source may read.
#[test]
fn the_new_document_keeps_what_guards_the_source() {
    let dir = Folder::new("history-document-roots");
    let roots = crate::file_access::AutomationRoots::new(&[&dir.0], &[]).unwrap().unwrap();
    let mut s = session();
    let st = s.active_mut().unwrap();
    st.imported_from = Some("art.ai".into());
    st.import_losses = vec!["hidden text".into()];
    st.opened_by = Some(roots.clone());
    s.execute(CMD, &json!({})).unwrap();
    assert_eq!(s.active_index(), Some(1), "the new document");
    let st = s.doc().unwrap();
    assert_eq!((st.imported_from.as_deref(), st.import_losses.as_slice()), (Some("art.ai"), &["hidden text".to_string()][..]));
    assert!(st.opened_by.as_ref().is_some_and(|r| Arc::ptr_eq(r, &roots)));
}

/// The created date is the new document's own and joins the journal entry, so a replay gives the
/// same date. An action leaves it out. The new document has no modified date until it is saved.
#[test]
fn the_created_date_is_the_new_documents_own() {
    let mut s = session();
    Arc::make_mut(&mut s.active_mut().unwrap().doc).metadata.modified = Some(500);
    let source = s.doc().unwrap().doc.metadata.created;
    s.execute(CMD, &json!({"created": 1000})).unwrap();
    assert_eq!(s.doc().unwrap().doc.metadata.created, Some(1000));
    assert_eq!(s.doc().unwrap().doc.metadata.modified, None, "the new document has not been saved");
    assert_eq!(s.documents()[0].doc.metadata.modified, Some(500));
    assert_eq!(s.documents()[0].doc.metadata.created, source);
    s.execute("document.activate", &json!({"index": 0})).unwrap();
    s.execute(CMD, &json!({"created": null})).unwrap();
    assert_eq!(s.doc().unwrap().doc.metadata.created, None);
    s.execute("document.activate", &json!({"index": 0})).unwrap();
    s.execute(CMD, &json!({})).unwrap();
    let noted = s.journal.last().unwrap().1["created"].as_i64();
    assert!(noted.is_some() && noted == s.doc().unwrap().doc.metadata.created, "{noted:?}");
    let mut action = s.journal.last().unwrap().1.clone();
    crate::command_sets::strip_replay_only(CMD, &mut action);
    assert_eq!(action, json!({}), "an action leaves the date out");
}
