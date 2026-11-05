//! Tests for `addon.contour.set`: a live Offset Path copy of a shape that stays editable until baked.

use serde_json::{Value, json};
use vectorcraft_doc::NodeKind;
use vectorcraft_geom::Rect;

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    let r = s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

fn ids_of(v: &Value, key: &str) -> Vec<NodeId> {
    v[key].as_array().unwrap().iter().map(|x| NodeId(x.as_u64().unwrap())).collect()
}

fn node(s: &Session, id: NodeId) -> vectorcraft_doc::Node {
    s.doc().unwrap().doc.node(id).cloned().unwrap()
}

fn effects_of(s: &Session, id: NodeId) -> Vec<(String, Value)> {
    node(s, id).appearance.effects.iter().map(|e| (e.id.clone(), e.params.clone())).collect()
}

fn bounds(s: &Session, id: NodeId) -> Rect {
    node(s, id).geometric_bounds().unwrap()
}

fn node_count(s: &Session) -> usize {
    s.doc().unwrap().doc.node_count()
}

fn select(s: &mut Session, ids: &[NodeId]) {
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
}

#[test]
fn contour_is_a_live_copy_above_the_original() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 50.0, 40.0);
    let before = bounds(&s, a);
    let r = s.execute("addon.contour.set", &json!({"offset": 10})).unwrap();
    let (made, updated) = (ids_of(&r, "created"), ids_of(&r, "updated"));
    assert_eq!((made.len(), updated.len()), (1, 0));
    // The original is untouched, the copy is above it and selected, and nothing is baked: the
    // copy's own path is still the original's, with the effect on top.
    assert!(effects_of(&s, a).is_empty());
    assert_eq!(bounds(&s, a), before);
    assert_eq!(s.doc().unwrap().selection.objects, made);
    let d = &s.doc().unwrap().doc;
    assert!(d.index_path(made[0]).unwrap() > d.index_path(a).unwrap());
    assert_eq!(bounds(&s, made[0]), before);
    let fx = effects_of(&s, made[0]);
    assert_eq!(fx.len(), 1);
    assert_eq!(fx[0].0, "path.offsetPath");
    assert_eq!(fx[0].1["offset"], json!(10.0));
    assert_eq!(fx[0].1["joins"], json!("miter"));
}

#[test]
fn baking_a_contour_gives_the_grown_and_the_shrunk_shape() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 50.0, 40.0);
    for (offset, want) in [(10.0, Rect::new(90.0, 90.0, 160.0, 150.0)), (-5.0, Rect::new(105.0, 105.0, 145.0, 135.0))] {
        select(&mut s, &[a]);
        s.execute("addon.contour.set", &json!({"offset": offset})).unwrap();
        let baked = ids_of(&s.execute("effect.expandAppearance", &json!({})).unwrap(), "ids");
        let b = bounds(&s, baked[0]);
        // The default stroke widens the baked art by its weight; compare the fill's edges.
        let fill = baked_fill_bounds(&s, baked[0]);
        assert!((fill.x0 - want.x0).abs() < 1e-2 && (fill.y1 - want.y1).abs() < 1e-2, "{offset}: {fill:?} vs {want:?} ({b:?})");
    }
}

/// The bounds of the fill path inside a baked object (a group of its fill and stroke), or of the
/// object itself when it is one path.
fn baked_fill_bounds(s: &Session, id: NodeId) -> Rect {
    match &node(s, id).kind {
        NodeKind::Group { children, .. } => children.iter().filter_map(|c| c.geometric_bounds()).next().unwrap(),
        _ => bounds(s, id),
    }
}

#[test]
fn a_second_drag_changes_the_contour_instead_of_nesting_one() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 50.0, 40.0);
    let made = ids_of(&s.execute("addon.contour.set", &json!({"offset": 10})).unwrap(), "created");
    let count = node_count(&s);
    // The Contour is selected: the next call edits it.
    let r = s.execute("addon.contour.set", &json!({"offset": -4, "corners": "round"})).unwrap();
    assert_eq!(ids_of(&r, "updated"), made);
    assert!(ids_of(&r, "created").is_empty());
    assert_eq!(node_count(&s), count, "no new object");
    let fx = effects_of(&s, made[0]);
    assert_eq!(fx.len(), 1, "one Offset Path, not two");
    assert_eq!((fx[0].1["offset"].clone(), fx[0].1["joins"].clone()), (json!(-4.0), json!("round")));
    assert!(effects_of(&s, a).is_empty());
}

