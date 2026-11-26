//! The undo history's memory (#1244): steps keep no lookup tables, unchanged selections are
//! shared between steps, and past a memory budget the oldest steps go (never the last few).

use serde_json::json;
use vectorcraft_doc::{Appearance, Node};
use vectorcraft_geom::shapes;

use crate::*;

/// A session with one document holding `n` rectangles, all selected.
fn session_with_rects(n: usize) -> Session {
    let mut s = Session::new();
    open_with_rects(&mut s, n);
    s
}

/// Open a new document in `s` holding `n` rectangles, all selected.
fn open_with_rects(s: &mut Session, n: usize) {
    s.execute("file.new", &json!({"width": 2000, "height": 2000})).unwrap();
    s.edit("Add", |d, _| {
        let layer = d.layers[0].id;
        for i in 0..n {
            let id = d.alloc_id();
            let r = shapes::rectangle(kurbo::Rect::new(i as f64, 0.0, i as f64 + 10.0, 10.0));
            d.insert(Some(layer), usize::MAX, Node::path(id, r, Appearance::default_art()))?;
        }
        Ok(())
    })
    .unwrap();
    s.execute("select.all", &json!({})).unwrap();
}

fn first_x(s: &Session) -> f64 {
    let d = &s.doc().unwrap().doc;
    let id = d.layers[0].children().unwrap()[0].id;
    d.node(id).unwrap().geometric_bounds().unwrap().x0
}

#[test]
fn past_the_budget_the_oldest_steps_go_but_the_last_few_stay() {
    let n = 500;
    let mut s = session_with_rects(n);
    s.execute("object.move", &json!({"dx": 1, "dy": 0})).unwrap();
    let step = s.doc().unwrap().history.undo.last().unwrap().bytes;
    // Each move keeps every rectangle's old node: about 700 bytes each (measured: ~775).
    assert!((300 * n..1500 * n).contains(&step), "{step} bytes for moving {n} rectangles");
    // A budget of about three moves.
    s.active_mut().unwrap().history.byte_budget = 3 * step;
    for _ in 0..20 {
        s.execute("object.move", &json!({"dx": 1, "dy": 0})).unwrap();
    }
    let st = s.doc().unwrap();
    assert_eq!(st.history.undo.len(), History::MIN_STEPS, "only the last few steps are kept");
    assert!(st.history.undo.iter().all(|e| e.label == "Move"), "the newest ones: {:?}", st.history.undo.iter().map(|e| &e.label).collect::<Vec<_>>());
    // The kept steps undo as before.
    let x = first_x(&s);
    for _ in 0..History::MIN_STEPS {
        s.execute("edit.undo", &json!({})).unwrap();
    }
    assert_eq!(first_x(&s), x - History::MIN_STEPS as f64);
    assert!(s.execute("edit.undo", &json!({})).is_err(), "the older steps went");
    // Redo too stays within the budget, keeping the next few steps.
    let st = s.doc().unwrap();
    assert_eq!(st.history.redo.len(), History::MIN_STEPS);
    for _ in 0..History::MIN_STEPS {
        s.execute("edit.redo", &json!({})).unwrap();
    }
    assert_eq!(first_x(&s), x);
}

#[test]
fn within_the_budget_the_step_limit_still_counts() {
    let mut s = session_with_rects(10);
    s.active_mut().unwrap().history.limit = 12;
    for _ in 0..30 {
        s.execute("object.move", &json!({"dx": 1, "dy": 0})).unwrap();
    }
    let st = s.doc().unwrap();
    assert_eq!(st.history.undo.len(), 12);
    assert!(st.history.bytes() < st.history.byte_budget);
}

fn moves(s: &mut Session, n: usize) {
    for _ in 0..n {
        s.execute("object.move", &json!({"dx": 1, "dy": 0})).unwrap();
    }
}

fn undo_lens(s: &Session) -> Vec<usize> {
    s.docs.iter().map(|d| d.history.undo.len()).collect()
}

fn history_bytes(s: &Session) -> usize {
    s.docs.iter().map(|d| d.history.bytes()).sum()
}

#[test]
fn one_document_keeps_its_steps_within_the_session_budget() {
    let mut s = session_with_rects(500);
    moves(&mut s, 1);
    let step = s.doc().unwrap().history.undo.last().unwrap().bytes;
    s.history_budget = 3 * step;
    moves(&mut s, 20);
    let st = s.doc().unwrap();
    assert_eq!(st.history.undo.len(), History::MIN_STEPS, "only the last few steps are kept");
    assert_eq!(st.history.byte_budget, default_history_budget(), "the document's own budget is unchanged");
    // A lower budget applies at the next command, even one that adds no step.
    s.history_budget = usize::MAX;
    moves(&mut s, 4);
    assert_eq!(s.doc().unwrap().history.undo.len(), History::MIN_STEPS + 4);
    s.history_budget = 3 * step;
    s.execute("select.none", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), History::MIN_STEPS);
}

