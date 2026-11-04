use super::*;
use crate::testutil::*;
use vectorcraft_doc::{Appearance, Document, NodeId, Selection};
use vectorcraft_geom::{Rect, shapes};

fn gesture(d: &Document, s: &Selection, points: &[(f64, f64)], mods: Mods, isolation: Option<NodeId>) -> Vec<Action> {
    let p = paint();
    let cx = ToolContext { isolation, ..cx(d, s, &p) };
    let mut tool = crate::create(ID);
    assert_eq!(tool.id(), ID);
    let &(x, y) = points.first().unwrap();
    tool.pointer(&cx, &PointerEvent::new(PointerKind::Down, x, y));
    for &(x, y) in points.iter().skip(1) {
        tool.pointer(&cx, &PointerEvent::new(PointerKind::Drag, x, y));
    }
    let &(x, y) = points.last().unwrap();
    tool.pointer(&cx, &PointerEvent::new(PointerKind::Up, x, y).with_mods(mods))
}

const AROUND: &[(f64, f64)] = &[(90.0, 90.0), (210.0, 90.0), (210.0, 210.0), (90.0, 210.0)];

#[test]
fn enclosing_lasso_selects_only_whole_objects_and_preserves_modifier_rules() {
    let (d, id) = doc_with_rect();
    let mut s = Selection::default();
    assert_eq!(gesture(&d, &s, AROUND, Mods::default(), None), vec![Action::Exec("select.set".into(), json!({"ids": [id.0]}))]);
    let corner = &[(90.0, 90.0), (150.0, 90.0), (90.0, 150.0)];
    assert_eq!(gesture(&d, &s, corner, Mods::default(), None), vec![Action::Exec("select.set".into(), json!({"ids": []}))]);
    let shift = Mods { shift: true, ..Mods::default() };
    let alt = Mods { alt: true, ..Mods::default() };
    assert_eq!(gesture(&d, &s, AROUND, shift, None), vec![Action::Exec("select.add".into(), json!({"ids": [id.0]}))]);
    assert!(gesture(&d, &s, AROUND, alt, None).is_empty());
    s.objects.push(id);
    assert_eq!(gesture(&d, &s, AROUND, alt, None), vec![Action::Exec("select.toggle".into(), json!({"ids": [id.0]}))]);
    assert!(gesture(&d, &s, corner, shift, None).is_empty());
}

#[test]
fn enclosing_lasso_checks_curves_and_concave_edges_between_inside_anchors() {
    let region = crate::xform::polygon(&[Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0), Point::new(0.0, 10.0)], true);
    let region = Region::new(region).unwrap();
    let mut curve = BezPath::new();
    curve.move_to((2.0, 5.0));
    curve.curve_to((3.0, -15.0), (7.0, -15.0), (8.0, 5.0));
    assert!(!enclosed(&curve, &region), "inside endpoints do not contain a curve that bulges outside");
    let notch = crate::xform::polygon(
        &[
            Point::new(0.0, 0.0),
            Point::new(10.0, 0.0),
            Point::new(10.0, 10.0),
            Point::new(6.0, 10.0),
            Point::new(6.0, 4.0),
            Point::new(4.0, 4.0),
            Point::new(4.0, 10.0),
            Point::new(0.0, 10.0),
        ],
        true,
    );
    let line = |a, b| {
        let mut p = BezPath::new();
        p.move_to(a);
        p.line_to(b);
        p
    };
    let notch = Region::new(notch).unwrap();
    assert!(!enclosed(&line((2.0, 8.0), (8.0, 8.0)), &notch));
    assert!(enclosed(&line((2.0, 2.0), (8.0, 2.0)), &notch));
    assert!(enclosed(&Rect::new(0.0, 0.0, 10.0, 10.0).to_path(0.01), &region), "boundary contact is enclosed");
}

