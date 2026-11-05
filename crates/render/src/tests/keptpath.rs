//! The paths of long paths and compound paths are kept by object id (`kept_paths`): after an edit,
//! the renderer changes only the parts of the path that the edit moved, and draws and culls the
//! object as a renderer that makes its path again does.

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, LineCap, LineJoin, Node, NodeId, NodeKind, StrokeLayer};
use vectorcraft_geom::{Anchor, PathData, Point, SubPath, Vec2};

use crate::*;

const SIZE: u32 = 120;

fn stroke(width: f64, join: LineJoin, cap: LineCap) -> StrokeLayer {
    let mut st = StrokeLayer::new(Paint::solid(Color::BLACK), width);
    st.join = join;
    st.cap = cap;
    st.miter_limit = 10.0;
    st
}

/// `n` anchors 8 pt apart from x = `x0`, up and down by 30 pt: sharp corners, every fifth anchor
/// smooth, and every 50th anchor doubled (a segment without length).
fn zigzag(n: usize, x0: f64) -> SubPath {
    let mut anchors = vec![];
    for i in 0..n {
        let p = Point::new(x0 + i as f64 * 8.0, if i % 2 == 0 { 0.0 } else { 30.0 });
        anchors.push(if i % 5 == 0 { Anchor::smooth(p, p + Vec2::new(3.0, 4.0)) } else { Anchor::corner(p) });
        if i % 50 == 25 {
            anchors.push(Anchor::corner(p));
        }
    }
    SubPath::new(anchors, false)
}

/// A closed polygon of `n` anchors round a circle of radius `r` about `c`.
fn ring(n: usize, c: Point, r: f64) -> SubPath {
    let points: Vec<Point> = (0..n).map(|i| c + Vec2::from_angle(i as f64 / n as f64 * std::f64::consts::TAU) * r).collect();
    SubPath::polyline(&points, true)
}

/// `d` with `n` added to its first layer, and `n`'s id.
fn add(d: &mut Document, n: Node) -> NodeId {
    let (id, l) = (n.id, d.layers[0].id);
    d.insert(Some(l), 0, n).unwrap();
    id
}

fn look(items: Vec<StrokeLayer>) -> Appearance {
    Appearance { items: items.into_iter().map(AppearanceItem::Stroke).collect(), ..Default::default() }
}

/// The view with (12,000, 15) in the middle of the frame.
fn view() -> Affine {
    Affine::translate((f64::from(SIZE) / 2.0 - 12_000.0, f64::from(SIZE) / 2.0 - 15.0))
}

fn render(r: &mut Renderer, d: &Document) -> Rendered {
    r.render(d, SIZE, SIZE, view(), &RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() })
}

/// Draw `d` with `r` and check what `r` drew and kept for object `id` against what a new renderer
/// draws and what the object measures without kept paths.
fn check(r: &mut Renderer, d: &Document, id: NodeId, what: &str) {
    let img = render(r, d);
    assert!(img.pixels == render(&mut Renderer::new(), d).pixels, "{what}: the same pixels as a new renderer");
    let n = d.node(id).unwrap();
    let e = r.geom.get(&(n as *const Node as usize)).unwrap_or_else(|| panic!("{what}: measured"));
    assert_eq!(e.bounds, crate::brush_fx::cull_bounds(&d.symbol_boxes(), n), "{what}: the same cull bounds");
    if let Some(p) = &e.path {
        assert!(**p == n.stroke_path().unwrap(), "{what}: the same path");
    }
    let k = r.kept.get(&id).unwrap_or_else(|| panic!("{what}: its path kept"));
    assert!(std::ptr::eq(Arc::as_ptr(&k.node), n), "{what}: its path kept");
    // The subpaths kept are those of the path measured whole, each run holds from one segment to
    // twice the segments of a run, and the boxes and miter tips kept, run by run, are those
    // measured again from the path.
    let whole = crate::kept_paths::KeptPath::new(&Arc::new(n.clone()), 0).unwrap();
    let (layout, fresh) = (k.layout(), whole.layout());
    assert!(layout.iter().map(|l| l.0).eq(fresh.iter().map(|l| l.0)), "{what}: the same subpaths as the path measured whole");
    for ((sub, runs), ((_, segs, ..), _)) in layout.iter().zip(&fresh) {
        assert_eq!(runs.iter().sum::<usize>(), *segs, "{what}: the runs of {sub:?} hold its segments");
        assert!(runs.iter().all(|&s| (1..=2 * crate::kept_paths::RUN).contains(&s)), "{what}: the runs of {sub:?}: {runs:?}");
    }
    let (runs, again, _) = k.runs();
    assert!(runs == again, "{what}: the same boxes and tips as measured again");
}

