//! Original synthetic geometry for the deletion half of #3380; no external fixtures.

use serde_json::json;
use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, Paint};
use vectorcraft_doc::{Appearance, NodeId, NodeKind, selection::Selection};
use vectorcraft_engine::Session;
use vectorcraft_geom::{Affine, Anchor, PathData, Rect, SubPath};
use vectorcraft_testkit::{invariants::check_session, raster::render_artboard};

/// Create the same rectangle for the edited path and its stacking-order neighbors.
fn rectangle(s: &mut Session) -> NodeId {
    NodeId(s.execute("shape.rectangle", &json!({"x": 30, "y": 30, "width": 80, "height": 60})).unwrap()["id"].as_u64().unwrap())
}

/// Start a small document suitable for exact geometry and rendered-history assertions.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    s
}

/// Clear, explicit anchor deletion and Cut preserve styled survivors, sibling order and history.
#[test]
fn deleting_opposite_rectangle_segments_makes_independent_styled_objects_and_undoes_once() {
    for command in ["edit.clear", "path.deleteAnchors", "edit.cut"] {
        let mut s = session();
        let below = rectangle(&mut s);
        let id = rectangle(&mut s);
        let above = rectangle(&mut s);
        s.execute("select.set", &json!({"ids": [below.0, id.0, above.0]})).unwrap();
        let group = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
        {
            let n = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(id).unwrap();
            n.name = Some("Styled rectangle".into());
            n.opacity = 0.65;
            n.transform(Affine::translate((12.0, 8.0)) * Affine::rotate(0.23) * Affine::scale_non_uniform(1.1, 0.8), true);
            n.appearance =
                Appearance::basic(vectorcraft_color::Paint::None, vectorcraft_color::Paint::solid(vectorcraft_color::Color::rgb(0.2, 0.4, 0.8)), 4.0)
                    .into();
        }
        s.execute("select.segments", &json!({"id": id.0, "segments": [[0, 1], [0, 3]]})).unwrap();
        let original = s.doc().unwrap().doc.node(id).unwrap().clone();
        let before_doc = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
        let before_selection = s.doc().unwrap().selection.clone();
        let history = s.doc().unwrap().history.undo.len();
        s.execute(command, &json!({})).unwrap();
        let st = s.doc().unwrap();
        let children = st.doc.node(group).unwrap().children().unwrap();
        assert_eq!(children.len(), 4, "{command}: two independent pieces between the existing siblings");
        assert_eq!([children[0].id, children[1].id, children[3].id], [below, id, above]);
        let ids = [id, children[2].id];
        assert_eq!(st.selection.objects, ids);
        assert!(st.selection.anchors.is_empty() && st.selection.segments.is_empty());
        assert_eq!(st.history.undo.len(), history + 1);
        // The two surviving sides, in contour order, carry every node property and baked transform.
        let anchors = &original.path_data().unwrap().subpaths[0].anchors;
        for (piece, indexes) in ids.into_iter().zip([[2, 3], [0, 1]]) {
            let mut expected = original.clone();
            expected.id = piece;
            let mut run = vec![anchors[indexes[0]], anchors[indexes[1]]];
            run[0].h_in = run[0].p;
            run[1].h_out = run[1].p;
            if let NodeKind::Path { path, live, .. } = &mut expected.kind {
                *path = PathData::single(SubPath::new(run, false));
                *live = None;
            }
            assert_eq!(st.doc.node(piece).unwrap(), &expected, "{command}");
        }
        let rendered = render_artboard(&st.doc);
        let after_doc = serde_json::to_value(&st.doc).unwrap();
        let after_selection = st.selection.clone();
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), before_doc);
        assert_eq!(s.doc().unwrap().selection, before_selection);
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), after_doc);
        assert_eq!(s.doc().unwrap().selection, after_selection);
        assert_eq!(render_artboard(&s.doc().unwrap().doc), rendered);
        check_session(&s).unwrap();
        s.execute("edit.clear", &json!({})).unwrap();
        assert!(ids.iter().all(|id| s.doc().unwrap().doc.node(*id).is_none()));
        assert!(s.doc().unwrap().doc.node(below).is_some() && s.doc().unwrap().doc.node(above).is_some());
    }
}

