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
    s.journal.get_mut(0).unwrap().1["created"] = json!(1_234_567_890);
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
    assert_eq!(s.journal_for_action(1), s.journal.since(1).to_vec());
}

/// The artboard the app passes to Paste in Place, in Front and in Back, to Align to Artboard, to
/// Distribute Objects and Distribute Spacing, to All on Active Artboard and to a batch (its active
/// one) is journaled and left out of recorded actions, so a played action uses the artboard active
/// when it plays.
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
        ("object.distribute", json!({"horizontal": "left", "to": "artboard", "artboard": 1})),
        ("object.distributeSpacing", json!({"axis": "horizontal", "to": "artboard", "artboard": 1})),
        ("command.batch", json!({"artboard": 1, "commands": [{"command": "select.allOnArtboard", "params": {}}]})),
    ];
    for (id, p) in &steps {
        s.execute(id, p).unwrap();
    }
    assert!(s.journal.since(start).iter().all(|(_, p)| p["artboard"] == json!(1)), "the journal keeps it: {:?}", s.journal.since(start));
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

/// A batch passes its `artboard` to each Fit to Artwork Bounds and Fit to Selected Art step that
/// names no `index`, as their `index` (past the document's last artboard, the last one), and a
/// step's own `index` is kept. A recorded action leaves out the `index` or `indices` the app
/// passes them, so the action resizes the artboard active when it plays.
#[test]
fn fit_steps_take_the_batchs_artboard_and_actions_leave_out_their_index() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
    // Across the three artboards, so it is on each.
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 610, "height": 30})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    let boards = |s: &Session| s.doc().unwrap().doc.artboards.iter().map(|a| a.rect).collect::<Vec<_>>();
    let (was, art) = (boards(&s), vectorcraft_geom::Rect::new(10.0, 10.0, 620.0, 40.0));
    let step = |command: &str, params: Value| json!({"command": command, "params": params});
    s.execute("command.batch", &json!({"artboard": 9, "commands": [step("artboard.fitToArt", json!({}))]})).unwrap();
    assert_eq!(boards(&s), [was[0], was[1], art], "artboard 3, the last one, for a batch's artboard past it");
    s.execute("command.batch", &json!({"artboard": 1, "commands": [step("artboard.fitToSelection", json!({}))]})).unwrap();
    assert_eq!(boards(&s), [was[0], art, art], "the batch's artboard 2");
    s.execute("command.batch", &json!({"artboard": 2, "commands": [step("artboard.fitToArt", json!({"index": 0}))]})).unwrap();
    assert_eq!(boards(&s)[0], art, "an index the step names is kept");
    let start = s.journal.len();
    s.execute("artboard.fitToArt", &json!({"index": 1})).unwrap();
    s.execute("artboard.fitToArt", &json!({"indices": [1, 2]})).unwrap();
    s.execute("artboard.fitToSelection", &json!({"index": 1})).unwrap();
    let kept: Vec<Value> = s.journal.since(start).iter().map(|(_, p)| p.clone()).collect();
    assert_eq!(kept, [json!({"index": 1}), json!({"indices": [1, 2]}), json!({"index": 1})], "the journal keeps them");
    let action = s.journal_for_action(start);
    let fit = |id: &str| (id.to_string(), json!({}));
    assert_eq!(action, [fit("artboard.fitToArt"), fit("artboard.fitToArt"), fit("artboard.fitToSelection")]);
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

/// Canceling undo groups open in two documents undoes the edits of both and drops the entries of
/// both from the journal, whichever document comes first in the list.
#[test]
fn canceling_undo_groups_in_two_documents_drops_the_edits_of_both_from_the_journal() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"created": null})).unwrap();
    s.execute("file.new", &json!({"created": null})).unwrap();
    let objects = |s: &Session| s.documents().iter().map(|d| d.doc.node_count()).collect::<Vec<_>>();
    let before = objects(&s);
    // The first document's group begins first, and the second document's group begins after it.
    let journal = s.journal.len();
    assert!(s.set_active(0));
    s.begin_undo_group();
    s.execute("shape.rectangle", &rect_step(10)["params"]).unwrap();
    assert!(s.set_active(1));
    s.begin_undo_group();
    s.execute("shape.rectangle", &rect_step(50)["params"]).unwrap();
    s.end_undo_group(true);
    assert_eq!(objects(&s), before, "both documents' edits are undone");
    assert_eq!(s.journal.len(), journal, "the journal holds neither group's edits");
    assert_eq!(documents(&replayed(&s)), documents(&s));

    // The commands do the same: neither group's begin nor its edit stays in the journal.
    let journal = s.journal.len();
    for (id, p) in [
        ("document.activate", json!({"index": 0})),
        ("edit.undoGroup.begin", json!({})),
        ("shape.rectangle", rect_step(10)["params"].clone()),
        ("document.activate", json!({"index": 1})),
        ("edit.undoGroup.begin", json!({})),
        ("shape.rectangle", rect_step(50)["params"].clone()),
        ("edit.undoGroup.end", json!({"cancel": true})),
    ] {
        s.execute(id, &p).unwrap();
    }
    assert_eq!(objects(&s), before);
    let ids = s.journal.since(journal).iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>();
    assert!(!ids.iter().any(|id| matches!(*id, "edit.undoGroup.begin" | "shape.rectangle")), "{ids:?}");
    assert_eq!(ids.last(), Some(&"edit.undoGroup.end"));
}