/// Check `d` as [`check`] does, then the document before the edit (undone) and `d` again (redone).
fn check_undo(r: &mut Renderer, before: &Document, d: &Document, id: NodeId, what: &str) {
    check(r, d, id, what);
    check(r, before, id, &format!("{what}, undone"));
    check(r, d, id, &format!("{what}, redone"));
}

/// The members of compound path `id` of `d`.
fn members(d: &mut Document, id: NodeId) -> &mut Vec<Arc<Node>> {
    match d.node_mut(id).map(|n| &mut n.kind) {
        Some(NodeKind::Compound { children, .. }) => children,
        _ => panic!("a compound path"),
    }
}

fn path_of(d: &mut Document, id: NodeId) -> &mut PathData {
    d.node_mut(id).and_then(|n| n.path_data_mut()).unwrap()
}

fn corner(x: f64, y: f64) -> Anchor {
    Anchor::corner(Point::new(x, y))
}

fn moved(a: &mut Anchor, by: Vec2) {
    *a = a.transform(Affine::translate(by));
}

#[test]
fn a_long_path_draws_and_culls_after_each_edit_as_when_its_path_is_made_again() {
    let mut d = Document::new(1000.0, 1000.0);
    let path = PathData::new(vec![
        zigzag(3000, 0.0),
        SubPath::new(vec![Anchor::corner(Point::new(50.0, 200.0))], false),
        SubPath::new(vec![], false),
        ring(200, Point::new(12_000.0, 3000.0), 100.0),
    ]);
    let n = Node::path(d.alloc_id(), path, look(vec![stroke(6.0, LineJoin::Miter, LineCap::Butt), stroke(14.0, LineJoin::Round, LineCap::Round)]));
    let id = add(&mut d, n);
    let mut r = Renderer::new();
    check(&mut r, &d, id, "drawn first");
    let first = d.clone();
    type Edit = fn(&mut PathData);
    let edits: [(&str, Edit); 12] = [
        ("one anchor moved", |p| moved(&mut p.subpaths[0].anchors[1530], Vec2::new(0.0, 40.0))),
        ("a handle moved", |p| {
            let a = &mut p.subpaths[0].anchors[1532];
            a.h_out = a.p + Vec2::new(10.0, -20.0);
        }),
        ("an anchor moved onto the one before it", |p| {
            let at = p.subpaths[0].anchors[1534].p;
            p.subpaths[0].anchors[1535] = Anchor::corner(at);
        }),
        ("that anchor moved off it", |p| moved(&mut p.subpaths[0].anchors[1535], Vec2::new(5.0, 5.0))),
        ("a doubled anchor moved off its twin", |p| moved(&mut p.subpaths[0].anchors[1536], Vec2::new(0.0, -12.0))),
        ("1,200 anchors moved", |p| p.subpaths[0].anchors[..1200].iter_mut().for_each(|a| moved(a, Vec2::new(2.0, 1.0)))),
        ("an anchor added", |p| _ = p.subpaths[0].insert_anchor(1520, 0.5)),
        ("an anchor deleted", |p| _ = p.subpaths[0].anchors.remove(1521)),
        ("the subpath closed", |p| p.subpaths[0].closed = true),
        ("the first anchor of the closed subpath moved", |p| moved(&mut p.subpaths[0].anchors[0], Vec2::new(0.0, -60.0))),
        ("the subpath opened", |p| p.subpaths[0].closed = false),
        ("an anchor of another subpath moved into the view", |p| p.subpaths[3].anchors[50] = Anchor::corner(Point::new(11_990.0, 20.0))),
    ];
    for (what, edit) in edits {
        edit(path_of(&mut d, id));
        check(&mut r, &d, id, what);
    }
    check(&mut r, &first, id, "undone to the start");
    moved(&mut path_of(&mut d, id).subpaths[1].anchors[0], Vec2::new(5.0, 5.0));
    check(&mut r, &d, id, "redone, the one-anchor subpath moved");
    path_of(&mut d, id).subpaths[2].anchors.push(Anchor::corner(Point::new(12_010.0, 40.0)));
    check(&mut r, &d, id, "the empty subpath given an anchor");
}