#[test]
fn open_documents_share_one_budget_and_the_oldest_steps_go_first() {
    let n = 300;
    let mut s = session_with_rects(n);
    moves(&mut s, 1);
    let step = s.doc().unwrap().history.undo.last().unwrap().bytes;
    // Room for about 22 moves in all the open documents together.
    s.history_budget = 22 * step;
    moves(&mut s, 11);
    assert_eq!(undo_lens(&s), [13], "Add and 12 moves fit in the budget");
    open_with_rects(&mut s, n);
    moves(&mut s, 12);
    let lens = undo_lens(&s);
    assert_eq!(lens[1], 13, "the newer document keeps all of its steps: {lens:?}");
    assert!((History::MIN_STEPS..13).contains(&lens[0]), "the older document's oldest steps went: {lens:?}");
    assert!(history_bytes(&s) <= s.history_budget, "{} bytes kept, budget {}", history_bytes(&s), s.history_budget);
    // Closing the older document frees its share: the other keeps more steps than before.
    assert!(s.close_document(0));
    assert!(s.set_active(0));
    moves(&mut s, 6);
    assert_eq!(undo_lens(&s), [19], "Add and 18 moves fit in the budget alone");
}

#[test]
fn an_undo_group_lets_no_other_documents_steps_go_until_it_ends() {
    let mut s = session_with_rects(50);
    moves(&mut s, 12);
    open_with_rects(&mut s, 50);
    // The steps kept now fill the budget exactly.
    s.history_budget = history_bytes(&s);
    s.execute("select.all", &json!({})).unwrap();
    assert_eq!(undo_lens(&s), [13, 1]);
    // A scrubbed field's edits in the second document, then canceled.
    s.begin_undo_group();
    moves(&mut s, 3);
    assert_eq!(undo_lens(&s), [13, 2], "the group's step waits for the group to end");
    s.end_undo_group(true);
    assert_eq!(undo_lens(&s), [13, 1], "a canceled group leaves the other document's steps");
    // The same edits kept: the other document's oldest steps go.
    s.begin_undo_group();
    moves(&mut s, 3);
    s.end_undo_group(false);
    let lens = undo_lens(&s);
    assert_eq!(lens[1], 2);
    assert!((History::MIN_STEPS..13).contains(&lens[0]), "the older document's oldest steps went: {lens:?}");
    assert!(history_bytes(&s) <= s.history_budget);
    // A scrub through an undo scope: its step counts once the scope ends.
    s.history_budget = usize::MAX;
    assert!(s.set_active(0));
    moves(&mut s, 6);
    assert!(s.set_active(1));
    s.history_budget = history_bytes(&s);
    let before = undo_lens(&s)[0];
    let scope = s.begin_undo_scope().unwrap();
    moves(&mut s, 3);
    assert_eq!(undo_lens(&s), [before, 3], "the scope's step waits for the scope to end");
    s.end_undo_scope(scope, false);
    let lens = undo_lens(&s);
    assert!(lens[0] < before, "the older document's oldest steps went once the scope ended: {lens:?}");
    assert!(history_bytes(&s) <= s.history_budget);
}

#[test]
fn steps_kept_outside_a_command_keep_to_the_shared_budget() {
    let mut s = session_with_rects(50);
    moves(&mut s, 12);
    open_with_rects(&mut s, 50);
    assert_eq!(undo_lens(&s), [13, 1]);
    // A drag the frontend commits itself.
    s.begin_interaction("Move").unwrap();
    s.preview("object.move", &json!({"dx": 3, "dy": 0})).unwrap();
    s.history_budget = 1;
    assert_eq!(undo_lens(&s), [13, 1]);
    s.commit_interaction().unwrap();
    assert_eq!(undo_lens(&s), [History::MIN_STEPS, 2], "the drag's step counts against the other document's");
    // A script run across frames, whose step is kept after `script.run` returns.
    s.history_budget = usize::MAX;
    assert!(s.set_active(0));
    moves(&mut s, 4);
    assert!(s.set_active(1));
    s.script_frames = true;
    let source = "for (var i = 0; i < 3000000; i++) {} vc.run('object.move', {dx: 1, dy: 0});";
    assert_eq!(s.execute("script.run", &json!({"source": source, "name": "t.js"})).unwrap()["running"], true);
    s.history_budget = 1;
    let mut slices = 0;
    let r = loop {
        slices += 1;
        assert!(slices < 100_000, "the script ends");
        if let Some(r) = s.script_pump() {
            break r;
        }
    };
    assert_eq!(r.unwrap()["ok"], true);
    assert_eq!(undo_lens(&s), [History::MIN_STEPS, 3], "the run's step counts against the other document's");
    // An edit a frontend makes itself, outside a command.
    s.history_budget = usize::MAX;
    assert!(s.set_active(0));
    moves(&mut s, 4);
    assert!(s.set_active(1));
    s.history_budget = 1;
    s.edit("Add", |d, _| {
        let id = d.alloc_id();
        let r = shapes::rectangle(kurbo::Rect::new(0.0, 20.0, 10.0, 30.0));
        d.insert(Some(d.layers[0].id), usize::MAX, Node::path(id, r, Appearance::default_art()))?;
        Ok(())
    })
    .unwrap();
    assert_eq!(undo_lens(&s), [History::MIN_STEPS, 4], "the edit's step counts against the other document's");
}

