//! Model-based / stateful property tests: random command sequences drawn from the registry, with
//! structural invariants checked after every step and history/serialization invariants at the end.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

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

/// Regression (flaky `journal_replay_reproduces_document`): a batch that rolls back must also
/// restore the Layers panel's highlighted rows. Its `layer.setCurrent` step used to leave layer 1
/// highlighted, so the next Collect in New Layer nested it in a new layer — which the replay,
/// where the failed batch isn't journaled, didn't do.
#[test]
fn rolled_back_batch_restores_highlighted_layer_rows() {
    use vectorcraft_testkit::strategies::Op;
    let ops = [Op::Batch(vec![Op::LayerCurrent(0), Op::Offset(0.0)]), Op::Collect, Op::Rect(0.0, 0.0, 1.0, 1.0), Op::Rect(0.0, 0.0, 1.0, 1.0)];
    let mut s = fixtures::session();
    let rows = s.doc().unwrap().layer_rows.clone();
    assert!(ops[0].apply(&mut s).is_err(), "the batch's Offset Path has nothing selected");
    assert_eq!(s.doc().unwrap().layer_rows, rows);
    assert!(ops[1].apply(&mut s).is_err(), "nothing highlighted or selected to collect");
    for op in &ops[2..] {
        op.apply(&mut s).unwrap();
    }
    let want = doc_json(&s.doc().unwrap().doc);
    let mut r = vectorcraft_engine::Session::new();
    for (id, p) in &s.journal {
        let _ = r.execute(id, p);
    }
    let got = doc_json(&r.doc().unwrap().doc);
    assert!(got == want, "replay differs: {}", first_diff(&want, &got, "$"));
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