#[test]
fn a_long_compound_path_draws_and_culls_after_each_edit_as_when_its_path_is_made_again() {
    let mut d = Document::new(1000.0, 1000.0);
    let none = Appearance::default();
    let members = [
        PathData::single(zigzag(1500, 6000.0)),
        PathData::single(ring(1500, Point::new(12_000.0, 15.0), 45.0)),
        PathData::single(SubPath::new(vec![Anchor::corner(Point::new(11_980.0, 0.0))], false)),
        PathData::new(vec![]),
    ];
    let ids: Vec<NodeId> = members.iter().map(|_| d.alloc_id()).collect();
    let children = ids.iter().zip(members).map(|(id, p)| Arc::new(Node::path(*id, p, none.clone()))).collect();
    let mut n = Node::new(d.alloc_id(), NodeKind::Compound { children, rule: FillRule::EvenOdd });
    n.appearance = look(vec![stroke(4.0, LineJoin::Miter, LineCap::Square)]).into();
    let id = add(&mut d, n);
    let mut r = Renderer::new();
    check(&mut r, &d, id, "drawn first");
    let first = d.clone();
    moved(&mut path_of(&mut d, ids[1]).subpaths[0].anchors[700], Vec2::new(-30.0, 10.0));
    check(&mut r, &d, id, "an anchor of the ring moved");
    moved(&mut path_of(&mut d, ids[0]).subpaths[0].anchors[0], Vec2::new(-30.0, 10.0));
    check(&mut r, &d, id, "the first anchor of the zigzag moved");
    moved(&mut path_of(&mut d, ids[2]).subpaths[0].anchors[0], Vec2::new(-30.0, 10.0));
    check(&mut r, &d, id, "the one-anchor member moved");
    path_of(&mut d, ids[3]).subpaths.push(SubPath::new(vec![Anchor::corner(Point::new(12_020.0, 30.0))], false));
    check(&mut r, &d, id, "the empty member given an anchor");
    check(&mut r, &first, id, "undone to the start");
}

#[test]
fn edits_in_runs_of_segments_without_length_measure_as_when_the_path_is_made_again() {
    // A closed subpath whose anchors move onto their neighbours and off them again, the joints'
    // miter tips skipping the segments without length, round the start too.
    let mut d = Document::new(1000.0, 1000.0);
    let mut sp = zigzag(1200, 11_900.0);
    sp.closed = true;
    let n = Node::path(d.alloc_id(), PathData::single(sp), look(vec![stroke(5.0, LineJoin::Miter, LineCap::Butt)]));
    let id = add(&mut d, n);
    let mut r = Renderer::new();
    check(&mut r, &d, id, "drawn first");
    let mut seed: u64 = 7;
    let mut next = |m: usize| {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) as usize % m
    };
    for step in 0..60 {
        let p = path_of(&mut d, id);
        let n = p.subpaths[0].anchors.len();
        // Near the start, the end, or where one run of 128 segments ends and the next starts.
        let k = [next(40), n - 1 - next(40), 128 * (1 + next(8)) + next(3)][step % 3];
        let to = match next(3) {
            0 => p.subpaths[0].anchors[(k + n - 1) % n].p,
            1 => p.subpaths[0].anchors[(k + 1) % n].p,
            _ => p.subpaths[0].anchors[k].p + Vec2::new(next(20) as f64 - 10.0, next(20) as f64 - 10.0),
        };
        p.subpaths[0].anchors[k] = Anchor::corner(to);
        check(&mut r, &d, id, &format!("step {step}, anchor {k}"));
    }
}

