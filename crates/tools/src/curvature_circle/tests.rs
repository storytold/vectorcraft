use super::*;
use crate::testutil::*;
use vectorcraft_doc::{Appearance, Document, Node, Selection};
use vectorcraft_geom::PathData;

fn ev(kind: PointerKind, (x, y): (f64, f64), mods: Mods) -> PointerEvent {
    PointerEvent::new(kind, x, y).with_mods(mods)
}

/// A document holding `paths`, the first one on top.
fn doc_with(paths: &[PathData]) -> Document {
    let mut d = Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    for path in paths.iter().rev() {
        let id = d.alloc_id();
        d.insert(Some(l), usize::MAX, Node::path(id, path.clone(), Appearance::default_art())).unwrap();
    }
    d
}

/// A circle of radius 100 centered at (200, 200).
fn circle() -> PathData {
    shapes::ellipse(Rect::new(100.0, 100.0, 300.0, 300.0))
}

fn click(t: &mut CurvatureCircleTool, cx: &ToolContext, p: (f64, f64), mods: Mods) -> Vec<Action> {
    let mut acts = t.pointer(cx, &ev(PointerKind::Move, p, mods));
    acts.extend(t.pointer(cx, &ev(PointerKind::Down, p, mods)));
    acts.extend(t.pointer(cx, &ev(PointerKind::Up, p, mods)));
    acts
}

/// The one command `acts` runs, followed by the tool's notice that it drew: the command's params.
fn params(acts: &[Action]) -> serde_json::Value {
    let [Action::Exec(c, v), Action::Notify(what)] = acts else { panic!("one command: {acts:?}") };
    assert_eq!(c, "shape.curvatureCircle");
    assert_eq!(what, "created");
    v.clone()
}

/// The dashed circle the tool shows: its bounding box.
fn preview(o: &[Overlay]) -> Option<Rect> {
    o.iter().find_map(|o| match o {
        Overlay::Path { path, dashed: true, .. } => Some(vectorcraft_geom::Shape::bounding_box(path)),
        _ => None,
    })
}

fn near(a: Rect, b: Rect) -> bool {
    [(a.x0, b.x0), (a.y0, b.y0), (a.x1, b.x1), (a.y1, b.y1)].iter().all(|(p, q)| (p - q).abs() < 1e-9)
}

fn label(o: &[Overlay]) -> Option<String> {
    o.iter().find_map(|o| match o {
        Overlay::Label { text, .. } => Some(text.clone()),
        _ => None,
    })
}

/// Over a circle, the tool shows the circle itself, its center, the radius to the point it touches
/// and "R: 100 pt*" (an arc's own radius); a click draws it on the point of the path nearest the
/// pointer.
#[test]
fn a_click_on_a_circle_draws_the_circle_that_matches_it() {
    let d = doc_with(&[circle()]);
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = CurvatureCircleTool::default();
    t.pointer(&cx, &ev(PointerKind::Move, (302.0, 200.0), Mods::default()));
    let o = t.overlays(&cx);
    assert!(near(preview(&o).expect("the circle is shown"), Rect::new(100.0, 100.0, 300.0, 300.0)), "{o:?}");
    let center = Point::new(200.0, 200.0);
    assert!(o.iter().any(|o| matches!(o, Overlay::Line { a, b, .. } if *a == center && b.distance(Point::new(300.0, 200.0)) < 1e-9)), "{o:?}");
    assert!(o.iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text.starts_with("R: 100") && text.ends_with('*'))), "{o:?}");
    let without_labels = ToolContext { measurement_labels: false, ..cx };
    assert!(!t.overlays(&without_labels).iter().any(|o| matches!(o, Overlay::Measure { .. })));
    let v = params(&click(&mut t, &cx, (302.0, 200.0), Mods::default()));
    assert_eq!(v["convex"], json!(false));
    let id = d.layers[0].children().unwrap()[0].id;
    assert_eq!(v["id"], json!(id.0));
    let sp = &d.node(id).unwrap().path_data().unwrap().subpaths[0];
    let seg = sp.segment(v["segment"].as_u64().unwrap() as usize);
    let at = vectorcraft_geom::ParamCurve::eval(&seg, v["t"].as_f64().unwrap());
    assert!(at.distance(Point::new(300.0, 200.0)) < 1e-6, "{v}");
}