/// A cancel drops only the canceled groups' own entries from the journal: an edit made in another
/// document while a group was open stays in that document and in the journal.
#[test]
fn canceling_undo_groups_keeps_the_journal_entries_made_outside_them() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"created": null})).unwrap();
    s.execute("file.new", &json!({"created": null})).unwrap();
    let objects = |s: &Session| s.documents().iter().map(|d| d.doc.node_count()).collect::<Vec<_>>();
    let before = objects(&s);
    // The second document's rectangle at 30 is drawn between the two groups and belongs to neither.
    for (id, p) in [
        ("document.activate", json!({"index": 0})),
        ("edit.undoGroup.begin", json!({})),
        ("shape.rectangle", rect_step(10)["params"].clone()),
        ("document.activate", json!({"index": 1})),
        ("shape.rectangle", rect_step(30)["params"].clone()),
        ("edit.undoGroup.begin", json!({})),
        ("shape.rectangle", rect_step(50)["params"].clone()),
        ("edit.undoGroup.end", json!({"cancel": true})),
    ] {
        s.execute(id, &p).unwrap();
    }
    assert_eq!(objects(&s), vec![before[0], before[1] + 1], "only the groups' edits are undone");
    let ids = s.journal.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>();
    assert_eq!(ids.iter().filter(|id| **id == "shape.rectangle").count(), 1, "{ids:?}");
    let r = replayed(&s);
    assert_eq!(documents(&r), documents(&s));
    assert_eq!(r.active_index(), s.active_index());

    // With one group open, a rectangle drawn in the other document stays in the journal too. A live
    // drag committed in the group goes with the group, and one committed in the other document stays.
    let before = objects(&s);
    let opacity = |s: &mut Session, opacity: i32| {
        s.begin_interaction("Opacity").unwrap();
        s.preview("transparency.set", &json!({ "opacity": opacity })).unwrap();
        s.commit_interaction().unwrap();
    };
    for (id, p) in
        [("document.activate", json!({"index": 0})), ("edit.undoGroup.begin", json!({})), ("shape.rectangle", rect_step(10)["params"].clone())]
    {
        s.execute(id, &p).unwrap();
    }
    opacity(&mut s, 50);
    for (id, p) in [("document.activate", json!({"index": 1})), ("shape.rectangle", rect_step(70)["params"].clone())] {
        s.execute(id, &p).unwrap();
    }
    opacity(&mut s, 40);
    for (id, p) in [("document.activate", json!({"index": 0})), ("edit.undoGroup.end", json!({"cancel": true}))] {
        s.execute(id, &p).unwrap();
    }
    assert_eq!(objects(&s), vec![before[0], before[1] + 1]);
    let opacities = s.journal.iter().filter(|(id, _)| id == "transparency.set").map(|(_, p)| p["opacity"].clone()).collect::<Vec<_>>();
    assert_eq!(opacities, vec![json!(40)]);
    let r = replayed(&s);
    assert_eq!(documents(&r), documents(&s));
    assert_eq!(r.active_index(), s.active_index());

    // A batch that edits the group's document and then switches documents goes with the group, and
    // only its switch stays, so the rectangle drawn after it replays in the second document.
    let before = objects(&s);
    let steps = json!([rect_step(90), {"command": "document.activate", "params": {"index": 1}}]);
    for (id, p) in [
        ("document.activate", json!({"index": 0})),
        ("edit.undoGroup.begin", json!({})),
        ("command.batch", json!({ "commands": steps })),
        ("shape.rectangle", rect_step(110)["params"].clone()),
        ("edit.undoGroup.end", json!({"cancel": true})),
    ] {
        s.execute(id, &p).unwrap();
    }
    assert_eq!(objects(&s), vec![before[0], before[1] + 1]);
    assert!(!s.journal.iter().any(|(id, _)| id == "command.batch"));
    let r = replayed(&s);
    assert_eq!(documents(&r), documents(&s));
    assert_eq!(r.active_index(), s.active_index());

    // So does a batch that opens the group itself before it switches documents.
    let before = objects(&s);
    s.execute("document.activate", &json!({"index": 0})).unwrap();
    let steps = json!([{"command": "edit.undoGroup.begin"}, rect_step(130), {"command": "document.activate", "params": {"index": 1}}]);
    s.execute("command.batch", &json!({ "commands": steps })).unwrap();
    s.end_undo_group(true);
    assert_eq!(objects(&s), before);
    assert!(!s.journal.iter().any(|(id, _)| id == "command.batch"));
    let r = replayed(&s);
    assert_eq!(documents(&r), documents(&s));
    assert_eq!(r.active_index(), s.active_index());
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