#[test]
fn a_long_path_draws_and_culls_after_anchors_are_added_or_deleted_as_when_its_path_is_made_again() {
    let mut d = Document::new(1000.0, 1000.0);
    let path = PathData::new(vec![
        zigzag(3000, 0.0),
        ring(1200, Point::new(12_000.0, 15.0), 45.0),
        SubPath::new(vec![Anchor::corner(Point::new(11_990.0, 20.0))], false),
        SubPath::new(vec![], false),
        SubPath::new(vec![Anchor::corner(Point::new(11_995.0, 0.0)), Anchor::corner(Point::new(12_005.0, 30.0))], false),
    ]);
    let n = Node::path(d.alloc_id(), path, look(vec![stroke(6.0, LineJoin::Miter, LineCap::Butt), stroke(14.0, LineJoin::Round, LineCap::Round)]));
    let id = add(&mut d, n);
    let mut r = Renderer::new();
    check(&mut r, &d, id, "drawn first");
    type Edit = fn(&mut PathData);
    let edits: [(&str, Edit); 28] = [
        // Anchor 1,152, a corner at (9,032, 30), ends the segments of the first nine runs: the
        // joint there is in the run before.
        ("an anchor added where a run of segments starts", |p| p.subpaths[0].anchors.insert(1153, corner(9_036.0, -30.0))),
        ("an anchor added", |p| _ = p.subpaths[0].insert_anchor(1520, 0.5)),
        ("an anchor deleted", |p| _ = p.subpaths[0].anchors.remove(1521)),
        ("an anchor added at the start", |p| p.subpaths[0].anchors.insert(0, corner(-8.0, 30.0))),
        ("an anchor added at the end", |p| p.subpaths[0].anchors.push(corner(24_100.0, 0.0))),
        ("the first anchor deleted", |p| _ = p.subpaths[0].anchors.remove(0)),
        ("the last anchor deleted", |p| _ = p.subpaths[0].anchors.pop()),
        ("an anchor added onto the one before it", |p| {
            let a = p.subpaths[0].anchors[1499];
            p.subpaths[0].anchors.insert(1500, Anchor::corner(a.p));
        }),
        ("300 anchors deleted", |p| _ = p.subpaths[0].anchors.drain(1100..1400)),
        ("400 anchors added", |p| {
            let at: Vec<Anchor> = (0..400).map(|i| corner(8_800.0 + f64::from(i) * 0.5, if i % 2 == 0 { -20.0 } else { 50.0 })).collect();
            p.subpaths[0].anchors.splice(1100..1100, at);
        }),
        ("anchors moved and others deleted", |p| {
            p.subpaths[0].anchors[10] = corner(80.0, -40.0);
            p.subpaths[0].anchors.drain(2000..2010);
        }),
        ("the subpath closed", |p| p.subpaths[0].closed = true),
        ("an anchor added at the start of the closed subpath", |p| p.subpaths[0].anchors.insert(0, corner(-8.0, 60.0))),
        ("an anchor added after the first of the closed subpath", |p| p.subpaths[0].anchors.insert(1, corner(-20.0, -30.0))),
        ("an anchor added at the end of the closed subpath", |p| p.subpaths[0].anchors.push(corner(24_000.0, 60.0))),
        ("the first anchor of the closed subpath deleted", |p| _ = p.subpaths[0].anchors.remove(0)),
        ("the last anchor of the closed subpath deleted", |p| _ = p.subpaths[0].anchors.pop()),
        ("the subpath opened", |p| p.subpaths[0].closed = false),
        ("an anchor of the ring added in the view", |p| p.subpaths[1].anchors.insert(300, corner(12_010.0, 10.0))),
        ("an anchor of the ring deleted", |p| _ = p.subpaths[1].anchors.remove(600)),
        ("the ring opened", |p| p.subpaths[1].closed = false),
        ("the ring closed", |p| p.subpaths[1].closed = true),
        ("the one-anchor subpath given a second anchor", |p| p.subpaths[2].anchors.push(corner(12_000.0, 40.0))),
        ("the second anchor deleted again", |p| _ = p.subpaths[2].anchors.pop()),
        ("the one-anchor subpath emptied", |p| p.subpaths[2].anchors.clear()),
        ("the empty subpath given an anchor", |p| p.subpaths[3].anchors.push(corner(12_015.0, 5.0))),
        ("the two-anchor subpath closed", |p| p.subpaths[4].closed = true),
        ("an anchor deleted from the two-anchor subpath", |p| _ = p.subpaths[4].anchors.remove(0)),
    ];
    for (what, edit) in edits {
        let before = d.clone();
        edit(path_of(&mut d, id));
        check_undo(&mut r, &before, &d, id, what);
    }
}

