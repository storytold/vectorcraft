//! Width tool depth (M3.64): discontinuous width points, copies, multi-point drags, Adjust
//! Adjoining Width Points and the render of a step.

use serde_json::json;
use vectorcraft_tools::{Mods, PointerEvent, PointerKind};

use crate::*;

/// A horizontal line from (20, 100) to (180, 100), 10 pt black stroke.
fn line_session() -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let r = s.execute("shape.line", &json!({"x1": 20, "y1": 100, "x2": 180, "y2": 100})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    s.execute("stroke.set", &json!({"ids": [id.0], "weight": 10})).unwrap();
    (s, id)
}

fn points(s: &Session, id: NodeId) -> Vec<(f64, f64, f64)> {
    s.doc().unwrap().doc.node(id).unwrap().appearance.stroke().unwrap().profile.as_ref().map(|p| p.points.clone()).unwrap_or_default()
}

fn set(s: &mut Session, p: serde_json::Value) -> usize {
    s.execute("stroke.widthPoint.set", &p).unwrap()["index"].as_u64().unwrap() as usize
}

fn dark(s: &Session, x: u32, y: u32) -> bool {
    let doc = s.doc().unwrap().doc.clone();
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    vectorcraft_render::Renderer::new().render(&doc, 200, 200, vectorcraft_geom::Affine::IDENTITY, &opts).pixel(x, y)[0] < 128
}

/// Invalid entries in an explicit multi-point delete must not delete the valid entries.
#[test]
fn invalid_width_point_indices_leave_the_entire_profile_unchanged() {
    let (mut s, id) = line_session();
    set(&mut s, json!({"id": id.0, "t": 0.5, "left": 10, "right": 10}));
    let before = points(&s, id);
    let history = s.doc().unwrap().history.undo.len();
    for bad in [
        json!({"indices": [1, "bad"]}),
        json!({"indices": [-1, 1]}),
        json!({"indices": [1.5, 1]}),
        json!({"indices": {"point": 1}}),
        json!({"index": "bad"}),
    ] {
        let mut params = bad.as_object().unwrap().clone();
        params.insert("id".into(), json!(id.0));
        let err = s.execute("stroke.widthPoint.remove", &serde_json::Value::Object(params));
        assert!(err.is_err(), "accepted malformed width point indices");
        assert_eq!(points(&s, id), before);
        assert_eq!(s.doc().unwrap().history.undo.len(), history);
    }
    s.execute("stroke.widthPoint.remove", &json!({"id": id.0, "indices": [1]})).unwrap();
    assert_eq!(points(&s, id).len(), before.len() - 1);
}

/// Setting or copying a width point with a malformed `index` is refused like one out of range: it
/// never adds a point instead, or wraps a huge index round. A null `index` is no index (a new point).
#[test]
fn a_malformed_width_point_index_is_refused_by_set_and_copy() {
    let (mut s, id) = line_session();
    set(&mut s, json!({"id": id.0, "t": 0.5, "left": 10, "right": 10}));
    let before = points(&s, id);
    let history = s.doc().unwrap().history.undo.len();
    for index in [json!("bad"), json!(-1), json!(1.5), json!([1]), json!(u64::MAX), json!(before.len())] {
        for cmd in ["stroke.widthPoint.set", "stroke.widthPoint.copy"] {
            let r = s.execute(cmd, &json!({"id": id.0, "index": index, "t": 0.3, "left": 4, "right": 4}));
            assert!(r.is_err(), "{cmd} took index {index}");
            assert_eq!(points(&s, id), before, "{cmd} with index {index}");
        }
    }
    assert_eq!(s.doc().unwrap().history.undo.len(), history);
    set(&mut s, json!({"id": id.0, "index": null, "t": 0.3, "left": 4, "right": 4}));
    assert_eq!(points(&s, id).len(), before.len() + 1, "a null index adds a point");
}