#[test]
fn enclosing_lasso_uses_path_geometry_instead_of_its_bounding_box() {
    let (mut d, id) = doc_with_rect();
    let mut triangle = BezPath::new();
    triangle.move_to((100.0, 100.0));
    triangle.line_to((190.0, 100.0));
    triangle.line_to((100.0, 190.0));
    triangle.close_path();
    let NodeKind::Path { path, .. } = &mut d.node_mut(id).unwrap().kind else { panic!() };
    *path = PathData::from_bezpath(&triangle);
    let loop_points = &[(90.0, 90.0), (210.0, 90.0), (90.0, 210.0)];
    assert_eq!(
        gesture(&d, &Selection::default(), loop_points, Mods::default(), None),
        vec![Action::Exec("select.set".into(), json!({"ids": [id.0]}))]
    );
}

#[test]
fn enclosing_lasso_skips_locked_hidden_and_out_of_isolation_objects() {
    let (mut d, id) = doc_with_rect();
    let s = Selection::default();
    d.node_mut(id).unwrap().locked = true;
    assert_eq!(gesture(&d, &s, AROUND, Mods::default(), None), vec![Action::Exec("select.set".into(), json!({"ids": []}))]);
    d.node_mut(id).unwrap().locked = false;
    d.node_mut(id).unwrap().visible = false;
    assert_eq!(gesture(&d, &s, AROUND, Mods::default(), None), vec![Action::Exec("select.set".into(), json!({"ids": []}))]);
    d.node_mut(id).unwrap().visible = true;
    let other = d.alloc_id();
    d.insert(Some(d.layers[0].id), 1, Node::path(other, shapes::rectangle(Rect::new(300.0, 300.0, 350.0, 350.0)), Appearance::default_art()))
        .unwrap();
    assert_eq!(gesture(&d, &s, AROUND, Mods::default(), Some(other)), vec![Action::Exec("select.set".into(), json!({"ids": []}))]);
}

#[test]
fn enclosing_lasso_escape_cancels_without_changing_selection() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = cx(&d, &s, &p);
    let mut tool = EnclosingLassoTool::default();
    tool.pointer(&cx, &PointerEvent::new(PointerKind::Down, 90.0, 90.0));
    tool.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 210.0, 90.0));
    assert!(tool.busy());
    assert_eq!(tool.overlays(&cx).len(), 1);
    assert!(tool.key(&cx, ToolKey::Escape, Mods::default()).is_empty());
    assert!(!tool.busy());
    assert!(tool.overlays(&cx).is_empty());
    assert!(tool.pointer(&cx, &PointerEvent::new(PointerKind::Up, 90.0, 210.0)).is_empty());
}

#[test]
fn enclosing_lasso_degenerate_loop_does_not_select_a_boundary_line() {
    let (mut d, id) = doc_with_rect();
    let mut line = BezPath::new();
    line.move_to((100.0, 100.0));
    line.line_to((200.0, 200.0));
    let n = d.node_mut(id).unwrap();
    *n.path_data_mut().unwrap() = PathData::from_bezpath(&line);
    n.appearance.set_fill(vectorcraft_color::Paint::None);
    let points = &[(90.0, 90.0), (150.0, 150.0), (210.0, 210.0)];
    assert_eq!(gesture(&d, &Selection::default(), points, Mods::default(), None), vec![Action::Exec("select.set".into(), json!({"ids": []}))]);
}

#[test]
fn enclosing_lasso_invalid_release_cancels_the_gesture() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = cx(&d, &s, &p);
    let mut tool = EnclosingLassoTool::default();
    tool.pointer(&cx, &PointerEvent::new(PointerKind::Down, 90.0, 90.0));
    tool.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 210.0, 90.0));
    assert!(tool.pointer(&cx, &PointerEvent::new(PointerKind::Up, f64::NAN, 210.0)).is_empty());
    assert!(!tool.busy());
    assert!(tool.overlays(&cx).is_empty());
}

fn loop_with_hole() -> BezPath {
    crate::xform::polygon(
        &[
            Point::new(0.0, 0.0),
            Point::new(10.0, 0.0),
            Point::new(10.0, 10.0),
            Point::new(0.0, 10.0),
            Point::new(0.0, 0.0),
            Point::new(3.0, 3.0),
            Point::new(3.0, 7.0),
            Point::new(7.0, 7.0),
            Point::new(7.0, 3.0),
            Point::new(3.0, 3.0),
            Point::new(0.0, 0.0),
        ],
        true,
    )
}

