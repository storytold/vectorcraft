//! The journal replays to the same document whenever it runs: values a command takes from the
//! clock (File ▸ New's created date, a save's modified date) are recorded in its entry, and left out
//! of recorded actions.

use serde_json::json;

use super::*;

fn created(s: &Session) -> Option<i64> {
    s.doc().unwrap().doc.metadata.created
}

#[test]
fn file_new_records_its_created_date_in_the_journal() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    let (id, p) = s.journal.last().unwrap().clone();
    assert_eq!(id, "file.new");
    assert!(created(&s).is_some(), "a new document is dated now");
    assert_eq!(p, json!({"width": 200, "height": 100, "created": created(&s)}));
}

#[test]
fn file_new_takes_a_given_created_date() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"created": 1_000_000_000})).unwrap();
    assert_eq!(created(&s), Some(1_000_000_000));
    s.execute("file.new", &json!({"created": null})).unwrap();
    assert_eq!(created(&s), None);
    // Given values are journaled as given.
    assert_eq!(s.journal.last().unwrap().1, json!({"created": null}));
    for bad in [json!("2026-01-01"), json!(1.5), json!(true)] {
        assert!(s.execute("file.new", &json!({"created": bad})).is_err(), "{bad}");
    }
    assert_eq!(s.documents().len(), 2, "a bad date makes no document");
}

#[test]
fn replay_reproduces_the_created_date_of_an_earlier_run() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    // As if the original run happened long ago: a replay now must still land on its date.
    s.journal[0].1["created"] = json!(1_234_567_890);
    let mut r = Session::new();
    for (id, p) in &s.journal {
        r.execute(id, p).unwrap();
    }
    assert_eq!(created(&r), Some(1_234_567_890));
    assert_eq!(r.doc().unwrap().doc.node_count(), s.doc().unwrap().doc.node_count());
}

#[test]
fn recorded_actions_leave_out_replay_only_values() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    s.execute("object.scale", &json!({"sx": 200})).unwrap();
    let steps = s.journal_for_action(0);
    assert_eq!(steps.len(), s.journal.len());
    // Playing the action later dates its new document then…
    assert_eq!(steps[0], ("file.new".to_string(), json!({"width": 300, "height": 200})));
    // …while values resolved from the preferences stay recorded.
    let scale = steps.iter().find(|(id, _)| id == "object.scale").unwrap();
    assert!(scale.1.get("strokes").is_some(), "{scale:?}");
    assert_eq!(s.journal_for_action(1), s.journal[1..].to_vec());
}

/// The artboard the app passes to Paste in Place, in Front and in Back, to Align to Artboard, to
/// All on Active Artboard and to a batch (its active one) is journaled and left out of recorded
/// actions, so a played action uses the artboard active when it plays.
#[test]
fn recorded_actions_leave_out_the_artboard_the_app_passes() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 2, "created": null})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    let start = s.journal.len();
    let steps = [
        ("edit.pasteInPlace", json!({"artboard": 1})),
        ("edit.pasteInFront", json!({"artboard": 1})),
        ("edit.pasteInBack", json!({"artboard": 1})),
        ("select.allOnArtboard", json!({"artboard": 1})),
        ("object.align", json!({"horizontal": "left", "to": "artboard", "artboard": 1})),
        ("command.batch", json!({"artboard": 1, "commands": [{"command": "select.allOnArtboard", "params": {}}]})),
    ];
    for (id, p) in &steps {
        s.execute(id, p).unwrap();
    }
    assert!(s.journal[start..].iter().all(|(_, p)| p["artboard"] == json!(1)), "the journal keeps it: {:?}", &s.journal[start..]);
    let action = s.journal_for_action(start);
    assert_eq!(action.len(), steps.len());
    for ((id, mut p), (step, recorded)) in steps.into_iter().zip(action) {
        p.as_object_mut().unwrap().remove("artboard");
        assert_eq!((step.as_str(), recorded), (id, p));
    }
}