#[test]
fn a_point_moved_onto_another_makes_a_discontinuous_point_that_renders_a_step() {
    let (mut s, id) = line_session();
    set(&mut s, json!({"id": id.0, "t": 0.25, "left": 5, "right": 5}));
    set(&mut s, json!({"id": id.0, "t": 0.75, "left": 15, "right": 15}));
    // Move the thin point (index 1) onto the wide one: it joins it from below, as its first side.
    let i = set(&mut s, json!({"id": id.0, "index": 1, "t": 0.75, "left": 5, "right": 5}));
    assert_eq!(i, 1);
    assert_eq!(points(&s, id), vec![(0.0, 1.0, 1.0), (0.75, 1.0, 1.0), (0.75, 3.0, 3.0), (1.0, 1.0, 1.0)]);
    // Editing one side's widths keeps the pair.
    set(&mut s, json!({"id": id.0, "index": 1, "t": 0.75, "left": 2, "right": 2}));
    assert_eq!(points(&s, id)[1..3], [(0.75, 0.4, 0.4), (0.75, 3.0, 3.0)]);
    let doc = s.execute("document.inspect", &json!({})).unwrap();
    let want = json!([[0.0, 1.0, 1.0], [0.75, 0.4, 0.4], [0.75, 3.0, 3.0], [1.0, 1.0, 1.0]]);
    assert_eq!(doc["layers"][0]["children"][0]["strokeOptions"]["widthPoints"], want, "agents read the points");
    // The step renders: just before x = 140 (t = 0.75) the stroke is 2 pt thick, just after 15.
    assert!(!dark(&s, 137, 92) && dark(&s, 143, 92), "a step, not a ramp");
    // A new point at that place replaces both; a pair round-trips through the native format.
    let back = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    let n = back.node(id).unwrap();
    assert_eq!(n.appearance.stroke().unwrap().profile.as_ref().unwrap().points, points(&s, id));
    set(&mut s, json!({"id": id.0, "t": 0.75, "left": 5, "right": 5}));
    assert_eq!(points(&s, id).len(), 3);
}

#[test]
fn copy_adds_one_point_and_the_tool_alt_drag_copies() {
    let (mut s, id) = line_session();
    set(&mut s, json!({"id": id.0, "t": 0.5, "left": 10, "right": 10}));
    let r = s.execute("stroke.widthPoint.copy", &json!({"id": id.0, "index": 1, "t": 0.25})).unwrap();
    assert_eq!(r["index"], json!(1));
    assert_eq!(points(&s, id), vec![(0.0, 1.0, 1.0), (0.25, 2.0, 2.0), (0.5, 2.0, 2.0), (1.0, 1.0, 1.0)]);
    assert!(s.execute("stroke.widthPoint.copy", &json!({"id": id.0, "t": 0.25})).is_err(), "needs index");
    // Alt-drag the centre of the point at 0.5 (x = 100) to x = 140: one more point, one undo step.
    let undo = s.doc().unwrap().history.undo.len();
    let v = ViewInfo::default();
    s.select_tool("width", v).unwrap();
    let alt = Mods { alt: true, ..Default::default() };
    for (kind, x) in [(PointerKind::Down, 100.0), (PointerKind::Drag, 120.0), (PointerKind::Drag, 140.0), (PointerKind::Up, 140.0)] {
        s.pointer(&PointerEvent::new(kind, x, 100.0).with_mods(alt), v).unwrap();
    }
    let pts = points(&s, id);
    assert_eq!(pts.len(), 5);
    assert!((pts[3].0 - 0.75).abs() < 1e-3 && pts[3].1 == 2.0 && pts[2] == (0.5, 2.0, 2.0), "{pts:?}");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, "Copy Width Point");
}

