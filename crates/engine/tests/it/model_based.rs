//! Model-based / stateful property tests: random command sequences drawn from the registry, with
//! structural invariants checked after every step and history/serialization invariants at the end.

use proptest::prelude::*;
use serde_json::json;
use vectorcraft_testkit::fixtures::{self, exec};
use vectorcraft_testkit::invariants::{self, check_all, check_session, doc_json, first_diff, redo_all, undo_all};
use vectorcraft_testkit::strategies::{Op, arb_ops};

fn run_sequence(ops: &[Op], full_checks_every: usize) -> Result<(), TestCaseError> {
    let mut s = fixtures::session();
    let initial = doc_json(&s.doc().unwrap().doc);
    for (i, op) in ops.iter().enumerate() {
        let (id, params) = op.command(&s);
        // Any result is fine; panics are not.
        let _ = s.execute(&id, &params);
        let check = if i % full_checks_every == 0 || i + 1 == ops.len() { check_all(&mut s) } else { check_session(&s) };
        if let Err(e) = check {
            return Err(TestCaseError::fail(format!("after step {i} `{id}` {params}: {e}")));
        }
    }
    // Reach the fully redone state first (the sequence may end with undos).
    redo_all(&mut s);
    let fin = doc_json(&s.doc().unwrap().doc);
    let n = undo_all(&mut s);
    let back = doc_json(&s.doc().unwrap().doc);
    prop_assert!(back == initial, "undo all ({n} steps) != initial: {}", first_diff(&initial, &back, "$"));
    check_session(&s).map_err(TestCaseError::fail)?;
    let m = redo_all(&mut s);
    prop_assert_eq!(n, m, "redo count differs from undo count");
    let again = doc_json(&s.doc().unwrap().doc);
    prop_assert!(again == fin, "redo all != final: {}", first_diff(&fin, &again, "$"));
    check_all(&mut s).map_err(TestCaseError::fail)?;
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 96, failure_persistence: None, max_shrink_iters: 2000, ..ProptestConfig::default() })]

    /// ~60 random commands; cheap invariants every step, round trips every 10 steps.
    #[test]
    fn random_command_sequences(ops in arb_ops(40..80)) {
        run_sequence(&ops, 10)?;
    }

    /// Shorter sequences with the full check (save/load, SVG, inspect) after every step.
    #[test]
    fn short_sequences_full_checks(ops in arb_ops(5..20)) {
        run_sequence(&ops, 1)?;
    }

    /// Replaying the journal of a random session into a fresh session reproduces the document.
    #[test]
        fn journal_replay_reproduces_document(ops in arb_ops(10..40)) {
        let mut s = fixtures::session();
        for op in &ops {
            let _ = op.apply(&mut s);
        }
        let want = doc_json(&s.doc().unwrap().doc);
        let journal = s.journal.clone();
        let mut r = vectorcraft_engine::Session::new();
        for (id, p) in &journal {
            let _ = r.execute(id, p);
        }
        // The journal includes clipboard state implicitly (copy/cut are journaled), so the replay
        // must land on the same document.
        let got = doc_json(&r.doc().unwrap().doc);
        prop_assert!(got == want, "replay differs: {}", first_diff(&want, &got, "$"));
    }
}

#[test]
fn rich_fixture_satisfies_invariants_and_undoes_to_blank() {
    let mut s = fixtures::rich_session();
    check_all(&mut s).unwrap();
    let fin = doc_json(&s.doc().unwrap().doc);
    let n = undo_all(&mut s);
    assert!(n >= 10, "expected a real history, got {n} steps");
    let blank = doc_json(&fixtures::session_with(500.0, 400.0).doc().unwrap().doc);
    // Titles differ ("Untitled-1" in both sessions since each has its own counter).
    assert_eq!(doc_json(&s.doc().unwrap().doc), blank, "{}", first_diff(&blank, &doc_json(&s.doc().unwrap().doc), "$"));
    redo_all(&mut s);
    assert_eq!(doc_json(&s.doc().unwrap().doc), fin);
}

#[test]
fn batch_is_one_undo_step_and_rolls_back_on_error() {
    let mut s = fixtures::session();
    let before = s.doc().unwrap().history.undo.len();
    exec(
        &mut s,
        "command.batch",
        json!({"commands": [
            {"command": "shape.rectangle", "params": {"x": 0, "y": 0, "width": 10, "height": 10}},
            {"command": "shape.ellipse", "params": {"x": 20, "y": 0, "width": 10, "height": 10}},
            {"command": "select.all", "params": {}},
            {"command": "object.group", "params": {}},
        ]}),
    );
    assert_eq!(s.doc().unwrap().history.undo.len(), before + 1);
    assert_eq!(s.doc().unwrap().doc.node_count(), 4);
    let snapshot = doc_json(&s.doc().unwrap().doc);
    // A failing step rolls the whole batch back and leaves no interaction open.
    let r = s.execute(
        "command.batch",
        &json!({"commands": [
            {"command": "shape.rectangle", "params": {"x": 0, "y": 0, "width": 10, "height": 10}},
            {"command": "no.such.command", "params": {}},
        ]}),
    );
    assert!(r.is_err());
    assert_eq!(doc_json(&s.doc().unwrap().doc), snapshot);
    assert!(!s.in_interaction());
    // Nested batches are rejected, also without leaking an interaction.
    let r = s.execute("command.batch", &json!({"commands": [{"command": "command.batch", "params": {"commands": []}}]}));
    assert!(r.is_err());
    assert!(!s.in_interaction());
    exec(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc().unwrap().doc.node_count(), 1);
}