/// Removing separated anchors retains each cubic run and retracts only its unused endpoint handles.
#[test]
fn deleting_separated_anchors_splits_an_open_path_and_keeps_cubic_handles() {
    let mut s = session();
    let id = rectangle(&mut s);
    let anchors: Vec<_> = (0..8)
        .map(|i| {
            let mut a = Anchor::corner((20.0 + f64::from(i) * 15.0, 50.0).into());
            a.h_in = a.p + (-4.0, -8.0);
            a.h_out = a.p + (4.0, 8.0);
            a
        })
        .collect();
    *std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(id).unwrap().path_data_mut().unwrap() =
        PathData::single(SubPath::new(anchors.clone(), false));
    s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 2], [0, 5]]})).unwrap();
    s.execute("edit.clear", &json!({})).unwrap();
    let st = s.doc().unwrap();
    assert_eq!(st.selection.objects.len(), 3);
    for (piece, start) in st.selection.objects.iter().zip([0, 3, 6]) {
        let pd = st.doc.node(*piece).unwrap().path_data().unwrap();
        assert_eq!(pd.subpaths.len(), 1);
        let sp = &pd.subpaths[0];
        assert!(!sp.closed);
        assert_eq!(sp.anchors.len(), 2);
        assert_eq!(sp.anchors[0].p, anchors[start].p);
        assert_eq!(sp.anchors[0].h_out, anchors[start].h_out);
        assert_eq!(sp.anchors[1].h_in, anchors[start + 1].h_in);
        assert_eq!(sp.anchors[0].h_in, sp.anchors[0].p);
        assert_eq!(sp.anchors[1].h_out, sp.anchors[1].p);
    }
    check_session(&s).unwrap();
}

/// Existing multi-subpath paths and explicit compound members retain structure and exact geometry.
#[test]
fn deletion_preserves_intentional_multi_subpaths_and_compound_members() {
    for command in ["edit.clear", "edit.cut"] {
        for compound in [false, true] {
            let mut s = session();
            let id = rectangle(&mut s);
            let other = rectangle(&mut s);
            std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(other).unwrap().transform(Affine::translate((110.0, 0.0)), true);
            let extra = s.doc().unwrap().doc.node(other).unwrap().path_data().unwrap().subpaths[0].clone();
            let original = s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().subpaths[0].clone();
            assert_ne!(extra, original, "the untouched contour must be distinguishable from the edited one");
            let container = if compound {
                s.execute("select.set", &json!({"ids": [id.0, other.0]})).unwrap();
                NodeId(s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap())
            } else {
                std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(id).unwrap().path_data_mut().unwrap().subpaths.push(extra.clone());
                s.doc().unwrap().doc.parent_of(id).unwrap()
            };
            let children = s.doc().unwrap().doc.node(container).unwrap().children().unwrap().len();
            let sibling = s.doc().unwrap().doc.node(other).unwrap().clone();
            let original = s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().subpaths[0].clone();
            let mut expected = vec![
                SubPath::new(vec![original.anchors[2], original.anchors[3]], false),
                SubPath::new(vec![original.anchors[0], original.anchors[1]], false),
            ];
            if !compound {
                expected.push(extra);
            }
            s.execute("select.segments", &json!({"id": id.0, "segments": [[0, 1], [0, 3]]})).unwrap();
            s.execute(command, &json!({})).unwrap();
            let st = s.doc().unwrap();
            assert_eq!(st.doc.node(container).unwrap().children().unwrap().len(), children);
            assert_eq!(st.doc.node(id).unwrap().path_data().unwrap().subpaths.len(), if compound { 2 } else { 3 });
            assert_eq!(st.doc.node(id).unwrap().path_data().unwrap().subpaths, expected, "{command}, compound: {compound}");
            assert_eq!(st.doc.node(other).unwrap(), &sibling);
            assert_eq!(st.selection.objects, [id]);
            check_session(&s).unwrap();
        }
    }
}