#[test]
fn a_long_compound_path_draws_and_culls_after_members_are_added_or_removed_as_when_its_path_is_made_again() {
    let mut d = Document::new(1000.0, 1000.0);
    let none = Appearance::default();
    let member = |d: &mut Document, p: PathData| Arc::new(Node::path(d.alloc_id(), p, none.clone()));
    let children = vec![
        member(&mut d, PathData::single(zigzag(1500, 6000.0))),
        member(&mut d, PathData::single(ring(1500, Point::new(12_000.0, 15.0), 45.0))),
        member(&mut d, PathData::single(SubPath::new(vec![Anchor::corner(Point::new(11_980.0, 0.0))], false))),
    ];
    let mut n = Node::new(d.alloc_id(), NodeKind::Compound { children, rule: FillRule::EvenOdd });
    n.appearance = look(vec![stroke(4.0, LineJoin::Miter, LineCap::Square)]).into();
    let id = add(&mut d, n);
    let mut r = Renderer::new();
    check(&mut r, &d, id, "drawn first");
    let edits: [(&str, PathData, usize); 4] = [
        ("a member added at the end", PathData::single(ring(40, Point::new(12_010.0, 20.0), 8.0)), 3),
        ("a member added at the start", PathData::single(zigzag(30, 11_950.0)), 0),
        ("a member added between", PathData::new(vec![]), 2),
        ("a member of two subpaths added between", PathData::new(vec![ring(5, Point::new(11_990.0, 25.0), 4.0), zigzag(3, 11_970.0)]), 1),
    ];
    for (what, p, at) in edits {
        let before = d.clone();
        let m = member(&mut d, p);
        members(&mut d, id).insert(at, m);
        check_undo(&mut r, &before, &d, id, what);
    }
    // Seven members: the first, the last, then the third removed.
    for (what, at) in [("the first member removed", 0), ("the last member removed", 5), ("a member removed between", 2)] {
        let before = d.clone();
        members(&mut d, id).remove(at);
        check_undo(&mut r, &before, &d, id, what);
    }
    // Left: the member of two subpaths, the zigzag, the ring and the one-anchor member.
    let ids: Vec<NodeId> = members(&mut d, id).iter().map(|m| m.id).collect();
    let before = d.clone();
    path_of(&mut d, ids[1]).subpaths.push(SubPath::new(vec![corner(12_020.0, 30.0)], false));
    check_undo(&mut r, &before, &d, id, "a subpath added to a member");
    let before = d.clone();
    path_of(&mut d, ids[1]).subpaths[0].anchors.insert(10, corner(12_000.0, 60.0));
    path_of(&mut d, ids[2]).subpaths[0].anchors.remove(700);
    check_undo(&mut r, &before, &d, id, "anchors added and deleted in two members");
}

