use crate::Session;
use serde_json::json;
use std::time::Duration;

fn fixture() -> (Session, u64) {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let id = s.execute("shape.rectangle", &json!({"x":0,"y":0,"width":20,"height":30})).unwrap()["id"].as_u64().unwrap();
    (s, id)
}

#[test]
fn changes_unsaved_properties_undo_redo_and_gaps() {
    let (mut s, id) = fixture();
    let cursor = s.changes.latest();
    s.execute("object.setProps", &json!({"ids":[id],"name":"unsaved","opacity":45})).unwrap();
    assert!(s.active().unwrap().path.is_none());
    assert_eq!(s.changes.latest(), cursor + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(s.changes.latest(), cursor + 3);
    let poll = s.changes.poll(cursor, Duration::ZERO);
    assert_eq!(poll["reset"], true);
    assert_eq!(poll["cursor"], cursor + 3);
    assert_eq!(s.changes.poll(cursor + 99, Duration::ZERO)["reset"], true);
}

#[test]
fn changes_noops_queries_and_failed_batch_do_not_emit() {
    let (mut s, id) = fixture();
    s.execute("object.setProps", &json!({"ids":[id],"name":"same","opacity":100})).unwrap();
    let cursor = s.changes.latest();
    s.execute("document.inspect", &json!({})).unwrap();
    s.execute("object.setProps", &json!({"ids":[id],"name":"same","opacity":100})).unwrap();
    assert_eq!(s.changes.latest(), cursor);
    assert!(
        s.execute(
            "command.batch",
            &json!({"commands":[
                {"command":"object.setProps","params":{"ids":[id],"name":"rolled back"}},
                {"command":"object.setProps","params":{"ids":[99999999],"name":"bad"}}
            ]})
        )
        .is_err()
    );
    assert_eq!(s.changes.latest(), cursor);
    assert_eq!(s.changes.poll(cursor, Duration::ZERO)["events"], json!([]));
}

#[test]
fn changes_wait_does_not_require_the_editor_thread() {
    let (mut s, id) = fixture();
    let feed = s.changes.clone();
    let cursor = feed.latest();
    let worker = std::thread::spawn(move || feed.poll(cursor, Duration::from_secs(2)));
    s.execute("object.setProps", &json!({"ids":[id],"name":"wake"})).unwrap();
    assert_eq!(worker.join().unwrap()["cursor"], cursor + 1);
    assert_eq!(s.changes.request(&json!({"after":"bad","timeoutMs":0}))["ok"], false);
}

#[test]
fn changes_closed_document_invalidates_and_revision_guard_fails() {
    let (mut s, _) = fixture();
    let state = s.active().unwrap();
    let (document, revision) = (state.uid, state.revision);
    s.check_property_revision(document, revision).unwrap();
    assert!(s.check_property_revision(document + 1, revision).is_err());
    assert!(s.check_property_revision(document, revision + 1).is_err());
    let cursor = s.changes.latest();
    s.execute("file.close", &json!({})).unwrap();
    assert_eq!(s.changes.latest(), cursor + 1);
    assert!(s.check_property_revision(document, revision).is_err());
}