/// Between `edit.undoGroup.begin` and `edit.undoGroup.end`, each document created meanwhile opens an
/// undo group with the begun group's name, also when no document was open at the begin: its edits
/// are one undo step there, and a cancel undoes them and leaves the document open. A group opened
/// without the command (a scrubbed field) and a begin that a failed batch rolled back open no group
/// in a new document.
#[test]
fn an_undo_group_begun_by_command_opens_in_each_document_created_before_its_end() {
    let mut s = Session::new();
    let rect = json!({"x": 10, "y": 10, "width": 100, "height": 50});
    let new = json!({"width": 400, "height": 300});
    let labels = |s: &Session| s.doc().unwrap().history.undo.iter().map(|e| e.label.clone()).collect::<Vec<_>>();
    let objects = |s: &Session| s.doc().unwrap().doc.layers[0].children().map_or(0, |c| c.len());
    s.execute("edit.undoGroup.begin", &json!({"label": "Job"})).unwrap();
    for _ in 0..2 {
        s.execute("file.new", &new).unwrap();
        s.execute("shape.rectangle", &rect).unwrap();
        s.execute("shape.ellipse", &rect).unwrap();
    }
    s.execute("edit.undoGroup.end", &json!({})).unwrap();
    assert_eq!(labels(&s), ["Job"]);
    s.execute("document.activate", &json!({"index": 0})).unwrap();
    assert_eq!(labels(&s), ["Job"]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(objects(&s), 0, "one undo takes back both edits");

    // Closed: the next document's edits are steps of their own.
    s.execute("file.new", &new).unwrap();
    s.execute("shape.rectangle", &rect).unwrap();
    s.execute("shape.rectangle", &rect).unwrap();
    assert_eq!(labels(&s).len(), 2);

    // Canceled: the new document stays open without the group's edits, and the journal replays to it.
    let count = s.documents().len();
    s.execute("edit.undoGroup.begin", &json!({})).unwrap();
    s.execute("file.new", &new).unwrap();
    s.execute("shape.rectangle", &rect).unwrap();
    s.execute("shape.rectangle", &rect).unwrap();
    s.execute("edit.undoGroup.end", &json!({"cancel": true})).unwrap();
    assert_eq!(s.documents().len(), count + 1);
    assert_eq!((objects(&s), labels(&s).len()), (0, 0));
    assert_eq!(documents(&replayed(&s)), documents(&s));

    // A scrubbed field's group stays in its own document.
    s.begin_undo_group();
    s.execute("file.new", &new).unwrap();
    assert!(s.doc().unwrap().undo_group.is_none());
    s.end_undo_group(false);

    // A group begun by command and closed with `end_undo_group` opens no group in a later document.
    s.execute("edit.undoGroup.begin", &json!({})).unwrap();
    s.end_undo_group(false);
    s.execute("file.new", &new).unwrap();
    assert!(s.doc().unwrap().undo_group.is_none());

    // A batch that begins a group and then fails leaves no group begun.
    let steps = json!([{"command": "edit.undoGroup.begin"}, {"command": "no.such.command"}]);
    assert!(s.execute("command.batch", &json!({ "commands": steps })).is_err());
    s.execute("file.new", &new).unwrap();
    assert!(s.doc().unwrap().undo_group.is_none());
}

/// Between `edit.undoGroup.begin` and `edit.undoGroup.end`, a command that creates a document and
/// edits it (a `command.batch`) keeps its edits as an undo step of its own in that document, and the
/// edits after it are the group's step. A cancel undoes only those, keeps the document and the
/// command's edits, and the journal replays to the same documents.
#[test]
fn an_undo_group_leaves_out_the_edits_of_the_command_that_creates_its_document() {
    let mut s = Session::new();
    let rect = json!({"x": 10, "y": 10, "width": 100, "height": 50});
    let batch = json!({"commands": [
        {"command": "file.new", "params": {"width": 400, "height": 300}},
        {"command": "shape.rectangle", "params": rect},
    ]});
    let labels = |s: &Session| s.doc().unwrap().history.undo.iter().map(|e| e.label.clone()).collect::<Vec<_>>();
    let objects = |s: &Session| s.doc().unwrap().doc.layers[0].children().map_or(0, |c| c.len());
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    for cancel in [false, true] {
        s.execute("edit.undoGroup.begin", &json!({"label": "Job"})).unwrap();
        s.execute("command.batch", &batch).unwrap();
        s.execute("shape.ellipse", &rect).unwrap();
        s.execute("shape.ellipse", &rect).unwrap();
        s.execute("edit.undoGroup.end", &json!({ "cancel": cancel })).unwrap();
        let (objects, steps) = (objects(&s), labels(&s).len());
        assert_eq!((objects, steps), if cancel { (1, 1) } else { (3, 2) }, "cancel: {cancel}");
        assert_eq!(documents(&replayed(&s)), documents(&s), "cancel: {cancel}");
    }
    assert_eq!(s.documents().len(), 3);
}

/// Between `edit.undoGroup.begin` and `edit.undoGroup.end`, switching to a document that was open
/// before the begin opens no group in it: each edit made there is an undo step of its own. The edits
/// made in the document active at the begin, and in a document created meanwhile, stay one undo step
/// each after a switch away and back, and the journal replays to the same documents.
#[test]
fn an_undo_group_begun_by_command_opens_no_group_in_a_document_switched_to() {
    let mut s = Session::new();
    let rect = json!({"x": 10, "y": 10, "width": 100, "height": 50});
    let new = json!({"width": 400, "height": 300});
    let labels = |s: &Session, i: usize| s.documents()[i].history.undo.iter().map(|e| e.label.clone()).collect::<Vec<_>>();
    let objects = |s: &Session| s.doc().unwrap().doc.layers[0].children().map_or(0, |c| c.len());
    s.execute("file.new", &new).unwrap();
    s.execute("file.new", &new).unwrap();
    s.execute("document.activate", &json!({"index": 0})).unwrap();
    s.execute("edit.undoGroup.begin", &json!({"label": "Job"})).unwrap();
    s.execute("shape.rectangle", &rect).unwrap();
    s.execute("document.activate", &json!({"index": 1})).unwrap();
    assert!(s.doc().unwrap().undo_group.is_none());
    s.execute("shape.rectangle", &rect).unwrap();
    s.execute("shape.ellipse", &rect).unwrap();
    s.execute("file.new", &new).unwrap();
    s.execute("shape.rectangle", &rect).unwrap();
    s.execute("document.activate", &json!({"index": 1})).unwrap();
    s.execute("shape.rectangle", &rect).unwrap();
    s.execute("document.activate", &json!({"index": 2})).unwrap();
    s.execute("shape.ellipse", &rect).unwrap();
    s.execute("document.activate", &json!({"index": 0})).unwrap();
    s.execute("shape.ellipse", &rect).unwrap();
    s.execute("edit.undoGroup.end", &json!({})).unwrap();
    assert_eq!((labels(&s, 0), labels(&s, 2)), (vec!["Job".to_string()], vec!["Job".to_string()]));
    assert_eq!(labels(&s, 1).len(), 3, "{:?}", labels(&s, 1));
    assert!(s.documents().iter().all(|d| d.undo_group.is_none()));
    assert_eq!(documents(&replayed(&s)), documents(&s));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(objects(&s), 0, "one undo takes back both edits in the document active at the begin");
    s.execute("document.activate", &json!({"index": 1})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(objects(&s), 2, "one undo takes back one edit in the document switched to");
}

/// `edit.undoGroup.begin`'s params text gives the undo step's default name next to `label`, and the
/// clause on a document switched to says only that its edits stay steps of their own.
#[test]
fn the_undo_group_params_text_names_the_default_label_next_to_label() {
    let params = cmd::find_command("edit.undoGroup.begin").unwrap().params;
    assert!(params.contains("named `label` (else after its first edit); in a document"), "{params}");
    assert!(params.contains("each edit stays its own undo step: for automation"), "{params}");
}

/// An undo scope ([`Session::begin_undo_scope`]) in a document without an open undo group opens
/// one, and [`Session::end_undo_scope`] closes that group only, whichever document is active, and
/// leaves another document's group open; canceled, it undoes only that group's edits and cuts the
/// journal back to where the group began.
#[test]
fn an_undo_scope_closes_only_its_own_documents_group() {
    let mut s = Session::new();
    let rect = json!({"x": 10, "y": 10, "width": 100, "height": 50});
    let objects = |d: &DocState| d.doc.layers[0].children().map_or(0, |c| c.len());
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s.begin_undo_group();
    s.execute("shape.rectangle", &rect).unwrap();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let journal = s.journal.len();
    let scope = s.begin_undo_scope().unwrap();
    assert!(s.doc().unwrap().undo_group.is_some(), "the scope opens a group");
    s.execute("shape.rectangle", &rect).unwrap();
    s.execute("shape.rectangle", &rect).unwrap();
    s.end_undo_scope(scope, true);
    assert_eq!(objects(s.doc().unwrap()), 0, "canceled: the document's edits are undone");
    assert_eq!(s.journal.len(), journal, "and dropped from the journal");
    assert!(s.documents()[0].undo_group.is_some(), "the other document's group stays open");
    assert_eq!(objects(&s.documents()[0]), 1, "with its edit");

    let undo = s.doc().unwrap().history.undo.len();
    let scope = s.begin_undo_scope().unwrap();
    s.execute("shape.rectangle", &rect).unwrap();
    s.execute("shape.rectangle", &rect).unwrap();
    s.set_active(0);
    s.end_undo_scope(scope, false);
    let second = &s.documents()[1];
    assert_eq!(second.history.undo.len(), undo + 1, "closed: one undo step");
    assert!(second.undo_group.is_none(), "closed while another document is active");
    assert!(s.documents()[0].undo_group.is_some(), "the active document's group stays open");
}

/// An undo scope begun while the document's undo group is open joins the group: ended, its edits
/// are part of the group's step and the group stays open; canceled, the document goes back to
/// where it stood when the scope began, with the edits made in the group before it, the journal
/// drops the scope's entries, and a step that the scope's first edit recorded goes too.
#[test]
fn an_undo_scope_in_an_open_group_takes_back_only_its_own_edits() {
    let mut s = Session::new();
    let rect = json!({"x": 10, "y": 10, "width": 100, "height": 50});
    let objects = |s: &Session| s.doc().unwrap().doc.layers[0].children().map_or(0, |c| c.len());
    let undo_len = |s: &Session| s.doc().unwrap().history.undo.len();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let undo = undo_len(&s);
    s.begin_undo_group();
    let journal = s.journal.len();
    let scope = s.begin_undo_scope().unwrap();
    s.execute("shape.rectangle", &rect).unwrap();
    s.end_undo_scope(scope, true);
    assert_eq!((objects(&s), undo_len(&s), s.journal.len()), (0, undo, journal), "the group had no step: none is left");
    assert!(s.doc().unwrap().undo_group.is_some(), "the group stays open");

    s.execute("shape.rectangle", &rect).unwrap();
    let journal = s.journal.len();
    let scope = s.begin_undo_scope().unwrap();
    s.execute("shape.rectangle", &rect).unwrap();
    s.execute("shape.rectangle", &rect).unwrap();
    s.end_undo_scope(scope, true);
    assert_eq!((objects(&s), undo_len(&s), s.journal.len()), (1, undo + 1, journal), "the group's own edit stays");

    let scope = s.begin_undo_scope().unwrap();
    s.execute("shape.rectangle", &rect).unwrap();
    s.end_undo_scope(scope, false);
    assert_eq!((objects(&s), undo_len(&s)), (2, undo + 1), "ended: the edit joins the group's step");
    assert!(s.doc().unwrap().undo_group.is_some(), "the group stays open");
    s.end_undo_group(false);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(objects(&s), 0, "one undo takes back the group's edits");
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

#[test]
fn a_failed_preview_is_not_journaled_at_commit() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 100, "height": 50})).unwrap();
    let journal = s.journal.len();

    // Only a failing preview: committing records nothing.
    s.begin_interaction("Bad").unwrap();
    assert!(s.preview("no.such.command", &json!({})).is_err());
    s.commit_interaction().unwrap();
    assert_eq!(s.journal.len(), journal);

    // A good preview followed by a failing one: the document is back at the snapshot, so the
    // earlier preview is not journaled either.
    s.begin_interaction("Opacity").unwrap();
    s.preview("transparency.set", &json!({"opacity": 50})).unwrap();
    assert!(s.preview("no.such.command", &json!({})).is_err());
    s.commit_interaction().unwrap();
    assert_eq!(s.journal.len(), journal);
}
