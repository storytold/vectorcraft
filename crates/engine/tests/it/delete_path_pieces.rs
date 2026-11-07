//! Original synthetic geometry for the deletion half of #3380; no external fixtures.

use serde_json::json;
use vectorcraft_doc::{Appearance, NodeId, NodeKind};
use vectorcraft_engine::Session;
use vectorcraft_geom::{Affine, Anchor, PathData, SubPath};
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
