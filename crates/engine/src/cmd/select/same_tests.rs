//! Select > Same and Select by Color through [`walk_selectable_in`], which hands each object the
//! node that holds it, against copies of the walk and the tests that looked each object's parent up
//! by its id.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};

use super::{fill_of, is_live_paint, matching, same_candidate, same_paint, same_reference, selectable, stroke_of, walk_selectable_in};
use crate::Session;

/// `walk_selectable` as it was before it handed each object its parent.
fn walk_by_id(d: &Document, iso: Option<NodeId>, mut f: impl FnMut(&Node) -> bool) {
    fn inside(n: &Node, f: &mut impl FnMut(&Node) -> bool) {
        if n.visible && !n.locked && f(n) {
            for c in n.children().into_iter().flatten() {
                inside(c, f);
            }
        }
    }
    fn find(n: &Node, tops: &BTreeSet<NodeId>, f: &mut impl FnMut(&Node) -> bool) {
        if tops.contains(&n.id) {
            return inside(n, f);
        }
        for c in n.children().into_iter().flatten() {
            find(c, tops, f);
        }
    }
    let tops: BTreeSet<NodeId> = selectable(d, iso).into_iter().collect();
    for l in &d.layers {
        find(l, &tops, &mut f);
    }
}

/// `paint_reported` as it was, looking up `n`'s parent by its id.
fn paint_reported_by_id(d: &Document, n: &Node) -> bool {
    let compound = |n: &Node| matches!(n.kind, NodeKind::Compound { .. });
    compound(n) || (!n.is_container() && !d.parent_of(n.id).and_then(|p| d.node(p)).is_some_and(compound))
}

/// `live_paint_group` as it was, looking up the parent of `id`.
fn live_paint_group_by_id(d: &Document, id: NodeId) -> Option<NodeId> {
    d.parent_of(id).filter(|p| d.node(*p).is_some_and(is_live_paint))
}

/// The objects `select_where`'s loop took before the walk handed each object its parent.
fn matching_by_id(d: &Document, iso: Option<NodeId>, f: impl Fn(&Document, &Node) -> bool) -> Vec<NodeId> {
    let mut ids = vec![];
    walk_by_id(d, iso, |n| {
        if f(d, n) {
            ids.push(n.id);
            return false;
        }
        true
    });
    ids
}

/// The objects `same_in` took before the walk handed each object its parent.
fn same_by_id(d: &Document, iso: Option<NodeId>, refn: &Node, members: bool, live_paint: bool, eq: impl Fn(&Node, &Node) -> bool) -> Vec<NodeId> {
    let group = live_paint_group_by_id(d, refn.id);
    matching_by_id(d, iso, |d, n| {
        (!live_paint || live_paint_group_by_id(d, n.id) == group)
            && (if members { !n.is_container() } else { paint_reported_by_id(d, n) })
            && eq(n, refn)
    })
}

/// The objects `same_in` takes now.
fn same_now(d: &Document, iso: Option<NodeId>, refn: &Node, members: bool, live_paint: bool, eq: impl Fn(&Node, &Node) -> bool) -> Vec<NodeId> {
    let group = super::live_paint_group(d.parent_of(refn.id).and_then(|p| d.node(p)));
    matching(d, iso, |n, parent| same_candidate(n, parent, live_paint, group, members) && eq(n, refn))
}

#[track_caller]
fn run(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("`{cmd}` {p} failed: {e}"))
}

fn id(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap_or_else(|| panic!("no id in {v}")))
}

fn sel(s: &mut Session, ids: &[NodeId]) {
    run(s, "select.set", json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()}));
}

/// A 10 × 10 rectangle at (`x`, `y`) filled with `hex`.
fn rect(s: &mut Session, x: f64, y: f64, hex: &str) -> NodeId {
    let r = id(&run(s, "shape.rectangle", json!({"x": x, "y": y, "width": 10.0, "height": 10.0})));
    run(s, "paint.setFill", json!({"ids": [r.0], "color": hex}));
    r
}