#[test]
fn corner_styles_choose_the_join() {
    for (corners, joins) in [("sharp", "miter"), ("round", "round"), ("square", "bevel"), ("Round", "round")] {
        let mut s = session();
        rect(&mut s, 0.0, 0.0, 30.0, 30.0);
        let made = ids_of(&s.execute("addon.contour.set", &json!({"offset": 3, "corners": corners})).unwrap(), "created");
        assert_eq!(effects_of(&s, made[0])[0].1["joins"], json!(joins), "{corners}");
    }
}

#[test]
fn contour_keeps_the_color_and_stroke_editable_on_the_copy() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 30.0, 30.0);
    let made = ids_of(&s.execute("addon.contour.set", &json!({"offset": 6})).unwrap(), "created");
    let original = node(&s, a).appearance.items.clone();
    assert_eq!(node(&s, made[0]).appearance.items, original, "the copy starts with the shape's own paint");
    // Recolouring the Contour (it is the selection) changes its fill only: the live effect stays
    // and the original keeps its paint.
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    assert_ne!(node(&s, made[0]).appearance.items, original, "the copy took the new fill");
    assert_eq!(node(&s, a).appearance.items, original, "the original is untouched");
    assert_eq!(effects_of(&s, made[0]).len(), 1);
}

#[test]
fn contour_is_one_undo_step() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 30.0, 30.0);
    let count = node_count(&s);
    s.execute("addon.contour.set", &json!({"offset": 6})).unwrap();
    assert_eq!(node_count(&s), count + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(node_count(&s), count);
    assert!(effects_of(&s, a).is_empty());
}

#[test]
fn contour_refuses_what_it_cannot_offset() {
    let mut s = session();
    rect(&mut s, 0.0, 0.0, 30.0, 30.0);
    let count = node_count(&s);
    for params in [json!({}), json!({"offset": "abc"}), json!({"offset": 1e300}), json!({"offset": 5, "corners": "pointy"})] {
        assert!(s.execute("addon.contour.set", &params).is_err(), "{params}");
    }
    // Nothing was added by the refused calls.
    assert_eq!(node_count(&s), count);
    // Type is not a path to offset.
    s.execute("text.create", &json!({"x": 10, "y": 10, "text": "Hi"})).unwrap();
    let err = s.execute("addon.contour.set", &json!({"offset": 5})).unwrap_err().to_string();
    assert!(err.contains("select paths or shapes"), "{err}");
}

/// The border-with-a-gap recipe: two Contours of one shape (the gap, then the border), baked, then
/// the gap taken out of the border with Minus Front, leaves a ring between the two.
#[test]
fn border_with_a_gap_from_two_contours_and_minus_front() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 50.0, 40.0);
    // Strip the default stroke so the areas are plain.
    s.execute("paint.setStroke", &json!({"none": true})).ok();
    // The gap first (+8), then the border (+20) from the same shape: both land directly above it,
    // the later one lower, so the gap ends up in front of the border.
    select(&mut s, &[a]);
    let gap = ids_of(&s.execute("addon.contour.set", &json!({"offset": 8})).unwrap(), "created")[0];
    select(&mut s, &[a]);
    let border = ids_of(&s.execute("addon.contour.set", &json!({"offset": 20})).unwrap(), "created")[0];
    let d = &s.doc().unwrap().doc;
    assert!(d.index_path(gap).unwrap() > d.index_path(border).unwrap(), "the gap is in front of the border");
    select(&mut s, &[border, gap]);
    s.execute("effect.expandAppearance", &json!({})).unwrap();
    let both: Vec<NodeId> = s.doc().unwrap().selection.objects.to_vec();
    assert_eq!(both.len(), 2);
    let ring = ids_of(&s.execute("object.pathfinder.minusFront", &json!({})).unwrap(), "ids");
    assert_eq!(ring.len(), 1);
    // 90 x 80 outer minus 66 x 56 inner, with sharp corners.
    let area = vectorcraft_pathops::area(&ring_path(&s, ring[0]).0, ring_path(&s, ring[0]).1);
    assert!((area - (90.0 * 80.0 - 66.0 * 56.0)).abs() < 1.0, "{area}");
}

fn ring_path(s: &Session, id: NodeId) -> (vectorcraft_geom::PathData, vectorcraft_geom::FillRule) {
    match &node(s, id).kind {
        NodeKind::Path { path, rule, .. } => (path.clone(), *rule),
        NodeKind::Compound { children, rule } => {
            (vectorcraft_geom::PathData::new(children.iter().flat_map(|c| c.path_data().unwrap().subpaths.clone()).collect()), *rule)
        }
        NodeKind::Group { children, .. } => children.first().map(|c| ring_path(s, c.id)).unwrap(),
        _ => panic!("not a path"),
    }
}