/// #1697: a failed batch brings back the whole document, not only its art: the file a Save As
/// step named and the history an Undo step moved; and a batch leaves a drag in progress its own
/// step.
#[test]
fn a_failed_batch_restores_the_file_the_history_and_a_drag_in_progress() {
    let mut s = fixtures::session();
    let rect = |x: i32| json!({"command": "shape.rectangle", "params": {"x": x, "y": 0, "width": 10, "height": 10}});
    let fail = json!({"command": "no.such.command", "params": {}});
    exec(&mut s, "shape.rectangle", json!({"x": 50, "y": 0, "width": 10, "height": 10}));
    let before = (doc_json(&s.doc().unwrap().doc), s.doc().unwrap().history.undo.len(), s.doc().unwrap().history.redo.len());
    let state = |s: &vectorcraft_engine::Session| {
        let st = s.doc().unwrap();
        (doc_json(&st.doc), st.history.undo.len(), st.history.redo.len())
    };
    // Save As, then a failing step: the document keeps no file and stays modified.
    let path = vectorcraft_testkit::temp_dir("batch-rollback").join("named.vectorcraft").to_string_lossy().to_string();
    let save = json!({"command": "file.saveAs", "params": {"path": path}});
    assert!(s.execute("command.batch", &json!({"commands": [rect(0), save, fail]})).is_err());
    assert_eq!(state(&s), before);
    assert_eq!((s.doc().unwrap().path.as_deref(), s.doc().unwrap().is_dirty()), (None, true));
    // Undo, then a failing step: the art and both history stacks are as before.
    let undo = json!({"command": "edit.undo", "params": {}});
    assert!(s.execute("command.batch", &json!({"commands": [undo, fail]})).is_err());
    assert_eq!(state(&s), before);
    // A batch during a drag: the drag is kept as a step, the batch is the next one.
    exec(&mut s, "select.all", json!({}));
    s.begin_interaction("Move").unwrap();
    s.preview("object.move", &json!({"dx": 5, "dy": 0})).unwrap();
    let moved = doc_json(&s.doc().unwrap().doc);
    exec(&mut s, "command.batch", json!({"commands": [rect(100)]}));
    assert!(!s.in_interaction());
    assert_eq!(s.doc().unwrap().history.undo.len(), before.1 + 2);
    exec(&mut s, "edit.undo", json!({}));
    assert_eq!(doc_json(&s.doc().unwrap().doc), moved, "undoing the batch leaves the drag's move");
    exec(&mut s, "edit.undo", json!({}));
    assert_eq!(doc_json(&s.doc().unwrap().doc), before.0);
}

#[test]
fn empty_batch_is_harmless() {
    let mut s = fixtures::session();
    let before = doc_json(&s.doc().unwrap().doc);
    let _ = s.execute("command.batch", &json!({"commands": []}));
    assert!(!s.in_interaction());
    assert_eq!(doc_json(&s.doc().unwrap().doc), before);
    check_all(&mut s).unwrap();
}

#[test]
fn save_open_via_commands_roundtrips() {
    let mut s = fixtures::rich_session();
    let dir = vectorcraft_testkit::temp_dir("engine-save");
    let path = dir.join("rich.vectorcraft");
    exec(&mut s, "document.save", json!({"path": path.to_str().unwrap()}));
    let want = doc_json(&s.doc().unwrap().doc);
    exec(&mut s, "document.open", json!({"path": path.to_str().unwrap()}));
    assert_eq!(s.documents().len(), 2);
    let got = doc_json(&s.doc().unwrap().doc);
    // Numbers to 1e-12 (bit-exactness is a known bug: known_bugs.rs::bug_native_roundtrip_not_bit_exact).
    assert!(invariants::json_approx_eq(&got, &want, 1e-12), "{}", first_diff(&want, &got, "$"));
    check_all(&mut s).unwrap();
}

#[test]
fn serialize_all_formats_on_rich_doc() {
    let mut s = fixtures::rich_session();
    for f in ["vectorcraft", "svg", "pdf", "png"] {
        let v = exec(&mut s, "document.serialize", json!({"format": f}));
        assert!(v.get("dataBase64").or(v.get("text")).is_some(), "{f}: {v}");
    }
    // The SVG re-imports with the same number of visible art objects or more (groups may nest).
    let svg = exec(&mut s, "document.serialize", json!({"format": "svg"}));
    let text = svg["text"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| String::from_utf8(vectorcraft_format::base64_decode(svg["dataBase64"].as_str().unwrap()).unwrap()).unwrap());
    let back = vectorcraft_svg::import(&text).unwrap();
    invariants::check_document(&back).unwrap();
    assert!(back.node_count() >= 5);
}
