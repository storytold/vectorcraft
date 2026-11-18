use super::*;
use crate::testutil::*;
use serde_json::Value;
use vectorcraft_doc::{Appearance, Document, Node, Selection};
use vectorcraft_geom::{Rect, shapes};

fn click(tool: &mut dyn Tool, cx: &ToolContext, (x, y): (f64, f64), mods: Mods) -> Vec<Action> {
    let mut acts = tool.pointer(cx, &PointerEvent::new(PointerKind::Down, x, y).with_mods(mods));
    acts.extend(tool.pointer(cx, &PointerEvent::new(PointerKind::Up, x, y).with_mods(mods)));
    acts
}

/// Clicks each vertex in turn with `first` held at the first click only.
fn clicks(tool: &mut dyn Tool, cx: &ToolContext, vertices: &[(f64, f64)], first: Mods) {
    for (i, v) in vertices.iter().enumerate() {
        let acts = click(tool, cx, *v, if i == 0 { first } else { Mods::default() });
        assert!(acts.is_empty(), "a vertex click selects nothing: {acts:?}");
    }
}

/// The anchors each selected path has in a `select.anchorsMany` command, and its mode.
fn selected(acts: &[Action]) -> (Vec<(u64, usize)>, String) {
    let [Action::Exec(c, v)] = acts else { panic!("one selection command: {acts:?}") };
    assert_eq!(c, "select.anchorsMany");
    let items = v["items"].as_array().unwrap();
    let ids = items.iter().map(|it| (it["id"].as_u64().unwrap(), it["anchors"].as_array().unwrap().len())).collect();
    (ids, v["mode"].as_str().unwrap().to_owned())
}

/// A triangle around the rectangle's top left corner (100, 100) only.
const CORNER: &[(f64, f64)] = &[(80.0, 80.0), (140.0, 80.0), (80.0, 140.0)];

#[test]
fn a_double_click_enter_or_a_click_on_the_first_vertex_closes_the_loop() {
    let (d, id) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = cx(&d, &s, &p);
    type Closing<'a> = &'a dyn Fn(&mut dyn Tool) -> Vec<Action>;
    let closings: [Closing; 3] = [
        &|t| {
            let mut acts = click(t, &cx, (80.0, 141.0), Mods::default());
            acts.extend(t.pointer(&cx, &PointerEvent::new(PointerKind::DoubleClick, 80.0, 141.0)));
            acts
        },
        &|t| t.key(&cx, ToolKey::Enter, Mods::default()),
        &|t| click(t, &cx, (82.0, 81.0), Mods::default()),
    ];
    for (i, close) in closings.iter().enumerate() {
        let mut tool = crate::create(ID);
        assert_eq!(tool.id(), ID);
        clicks(tool.as_mut(), &cx, CORNER, Mods::default());
        assert!(tool.ends_on_escape(&cx) && tool.claims_key(&cx, ToolKey::Enter), "closing {i}");
        assert_eq!(selected(&close(tool.as_mut())), (vec![(id.0, 1)], "set".to_owned()), "closing {i}");
        assert!(!tool.ends_on_escape(&cx), "closing {i}");
    }
}

#[test]
fn a_double_click_at_a_new_point_sets_that_vertex_and_closes() {
    let (d, id) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = cx(&d, &s, &p);
    let mut tool = PolygonalLassoTool::default();
    clicks(&mut tool, &cx, &CORNER[..2], Mods::default());
    let mut acts = click(&mut tool, &cx, (80.0, 140.0), Mods::default());
    acts.extend(click(&mut tool, &cx, (80.0, 140.0), Mods::default()));
    assert!(acts.is_empty(), "{acts:?}");
    assert_eq!(tool.vertices.len(), 3, "the second click of the double-click sets no vertex");
    let acts = tool.pointer(&cx, &PointerEvent::new(PointerKind::DoubleClick, 80.0, 140.0));
    assert_eq!(selected(&acts), (vec![(id.0, 1)], "set".to_owned()));
    assert!(!tool.ends_on_escape(&cx));
}