#[test]
fn shift_selected_points_drag_together_in_one_step() {
    let (mut s, id) = line_session();
    set(&mut s, json!({"id": id.0, "t": 0.25, "left": 5, "right": 5}));
    set(&mut s, json!({"id": id.0, "t": 0.5, "left": 5, "right": 5}));
    let v = ViewInfo::default();
    s.select_tool("width", v).unwrap();
    let (none, shift) = (Mods::default(), Mods { shift: true, ..Default::default() });
    let undo = s.doc().unwrap().history.undo.len();
    // Click the point at x = 60, Shift-click the one at x = 100, then drag the first 16 pt along.
    for (kind, x, m) in [
        (PointerKind::Down, 60.0, none),
        (PointerKind::Up, 60.0, none),
        (PointerKind::Down, 100.0, shift),
        (PointerKind::Up, 100.0, shift),
        (PointerKind::Down, 60.0, none),
        (PointerKind::Drag, 70.0, none),
        (PointerKind::Drag, 76.0, none),
        (PointerKind::Up, 76.0, none),
    ] {
        s.pointer(&PointerEvent::new(kind, x, 100.0).with_mods(m), v).unwrap();
    }
    let ts: Vec<f64> = points(&s, id).iter().map(|p| (p.0 * 100.0).round() / 100.0).collect();
    assert_eq!(ts, vec![0.0, 0.35, 0.6, 1.0]);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step for the drag (clicks change nothing)");
    // Delete removes both selected points.
    s.tool_key(vectorcraft_tools::ToolKey::Delete, none, v).unwrap();
    assert_eq!(points(&s, id).len(), 2);
}

#[test]
fn adjust_adjoining_scales_the_neighbours() {
    let (mut s, id) = line_session();
    set(&mut s, json!({"id": id.0, "t": 0.5, "left": 5, "right": 5}));
    // Doubling the middle point with Adjust Adjoining doubles the end points too.
    set(&mut s, json!({"id": id.0, "index": 1, "t": 0.5, "left": 10, "right": 5, "adjustAdjoining": true}));
    assert_eq!(points(&s, id), vec![(0.0, 2.0, 1.0), (0.5, 2.0, 1.0), (1.0, 2.0, 1.0)]);
    // Without it only the point changes.
    set(&mut s, json!({"id": id.0, "index": 1, "t": 0.5, "left": 20, "right": 5}));
    assert_eq!(points(&s, id), vec![(0.0, 2.0, 1.0), (0.5, 4.0, 1.0), (1.0, 2.0, 1.0)]);
    s.execute("stroke.widthPoint.remove", &json!({"id": id.0, "indices": [0, 2]})).unwrap();
    assert_eq!(points(&s, id), vec![(0.5, 4.0, 1.0)]);
}