/// A compound path of three rectangles at `y`, filled with `hex` → the compound path and its
/// member paths.
fn compound(s: &mut Session, y: f64, hex: &str) -> (NodeId, Vec<NodeId>) {
    let members: Vec<NodeId> = [0.0, 20.0, 40.0].map(|x| rect(s, 300.0 + x, y, hex)).into();
    sel(s, &members);
    let c = id(&run(s, "object.compoundPath.make", json!({})));
    run(s, "paint.setFill", json!({"ids": [c.0], "color": hex}));
    (c, members)
}

/// A Live Paint group of two overlapping squares at `y`, its left face red, its middle face blue and
/// its right face red → the group and its faces.
fn live_paint(s: &mut Session, y: f64) -> (NodeId, Vec<NodeId>) {
    let a = id(&run(s, "shape.rectangle", json!({"x": 0.0, "y": y, "width": 100.0, "height": 100.0})));
    let b = id(&run(s, "shape.rectangle", json!({"x": 50.0, "y": y, "width": 100.0, "height": 100.0})));
    sel(s, &[a, b]);
    let g = id(&run(s, "livePaint.make", json!({})));
    let faces = [(25.0, "#ff0000"), (75.0, "#0000ff"), (125.0, "#ff0000")]
        .map(|(x, hex)| NodeId(run(s, "livePaint.fill", json!({"group": g.0, "point": [x, y + 50.0], "color": hex}))["face"].as_u64().unwrap()))
        .into();
    (g, faces)
}

/// The objects of [`rich`] that tests pick from.
struct Rich {
    plain: Vec<NodeId>,
    compound: NodeId,
    members: Vec<NodeId>,
    group: NodeId,
    live_paint: NodeId,
    faces: Vec<NodeId>,
    sublayer: NodeId,
}

/// A document with red and blue rectangles, a compound path, a group with a hidden and a locked
/// rectangle, a locked group, a Live Paint group, a sublayer holding a rectangle and a second
/// compound path, and a hidden and a locked layer with a red rectangle each.
fn rich() -> (Session, Rich) {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 800, "height": 600}));
    let layer = s.doc().unwrap().doc.layers[0].id;
    let mut plain = vec![rect(&mut s, 0.0, 0.0, "#ff0000"), rect(&mut s, 20.0, 0.0, "#ff0000"), rect(&mut s, 40.0, 0.0, "#0000ff")];
    let (compound_id, members) = compound(&mut s, 0.0, "#ff0000");
    let [in_group, hidden, locked] = [0.0, 20.0, 40.0].map(|x| rect(&mut s, x, 200.0, "#ff0000"));
    let blue = rect(&mut s, 60.0, 200.0, "#0000ff");
    sel(&mut s, &[in_group, hidden, locked, blue]);
    let group = id(&run(&mut s, "object.group", json!({})));
    run(&mut s, "layer.setProps", json!({"id": hidden.0, "visible": false}));
    run(&mut s, "layer.setProps", json!({"id": locked.0, "locked": true}));
    let in_locked = rect(&mut s, 100.0, 200.0, "#ff0000");
    sel(&mut s, &[in_locked]);
    let locked_group = id(&run(&mut s, "object.group", json!({})));
    run(&mut s, "layer.setProps", json!({"id": locked_group.0, "locked": true}));
    let (live_paint_id, faces) = live_paint(&mut s, 300.0);
    plain.extend([in_group, blue]);
    let sublayer = id(&run(&mut s, "layer.newSublayer", json!({"parent": layer.0})));
    plain.push(rect(&mut s, 0.0, 500.0, "#ff0000"));
    compound(&mut s, 500.0, "#0000ff");
    for prop in ["visible", "locked"] {
        let l = id(&run(&mut s, "layer.new", json!({"top": true})));
        rect(&mut s, 200.0, 500.0, "#ff0000");
        run(&mut s, "layer.setProps", json!({"id": l.0, prop: prop == "locked"}));
    }
    let rich = Rich { plain, compound: compound_id, members, group, live_paint: live_paint_id, faces, sublayer };
    (s, rich)
}

/// The isolation scopes to try: none, the group, the compound path, and the Live Paint group.
fn scopes(r: &Rich) -> [Option<NodeId>; 4] {
    [None, Some(r.group), Some(r.compound), Some(r.live_paint)]
}