/// Shift held at the press puts the circle on the other side of the path, and the preview follows
/// Shift too.
#[test]
fn shift_puts_the_circle_on_the_other_side() {
    let d = doc_with(&[circle()]);
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let shift = Mods { shift: true, ..Mods::default() };
    let mut t = CurvatureCircleTool::default();
    t.pointer(&cx, &ev(PointerKind::Move, (300.0, 200.0), shift));
    assert!(near(preview(&t.overlays(&cx)).expect("the circle is shown"), Rect::new(300.0, 100.0, 500.0, 300.0)), "{:?}", t.overlays(&cx));
    assert_eq!(params(&click(&mut t, &cx, (300.0, 200.0), shift))["convex"], json!(true));
}

/// On a straight segment the pointer shows "Straight segment", at a cusp "Cusp", at a corner
/// anchor "Corner", and where the circle would reach past the canvas "Circle too large"; a click
/// there draws nothing.
#[test]
fn straight_segments_cusps_and_huge_circles_are_refused() {
    let line = PathData::from_bezpath(&vectorcraft_geom::BezPath::from_svg("M0 400 L400 400").unwrap());
    // A handle drawn back onto its anchor at (0, 0).
    let cusp = PathData::from_bezpath(&vectorcraft_geom::BezPath::from_svg("M0 0 C0 0 60 40 100 0").unwrap());
    let gentle = PathData::from_bezpath(&vectorcraft_geom::BezPath::from_svg("M0 450 C100 450.0000001 200 450.0000001 300 450").unwrap());
    // A curve that meets a line at a corner anchor (150, 250).
    let corner = PathData::from_bezpath(&vectorcraft_geom::BezPath::from_svg("M0 250 C50 200 100 200 150 250 L300 250").unwrap());
    let d = doc_with(&[line, cusp, gentle, corner]);
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = CurvatureCircleTool::default();
    let refused = [((200.0, 401.0), "Straight segment"), ((-1.0, -1.0), "Cusp"), ((150.0, 251.0), "Corner"), ((150.0, 451.0), "Circle too large")];
    for (at, want) in refused {
        t.pointer(&cx, &ev(PointerKind::Move, at, Mods::default()));
        let o = t.overlays(&cx);
        assert_eq!(label(&o).as_deref(), Some(want), "{o:?}");
        assert_eq!(preview(&o), None);
        assert_eq!(click(&mut t, &cx, at, Mods::default()), vec![], "{want}");
    }
}

/// Away from every path the tool shows nothing and a click draws nothing; the pointer reaches a
/// path within 5 screen points, so at 200% within 2.5 pt.
#[test]
fn the_pointer_reaches_a_path_within_5_screen_points() {
    let d = doc_with(&[circle()]);
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, snapping_tolerance: 0.0, ..cx(&d, &s, &p) };
    let mut t = CurvatureCircleTool::default();
    assert_eq!(click(&mut t, &cx, (200.0, 200.0), Mods::default()), vec![]);
    assert!(t.overlays(&cx).is_empty());
    assert_eq!(click(&mut t, &cx, (306.0, 200.0), Mods::default()), vec![]);
    assert_eq!(click(&mut t, &cx, (304.0, 200.0), Mods::default()).len(), 2);
    let zoomed = ToolContext { zoom: 2.0, ..cx };
    assert_eq!(click(&mut t, &zoomed, (303.0, 200.0), Mods::default()), vec![]);
    assert_eq!(click(&mut t, &zoomed, (302.0, 200.0), Mods::default()).len(), 2);
    // A larger snapping tolerance reaches farther, with Smart Guides off or on.
    for smart_guides in [false, true] {
        let wide = ToolContext { snapping_tolerance: 10.0, smart_guides, ..cx };
        assert_eq!(click(&mut t, &wide, (309.0, 200.0), Mods::default()).len(), 2, "{smart_guides}");
        assert_eq!(click(&mut t, &wide, (311.0, 200.0), Mods::default()), vec![], "{smart_guides}");
    }
}