/// A batch passes its `artboard` (the app's active one) to each step that acts on the active
/// artboard and names none, when the step runs: once a step deletes the last artboard, the new
/// last one, and in a new document with fewer artboards, its last one. A step that names an
/// artboard keeps it.
#[test]
fn a_batch_passes_its_artboard_to_each_step_when_it_runs() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
    let boards: Vec<_> = s.doc().unwrap().doc.artboards.iter().map(|a| a.rect).collect();
    let ids: Vec<u64> = boards
        .iter()
        .map(|b| {
            s.execute("shape.rectangle", &json!({"x": b.x0 + 10.0, "y": b.y0 + 10.0, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap()
        })
        .collect();
    let picked = |s: &Session| s.doc().unwrap().selection.objects.iter().map(|id| id.0).collect::<Vec<_>>();
    let step = |command: &str, params: Value| json!({"command": command, "params": params});
    let all_on_artboard = step("select.allOnArtboard", json!({}));
    s.execute("command.batch", &json!({"artboard": 2, "commands": [all_on_artboard]})).unwrap();
    assert_eq!(picked(&s), [ids[2]], "the batch's artboard 3");
    s.execute("command.batch", &json!({"artboard": 2, "commands": [step("select.allOnArtboard", json!({"artboard": 0}))]})).unwrap();
    assert_eq!(picked(&s), [ids[0]], "an artboard the step names is kept");
    s.execute("command.batch", &json!({"artboard": 2, "commands": [step("artboard.delete", json!({"index": 2})), all_on_artboard]})).unwrap();
    assert_eq!(picked(&s), [ids[1]], "artboard 2, the last one once artboard 3 is deleted");
    let new = step("file.new", json!({"width": 100, "height": 100}));
    s.execute("command.batch", &json!({"artboard": 2, "commands": [new, all_on_artboard]})).unwrap();
    assert_eq!(s.doc().unwrap().doc.artboards.len(), 1, "in the new document");
}

#[test]
fn a_save_records_its_date_and_replays_it() {
    let path = vectorcraft_testkit::temp_dir("journal-save").join("a.vectorcraft").to_string_lossy().into_owned();
    let mut s = Session::new();
    s.execute("file.new", &json!({"created": null})).unwrap();
    s.execute("document.save", &json!({"path": path})).unwrap();
    let m = &s.doc().unwrap().doc.metadata;
    assert!(m.modified.is_some() && m.created == m.modified, "a save dates a document without one");
    let (id, p) = s.journal.last().unwrap().clone();
    assert_eq!((id.as_str(), &p["modified"]), ("document.save", &json!(m.modified)));
    // A given date is used as is; null leaves the dates alone.
    s.execute("document.save", &json!({"path": path, "modified": 2_000_000_000})).unwrap();
    assert_eq!(s.doc().unwrap().doc.metadata.modified, Some(2_000_000_000));
    s.execute("document.save", &json!({"path": path, "modified": null})).unwrap();
    assert_eq!(s.doc().unwrap().doc.metadata.modified, Some(2_000_000_000));
    assert!(s.execute("document.save", &json!({"path": path, "modified": "today"})).is_err());
    // Save As stamps the same way; Save a Copy leaves the document's dates (and records none).
    let dir = vectorcraft_testkit::temp_dir("journal-save");
    s.execute("file.saveAs", &json!({"path": dir.join("b.vectorcraft").to_string_lossy(), "modified": 2_000_000_100})).unwrap();
    assert_eq!(s.journal.last().unwrap().1["modified"], json!(2_000_000_100));
    s.execute("file.saveCopy", &json!({"path": dir.join("c.vectorcraft").to_string_lossy()})).unwrap();
    assert!(s.journal.last().unwrap().1.get("modified").is_none());
    assert_eq!(s.doc().unwrap().doc.metadata.modified, Some(2_000_000_100));
    // Replayed later, the saves stamp the recorded dates; an action stamps the time it plays.
    let mut r = Session::new();
    for (id, p) in &s.journal {
        r.execute(id, p).unwrap();
    }
    assert_eq!(r.doc().unwrap().doc.metadata, s.doc().unwrap().doc.metadata);
    assert!(s.journal_for_action(0).iter().all(|(_, p)| p.get("modified").is_none() && p.get("created").is_none()));
}

/// The document `s` ends on after replaying the journal of `s` in a new session.
fn replayed(s: &Session) -> Session {
    let mut r = Session::new();
    for (id, p) in &s.journal {
        r.execute(id, p).unwrap();
    }
    r
}

#[test]
fn a_save_in_a_batch_records_its_date_and_replays_it() {
    let path = vectorcraft_testkit::temp_dir("journal-batch").join("a.vectorcraft").to_string_lossy().into_owned();
    let mut s = Session::new();
    s.execute("file.new", &json!({"created": 1_000_000_000})).unwrap();
    let rect = json!({"x": 10, "y": 10, "width": 50, "height": 40});
    let steps = json!([{"command": "shape.rectangle", "params": rect}, {"command": "document.save", "params": {"path": path}}]);
    s.execute("command.batch", &json!({ "commands": steps })).unwrap();
    let modified = s.doc().unwrap().doc.metadata.modified;
    assert!(modified.is_some(), "the save in the batch dates the document");
    // The step's entry records the date it used; steps that take nothing from elsewhere stay as given.
    let (id, p) = s.journal.last().unwrap().clone();
    assert_eq!(id, "command.batch");
    assert_eq!(p["commands"][0], steps[0]);
    assert_eq!(p["commands"][1]["params"], json!({"path": path, "modified": modified}));
    // As if the original run happened at another time: the replay lands on the recorded date.
    s.journal.last_mut().unwrap().1["commands"][1]["params"]["modified"] = json!(1_234_567_890);
    let r = replayed(&s);
    assert_eq!(r.doc().unwrap().doc.metadata.modified, Some(1_234_567_890));
    assert_eq!(r.doc().unwrap().doc.metadata.created, Some(1_000_000_000));
    assert_eq!(r.doc().unwrap().doc.node_count(), s.doc().unwrap().doc.node_count());
    // An action leaves the step's date out, so playing it later dates the document then.
    let action = s.journal_for_action(0);
    assert_eq!(action[1].1["commands"][1]["params"], json!({ "path": path }));
    assert_eq!(action[1].1["commands"][0], steps[0]);
}

#[test]
fn a_batch_step_records_what_it_took_from_the_preferences() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"created": null})).unwrap();
    s.prefs.scale_strokes = true;
    let steps = json!([
        {"command": "shape.rectangle", "params": {"x": 10, "y": 10, "width": 50, "height": 40}},
        {"command": "stroke.set", "params": {"weight": 2}},
        {"command": "object.scale", "params": {"sx": 300}},
    ]);
    s.execute("command.batch", &json!({ "commands": steps })).unwrap();
    let p = &s.journal.last().unwrap().1;
    assert_eq!(p["commands"][2]["params"], json!({"sx": 300, "strokes": true, "corners": false}));
    // Replayed with other preferences, the scale still scales the stroke.
    let r = replayed(&s);
    assert!(!r.prefs.scale_strokes);
    let doc = |s: &Session| serde_json::to_value(&*s.doc().unwrap().doc).unwrap();
    assert_eq!(doc(&r), doc(&s));
}