/// Isolate `iso`, leaving isolation mode first.
fn isolate(s: &mut Session, iso: Option<NodeId>) {
    if s.doc().unwrap().isolation.is_some() {
        run(s, "object.exitIsolation", json!({}));
    }
    if let Some(i) = iso {
        run(s, "object.isolate", json!({"id": i.0}));
        assert_eq!(s.doc().unwrap().isolation, Some(i));
    }
}

/// The walk visits the objects it visited before, in the same order and with the same objects left
/// out, and hands each the node that `Document::parent_of` names for it.
#[test]
fn the_walk_hands_each_object_the_node_that_holds_it() {
    let (mut s, r) = rich();
    // Look inside everything, and inside every other object only.
    let deeper: [fn(&Node) -> bool; 2] = [|_| true, |n| n.id.0 % 2 == 0];
    for iso in scopes(&r) {
        isolate(&mut s, iso);
        let st = s.doc().unwrap();
        let d = &st.doc;
        for f in deeper {
            let mut before = vec![];
            walk_by_id(d, st.isolation, |n| {
                before.push((n.id, d.parent_of(n.id)));
                f(n)
            });
            let mut now = vec![];
            walk_selectable_in(d, st.isolation, |n, parent| {
                now.push((n.id, parent.map(|p| p.id)));
                f(n)
            });
            assert!(!now.is_empty(), "isolating {iso:?}");
            assert_eq!(now, before, "isolating {iso:?}");
        }
    }
    // Every face, member path and the sublayer's art were visited somewhere above.
    let st = s.doc().unwrap();
    assert!(r.faces.iter().chain(&r.members).all(|f| st.doc.node(*f).is_some()));
    assert!(st.doc.node(r.sublayer).is_some_and(|n| n.is_layer()));
}

/// Select > Same and Select by Color select the objects they selected before, in the same order,
/// with the same reply and the same Select > Reselect state, from every kind of reference object
/// and in every isolation scope.
#[test]
fn select_same_takes_the_objects_it_took_when_it_looked_up_parents() {
    type Eq = fn(&Node, &Node) -> bool;
    let fill: Eq = |a, b| same_paint(&fill_of(a), &fill_of(b), false);
    let stroke: Eq = |a, b| same_paint(&stroke_of(a).0, &stroke_of(b).0, false);
    let commands: [(&str, bool, Eq); 6] = [
        ("select.same.fillColor", true, fill),
        ("select.same.fillAndStroke", true, |a, b| {
            same_paint(&fill_of(a), &fill_of(b), false) && same_paint(&stroke_of(a).0, &stroke_of(b).0, false)
        }),
        ("select.same.strokeColor", false, stroke),
        ("select.same.opacity", false, |a, b| (a.opacity - b.opacity).abs() < 1e-6),
        ("select.same.blendingMode", false, |a, b| a.blend == b.blend),
        ("select.same.shapeType", false, |a, b| a.kind_label() == b.kind_label()),
    ];
    let (mut s, r) = rich();
    assert!(!s.prefs.select_same_tint_percent);
    let froms: Vec<NodeId> = r.plain.iter().chain(&r.members).chain(&r.faces).copied().chain([r.compound]).collect();
    let mut checked = 0;
    for iso in scopes(&r) {
        isolate(&mut s, iso);
        for from in &froms {
            for (cmd, live_paint, eq) in commands {
                // An object outside the isolated one may not be selectable; the selection then
                // stays, and both sides compare with what it holds.
                let _ = s.execute("select.set", &json!({"ids": [from.0]}));
                let st = s.doc().unwrap();
                let Ok((refn, members)) = same_reference(st, cmd) else {
                    assert!(s.execute(cmd, &json!({})).is_err(), "{cmd} from {from:?} isolating {iso:?}");
                    continue;
                };
                let refn = refn.clone();
                let expected = same_by_id(&st.doc, st.isolation, &refn, members, live_paint, eq);
                assert_eq!(same_now(&st.doc, st.isolation, &refn, members, live_paint, eq), expected, "{cmd} from {from:?} isolating {iso:?}");
                let before = st.selection.objects.clone();
                let reply = run(&mut s, cmd, json!({}));
                let st = s.doc().unwrap();
                assert_eq!(st.selection.objects, expected, "{cmd} from {from:?} isolating {iso:?}");
                assert_eq!(reply, json!({"count": expected.len()}));
                assert_eq!(st.last_selection_cmd, Some((cmd.to_string(), json!({}), before)));
                checked += 1;
            }
        }
        for hex in ["#ff0000", "#0000ff"] {
            let st = s.doc().unwrap();
            let reads_as = |n: &Node| fill_of(n).color().is_some_and(|c| c.to_hex() == hex);
            let expected = matching_by_id(&st.doc, st.isolation, |d, n| paint_reported_by_id(d, n) && reads_as(n));
            let p = json!({"fill": hex});
            let reply = run(&mut s, "select.color", p.clone());
            let st = s.doc().unwrap();
            assert_eq!(st.selection.objects, expected, "select.color {hex} isolating {iso:?}");
            assert_eq!(reply, json!({"count": expected.len()}));
            assert_eq!(st.last_selection_cmd.as_ref().map(|(c, p, _)| (c.as_str(), p)), Some(("select.color", &p)));
        }
    }
    // Most of the references can be selected in some scope.
    assert!(checked > froms.len() * commands.len(), "{checked} checks");
}