#[test]
fn each_document_keeps_its_last_steps_however_small_the_budget() {
    let mut s = Session::new();
    // Smaller than any one step.
    s.history_budget = 1;
    for _ in 0..4 {
        open_with_rects(&mut s, 50);
        moves(&mut s, 12);
    }
    assert_eq!(undo_lens(&s), [History::MIN_STEPS; 4]);
    // The kept steps undo and redo, and the document now is as it was.
    let x = first_x(&s);
    assert_eq!(x, 12.0);
    for _ in 0..History::MIN_STEPS {
        s.execute("edit.undo", &json!({})).unwrap();
    }
    assert_eq!(first_x(&s), x - History::MIN_STEPS as f64);
    assert!(s.execute("edit.undo", &json!({})).is_err(), "the older steps went");
    assert_eq!(s.doc().unwrap().history.redo.len(), History::MIN_STEPS);
    // A held history keeps all of its steps until it is released.
    assert!(s.set_active(0));
    s.docs[0].history.held = true;
    moves(&mut s, 3);
    assert_eq!(undo_lens(&s)[0], History::MIN_STEPS + 3, "a held history keeps its steps");
    s.docs[0].history.held = false;
    s.trim_histories();
    assert_eq!(undo_lens(&s), [History::MIN_STEPS, History::MIN_STEPS, History::MIN_STEPS, 0]);
}

#[test]
#[cfg(target_pointer_width = "64")]
fn the_default_budget_is_a_quarter_of_installed_memory() {
    assert_eq!(history_budget_for(Some(16 << 30)), 4 << 30);
    assert_eq!(history_budget_for(Some(64 << 30)), 16 << 30, "no fixed ceiling");
    // On Linux `physical_memory` returns a cgroup memory limit lower than the installed memory in
    // its place (the `cgroup_limit_*` tests in vectorcraft-render), so a 2 GiB limit on a 64 GiB
    // machine gives the budget of 2 GiB installed.
    assert_eq!(history_budget_for(Some(2 << 30)), 512 << 20, "2 GiB, or a 2 GiB cgroup limit");
    assert_eq!(history_budget_for(Some(128 << 20)), 64 << 20, "at least 64 MiB");
    assert_eq!(history_budget_for(Some(1)), 64 << 20, "at least 64 MiB");
    assert_eq!(history_budget_for(Some(0)), 1 << 30, "a reading of 0 is the 1 GiB fallback");
    assert_eq!(history_budget_for(None), 1 << 30, "no reading is the 1 GiB fallback");
}

#[test]
#[cfg(any(target_os = "linux", windows, target_os = "macos", target_os = "freebsd"))]
fn the_default_budget_follows_the_installed_memory_read() {
    // Where the memory can't be read (a sandbox that hides it), the budget is the 1 GiB fallback.
    let memory = vectorcraft_render::physical_memory();
    assert_eq!(default_history_budget(), history_budget_for(memory), "{memory:?}");
    assert_eq!(History::default().byte_budget, default_history_budget());
    assert_eq!(Session::new().history_budget, default_history_budget(), "the open documents share it");
}