/// Splitting keeps the original gradient fit on every piece and leaves placed vectors unchanged.
#[test]
fn deletion_split_preserves_gradient_placement() {
    for command in ["edit.clear", "path.deleteAnchors", "edit.cut"] {
        for kind in [GradientKind::Linear, GradientKind::Radial] {
            for placed in [false, true] {
                let mut s = session();
                let id = rectangle(&mut s);
                let mut fill = GradientPaint::new(Gradient { kind, ..Default::default() });
                fill.angle = 45.0;
                let mut stroke = GradientPaint::new(Gradient { kind, ..Default::default() });
                stroke.angle = 90.0;
                if placed {
                    fill.geom = Some(GradientGeom::fit(kind, Rect::new(20.0, 10.0, 160.0, 150.0), 35.0));
                    stroke.geom = Some(GradientGeom::fit(kind, Rect::new(10.0, 20.0, 150.0, 160.0), -25.0));
                }
                {
                    let n = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(id).unwrap();
                    n.transform(Affine::translate((10.0, 5.0)) * Affine::rotate(0.15), false);
                    n.appearance = Appearance::basic(Paint::Gradient(Box::new(fill.clone())), Paint::Gradient(Box::new(stroke.clone())), 8.0).into();
                }
                s.execute("select.segments", &json!({"id": id.0, "segments": [[0, 1], [0, 3]]})).unwrap();
                let st = s.doc().unwrap();
                let before = st.doc.clone();
                let before_selection = st.selection.clone();
                let history = st.history.undo.len();
                let original = before.node(id).unwrap();
                let bounds = original.geometric_bounds().unwrap();
                let fill_geom = fill.resolve(bounds);
                let stroke_geom = stroke.resolve(bounds.inflate(4.0, 4.0));
                // The old single-node remainder keeps the original full bounds. Its rendering is
                // an independent reference for the two new objects, including unplaced gradients.
                let mut expected = (*before).clone();
                let a = &original.path_data().unwrap().subpaths[0].anchors;
                if let NodeKind::Path { path, live, .. } = &mut expected.node_mut(id).unwrap().kind {
                    *path = PathData::new(vec![SubPath::new(vec![a[2], a[3]], false), SubPath::new(vec![a[0], a[1]], false)]);
                    *live = None;
                }
                let expected_image = render_artboard(&expected);
                assert!(expected_image.ink() > 0);
                s.execute(command, &json!({})).unwrap();
                let st = s.doc().unwrap();
                assert_eq!(st.selection.objects.len(), 2);
                assert_eq!(st.history.undo.len(), history + 1);
                assert!(render_artboard(&st.doc) == expected_image, "{command}, {kind:?}, placed: {placed}: gradient colors moved");
                for piece in &st.selection.objects {
                    let n = st.doc.node(*piece).unwrap();
                    for (paint, expected_geom) in [(n.appearance.fill_paint(), fill_geom), (n.appearance.stroke_paint(), stroke_geom)] {
                        let Paint::Gradient(g) = paint else { panic!("gradient was replaced") };
                        assert_eq!(g.geom, Some(expected_geom), "{command}, {kind:?}, placed: {placed}");
                    }
                }
                let after = st.doc.clone();
                let after_selection = st.selection.clone();
                s.execute("edit.undo", &json!({})).unwrap();
                assert_eq!(s.doc().unwrap().doc, before);
                assert_eq!(s.doc().unwrap().selection, before_selection);
                s.execute("edit.redo", &json!({})).unwrap();
                assert_eq!(s.doc().unwrap().doc, after);
                assert_eq!(s.doc().unwrap().selection, after_selection);
                assert!(render_artboard(&s.doc().unwrap().doc) == expected_image);
                check_session(&s).unwrap();
            }
        }
    }
}

/// Subpath `i` of a multi-subpath test path: four cubic anchors on their own row, so every piece
/// that survives a deletion can be matched exactly.
fn row(i: usize, closed: bool) -> SubPath {
    let y = 10.0 + 3.0 * i as f64;
    let anchors = (0..4)
        .map(|k| {
            let mut a = Anchor::corner((10.0 + 20.0 * f64::from(k), y).into());
            a.h_in = a.p + (-3.0, -1.0);
            a.h_out = a.p + (3.0, 1.0);
            a
        })
        .collect();
    SubPath::new(anchors, closed)
}