#[test]
fn flipping_along_swaps_the_sides_of_a_discontinuous_point() {
    let (mut s, id) = line_session();
    s.execute("stroke.widthProfile.set", &json!({"ids": [id.0], "points": [[0, 1, 1], [0.25, 1, 1], [0.25, 2, 2], [1, 2, 2]]})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("stroke.setAdvanced", &json!({"flipProfile": "along"})).unwrap();
    assert_eq!(points(&s, id), vec![(0.0, 2.0, 2.0), (0.75, 2.0, 2.0), (0.75, 1.0, 1.0), (1.0, 1.0, 1.0)]);
}

fn new_point_adjoining(adjust: bool) {
    let (mut s, id) = line_session();
    s.execute("stroke.widthProfile.set", &json!({"ids": [id.0], "points": [[0, 1, 1], [1, 1, 1]]})).unwrap();
    s.doc_mut().unwrap().mark_saved();
    let before = s.doc().unwrap().doc.clone();
    let selection = s.doc().unwrap().selection.clone();
    let undo = s.doc().unwrap().history.undo.len();
    let i = set(&mut s, json!({"id": id.0, "index": null, "t": 0.5, "left": 10, "right": 10, "adjustAdjoining": adjust}));
    assert_eq!(i, 1);
    let end = if adjust { 2.0 } else { 1.0 };
    assert_eq!(points(&s, id), vec![(0.0, end, end), (0.5, 2.0, 2.0), (1.0, end, end)], "new-point adjoining choice must affect neighbours");
    let after = s.doc().unwrap().doc.clone();
    let st = s.doc().unwrap();
    assert_eq!(st.selection, selection);
    assert_eq!((st.history.undo.len(), st.history.redo.len(), st.is_dirty()), (undo + 1, 0, true));
    assert_eq!(st.history.undo.last().unwrap().label, "Width Point");
    s.execute("edit.undo", &json!({})).unwrap();
    let st = s.doc().unwrap();
    assert_eq!(st.doc, before);
    assert_eq!(st.selection, selection);
    assert_eq!((st.history.undo.len(), st.history.redo.len(), st.is_dirty()), (undo, 1, false));
    s.execute("edit.redo", &json!({})).unwrap();
    let st = s.doc().unwrap();
    assert_eq!(st.doc, after);
    assert_eq!(st.selection, selection);
    assert_eq!((st.history.undo.len(), st.history.redo.len(), st.is_dirty()), (undo + 1, 0, true));
}

#[test]
fn new_width_point_adjust_adjoining_scales_neighbours() {
    new_point_adjoining(true);
}

#[test]
fn new_width_point_without_adjoining_preserves_neighbours() {
    new_point_adjoining(false);
}

#[test]
fn new_width_point_adjust_adjoining_uses_sampled_widths() {
    for indexed in [false, true] {
        let (mut s, id) = line_session();
        s.execute("stroke.widthProfile.set", &json!({"ids": [id.0], "points": [[0, 9, 9], [0.25, 1, 0], [0.75, 3, 0], [1, 7, 7]]})).unwrap();
        // At t=0.5 the old sides are 10pt and 0pt. Indexed editing of that sampled
        // point is the existing convention: scale the nonzero side, add from zero.
        let index = if indexed { Some(set(&mut s, json!({"id": id.0, "t": 0.5, "left": 10, "right": 0}))) } else { None };
        set(&mut s, json!({"id": id.0, "index": index, "t": 0.5, "left": 20, "right": 5, "adjustAdjoining": true}));
        assert_eq!(
            points(&s, id),
            vec![(0.0, 9.0, 9.0), (0.25, 2.0, 1.0), (0.5, 4.0, 1.0), (0.75, 6.0, 1.0), (1.0, 7.0, 7.0)],
            "only nearest neighbours change, using the sampled sides; indexed={indexed}"
        );
    }
}

#[test]
fn width_point_finite_extreme_adjoining_roundtrips_with_history() {
    // Ordinary scaling, finite factors whose product overflows, and a representable
    // normalization with a subnormal half weight. Exercise both adjoining callers.
    for (weight, factor) in [(10.0, 1.0), (1e-200, 1e200), (f64::MIN_POSITIVE, 1.0)] {
        for indexed in [false, true] {
            let (mut s, id) = line_session();
            s.execute("stroke.set", &json!({"ids": [id.0], "weight": weight})).unwrap();
            let mut profile = vec![(0.0, factor, factor), (1.0, factor, factor)];
            if indexed {
                profile.insert(1, (0.5, factor, factor));
            }
            s.execute("stroke.widthProfile.set", &json!({"ids": [id.0], "points": profile})).unwrap();
            s.doc_mut().unwrap().mark_saved();
            let before = s.doc().unwrap().doc.clone();
            let selection = s.doc().unwrap().selection.clone();
            let undo = s.doc().unwrap().history.undo.len();
            let index = indexed.then_some(1);
            let side = weight * factor;
            set(&mut s, json!({"id": id.0, "index": index, "t": 0.5, "left": side, "right": side / 4.0, "adjustAdjoining": true}));
            let pts = points(&s, id);
            assert_eq!(pts.len(), 3);
            for (i, &(t, left, right)) in pts.iter().enumerate() {
                assert_eq!(t, i as f64 / 2.0);
                assert!(left.is_finite() && right.is_finite(), "adjoining factors must stay finite: {pts:?}");
                assert!((left / factor - 2.0).abs() < 1e-14 && (right / factor - 0.5).abs() < 1e-14, "{pts:?}");
            }
            let after = s.doc().unwrap().doc.clone();
            let st = s.doc().unwrap();
            assert_eq!(st.selection, selection);
            assert_eq!((st.history.undo.len(), st.history.redo.len(), st.is_dirty()), (undo + 1, 0, true));
            assert_eq!(st.history.undo.last().unwrap().label, "Width Point");
            let back = vectorcraft_format::load(&vectorcraft_format::save(&after, false)).unwrap();
            assert_eq!(back, *after, "finite width edits must round-trip through the native format");
            s.execute("edit.undo", &json!({})).unwrap();
            let st = s.doc().unwrap();
            assert_eq!(st.doc, before);
            assert_eq!(st.selection, selection);
            assert_eq!((st.history.undo.len(), st.history.redo.len(), st.is_dirty()), (undo, 1, false));
            s.execute("edit.redo", &json!({})).unwrap();
            let st = s.doc().unwrap();
            assert_eq!(st.doc, after);
            assert_eq!(st.selection, selection);
            assert_eq!((st.history.undo.len(), st.history.redo.len(), st.is_dirty()), (undo + 1, 0, true));
        }
    }
}

#[test]
fn width_point_finite_scaling_preserves_small_results() {
    // Blindly changing v*n/o to v*(n/o) underflows n/o to zero in this case.
    for indexed in [false, true] {
        let (mut s, id) = line_session();
        s.execute("stroke.set", &json!({"ids": [id.0], "weight": 2})).unwrap();
        let mut profile = vec![(0.0, 1e200, 1e200), (1.0, 1e200, 1e200)];
        if indexed {
            profile.insert(1, (0.5, 1e200, 1e200));
        }
        s.execute("stroke.widthProfile.set", &json!({"ids": [id.0], "points": profile})).unwrap();
        set(&mut s, json!({"id": id.0, "index": indexed.then_some(1), "t": 0.5, "left": 1e-200, "right": 1e-200, "adjustAdjoining": true}));
        for (_, left, right) in points(&s, id) {
            assert!(left > 0.0 && right > 0.0, "representable small widths must not become zero");
            assert!((left / 1e-200 - 1.0).abs() < 1e-14 && (right / 1e-200 - 1.0).abs() < 1e-14);
        }
    }
}

#[test]
fn width_point_nonrepresentable_widths_reject_atomically() {
    let history = |s: &Session| {
        let h = &s.doc().unwrap().history;
        let entries = |es: &[HistoryEntry]| es.iter().map(|e| (e.label.clone(), e.doc.clone(), e.selection.clone(), e.bytes)).collect::<Vec<_>>();
        (entries(&h.undo), entries(&h.redo), h.limit, h.byte_budget)
    };
    for (case, weight, ends, middle, left) in [
        ("normalized overflow", 1e-310, 1.0, 1.0, 1.0),
        ("half weight rounds to zero", f64::from_bits(1), 1.0, 1.0, 0.0),
        ("adjoining proportional overflow", 2.0, 1e308, 1.0, 4.0),
        ("adjoining additive overflow", 1e-308, 1e308, 0.0, 0.5),
    ] {
        for indexed in [false, true] {
            let (mut s, id) = line_session();
            s.execute("stroke.set", &json!({"ids": [id.0], "weight": weight})).unwrap();
            // The first neighbour can change before the second overflows; neither may commit.
            s.execute("stroke.widthProfile.set", &json!({"ids": [id.0], "points": [[0, 1, 1], [0.5, middle, 1], [1, ends, 1]]})).unwrap();
            s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
            let redo_doc = s.doc().unwrap().doc.clone();
            s.execute("edit.undo", &json!({})).unwrap();
            s.doc_mut().unwrap().mark_saved();
            let before = s.doc().unwrap().doc.clone();
            let selection = s.doc().unwrap().selection.clone();
            let revision = s.doc().unwrap().revision;
            let before_history = history(&s);
            assert_eq!(s.doc().unwrap().history.redo.len(), 1);
            let result = s.execute(
                "stroke.widthPoint.set",
                &json!({"id": id.0, "index": indexed.then_some(1), "t": 0.5, "left": left, "right": 0, "adjustAdjoining": true}),
            );
            assert!(result.is_err(), "{case}, indexed={indexed}: accepted nonrepresentable width factors");
            assert_eq!(s.doc().unwrap().doc, before, "{case}");
            assert_eq!(s.doc().unwrap().selection, selection, "{case}");
            assert_eq!(s.doc().unwrap().revision, revision, "{case}");
            assert_eq!(history(&s), before_history, "{case}");
            assert!(!s.doc().unwrap().is_dirty(), "{case}");
            s.execute("edit.redo", &json!({})).unwrap();
            assert_eq!(s.doc().unwrap().doc, redo_doc, "{case}: pre-existing redo remains usable");
            s.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(s.doc().unwrap().doc, before, "{case}");
        }
    }
}