/// Every open document of `s`, as JSON.
fn documents(s: &Session) -> Vec<serde_json::Value> {
    s.documents().iter().map(|d| serde_json::to_value(&*d.doc).unwrap()).collect()
}

fn rect_step(x: i32) -> serde_json::Value {
    json!({"command": "shape.rectangle", "params": {"x": x, "y": 10, "width": 20, "height": 20}})
}

#[test]
fn a_batch_that_opens_a_new_document_is_journaled_and_replays() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"created": null})).unwrap();
    let steps = json!([rect_step(10), {"command": "file.new", "params": {"width": 300, "height": 200}}, rect_step(50), rect_step(90)]);
    s.execute("command.batch", &json!({"label": "Two documents", "commands": steps})).unwrap();
    // The steps before the new document stay in the first one and the rest go in the new one: one
    // undo step in each, nothing left in progress.
    assert_eq!(s.documents().len(), 2);
    assert_eq!(s.active_index(), Some(1));
    for (st, objects) in s.documents().iter().zip([1, 2]) {
        assert_eq!(st.doc.node_count(), 1 + objects);
        assert_eq!(st.history.undo.iter().map(|h| h.label.as_str()).collect::<Vec<_>>(), ["Two documents"]);
        assert!(st.interaction.is_none());
    }
    // The batch is journaled, with the new document's date, and replays to the same documents.
    let (id, p) = s.journal.last().unwrap().clone();
    assert_eq!(id, "command.batch");
    assert!(p["commands"][1]["params"]["created"].is_i64(), "{p}");
    let r = replayed(&s);
    assert_eq!(documents(&r), documents(&s));
    assert_eq!(r.active_index(), s.active_index());
    // Undo in the new document takes its part of the batch back.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc.node_count(), 1);
}