#[test]
fn undo_steps_keep_no_lookup_tables() {
    let mut s = session_with_rects(200);
    for _ in 0..5 {
        s.execute("object.move", &json!({"dx": 1, "dy": 0})).unwrap();
        // Lookups build the current document's table.
        first_x(&s);
        assert!(s.doc().unwrap().doc.has_lookup_cache());
    }
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    let st = s.doc().unwrap();
    assert!(!st.history.undo.is_empty() && !st.history.redo.is_empty());
    for e in st.history.undo.iter().chain(&st.history.redo) {
        assert!(!e.doc.has_lookup_cache(), "step {} keeps a lookup table", e.label);
    }
    // The document undone to builds its table again on its first lookup.
    first_x(&s);
    assert!(s.doc().unwrap().doc.has_lookup_cache());
    // A drag's snapshot goes into the history without its table too.
    s.begin_interaction("Move").unwrap();
    s.preview("object.move", &json!({"dx": 3, "dy": 0})).unwrap();
    first_x(&s);
    s.commit_interaction().unwrap();
    assert!(!s.doc().unwrap().history.undo.last().unwrap().doc.has_lookup_cache());
}

#[test]
fn steps_share_an_unchanged_selection() {
    let mut s = session_with_rects(100);
    for _ in 0..4 {
        s.execute("object.move", &json!({"dx": 1, "dy": 0})).unwrap();
    }
    s.begin_interaction("Move").unwrap();
    s.preview("object.move", &json!({"dx": 3, "dy": 0})).unwrap();
    s.preview("object.move", &json!({"dx": 4, "dy": 0})).unwrap();
    s.commit_interaction().unwrap();
    let st = s.doc().unwrap();
    let moves: Vec<_> = st.history.undo.iter().filter(|e| e.label == "Move").collect();
    assert_eq!(moves.len(), 5);
    assert_eq!(moves[0].selection.objects.len(), 100);
    assert!(moves.windows(2).all(|w| Arc::ptr_eq(&w[0].selection, &w[1].selection)), "one selection for all the moves");
    // Undo and redo keep it shared, and restore it.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    let st = s.doc().unwrap();
    assert!(Arc::ptr_eq(&st.history.redo[0].selection, &st.history.redo[1].selection));
    assert_eq!(st.selection.objects.len(), 100);
}

/// Undo and redo don't estimate the steps again: the step an undo (redo) makes takes the undone
/// step's figures the other way round, which are what a fresh estimate gives.
#[test]
fn undo_and_redo_reuse_the_estimates() {
    let mut s = session_with_rects(200);
    s.execute("object.move", &json!({"dx": 5, "dy": 0})).unwrap();
    // A deletion is lopsided: the step before it keeps the objects, the one after keeps nothing.
    s.execute("edit.clear", &json!({})).unwrap();
    let fresh = |e: &HistoryEntry, now: &Document| e.doc.retained_bytes(now);
    let st = s.doc().unwrap();
    let delete = st.history.undo.last().unwrap();
    assert!(delete.bytes > 200 * 300, "the deleted objects: {}", delete.bytes);
    s.execute("edit.undo", &json!({})).unwrap();
    let st = s.doc().unwrap();
    let redo = st.history.redo.last().unwrap();
    assert_eq!(redo.bytes, fresh(redo, &st.doc));
    assert!(redo.bytes < 4096, "the document without the objects keeps little of its own: {}", redo.bytes);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.redo", &json!({})).unwrap();
    let st = s.doc().unwrap();
    let undo = st.history.undo.last().unwrap();
    assert_eq!(undo.label, "Move");
    assert_eq!(undo.bytes, fresh(undo, &st.doc));
    s.execute("edit.redo", &json!({})).unwrap();
    let st = s.doc().unwrap();
    let delete = st.history.undo.last().unwrap();
    assert_eq!(delete.bytes, fresh(delete, &st.doc));
}

/// #1719: Redo while a drag is in progress keeps the drag as a step (which ends the steps to redo,
/// as any new edit does) instead of swapping the document under it: before, the next preview or a
/// cancel went back to the document from before the redo while the history said it was redone.
#[test]
fn redo_during_an_interaction_keeps_the_history_and_the_document_in_step() {
    let mut s = session_with_rects(1);
    s.execute("object.move", &json!({"dx": 10, "dy": 0})).unwrap();
    let moved = first_x(&s);
    s.execute("edit.undo", &json!({})).unwrap();
    let start = first_x(&s);
    // A drag that changed nothing yet is no step: Redo redoes, and a cancel has nothing to undo.
    s.begin_interaction("Move").unwrap();
    s.execute("edit.redo", &json!({})).unwrap();
    s.cancel_interaction().unwrap();
    assert_eq!(first_x(&s), moved, "the redone move stays");
    s.execute("edit.undo", &json!({})).unwrap();
    // A drag that moved the art is the newest step: nothing is left to redo, and nothing is lost.
    s.begin_interaction("Move").unwrap();
    s.preview("object.move", &json!({"dx": 3, "dy": 0})).unwrap();
    assert!(s.execute("edit.redo", &json!({})).is_err(), "the drag ended the steps to redo");
    assert!(!s.in_interaction());
    assert_eq!(first_x(&s), start + 3.0);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(first_x(&s), start);
}