mod tool {
    use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

    use super::*;
    use crate::tooling::ViewInfo;

    fn view() -> ViewInfo {
        ViewInfo { smart_guides: false, ..Default::default() }
    }

    fn at(s: &mut Session, kind: PointerKind, x: f64, y: f64, mods: Mods) {
        s.pointer(&PointerEvent::new(kind, x, y).with_mods(mods), view()).unwrap();
    }

    fn undo_depth(s: &Session) -> usize {
        s.doc().unwrap().history.undo.len()
    }

    /// A 50 x 40 rectangle at (100, 100), selected, with the Contour tool active.
    fn ready() -> (Session, NodeId) {
        let mut s = session();
        let a = rect(&mut s, 100.0, 100.0, 50.0, 40.0);
        select(&mut s, &[a]);
        s.select_tool("contour", view()).unwrap();
        (s, a)
    }

    fn offset_of(s: &Session, id: NodeId) -> f64 {
        effects_of(s, id)[0].1["offset"].as_f64().unwrap()
    }

    #[test]
    fn dragging_out_previews_live_and_commits_one_undo_step() {
        let (mut s, a) = ready();
        let (before, nodes) = (undo_depth(&s), node_count(&s));
        // Press on the right edge and pull 10 pt away from the shape.
        at(&mut s, PointerKind::Down, 150.0, 120.0, Mods::default());
        at(&mut s, PointerKind::Drag, 160.0, 120.0, Mods::default());
        assert!(s.in_interaction(), "the Contour is previewed while the button is down");
        at(&mut s, PointerKind::Up, 160.0, 120.0, Mods::default());
        assert!(!s.in_interaction());
        assert_eq!(undo_depth(&s), before + 1);
        assert_eq!(node_count(&s), nodes + 1);
        assert_eq!(s.journal.last().map(|j| j.0.as_str()), Some("addon.contour.set"));
        let made = s.doc().unwrap().selection.objects.clone();
        assert_eq!(made.len(), 1);
        assert_ne!(made[0], a);
        assert!((offset_of(&s, made[0]) - 10.0).abs() < 1e-6, "{}", offset_of(&s, made[0]));
        assert!(effects_of(&s, a).is_empty(), "the original is kept as it was");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(node_count(&s), nodes);
    }

    #[test]
    fn dragging_in_shrinks_and_a_second_drag_continues_from_where_the_contour_stands() {
        let (mut s, _) = ready();
        // From inside, 5 pt toward the middle of the right edge's neighbourhood.
        at(&mut s, PointerKind::Down, 145.0, 120.0, Mods::default());
        at(&mut s, PointerKind::Drag, 140.0, 120.0, Mods::default());
        at(&mut s, PointerKind::Up, 140.0, 120.0, Mods::default());
        let contour = s.doc().unwrap().selection.objects[0];
        assert!((offset_of(&s, contour) + 5.0).abs() < 1e-6);
        // The Contour is selected: pressing at the edge and pulling out 12 pt adds to its -5.
        let nodes = node_count(&s);
        at(&mut s, PointerKind::Down, 150.0, 120.0, Mods::default());
        at(&mut s, PointerKind::Drag, 162.0, 120.0, Mods::default());
        at(&mut s, PointerKind::Up, 162.0, 120.0, Mods::default());
        assert_eq!(node_count(&s), nodes, "the same Contour changed, none was nested");
        assert_eq!(s.doc().unwrap().selection.objects, vec![contour]);
        assert_eq!(effects_of(&s, contour).len(), 1);
        assert!((offset_of(&s, contour) - 7.0).abs() < 1e-6, "{}", offset_of(&s, contour));
    }

    #[test]
    fn escape_and_a_plain_click_change_nothing() {
        let (mut s, _) = ready();
        let (before, nodes) = (undo_depth(&s), node_count(&s));
        at(&mut s, PointerKind::Down, 150.0, 120.0, Mods::default());
        at(&mut s, PointerKind::Drag, 170.0, 120.0, Mods::default());
        assert!(s.in_interaction());
        s.tool_key(ToolKey::Escape, Mods::default(), view()).unwrap();
        at(&mut s, PointerKind::Up, 170.0, 120.0, Mods::default());
        assert!(!s.in_interaction());
        assert_eq!((undo_depth(&s), node_count(&s)), (before, nodes));
        // A click with no movement.
        at(&mut s, PointerKind::Down, 150.0, 120.0, Mods::default());
        at(&mut s, PointerKind::Up, 150.0, 120.0, Mods::default());
        assert_eq!((undo_depth(&s), node_count(&s)), (before, nodes));
    }