#[test]
fn enclosing_lasso_rejects_filled_objects_covering_a_hole_in_the_loop() {
    let region = Region::new(loop_with_hole()).unwrap();
    let path = shapes::rectangle(Rect::new(2.0, 2.0, 8.0, 8.0));
    assert!(enclosed(&path.to_bezpath(), &region), "the hole does not cross the object's outline");
    assert!(!path_inside(&path, FillRule::NonZero, true, &region));
    assert!(path_inside(&path, FillRule::NonZero, false, &region), "an unfilled outline may surround the hole");
    let mut node = Node::path(NodeId(10), path, Appearance::default_art());
    let doc = Document::new(20.0, 20.0);
    let symbols = doc.symbol_boxes();
    assert!(!node_inside(&node, &symbols, &region));
    for item in &mut node.appearance.items {
        if let AppearanceItem::Fill(fill) = item {
            fill.visible = false;
        }
    }
    assert!(node_inside(&node, &symbols, &region), "a hidden fill does not cover the loop's hole");
}

#[test]
fn enclosing_lasso_selects_group_members_but_keeps_compounds_whole() {
    let (mut d, id) = doc_with_rect();
    let group = d.alloc_id();
    let a = d.alloc_id();
    let b = d.alloc_id();
    let children = vec![
        std::sync::Arc::new(Node::path(a, shapes::rectangle(Rect::new(120.0, 120.0, 140.0, 140.0)), Appearance::default_art())),
        std::sync::Arc::new(Node::path(b, shapes::rectangle(Rect::new(300.0, 300.0, 320.0, 320.0)), Appearance::default_art())),
    ];
    d.insert(Some(d.layers[0].id), 1, Node::new(group, NodeKind::Group { children: children.clone(), clip: false })).unwrap();
    let s = Selection::default();
    assert_eq!(gesture(&d, &s, AROUND, Mods::default(), None), vec![Action::Exec("select.set".into(), json!({"ids": [id.0, a.0]}))]);
    d.node_mut(group).unwrap().kind = NodeKind::Compound { children, rule: FillRule::NonZero };
    assert_eq!(gesture(&d, &s, AROUND, Mods::default(), None), vec![Action::Exec("select.set".into(), json!({"ids": [id.0]}))]);
}

#[test]
fn enclosing_lasso_accepts_a_figure_eight_with_zero_signed_area() {
    let (mut d, id) = doc_with_rect();
    *d.node_mut(id).unwrap().path_data_mut().unwrap() = shapes::rectangle(Rect::new(130.0, 105.0, 170.0, 115.0));
    let points = &[(90.0, 90.0), (210.0, 210.0), (90.0, 210.0), (210.0, 90.0)];
    assert_eq!(gesture(&d, &Selection::default(), points, Mods::default(), None), vec![Action::Exec("select.set".into(), json!({"ids": [id.0]}))]);
}

#[test]
fn enclosing_lasso_checks_nonpath_box_interiors_as_well_as_edges() {
    let (d, _) = doc_with_rect();
    let mut n = Node::new(NodeId(900), NodeKind::SymbolInstance { symbol: "missing".into(), xf: vectorcraft_geom::Affine::IDENTITY });
    // Place the missing-symbol fallback box exactly inside the outer loop.
    let b = n.geometric_bounds().unwrap();
    let xf = vectorcraft_geom::Affine::translate((2.0, 2.0))
        * vectorcraft_geom::Affine::scale_non_uniform(6.0 / b.width(), 6.0 / b.height())
        * vectorcraft_geom::Affine::translate((-b.x0, -b.y0));
    n.kind = NodeKind::SymbolInstance { symbol: "missing".into(), xf };
    let region = Region::new(loop_with_hole()).unwrap();
    assert!(!node_inside(&n, &d.symbol_boxes(), &region));
}