/// `History::step` changes with every new step, Undo and Redo, also when an undo and a new edit
/// leave as many steps as before, and with an edit merged into an undo group's step. It stays the
/// same for a selection change.
#[test]
fn the_history_step_changes_with_each_step_undo_and_redo() {
    let mut s = session_with_rects(2);
    let step = |s: &Session| s.doc().unwrap().history.step();
    let start = step(&s);
    assert!(start.0 != 0 && start.1 == 0, "the Add step, and nothing to redo: {start:?}");
    s.execute("object.move", &json!({"dx": 1, "dy": 0})).unwrap();
    let moved = step(&s);
    assert!(moved.0 != start.0 && moved.1 == 0, "{moved:?}");
    s.execute("edit.undo", &json!({})).unwrap();
    let undone = step(&s);
    assert!(undone.0 == start.0 && undone.1 != 0, "{undone:?}");
    s.execute("edit.redo", &json!({})).unwrap();
    let redone = step(&s);
    assert!(![start, moved, undone].contains(&redone) && redone.1 == 0, "{redone:?}");
    // An undo and a new edit: as many undo steps as after the move, at a different step.
    s.execute("edit.undo", &json!({})).unwrap();
    let depth = s.doc().unwrap().history.undo.len();
    s.execute("object.move", &json!({"dx": 0, "dy": 1})).unwrap();
    let other = step(&s);
    assert_eq!(s.doc().unwrap().history.undo.len(), depth + 1);
    assert!(![start, moved, undone, redone].contains(&other), "{other:?}");
    s.execute("select.none", &json!({})).unwrap();
    assert_eq!(step(&s), other, "a selection change is no step");
    s.execute("select.all", &json!({})).unwrap();
    s.begin_undo_group();
    s.execute("object.move", &json!({"dx": 1, "dy": 0})).unwrap();
    let (grouped, depth) = (step(&s), s.doc().unwrap().history.undo.len());
    assert_ne!(grouped, other);
    s.execute("object.move", &json!({"dx": 1, "dy": 0})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), depth, "the second move is merged into the first");
    let merged = step(&s);
    assert!(merged != grouped && merged.1 == 0, "a merged edit moves the step: {merged:?}");
    s.execute("select.none", &json!({})).unwrap();
    assert_eq!(step(&s), merged);
    s.end_undo_group(false);
}

#[test]
fn color_mode_paint_snapshots_count_toward_the_history_budget() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width":100,"height":100})).unwrap();
    let mut appearance = s.new_art();
    appearance.effects.push(vectorcraft_doc::Effect {
        id: "stylize.dropShadow".into(),
        visible: true,
        params: json!({"color":"#ab6739", "retainedTemplateData":"x".repeat(128*1024)}),
    });
    s.paint.appearance = Some(appearance);
    let original = s.paint.clone();
    s.active_mut().unwrap().history.byte_budget = 64 * 1024;
    for i in 0..12 {
        s.execute("file.documentColorMode", &json!({"mode": if i%2 == 0 {"cmyk"} else {"rgb"}})).unwrap();
    }
    let st = s.doc().unwrap();
    assert_eq!(st.history.undo.len(), History::MIN_STEPS);
    assert!(st.history.undo.iter().all(|e| e.bytes > 256 * 1024), "both large templates count even without artwork");
    assert!(st.doc.layers.iter().all(|l| l.children().is_none_or(|c| c.is_empty())));
    let retained = st.history.bytes();
    let after = s.paint.clone();
    for _ in 0..History::MIN_STEPS {
        s.execute("edit.undo", &json!({})).unwrap();
    }
    assert_eq!(s.doc().unwrap().history.redo.len(), History::MIN_STEPS);
    assert!(s.doc().unwrap().history.redo.iter().all(|e| e.bytes > 256 * 1024));
    assert!(s.doc().unwrap().history.bytes().abs_diff(retained) < 64 * 1024, "turning entries must not double-count paint snapshots");
    for _ in 0..History::MIN_STEPS {
        s.execute("edit.redo", &json!({})).unwrap();
    }
    assert_eq!(s.paint, after);
    assert!(s.doc().unwrap().history.bytes().abs_diff(retained) < 64 * 1024);
    assert_eq!(
        s.paint.appearance.as_ref().unwrap().effects[0].params["retainedTemplateData"],
        original.appearance.unwrap().effects[0].params["retainedTemplateData"]
    );
}
