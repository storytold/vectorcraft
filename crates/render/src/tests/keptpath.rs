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
    // The boxes and miter tips kept, run by run, are those of the path measured whole.
    let (runs, tips_of) = k.runs();
    let mut whole = crate::kept_paths::KeptPath::new(&Arc::new(n.clone()), 0).unwrap();
    if let Some(st) = &tips_of {
        whole.tips(st);
    }
    assert!(runs == whole.runs().0, "{what}: the same boxes and tips as the path measured whole");
}

fn path_of(d: &mut Document, id: NodeId) -> &mut PathData {
    d.node_mut(id).and_then(|n| n.path_data_mut()).unwrap()
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