#[test]
fn enclosing_lasso_uses_the_release_position_and_click_clears_only_without_modifiers() {
    let (d, id) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = cx(&d, &s, &p);
    let mut tool = EnclosingLassoTool::default();
    tool.pointer(&cx, &PointerEvent::new(PointerKind::Down, 90.0, 90.0));
    tool.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 210.0, 90.0));
    tool.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 210.0, 210.0));
    assert_eq!(tool.pointer(&cx, &PointerEvent::new(PointerKind::Up, 90.0, 210.0)), vec![Action::Exec("select.set".into(), json!({"ids": [id.0]}))]);
    assert!(!tool.busy());
    assert!(tool.overlays(&cx).is_empty());
    assert_eq!(gesture(&d, &s, &[(90.0, 90.0)], Mods::default(), None), vec![Action::Exec("select.set".into(), json!({"ids": []}))]);
    assert!(gesture(&d, &s, &[(90.0, 90.0)], Mods { shift: true, ..Mods::default() }, None).is_empty());
}

#[test]
fn enclosing_lasso_selects_single_anchor_objects_but_checks_every_disconnected_point() {
    let (mut d, id) = doc_with_rect();
    let mut point = BezPath::new();
    point.move_to((150.0, 150.0));
    *d.node_mut(id).unwrap().path_data_mut().unwrap() = PathData::from_bezpath(&point);
    assert_eq!(gesture(&d, &Selection::default(), AROUND, Mods::default(), None), vec![Action::Exec("select.set".into(), json!({"ids": [id.0]}))]);
    let mut path = shapes::rectangle(Rect::new(120.0, 120.0, 140.0, 140.0)).to_bezpath();
    path.move_to((300.0, 300.0));
    *d.node_mut(id).unwrap().path_data_mut().unwrap() = PathData::from_bezpath(&path);
    assert_eq!(gesture(&d, &Selection::default(), AROUND, Mods::default(), None), vec![Action::Exec("select.set".into(), json!({"ids": []}))]);
}

#[test]
fn enclosing_lasso_cancels_an_overlong_gesture_instead_of_silently_truncating_it() {
    let (d, _) = doc_with_rect();
    let (s, p) = (Selection::default(), paint());
    let cx = cx(&d, &s, &p);
    let mut tool = EnclosingLassoTool::default();
    tool.pointer(&cx, &PointerEvent::new(PointerKind::Down, 0.0, 0.0));
    for i in 1..=MAX_POINTS {
        assert!(tool.pointer(&cx, &PointerEvent::new(PointerKind::Drag, i as f64, 100.0)).is_empty());
    }
    assert!(!tool.busy());
    assert!(tool.overlays(&cx).is_empty());
    assert!(tool.pointer(&cx, &PointerEvent::new(PointerKind::Up, 0.0, 200.0)).is_empty());
}

#[test]
#[ignore = "manual large-document selection timing"]
fn enclosing_lasso_dense_document_timing() {
    let count = std::env::var("ENCLOSING_LASSO_BENCH_OBJECTS").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(1000).clamp(1, 50_000);
    let d = Document::new(1000.0, 1000.0);
    let symbols = d.symbol_boxes();
    let nodes: Vec<_> = (0..count)
        .map(|i| {
            let (x, y) = if count <= 1000 {
                (350.0 + (i % 25) as f64 * 5.0, 350.0 + (i / 25) as f64 * 5.0)
            } else {
                (250.0 + (i % 250) as f64 * 2.0, 300.0 + (i / 250) as f64 * 2.0)
            };
            Node::path(NodeId(i as u64 + 10), shapes::rectangle(Rect::new(x, y, x + 2.0, y + 2.0)), Appearance::default_art())
        })
        .collect();
    let points: Vec<_> = (0..1000)
        .map(|i| {
            let angle = f64::from(i) * std::f64::consts::TAU / 1000.0;
            Point::new(500.0 + angle.cos() * 450.0, 500.0 + angle.sin() * 450.0)
        })
        .collect();
    let start = std::time::Instant::now();
    let region = Region::new(crate::xform::polygon(&points, true)).unwrap();
    let selected = nodes.iter().filter(|n| node_inside(n, &symbols, &region)).count();
    let elapsed = start.elapsed();
    assert_eq!(selected, count);
    eprintln!("Enclosing Lasso: {selected} objects, 1000-point loop, {} microseconds", elapsed.as_micros());
}