/// An open piece made of `indexes` of `sp`, its unused end handles retracted as deletion leaves them.
fn piece(sp: &SubPath, indexes: &[usize]) -> SubPath {
    let mut run: Vec<Anchor> = indexes.iter().map(|&i| sp.anchors[i]).collect();
    if let Some(first) = run.first_mut() {
        first.h_in = first.p;
    }
    if let Some(last) = run.last_mut() {
        last.h_out = last.p;
    }
    SubPath::new(run, false)
}

/// Put `subpaths` into a new path object and return its ID.
fn multi_subpath_path(s: &mut Session, subpaths: Vec<SubPath>) -> NodeId {
    let id = rectangle(s);
    *std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(id).unwrap().path_data_mut().unwrap() = PathData::new(subpaths);
    id
}

/// Run `command` on the current selection, check the path's surviving subpaths and that one Undo
/// restores the document and `before_selection` and one Redo repeats the edit, then check that a
/// second Clear removes the survivors.
fn delete_and_check(s: &mut Session, command: &str, id: NodeId, expected: &[SubPath], before_selection: &Selection) {
    let before_doc = serde_json::to_value(&s.doc().unwrap().doc).unwrap();
    let history = s.doc().unwrap().history.undo.len();
    s.execute(command, &json!({})).unwrap();
    let st = s.doc().unwrap();
    assert_eq!(st.doc.node(id).unwrap().path_data().unwrap().subpaths, expected, "{command}");
    assert_eq!(st.selection.objects, [id], "{command}: a multi-subpath path keeps its object");
    assert!(st.selection.anchors.is_empty() && st.selection.segments.is_empty());
    assert_eq!(st.history.undo.len(), history + 1);
    let after_doc = serde_json::to_value(&st.doc).unwrap();
    let after_selection = st.selection.clone();
    check_session(s).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), before_doc);
    assert_eq!(&s.doc().unwrap().selection, before_selection);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(serde_json::to_value(&s.doc().unwrap().doc).unwrap(), after_doc);
    assert_eq!(s.doc().unwrap().selection, after_selection);
    s.execute("edit.clear", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(id).is_none(), "{command}: the second Clear removes the survivors");
}

/// Selected anchors on many subpaths each remove only their own subpath's anchors; anchors named on
/// a subpath or index the path does not have remove nothing.
#[test]
fn deleting_anchors_on_many_subpaths_removes_each_subpaths_own_anchors() {
    for command in ["edit.clear", "path.deleteAnchors", "edit.cut"] {
        let mut s = session();
        let subpaths: Vec<SubPath> = (0..6).map(|i| row(i, i % 2 == 1)).collect();
        let id = multi_subpath_path(&mut s, subpaths.clone());
        s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 1], [1, 2], [2, 0], [2, 9], [4, 3], [5, 1], [7, 1]]})).unwrap();
        // Subpath 3 has no selected anchor and stays closed and unchanged.
        let expected = [
            piece(&subpaths[0], &[2, 3]),
            piece(&subpaths[1], &[3, 0, 1]),
            piece(&subpaths[2], &[1, 2, 3]),
            subpaths[3].clone(),
            piece(&subpaths[4], &[0, 1, 2]),
            piece(&subpaths[5], &[2, 3, 0]),
        ];
        let before = s.doc().unwrap().selection.clone();
        delete_and_check(&mut s, command, id, &expected, &before);
    }
}