#[test]
fn backspace_or_delete_removes_the_last_vertex() {
    let (d, id) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = cx(&d, &s, &p);
    let mut tool = PolygonalLassoTool::default();
    // (300, 300) would take in the corner at (200, 200) too.
    clicks(&mut tool, &cx, &[(80.0, 80.0), (140.0, 80.0), (300.0, 300.0)], Mods::default());
    assert!(tool.claims_key(&cx, ToolKey::Backspace));
    assert!(tool.key(&cx, ToolKey::Backspace, Mods::default()).is_empty());
    assert_eq!(tool.vertices.len(), 2);
    // The preview runs along the two vertices left, not back to the removed one.
    let overlays = tool.overlays(&cx);
    let [Overlay::Path { path, .. }] = overlays.as_slice() else { panic!("one preview path") };
    assert_eq!(path.elements().len(), 2);
    clicks(&mut tool, &cx, &[(300.0, 300.0)], Mods::default());
    assert!(tool.claims_key(&cx, ToolKey::Delete));
    assert!(tool.key(&cx, ToolKey::Delete, Mods::default()).is_empty());
    assert_eq!(tool.vertices.len(), 2);
    clicks(&mut tool, &cx, &[(80.0, 140.0)], Mods::default());
    assert_eq!(selected(&tool.key(&cx, ToolKey::Enter, Mods::default())), (vec![(id.0, 1)], "set".to_owned()));
    assert!(!tool.claims_key(&cx, ToolKey::Delete), "with no loop open, Delete deletes");
}

#[test]
fn escape_drops_the_loop_and_leaves_the_selection() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = cx(&d, &s, &p);
    let mut tool = PolygonalLassoTool::default();
    clicks(&mut tool, &cx, CORNER, Mods::default());
    assert!(tool.claims_key(&cx, ToolKey::Escape));
    assert!(tool.key(&cx, ToolKey::Escape, Mods::default()).is_empty());
    assert!(!tool.ends_on_escape(&cx) && !tool.claims_key(&cx, ToolKey::Enter));
    assert!(tool.overlays(&cx).is_empty());
    assert!(tool.key(&cx, ToolKey::Enter, Mods::default()).is_empty());
    assert!(tool.pointer(&cx, &PointerEvent::new(PointerKind::DoubleClick, 80.0, 140.0)).is_empty());
}

#[test]
fn shift_keeps_the_segment_to_45_degree_steps() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = cx(&d, &s, &p);
    let shift = Mods { shift: true, ..Mods::default() };
    let mut tool = PolygonalLassoTool::default();
    clicks(&mut tool, &cx, &[(100.0, 100.0)], Mods::default());
    tool.pointer(&cx, &PointerEvent::new(PointerKind::Move, 200.0, 110.0).with_mods(shift));
    let hover = tool.hover.unwrap();
    assert!((hover.y - 100.0).abs() < 1e-9 && hover.x > 199.0, "{hover:?}");
    assert_eq!(tool.overlays(&cx).len(), 1, "the segment follows the pointer");
    click(&mut tool, &cx, (190.0, 175.0), shift);
    let v = tool.vertices[1];
    assert!(((v.x - 100.0) - (v.y - 100.0)).abs() < 1e-9 && v.x > 150.0, "on the 45° line: {v:?}");
    tool.pointer(&cx, &PointerEvent::new(PointerKind::Move, 200.0, 110.0));
    assert_eq!(tool.hover, Some(Point::new(200.0, 110.0)), "without Shift the segment goes to the pointer");
}

#[test]
fn shift_or_alt_at_the_first_click_adds_or_subtracts() {
    let (d, id) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = cx(&d, &s, &p);
    let shift = Mods { shift: true, ..Mods::default() };
    let alt = Mods { alt: true, ..Mods::default() };
    for (first, mode) in [(shift, "add"), (alt, "subtract"), (Mods::default(), "set")] {
        let mut tool = PolygonalLassoTool::default();
        clicks(&mut tool, &cx, CORNER, first);
        assert_eq!(selected(&tool.key(&cx, ToolKey::Enter, Mods::default())), (vec![(id.0, 1)], mode.to_owned()));
    }
    // Shift held only at a later click constrains that segment and does not add.
    let mut tool = PolygonalLassoTool::default();
    clicks(&mut tool, &cx, &CORNER[..2], Mods::default());
    click(&mut tool, &cx, (80.0, 140.0), shift);
    assert_eq!(selected(&tool.key(&cx, ToolKey::Enter, Mods::default())).1, "set");
}

#[test]
fn a_concave_loop_leaves_out_anchors_in_its_notch() {
    let (d, id) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = cx(&d, &s, &p);
    let mut tool = PolygonalLassoTool::default();
    // An L: the bottom left corner (100, 200) is in the notch.
    clicks(&mut tool, &cx, &[(60.0, 60.0), (240.0, 60.0), (240.0, 240.0), (160.0, 240.0), (160.0, 170.0), (60.0, 170.0)], Mods::default());
    let acts = tool.key(&cx, ToolKey::Enter, Mods::default());
    assert_eq!(selected(&acts), (vec![(id.0, 3)], "set".to_owned()));
    let Action::Exec(_, v) = &acts[0] else { panic!() };
    let picked: Vec<Value> = v["items"][0]["anchors"].as_array().unwrap().clone();
    // The rectangle's anchors run (100, 100), (200, 100), (200, 200), (100, 200).
    assert!(!picked.contains(&serde_json::json!([0, 3])), "{picked:?}");
}