    #[test]
    fn shift_rounds_to_whole_points_and_the_corner_option_is_used() {
        let (mut s, _) = ready();
        s.set_tool_option("corners", &json!("round"));
        let shift = Mods { shift: true, ..Mods::default() };
        at(&mut s, PointerKind::Down, 150.0, 120.0, shift);
        at(&mut s, PointerKind::Drag, 160.4, 120.0, shift);
        at(&mut s, PointerKind::Up, 160.4, 120.0, shift);
        let contour = s.doc().unwrap().selection.objects[0];
        assert_eq!(offset_of(&s, contour), 10.0);
        assert_eq!(effects_of(&s, contour)[0].1["joins"], json!("round"));
    }

    /// A Contour keeps its own corner style and miter limit when it is dragged again, whatever the
    /// tool's options are by then (they said Sharp when the Contour was made Square).
    #[test]
    fn dragging_a_contour_again_keeps_its_corners_and_miter_limit() {
        let (mut s, _) = ready();
        s.execute("addon.contour.set", &json!({"offset": 5, "corners": "square", "miterLimit": 7})).unwrap();
        let contour = s.doc().unwrap().selection.objects[0];
        s.set_tool_option("corners", &json!("sharp"));
        at(&mut s, PointerKind::Down, 150.0, 120.0, Mods::default());
        at(&mut s, PointerKind::Drag, 158.0, 120.0, Mods::default());
        at(&mut s, PointerKind::Up, 158.0, 120.0, Mods::default());
        let fx = effects_of(&s, contour);
        assert_eq!(fx.len(), 1);
        assert!((offset_of(&s, contour) - 13.0).abs() < 1e-6, "{}", offset_of(&s, contour));
        assert_eq!(fx[0].1["joins"], json!("bevel"), "{:?}", fx[0].1);
        assert_eq!(fx[0].1["miterLimit"], json!(7.0), "{:?}", fx[0].1);
    }

    /// A drag that never moves the distance (along the edge, or rounded back to nothing by Shift)
    /// leaves no unchanged copy and no undo step.
    #[test]
    fn a_drag_along_the_edge_changes_nothing() {
        let (mut s, _) = ready();
        let (before, nodes) = (undo_depth(&s), node_count(&s));
        let shift = Mods { shift: true, ..Mods::default() };
        for (mods, to) in [(Mods::default(), (150.0, 135.0)), (shift, (150.3, 120.0))] {
            at(&mut s, PointerKind::Down, 150.0, 120.0, mods);
            at(&mut s, PointerKind::Drag, to.0, to.1, mods);
            at(&mut s, PointerKind::Up, to.0, to.1, mods);
            assert_eq!((undo_depth(&s), node_count(&s)), (before, nodes), "{to:?}");
            assert!(!s.in_interaction());
        }
    }

    /// The outline a Contour was made from shows as a dashed guide over a solid halo (so it reads
    /// over any fill); a shape that is not a Contour yet has none.
    #[test]
    fn a_selected_contour_shows_the_outline_it_was_made_from() {
        use vectorcraft_tools::Overlay;
        let (mut s, _) = ready();
        let paths = |s: &mut Session| -> Vec<(bool, f32)> {
            s.overlays(view())
                .into_iter()
                .filter_map(|o| match o {
                    Overlay::Path { dashed, width, .. } => Some((dashed, width)),
                    _ => None,
                })
                .collect()
        };
        assert!(paths(&mut s).is_empty(), "a plain shape has no guide");
        at(&mut s, PointerKind::Down, 150.0, 120.0, Mods::default());
        at(&mut s, PointerKind::Drag, 160.0, 120.0, Mods::default());
        at(&mut s, PointerKind::Up, 160.0, 120.0, Mods::default());
        // The solid halo first, the dashes on top of it.
        assert_eq!(paths(&mut s), [(false, 3.0), (true, 1.0)]);
    }

    #[test]
    fn nothing_selected_does_nothing() {
        let mut s = session();
        rect(&mut s, 100.0, 100.0, 50.0, 40.0);
        s.execute("select.none", &json!({})).unwrap();
        s.select_tool("contour", view()).unwrap();
        let before = undo_depth(&s);
        at(&mut s, PointerKind::Down, 150.0, 120.0, Mods::default());
        at(&mut s, PointerKind::Drag, 170.0, 120.0, Mods::default());
        at(&mut s, PointerKind::Up, 170.0, 120.0, Mods::default());
        assert_eq!(undo_depth(&s), before);
    }
}