/// In a damaged document that holds an id twice, the parent that decides whether an object is a
/// member of a compound path or a face of a Live Paint group is the one that holds the object the
/// walk is at, not the one that holds the id's first place in the tree.
#[test]
fn select_same_reads_the_parent_of_the_object_it_is_at() {
    for in_live_paint in [false, true] {
        let mut s = Session::new();
        run(&mut s, "file.new", json!({"width": 800, "height": 600}));
        let layer = s.doc().unwrap().doc.layers[0].id;
        let (_, inner) = if in_live_paint { live_paint(&mut s, 300.0) } else { compound(&mut s, 0.0, "#0000ff") };
        let from = rect(&mut s, 0.0, 0.0, "#ff0000");
        // A red rectangle above the others that has the id of a face or member path.
        let mut copy = s.doc().unwrap().doc.node(from).unwrap().clone();
        copy.id = inner[0];
        Arc::make_mut(&mut s.doc_mut().unwrap().doc).insert(Some(layer), usize::MAX, copy).unwrap();
        let mut commands = vec![("select.same.fillColor", json!({})), ("select.same.fillAndStroke", json!({}))];
        // Select by Color takes the red faces of a Live Paint group too, so it is tried on the
        // compound path only.
        if !in_live_paint {
            commands.push(("select.color", json!({"fill": "#ff0000"})));
        }
        for (cmd, p) in commands {
            sel(&mut s, &[from]);
            let reply = run(&mut s, cmd, p);
            assert_eq!(reply["count"], 2, "{cmd}, Live Paint: {in_live_paint}");
        }
    }
}

/// A document of `n` objects on one layer: red and blue rectangles, one in 25 a compound path of
/// three, and two Live Paint groups that hold one in 25 each as faces → the session, with the first
/// rectangle selected.
fn big(n: usize) -> Session {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 800, "height": 600}));
    let (c, _) = compound(&mut s, 0.0, "#ff0000");
    let (g, _) = live_paint(&mut s, 300.0);
    let [red, blue] = ["#ff0000", "#0000ff"].map(|hex| rect(&mut s, 0.0, 0.0, hex));
    let mut d = Document::clone(&s.doc().unwrap().doc);
    let layer = d.layers[0].id;
    let template = |id: NodeId| d.node(id).unwrap().clone();
    let (c, g, red, blue) = (template(c), template(g), template(red), template(blue));
    fn renumber(n: &Node, d: &mut Document) -> Node {
        let mut n = n.clone();
        n.id = d.alloc_id();
        for c in n.children_mut().into_iter().flatten() {
            *c = Arc::new(renumber(c, d));
        }
        n
    }
    d.children_mut(Some(layer)).unwrap().clear();
    let faces: Vec<Arc<Node>> = g.children().into_iter().flatten().cloned().collect();
    for _ in 0..2 {
        let mut g = renumber(&g, &mut d);
        if let Some(ch) = g.children_mut() {
            ch.clear();
            for i in 0..n / 25 {
                ch.push(Arc::new(renumber(&faces[i % faces.len()], &mut d)));
            }
        }
        d.insert(Some(layer), usize::MAX, g).unwrap();
    }
    for i in 0..n - 2 * (n / 25) {
        let node = match i % 25 {
            0 => renumber(&c, &mut d),
            k if k % 2 == 0 => renumber(&red, &mut d),
            _ => renumber(&blue, &mut d),
        };
        d.insert(Some(layer), usize::MAX, node).unwrap();
    }
    let first = d.layers[0].children().unwrap().iter().find(|n| matches!(n.kind, NodeKind::Path { .. })).unwrap().id;
    s.add_document(d, None);
    sel(&mut s, &[first]);
    s
}