#[test]
fn a_failed_batch_closes_the_documents_it_opened_and_restores_the_ones_it_closed() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"created": null})).unwrap();
    s.execute("file.new", &json!({"created": null})).unwrap();
    s.execute("shape.ellipse", &json!({"x": 0, "y": 0, "width": 30, "height": 30})).unwrap();
    let (before, active, journal, next) = (documents(&s), s.active_index(), s.journal.len(), s.peek_untitled());
    let steps = json!([
        rect_step(10),
        {"command": "file.close", "params": {}},
        rect_step(30),
        {"command": "file.new", "params": {}},
        rect_step(50),
        {"command": "no.such.command", "params": {}},
    ]);
    let e = s.execute("command.batch", &json!({ "commands": steps })).unwrap_err();
    assert!(e.to_string().contains("batch step 5"), "{e}");
    assert_eq!(documents(&s), before);
    assert_eq!(s.active_index(), active);
    assert_eq!(s.journal.len(), journal, "a failed batch isn't journaled");
    assert_eq!(s.peek_untitled(), next, "the next new document keeps its name");
    assert!(s.documents().iter().all(|d| d.interaction.is_none()));
    assert_eq!(s.documents()[1].history.undo.len(), 1, "only the ellipse drawn before the batch");
}

#[test]
fn a_failed_batch_brings_back_a_document_it_reverted() {
    let path = vectorcraft_testkit::temp_dir("journal-batch").join("revert.vectorcraft").to_string_lossy().into_owned();
    let mut s = Session::new();
    s.execute("file.new", &json!({"created": null})).unwrap();
    s.execute("document.save", &json!({"path": path, "modified": null})).unwrap();
    s.execute("shape.ellipse", &json!({"x": 0, "y": 0, "width": 30, "height": 30})).unwrap();
    let (before, revision) = (documents(&s), s.doc().unwrap().revision);
    let steps = json!([rect_step(10), {"command": "file.revert", "params": {}}, rect_step(30), {"command": "no.such.command"}]);
    assert!(s.execute("command.batch", &json!({ "commands": steps })).is_err());
    assert_eq!(documents(&s), before, "the unsaved ellipse is back");
    let st = s.doc().unwrap();
    assert_eq!(st.history.undo.len(), 1);
    assert!(st.is_dirty() && st.interaction.is_none());
    assert!(st.revision > revision, "the restored document redraws");
}