/// Of two paths within reach, the nearer one is used, even when the other one is selected: a
/// selected circle 4 pt from the pointer loses to a circle under it.
#[test]
fn the_nearest_path_wins_over_a_selected_one() {
    let far = shapes::ellipse(Rect::new(104.0, 100.0, 304.0, 300.0));
    let d = doc_with(&[circle(), far]);
    let ids: Vec<NodeId> = d.layers[0].children().unwrap().iter().map(|n| n.id).collect();
    let far_id = *ids.iter().find(|id| d.node(**id).unwrap().path_data().unwrap().bounds().unwrap().x0 > 100.5).unwrap();
    let s = Selection { objects: vec![far_id], ..Selection::default() };
    let p = paint();
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = CurvatureCircleTool::default();
    let v = params(&click(&mut t, &cx, (300.0, 200.0), Mods::default()));
    assert_ne!(v["id"], json!(far_id.0), "{v}");
}

/// A guide is not a path the tool draws from, even when it is selected.
#[test]
fn a_selected_guide_is_left_out() {
    let mut d = doc_with(&[circle()]);
    let id = d.layers[0].children().unwrap()[0].id;
    if let Some(vectorcraft_doc::NodeKind::Path { guide, .. }) = d.node_mut(id).map(|n| &mut n.kind) {
        *guide = true;
    }
    let s = Selection { objects: vec![id], ..Selection::default() };
    let p = paint();
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = CurvatureCircleTool::default();
    assert_eq!(click(&mut t, &cx, (300.0, 200.0), Mods::default()), vec![]);
    assert!(t.overlays(&cx).is_empty());
}

/// After a click on a circle drawn with the Ellipse tool, the next hover and click pass over the
/// circle the tool drew (selected and on top of it) and use the circle under it again, until an
/// edit made outside the tool.
#[test]
fn the_next_click_passes_over_the_circle_just_drawn() {
    let d = doc_with(&[circle(), circle()]);
    let ids: Vec<NodeId> = d.layers[0].children().unwrap().iter().rev().map(|n| n.id).collect();
    let (drawn, under) = (ids[0], ids[1]);
    let s = Selection { objects: vec![drawn], ..Selection::default() };
    let p = paint();
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = CurvatureCircleTool::default();
    t.notify(&cx, "created");
    t.pointer(&cx, &ev(PointerKind::Move, (302.0, 200.0), Mods::default()));
    assert_eq!(t.hover.map(|(h, _)| h.id), Some(under));
    assert_eq!(params(&click(&mut t, &cx, (302.0, 200.0), Mods::default()))["id"], json!(under.0));
    t.history_moved();
    assert_eq!(params(&click(&mut t, &cx, (302.0, 200.0), Mods::default()))["id"], json!(drawn.0));
}

/// A press that a second finger or a canceled touch drops draws nothing on the release, and an
/// edit made outside the tool drops what the pointer was over.
#[test]
fn a_dropped_press_and_an_outside_edit_draw_nothing() {
    let d = doc_with(&[circle()]);
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = CurvatureCircleTool::default();
    t.pointer(&cx, &ev(PointerKind::Down, (300.0, 200.0), Mods::default()));
    t.drop_press(&cx);
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, (300.0, 200.0), Mods::default())), vec![]);
    t.pointer(&cx, &ev(PointerKind::Move, (300.0, 200.0), Mods::default()));
    t.history_moved();
    assert!(t.overlays(&cx).is_empty());
}