/// Selected segments on many subpaths each cut only their own subpath; a segment reference to a
/// subpath or segment the path does not have cuts nothing.
#[test]
fn deleting_segments_on_many_subpaths_cuts_each_subpaths_own_segments() {
    for command in ["edit.clear", "path.deleteAnchors", "edit.cut"] {
        let mut s = session();
        let subpaths: Vec<SubPath> = (0..6).map(|i| row(i, i % 2 == 1)).collect();
        let id = multi_subpath_path(&mut s, subpaths.clone());
        s.execute("select.segments", &json!({"id": id.0, "segments": [[0, 1], [1, 3], [4, 0], [5, 1]]})).unwrap();
        // Undo prunes the restored selection (`Selection::prune`), which drops references to
        // segments the path does not have, so it restores the selection as it is here.
        let before = s.doc().unwrap().selection.clone();
        // `select.segments` refuses references to missing segments, so these are added directly.
        let refs = s.doc_mut().unwrap().selection.segments.get_mut(&id).unwrap();
        refs.extend([(2, 9), (7, 0)]);
        let expected = [
            piece(&subpaths[0], &[0, 1]),
            piece(&subpaths[0], &[2, 3]),
            piece(&subpaths[1], &[0, 1, 2, 3]),
            subpaths[2].clone(),
            subpaths[3].clone(),
            piece(&subpaths[4], &[1, 2, 3]),
            piece(&subpaths[5], &[2, 3, 0, 1]),
        ];
        delete_and_check(&mut s, command, id, &expected, &before);
    }
}

/// One selected interior anchor on each of many subpaths removes exactly that anchor from each,
/// keeping the subpaths in their original order.
#[test]
fn deleting_one_anchor_on_each_of_many_subpaths_keeps_subpath_order() {
    let mut s = session();
    let subpaths: Vec<SubPath> = (0..400).map(|i| row(i, false)).collect();
    let id = multi_subpath_path(&mut s, subpaths.clone());
    let refs: Vec<[usize; 2]> = (0..400).filter(|i| i % 7 != 3).map(|i| [i, 1]).collect();
    s.execute("select.anchors", &json!({"id": id.0, "anchors": refs})).unwrap();
    let expected: Vec<SubPath> = subpaths.iter().enumerate().map(|(i, sp)| if i % 7 == 3 { sp.clone() } else { piece(sp, &[2, 3]) }).collect();
    let before = s.doc().unwrap().selection.clone();
    delete_and_check(&mut s, "edit.clear", id, &expected, &before);
}

/// Cut Path cuts each subpath of a multi-subpath path only at that subpath's selected anchors.
#[test]
fn cutting_at_anchors_on_many_subpaths_cuts_each_subpath_at_its_own_anchors() {
    let mut s = session();
    let subpaths: Vec<SubPath> = (0..6).map(|i| row(i, i % 2 == 1)).collect();
    let id = multi_subpath_path(&mut s, subpaths.clone());
    let refs = [(0, 1), (1, 2), (2, 9), (4, 2), (5, 0), (5, 2), (7, 1)];
    s.execute("select.anchors", &json!({"id": id.0, "anchors": refs.map(|(a, b)| [a, b])})).unwrap();
    s.execute("path.cutAtAnchors", &json!({})).unwrap();
    let expected: Vec<SubPath> = subpaths
        .iter()
        .enumerate()
        .flat_map(|(si, sp)| sp.cut_at(&refs.iter().filter(|r| r.0 == si).map(|r| r.1).collect::<std::collections::BTreeSet<_>>()))
        .collect();
    // Subpaths 0 and 4 become two pieces each, closed subpath 1 opens into one, closed subpath 5
    // becomes two, and subpaths 2 and 3 stay as they were.
    assert_eq!(expected.len(), 9);
    assert_eq!(s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().subpaths, expected);
    check_session(&s).unwrap();
}

/// A connected remainder keeps automatic fitting; pinning is needed only for separate objects.
#[test]
fn deleting_one_segment_keeps_unplaced_gradients() {
    let mut s = session();
    let id = rectangle(&mut s);
    let paint = Paint::Gradient(Box::new(GradientPaint::new(Default::default())));
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(id).unwrap().appearance = Appearance::basic(paint.clone(), paint, 8.0).into();
    let appearance = s.doc().unwrap().doc.node(id).unwrap().appearance.clone();
    s.execute("select.segments", &json!({"id": id.0, "segments": [[0, 1]]})).unwrap();
    s.execute("edit.clear", &json!({})).unwrap();
    let st = s.doc().unwrap();
    assert_eq!(st.selection.objects, [id]);
    assert_eq!(st.doc.node(id).unwrap().appearance, appearance);
    assert_eq!(st.doc.node(id).unwrap().path_data().unwrap().subpaths.len(), 1);
}