#[test]
fn adding_and_deleting_anchors_of_a_long_path_changes_its_kept_path_in_place() {
    let mut d = Document::new(1000.0, 1000.0);
    let n = Node::path(d.alloc_id(), PathData::single(zigzag(3000, 0.0)), look(vec![stroke(6.0, LineJoin::Miter, LineCap::Butt)]));
    let id = add(&mut d, n);
    let mut r = Renderer::new();
    render(&mut r, &d);
    let kept = |r: &Renderer| r.kept.get(&id).map(|k| Arc::as_ptr(&k.path) as usize);
    let before = kept(&r);
    assert!(before.is_some(), "the path is kept");
    type Edit = fn(&mut PathData);
    let edits: [(&str, Edit); 3] = [
        ("an anchor added", |p| _ = p.subpaths[0].insert_anchor(1500, 0.5)),
        ("an anchor deleted", |p| _ = p.subpaths[0].anchors.remove(1700)),
        ("the subpath closed", |p| p.subpaths[0].closed = true),
    ];
    for (what, edit) in edits {
        edit(path_of(&mut d, id));
        check(&mut r, &d, id, what);
        assert_eq!(kept(&r), before, "{what}: the kept path changed in place");
    }
    // An edit that leaves the anchors as they were leaves the kept path as it is, also while
    // something else holds it.
    let held = r.kept.get(&id).map(|k| k.path.clone()).unwrap();
    d.node_mut(id).unwrap().appearance = look(vec![stroke(8.0, LineJoin::Miter, LineCap::Butt)]).into();
    check(&mut r, &d, id, "the stroke changed");
    assert!(Arc::ptr_eq(&held, &r.kept.get(&id).unwrap().path), "the stroke changed: the kept path left as it is");
}

#[test]
fn deleting_an_anchor_of_an_open_path_and_clearing_the_handles_past_its_ends_measures_one_run_again() {
    let mut d = Document::new(1000.0, 1000.0);
    let mut sp = zigzag(3000, 0.0);
    if let Some(a) = sp.anchors.last_mut() {
        a.h_out = a.p + Vec2::new(3.0, 4.0);
    }
    assert!(sp.anchors[0].has_in(), "the first anchor has a handle before it");
    let n = Node::path(d.alloc_id(), PathData::single(sp), look(vec![stroke(6.0, LineJoin::Miter, LineCap::Butt)]));
    let id = add(&mut d, n);
    let mut r = Renderer::new();
    render(&mut r, &d);
    let runs = |r: &Renderer| r.kept.get(&id).map(|k| k.layout()[0].1.clone()).unwrap();
    let before = runs(&r);
    // Anchor 1,500 deleted, and the handles before the first anchor and after the last cleared:
    // they draw nothing on an open subpath.
    let p = &mut path_of(&mut d, id).subpaths[0];
    p.anchors.remove(1500);
    if let Some(a) = p.anchors.first_mut() {
        a.h_in = a.p;
    }
    if let Some(a) = p.anchors.last_mut() {
        a.h_out = a.p;
    }
    check(&mut r, &d, id, "an anchor deleted and the end handles cleared");
    let after = runs(&r);
    let changed: Vec<usize> = (0..before.len().max(after.len())).filter(|&i| before.get(i) != after.get(i)).collect();
    assert_eq!(changed, vec![1499 / crate::kept_paths::RUN], "only the run that held the anchor changed: {before:?} -> {after:?}");
}

#[test]
fn moving_one_anchor_of_a_long_path_changes_its_kept_path_in_place() {
    let mut d = Document::new(1000.0, 1000.0);
    let n = Node::path(d.alloc_id(), PathData::single(zigzag(3000, 0.0)), look(vec![stroke(6.0, LineJoin::Miter, LineCap::Butt)]));
    let id = add(&mut d, n);
    let mut r = Renderer::new();
    render(&mut r, &d);
    let kept = |r: &Renderer| r.kept.get(&id).map(|k| Arc::as_ptr(&k.path) as usize);
    let before = kept(&r);
    assert!(before.is_some(), "the path is kept");
    moved(&mut path_of(&mut d, id).subpaths[0].anchors[1500], Vec2::new(0.0, 40.0));
    check(&mut r, &d, id, "one anchor moved");
    assert_eq!(kept(&r), before, "the kept path changed in place");
}