/// Escape during a press cancels it, and the tool takes Escape only while a press is held; the
/// tool asks to be hovered again after an edit or a zoom.
#[test]
fn escape_cancels_a_press() {
    let d = doc_with(&[circle()]);
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = CurvatureCircleTool::default();
    assert!(!t.claims_key(&cx, ToolKey::Escape));
    t.pointer(&cx, &ev(PointerKind::Down, (300.0, 200.0), Mods::default()));
    assert!(t.claims_key(&cx, ToolKey::Escape));
    t.key(&cx, ToolKey::Escape, Mods::default());
    assert!(t.overlays(&cx).is_empty());
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, (300.0, 200.0), Mods::default())), vec![]);
    assert!(t.wants_rehover());
}

/// The preview of a circle more than 50,000 screen points around is drawn solid; a smaller one is
/// dashed.
#[test]
fn a_huge_preview_is_drawn_solid() {
    let big = shapes::ellipse(Rect::new(-10_000.0, -10_000.0, 10_000.0, 10_000.0));
    let d = doc_with(&[big]);
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = CurvatureCircleTool::default();
    t.pointer(&cx, &ev(PointerKind::Move, (10_001.0, 0.0), Mods::default()));
    let o = t.overlays(&cx);
    assert_eq!(preview(&o), None, "{o:?}");
    assert!(o.iter().any(|o| matches!(o, Overlay::Path { dashed: false, .. })), "{o:?}");
}

/// A press dragged 2 screen points or more is not a click and draws nothing; a press that moves
/// less still draws the circle.
#[test]
fn a_drag_draws_nothing() {
    let d = doc_with(&[circle()]);
    let (s, p) = (Selection::default(), paint());
    let cx = ToolContext { smart_guides: false, ..cx(&d, &s, &p) };
    let mut t = CurvatureCircleTool::default();
    let press = |t: &mut CurvatureCircleTool, to: (f64, f64)| {
        t.pointer(&cx, &ev(PointerKind::Down, (300.0, 200.0), Mods::default()));
        t.pointer(&cx, &ev(PointerKind::Drag, to, Mods::default()));
        t.pointer(&cx, &ev(PointerKind::Up, to, Mods::default()))
    };
    assert_eq!(press(&mut t, (300.0, 230.0)), vec![]);
    // The circle at the press is not shown once the press is a drag; at a release away from every
    // path nothing is shown.
    t.pointer(&cx, &ev(PointerKind::Down, (300.0, 200.0), Mods::default()));
    t.pointer(&cx, &ev(PointerKind::Drag, (250.0, 200.0), Mods::default()));
    assert!(t.overlays(&cx).is_empty());
    t.pointer(&cx, &ev(PointerKind::Up, (250.0, 200.0), Mods::default()));
    assert!(t.overlays(&cx).is_empty());
    assert_eq!(press(&mut t, (300.0, 202.0)), vec![]);
    assert_eq!(press(&mut t, (300.0, 201.0)).len(), 2);
    // A release that far away with no drag before it is not a click either.
    t.pointer(&cx, &ev(PointerKind::Down, (300.0, 200.0), Mods::default()));
    assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, (300.0, 230.0), Mods::default())), vec![]);
}

/// The tool is in the Ellipse tool's group, after the Circle tool.
#[test]
fn the_curvature_circle_tool_follows_the_circle_tool() {
    let tool = crate::create(ID);
    assert_eq!(tool.id(), ID);
    let group = crate::catalog::group_of(ID).unwrap();
    let ids: Vec<&str> = crate::TOOL_GROUPS[group].iter().map(|t| t.id).collect();
    let circle = ids.iter().position(|id| *id == crate::circle::ID).unwrap();
    assert_eq!(ids.get(circle + 1), Some(&ID));
}