#[test]
fn enclosing_lasso_selection_does_not_change_when_a_concave_loop_has_more_samples() {
    let (d, _) = doc_with_rect();
    let sparse = [(90.0, 90.0), (210.0, 90.0), (210.0, 210.0), (160.0, 210.0), (160.0, 150.0), (140.0, 150.0), (140.0, 210.0), (90.0, 210.0)];
    let mut dense = vec![];
    for (a, b) in sparse.iter().zip(sparse.iter().cycle().skip(1)) {
        let (a, b) = (Point::new(a.0, a.1), Point::new(b.0, b.1));
        for i in 0..40 {
            let p = a.lerp(b, f64::from(i) / 40.0);
            dense.push((p.x, p.y));
        }
    }
    let s = Selection::default();
    assert_eq!(gesture(&d, &s, &sparse, Mods::default(), None), vec![Action::Exec("select.set".into(), json!({"ids": []}))]);
    assert_eq!(gesture(&d, &s, &dense, Mods::default(), None), gesture(&d, &s, &sparse, Mods::default(), None));
}

#[test]
fn enclosing_lasso_boundary_query_does_not_repeat_an_edge_across_its_height() {
    let region = Region::new(Rect::new(0.0, 0.0, 10.0, 10.0).to_path(0.01)).unwrap();
    assert_eq!(
        region.near(Rect::new(0.0, 0.0, 10.0, 10.0)).count(),
        4,
        "one intersection check per boundary edge, even for a query spanning its full height"
    );
}

#[test]
fn enclosing_lasso_considers_the_geometry_a_visible_compound_paints() {
    let (mut d, rect) = doc_with_rect();
    let compound = d.alloc_id();
    let mut children = vec![];
    for x in [120.0, 160.0] {
        let mut member = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(x, 120.0, x + 20.0, 140.0)), Appearance::default_art());
        member.visible = false;
        children.push(std::sync::Arc::new(member));
    }
    // Compound painting assembles all member paths, independently of their own visibility flags.
    let mut object = Node::new(compound, NodeKind::Compound { children, rule: FillRule::NonZero });
    object.appearance = Appearance::default_art().into();
    d.insert(Some(d.layers[0].id), 1, object).unwrap();
    let s = Selection::default();
    assert_eq!(
        vectorcraft_doc::hit::marquee(&d, Rect::new(90.0, 90.0, 210.0, 210.0), None, true),
        vec![rect],
        "the existing rectangular marquee rule is unchanged"
    );
    assert_eq!(gesture(&d, &s, AROUND, Mods::default(), None), vec![Action::Exec("select.set".into(), json!({"ids": [rect.0, compound.0]}))]);
    d.node_mut(compound).unwrap().locked = true;
    assert_eq!(gesture(&d, &s, AROUND, Mods::default(), None), vec![Action::Exec("select.set".into(), json!({"ids": [rect.0]}))]);
    d.node_mut(compound).unwrap().locked = false;
    d.node_mut(compound).unwrap().visible = false;
    assert_eq!(gesture(&d, &s, AROUND, Mods::default(), None), vec![Action::Exec("select.set".into(), json!({"ids": [rect.0]}))]);
    d.node_mut(compound).unwrap().visible = true;
    let NodeKind::Compound { children, .. } = &mut d.node_mut(compound).unwrap().kind else { panic!() };
    *std::sync::Arc::make_mut(&mut children[1]).path_data_mut().unwrap() = shapes::rectangle(Rect::new(300.0, 120.0, 320.0, 140.0));
    assert_eq!(
        gesture(&d, &s, AROUND, Mods::default(), None),
        vec![Action::Exec("select.set".into(), json!({"ids": [rect.0]}))],
        "every member contributes to the compound's geometry"
    );
    for visible in [[true, false], [false, true], [true, true], [false, false]] {
        let NodeKind::Compound { children, .. } = &mut d.node_mut(compound).unwrap().kind else { panic!() };
        for (child, visible) in children.iter_mut().zip(visible) {
            std::sync::Arc::make_mut(child).visible = visible;
        }
        assert_eq!(
            gesture(&d, &s, AROUND, Mods::default(), None),
            vec![Action::Exec("select.set".into(), json!({"ids": [rect.0]}))],
            "a compound with an outside member is not enclosed: {visible:?}"
        );
    }
}