#[test]
fn a_failed_batch_gives_back_the_highlighted_rows_and_the_current_layer() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"created": null})).unwrap();
    let layer = s.doc().unwrap().doc.layers[0].id;
    // A step that highlights the layer's row, then one that fails: the highlight goes back too.
    let steps = json!([{"command": "layer.setCurrent", "params": {"id": layer.0}}, {"command": "no.such.command"}]);
    assert!(s.execute("command.batch", &json!({ "commands": steps })).is_err());
    assert!(s.doc().unwrap().layer_rows.is_empty(), "no row stays highlighted");
    assert_eq!(s.doc().unwrap().active_layer, Some(layer));
    // So Collect in New Layer acts on the drawn rectangle, as it does when the journal is replayed
    // (a failed batch isn't journaled).
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 1, "height": 1})).unwrap();
    s.execute("layer.collectInNew", &json!({})).unwrap();
    let r = replayed(&s);
    assert_eq!(documents(&r), documents(&s));
}

/// The edits made in an undo group (a scrubbed numeric field, #400) are one undo step, journaled
/// one by one; a cancelled group undoes them and leaves the journal as it was.
#[test]
fn an_undo_group_is_one_undo_step() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 100, "height": 50})).unwrap();
    let width = |s: &Session| s.transform_box(&s.doc().unwrap().selection.objects).unwrap().rect.width();
    let undo_len = |s: &Session| s.doc().unwrap().history.undo.len();
    let (undo, journal) = (undo_len(&s), s.journal.len());
    s.begin_undo_group();
    for w in [101, 102, 103] {
        s.execute("object.setBounds", &json!({"width": w})).unwrap();
    }
    // A live drag committed in the group (the Transparency panel's opacity field) joins it too.
    s.begin_interaction("Opacity").unwrap();
    s.preview("transparency.set", &json!({"opacity": 50})).unwrap();
    s.commit_interaction().unwrap();
    s.end_undo_group(false);
    assert!((width(&s) - 103.0).abs() < 1e-9);
    assert_eq!(undo_len(&s), undo + 1, "one undo step");
    assert_eq!(s.journal.len(), journal + 4, "each edit journaled");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!((width(&s) - 100.0).abs() < 1e-9, "undo takes the whole drag back");
    assert_eq!(undo_len(&s), undo);
    s.execute("edit.redo", &json!({})).unwrap();
    assert!((width(&s) - 103.0).abs() < 1e-9);

    // Escape: back where it started, with no trace in the history or the journal.
    let (undo, journal) = (undo_len(&s), s.journal.len());
    s.begin_undo_group();
    s.execute("object.setBounds", &json!({"width": 150})).unwrap();
    s.execute("object.setBounds", &json!({"width": 160})).unwrap();
    s.end_undo_group(true);
    assert!((width(&s) - 103.0).abs() < 1e-9);
    assert_eq!((undo_len(&s), s.journal.len()), (undo, journal));
    // Closed: the next edits are steps of their own again.
    s.execute("object.setBounds", &json!({"width": 110})).unwrap();
    s.execute("object.setBounds", &json!({"width": 120})).unwrap();
    assert_eq!(undo_len(&s), undo + 2);
    // A group with no edits records nothing, and cancelling it undoes nothing.
    s.begin_undo_group();
    s.end_undo_group(true);
    assert!((width(&s) - 120.0).abs() < 1e-9);
    assert_eq!(undo_len(&s), undo + 2);
}

/// A move drag is named for what its last preview did: Alt pressed or released during the drag
/// turns it into a copy or back (the Selection and Direct Selection tools).
#[test]
fn a_move_drag_is_named_for_whether_it_copied() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    let last = |s: &Session| s.doc().unwrap().history.undo.last().map(|e| e.label.clone());
    for (begin, copy, label) in [("Move", true, "Copy"), ("Copy", false, "Move"), ("Move", false, "Move")] {
        s.begin_interaction(begin).unwrap();
        s.preview("object.transform", &json!({"matrix": [1, 0, 0, 1, 5, 0], "copy": copy})).unwrap();
        s.commit_interaction().unwrap();
        assert_eq!(last(&s).as_deref(), Some(label), "begun as {begin}, copy: {copy}");
    }
}