#[test]
fn fewer_than_three_vertices_select_nothing() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = cx(&d, &s, &p);
    let mut tool = PolygonalLassoTool::default();
    clicks(&mut tool, &cx, &CORNER[..2], Mods::default());
    // A click on the first vertex of a two-vertex loop sets no vertex on top of it and closes nothing.
    assert!(click(&mut tool, &cx, (81.0, 80.0), Mods::default()).is_empty());
    assert_eq!(tool.vertices.len(), 2);
    assert!(tool.pointer(&cx, &PointerEvent::new(PointerKind::DoubleClick, 81.0, 80.0)).is_empty());
    assert!(!tool.ends_on_escape(&cx));
    clicks(&mut tool, &cx, &CORNER[..2], Mods::default());
    assert!(tool.key(&cx, ToolKey::Enter, Mods::default()).is_empty());
    assert!(!tool.ends_on_escape(&cx));
    // A double-click on empty canvas: its second click sets no second vertex.
    assert!(click(&mut tool, &cx, (80.0, 80.0), Mods::default()).is_empty());
    assert!(click(&mut tool, &cx, (81.0, 80.0), Mods::default()).is_empty());
    assert!(tool.pointer(&cx, &PointerEvent::new(PointerKind::DoubleClick, 81.0, 80.0)).is_empty());
    assert!(!tool.ends_on_escape(&cx));
}

#[test]
fn a_loop_that_crosses_itself_selects_by_the_even_odd_rule() {
    // A small square in the middle of a five-pointed star, where the star's edges wind twice, and
    // one in the star's top point, where they wind once.
    let mut d = Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    let (middle, point) = (d.alloc_id(), d.alloc_id());
    d.insert(Some(l), 0, Node::path(middle, shapes::rectangle(Rect::new(140.0, 140.0, 160.0, 160.0)), Appearance::default_art())).unwrap();
    d.insert(Some(l), 1, Node::path(point, shapes::rectangle(Rect::new(148.0, 78.0, 152.0, 82.0)), Appearance::default_art())).unwrap();
    let (s, p) = (Selection::default(), paint());
    let cx = cx(&d, &s, &p);
    let star: Vec<(f64, f64)> = (0..5)
        .map(|k| {
            let a = f64::from(-90 + 144 * k).to_radians();
            (150.0 + 100.0 * a.cos(), 150.0 + 100.0 * a.sin())
        })
        .collect();
    let mut tool = PolygonalLassoTool::default();
    clicks(&mut tool, &cx, &star, Mods::default());
    assert_eq!(selected(&tool.key(&cx, ToolKey::Enter, Mods::default())), (vec![(point.0, 4)], "set".to_owned()));
}

#[test]
fn a_dropped_press_takes_back_only_its_vertex() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = cx(&d, &s, &p);
    let mut tool = PolygonalLassoTool::default();
    clicks(&mut tool, &cx, &CORNER[..2], Mods::default());
    tool.pointer(&cx, &PointerEvent::new(PointerKind::Down, 80.0, 140.0));
    assert!(tool.drop_press(&cx).is_empty());
    assert_eq!(tool.vertices, vec![Point::new(80.0, 80.0), Point::new(140.0, 80.0)]);
    assert_eq!(tool.hover, None);
}

#[test]
fn a_dropped_press_after_backspace_keeps_the_earlier_vertices() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = cx(&d, &s, &p);
    let mut tool = PolygonalLassoTool::default();
    clicks(&mut tool, &cx, &CORNER[..2], Mods::default());
    tool.pointer(&cx, &PointerEvent::new(PointerKind::Down, 80.0, 140.0));
    assert!(tool.key(&cx, ToolKey::Backspace, Mods::default()).is_empty());
    assert!(tool.drop_press(&cx).is_empty());
    assert_eq!(tool.vertices, vec![Point::new(80.0, 80.0), Point::new(140.0, 80.0)]);
}

#[test]
fn the_tool_shows_the_lasso_cursor() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = cx(&d, &s, &p);
    assert_eq!(PolygonalLassoTool::default().cursor(&cx, Point::new(10.0, 10.0), Mods::default()), Cursor::Lasso);
}