/// The CPU time this thread has used, from Linux's scheduler statistics; `None` elsewhere.
fn thread_cpu() -> Option<Duration> {
    let stat = std::fs::read_to_string("/proc/thread-self/schedstat").ok()?;
    stat.split_whitespace().next()?.parse().ok().map(Duration::from_nanos)
}

/// The time `f` takes: this thread's CPU time where Linux reports it, otherwise the wall clock.
fn time(f: impl FnOnce()) -> Duration {
    match thread_cpu() {
        Some(before) => {
            f();
            thread_cpu().map_or(Duration::ZERO, |after| after.saturating_sub(before))
        }
        None => {
            let start = Instant::now();
            f();
            start.elapsed()
        }
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Timings of Select > Same's search, by id lookups and with the walk's parents, on 5,000, 50,000
/// and 200,000 objects, run in turn on the same document, 7 runs each. Ignored by default; it
/// prints the median, fastest and slowest run and sets no bound on them:
/// `cargo test --release -p vectorcraft-engine --lib select_same_perf -- --ignored --nocapture`.
#[test]
#[ignore = "a benchmark: run with --ignored --nocapture"]
fn select_same_perf() {
    const RUNS: usize = 7;
    type Eq = fn(&Node, &Node) -> bool;
    let fill: Eq = |a, b| same_paint(&fill_of(a), &fill_of(b), false);
    let stroke: Eq = |a, b| same_paint(&stroke_of(a).0, &stroke_of(b).0, false);
    eprintln!("select_same_perf: {}", if thread_cpu().is_some() { "thread CPU time" } else { "wall clock" });
    eprintln!(
        "select_same_perf: {:>7} {:<12} {:>8} {:>26} {:>26}",
        "objects", "command", "selected", "by id ms (min-max)", "walk parents ms (min-max)"
    );
    for n in [5_000, 50_000, 200_000] {
        let s = big(n);
        let st = s.doc().unwrap();
        let (d, iso) = (&st.doc, st.isolation);
        let objects = {
            let mut k = 0;
            walk_selectable_in(d, iso, |_, _| {
                k += 1;
                true
            });
            k
        };
        for (cmd, live_paint, eq) in [("fillColor", true, fill), ("strokeColor", false, stroke)] {
            let (refn, members) = same_reference(st, cmd).unwrap();
            let (mut old, mut new) = (Vec::new(), Vec::new());
            let (mut a, mut b) = (vec![], vec![]);
            for run in 0..RUNS {
                // The two take turns going first.
                if run % 2 == 0 {
                    old.push(time(|| a = std::hint::black_box(same_by_id(d, iso, refn, members, live_paint, eq))));
                    new.push(time(|| b = std::hint::black_box(same_now(d, iso, refn, members, live_paint, eq))));
                } else {
                    new.push(time(|| b = std::hint::black_box(same_now(d, iso, refn, members, live_paint, eq))));
                    old.push(time(|| a = std::hint::black_box(same_by_id(d, iso, refn, members, live_paint, eq))));
                }
                assert_eq!(a, b, "{n} objects, {cmd}");
            }
            old.sort();
            new.sort();
            eprintln!(
                "select_same_perf: {objects:>7} {cmd:<12} {:>8} {:>10.3} ({:>6.3}-{:>7.3}) {:>10.3} ({:>6.3}-{:>7.3})",
                b.len(),
                ms(old[RUNS / 2]),
                ms(old[0]),
                ms(old[RUNS - 1]),
                ms(new[RUNS / 2]),
                ms(new[0]),
                ms(new[RUNS - 1]),
            );
        }
    }
}
