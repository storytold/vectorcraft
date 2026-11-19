//! The Fill/Stroke proxies' "?" ([`DocState::proxy_mixed`]) finds the selected objects through the
//! id cache: it answers as a walk of the whole document does, and with a few objects selected it
//! reads only those.

use std::cell::Cell;
use std::collections::HashSet;
use std::sync::Arc;

use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, LayerColor, Node, NodeId, NodeKind};
use vectorcraft_geom::{Rect, shapes};

use super::{live_paint_parts, painted, picked_colors, proxy_paint, same_in_proxy};
use crate::{DocState, Session};

thread_local! {
    /// [`gathered`] of this thread.
    static GATHERED: Cell<u64> = const { Cell::new(0) };
}

/// Adds one to [`gathered`]: `proxy_mixed_with` read a node while looking for the selected ones.
pub(super) fn note_gathered() {
    GATHERED.set(GATHERED.get() + 1);
}

/// How many nodes `proxy_mixed_with` has read on this thread while looking for the selected ones.
fn gathered() -> u64 {
    GATHERED.get()
}

/// `proxy_mixed_with` as it was when it found the selected objects by walking the whole document.
fn walked(st: &DocState, item: Option<usize>, points: &[(NodeId, Vec<usize>)]) -> (bool, bool) {
    if st.selection.is_empty() {
        return (false, false);
    }
    if st.selection.symbol_part.is_some() {
        return (false, false);
    }
    let ids: HashSet<NodeId> = st.selection.objects.iter().copied().collect();
    let (mut selected, mut fills, mut strokes) = (vec![], vec![], vec![]);
    let mut covered = HashSet::new();
    st.doc.walk(|n| {
        if ids.contains(&n.id) {
            selected.push(n);
            if !covered.contains(&n.id) {
                covered.extend(live_paint_parts(n));
                painted(n, true, false, &mut fills);
                painted(n, true, true, &mut strokes);
            }
        }
    });
    let first = st.selection.objects.first().and_then(|id| st.doc.node(*id));
    let differ = |stroke: bool| {
        let item = first.and_then(|f| f.appearance.item_of_kind(item, !stroke));
        let set = if item.is_some() {
            &selected
        } else if stroke {
            &strokes
        } else {
            &fills
        };
        let mut paints = set.iter().flat_map(|n| match picked_colors(n, if stroke || item.is_some() { &[] } else { points }) {
            c if c.is_empty() => vec![proxy_paint(n, stroke, item)],
            c => c.into_iter().map(Paint::solid).collect(),
        });
        let Some(p0) = paints.next() else { return false };
        paints.any(|p| !same_in_proxy(&p0, &p))
    };
    (differ(false), differ(true))
}

/// `proxy_mixed_with` answers as [`walked`] does for `st`'s selection, for every Appearance item
/// and with and without `points`, with the id cache built and without it.
fn agree(st: &DocState, points: &[(NodeId, Vec<usize>)], what: &str) {
    for cold in [true, false] {
        if cold {
            st.doc.drop_lookup_cache();
        }
        for item in [None, Some(0), Some(1), Some(2), Some(3)] {
            for points in [points, &[]] {
                assert_eq!(
                    st.proxy_mixed_with(item, points),
                    walked(st, item, points),
                    "{what}: item {item:?}, {} picked points, cold cache {cold}",
                    points.len()
                );
            }
        }
    }
}

fn id_of(v: &serde_json::Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn rect(s: &mut Session, x: f64, fill: &str, stroke: &str) -> NodeId {
    let id = id_of(&s.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 20, "height": 20})).unwrap());
    s.execute("paint.setFill", &json!({"ids": [id.0], "color": fill})).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [id.0], "color": stroke})).unwrap();
    id
}

/// Every case of [`DocState::proxy_mixed`]'s rule, each answered as a walk answers it: one object,
/// different fills or strokes, a group with different contents, a group with one of its objects,
/// objects on two layers, a hidden object, an object with two fills under an Appearance item, a
/// gradient mesh with points picked beside another object, ids not in the document and an id that
/// appears twice (a damaged file) with different paints.
#[test]
fn the_question_marks_match_a_walk_of_the_document() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    let a = rect(&mut s, 10.0, "#ff0000", "#000000");
    let b = rect(&mut s, 40.0, "#0000ff", "#000000");
    let c = rect(&mut s, 70.0, "#ff0000", "#00ff00");
    // Two fills: [Fill, Stroke, Fill].
    s.execute("select.set", &json!({"ids": [c.0]})).unwrap();
    s.execute("appearance.addFill", &json!({})).unwrap();
    s.execute("appearance.setItem", &json!({"index": 2, "color": "#0000ff"})).unwrap();
    let e = rect(&mut s, 100.0, "#ff0000", "#000000");
    let f = rect(&mut s, 130.0, "#00ff00", "#000000");
    let g = id_of(&s.execute("object.group", &json!({"ids": [e.0, f.0]})).unwrap());
    let mesh = rect(&mut s, 160.0, "#ff0000", "#000000");
    s.execute("object.mesh.create", &json!({"ids": [mesh.0], "rows": 2, "cols": 2})).unwrap();
    let hidden = rect(&mut s, 190.0, "#0000ff", "#000000");
    s.execute("object.hide", &json!({"ids": [hidden.0]})).unwrap();
    s.execute("layer.new", &json!({})).unwrap();
    let h = rect(&mut s, 220.0, "#ff0000", "#000000");
    let missing = NodeId(999_999);
    let Some(NodeKind::Mesh(m)) = s.doc().unwrap().doc.node(mesh).map(|n| &n.kind) else { panic!("a gradient mesh") };
    let (p00, p01, p11) = (m.idx(0, 0), m.idx(0, 1), m.idx(1, 1));
    s.execute("object.mesh.setPointColor", &json!({"id": mesh.0, "index": p00, "color": "#00ff00"})).unwrap();
    let points = vec![(mesh, vec![p00, p01])];
    let points_alike = vec![(mesh, vec![p01, p11])];

    let selections = [
        vec![a],
        vec![a, b],
        vec![b, a],
        vec![a, h],
        vec![g],
        vec![g, e],
        vec![e, g],
        vec![f, e],
        vec![c],
        vec![c, a],
        vec![a, c],
        vec![mesh],
        vec![mesh, a],
        vec![a, mesh, h],
        vec![hidden],
        vec![hidden, b],
        vec![missing],
        vec![missing, a],
        vec![a, missing, b],
        vec![a, a],
    ];
    let check = |s: &mut Session, what: &str| {
        for sel in &selections {
            let st = s.active_mut().unwrap();
            st.selection.objects = sel.clone();
            let st = s.active().unwrap();
            agree(st, &points, &format!("{what}: {sel:?}"));
            agree(st, &points_alike, &format!("{what}: {sel:?}, alike points"));
        }
        s.execute("select.all", &json!({})).unwrap();
        agree(s.active().unwrap(), &points, &format!("{what}: Select All"));
    };
    check(&mut s, "unique ids");

    // A second node with a's id and another fill, at the end of the tree.
    let st = s.active_mut().unwrap();
    let mut copy = st.doc.node(a).unwrap().clone();
    copy.appearance.set_fill(Paint::solid(Color::rgb(0.0, 0.0, 1.0)));
    let doc = Arc::make_mut(&mut st.doc);
    let last = doc.layers.len() - 1;
    Arc::make_mut(&mut doc.layers[last]).children_mut().unwrap().push(Arc::new(copy));
    doc.drop_lookup_cache();
    check(&mut s, "a repeated id");
    let st = s.active_mut().unwrap();
    st.selection.objects = vec![a];
    assert_eq!(st.proxy_mixed_with(None, &[]), (true, false), "the two places of a's id differ in fill");
}

/// A document of `layers` layers, each with `groups` groups of `per_group` rectangles; every other
/// rectangle red, the rest blue. Returns it with the rectangles' ids, in tree order.
fn big(layers: usize, groups: usize, per_group: usize) -> (Document, Vec<NodeId>) {
    let mut d = Document::new(612.0, 792.0);
    d.layers.clear();
    let mut rects = vec![];
    for l in 0..layers {
        let mut layer = Node::layer(d.alloc_id(), &format!("L{l}"), LayerColor::Preset(0));
        for _ in 0..groups {
            let children = (0..per_group)
                .map(|_| {
                    let id = d.alloc_id();
                    let mut ap = Appearance::default_art();
                    let red = rects.len() % 2 == 0;
                    ap.set_fill(Paint::solid(if red { Color::rgb(1.0, 0.0, 0.0) } else { Color::rgb(0.0, 0.0, 1.0) }));
                    rects.push(id);
                    Arc::new(Node::path(id, shapes::rectangle(Rect::new(0.0, 0.0, 5.0, 5.0)), ap))
                })
                .collect();
            let group = Node::group(d.alloc_id(), children);
            layer.children_mut().unwrap().push(Arc::new(group));
        }
        d.layers.push(Arc::new(layer));
    }
    (d, rects)
}

fn state(doc: Document) -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 612, "height": 792})).unwrap();
    s.active_mut().unwrap().doc = Arc::new(doc);
    s
}

/// With one object selected, the "?" reads only that object, not every node of the document, and
/// does not walk the tree once the id cache is built; with every object selected it walks the tree
/// once.
#[test]
fn the_question_marks_read_only_the_selected_objects() {
    let (doc, rects) = big(2, 40, 25);
    let mut s = state(doc);
    let st = s.active_mut().unwrap();
    st.selection.objects = vec![rects[777]];
    assert_eq!(st.proxy_mixed(None), (false, false));
    let (read, walks) = (gathered(), vectorcraft_doc::tree_walks());
    assert_eq!(st.proxy_mixed(None), (false, false));
    assert_eq!(gathered() - read, 1, "only the selected object is read");
    assert_eq!(vectorcraft_doc::tree_walks() - walks, 0, "the cached path answers");

    st.selection.objects = vec![rects[3], rects[1500]];
    let (read, walks) = (gathered(), vectorcraft_doc::tree_walks());
    assert_eq!(st.proxy_mixed(None), (true, false));
    assert_eq!((gathered() - read, vectorcraft_doc::tree_walks() - walks), (2, 0));

    st.selection.objects = rects.clone();
    let (read, walks) = (gathered(), vectorcraft_doc::tree_walks());
    assert_eq!(st.proxy_mixed(None), (true, false));
    assert_eq!(gathered() - read, rects.len() as u64);
    assert!(vectorcraft_doc::tree_walks() - walks <= 1, "one walk for a large selection");
}

/// Random selections of a document with nested groups answer as a walk does.
#[test]
fn random_selections_match_a_walk_of_the_document() {
    let (doc, rects) = big(3, 12, 6);
    let mut s = state(doc);
    let groups: Vec<NodeId> = s.active().unwrap().doc.layers.iter().flat_map(|l| l.children().unwrap().iter().map(|g| g.id)).collect();
    let layers: Vec<NodeId> = s.active().unwrap().doc.layers.iter().map(|l| l.id).collect();
    let all: Vec<NodeId> = rects.iter().chain(&groups).chain(&layers).copied().chain([NodeId(999_999)]).collect();
    let mut r = 0x2545_f491_4f6c_dd1d_u64;
    let mut next = |n: usize| {
        r ^= r << 13;
        r ^= r >> 7;
        r ^= r << 17;
        (r % n as u64) as usize
    };
    for round in 0..200 {
        let count = 1 + next(if round % 10 == 0 { all.len() } else { 6 });
        let sel: Vec<NodeId> = (0..count).map(|_| all[next(all.len())]).collect();
        let st = s.active_mut().unwrap();
        st.selection.objects = sel.clone();
        agree(s.active().unwrap(), &[], &format!("round {round}: {sel:?}"));
    }
}

/// The CPU time this thread has used, from Linux's scheduler statistics; `None` elsewhere.
fn thread_cpu() -> Option<std::time::Duration> {
    let stat = std::fs::read_to_string("/proc/thread-self/schedstat").ok()?;
    stat.split_whitespace().next()?.parse().ok().map(std::time::Duration::from_nanos)
}

/// The time `f` takes: this thread's CPU time where Linux reports it, otherwise the wall clock.
fn time(f: impl FnOnce()) -> std::time::Duration {
    match thread_cpu() {
        Some(before) => {
            f();
            thread_cpu().map_or(std::time::Duration::ZERO, |after| after.saturating_sub(before))
        }
        None => {
            let start = std::time::Instant::now();
            f();
            start.elapsed()
        }
    }
}

/// The time of `proxy_mixed` against a walk of the document, on 50,000 rectangles in 500 groups on
/// 5 layers: `cargo test --release -p vectorcraft-engine --lib proxy_mixed_timing -- --ignored --nocapture`.
#[test]
#[ignore = "timing; run by hand"]
fn proxy_mixed_timing() {
    let (doc, rects) = big(5, 100, 100);
    let mut s = state(doc);
    let groups: Vec<NodeId> = s.active().unwrap().doc.layers.iter().flat_map(|l| l.children().unwrap().iter().map(|g| g.id)).collect();
    let cases: [(&str, Vec<NodeId>); 5] = [
        ("1 object", vec![rects[25_000]]),
        ("2 objects, different fills", vec![rects[10], rects[40_001]]),
        ("a group of 100", vec![groups[250]]),
        ("all 500 groups", groups.clone()),
        ("all 50,000 objects", rects.clone()),
    ];
    const CALLS: usize = 200;
    let median = |mut v: Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    };
    println!("proxy_mixed_timing: {}", if thread_cpu().is_some() { "thread CPU time" } else { "wall clock" });
    println!("| Selection | Id cache | Walk (µs/call) | Id cache (µs/call) | Runs (walk / id cache, µs) |");
    println!("|---|---|---|---|---|");
    for (name, sel) in &cases {
        for cold in [false, true] {
            let st = s.active_mut().unwrap();
            st.selection.objects = sel.clone();
            let st = s.active().unwrap();
            assert_eq!(st.proxy_mixed_with(None, &[]), walked(st, None, &[]), "{name}");
            let (mut old, mut new) = (vec![], vec![]);
            let calls = if cold { CALLS / 10 } else { CALLS };
            for _ in 0..7 {
                for (times, use_walk) in [(&mut old, true), (&mut new, false)] {
                    let d = time(|| {
                        for _ in 0..calls {
                            if cold {
                                st.doc.drop_lookup_cache();
                            }
                            std::hint::black_box(if use_walk { walked(st, None, &[]) } else { st.proxy_mixed_with(None, &[]) });
                        }
                    });
                    times.push(d.as_secs_f64() * 1e6 / calls as f64);
                }
            }
            let runs = |v: &[f64]| v.iter().map(|x| format!("{x:.1}")).collect::<Vec<_>>().join(" ");
            println!(
                "| {name} | {} | {:.1} | {:.1} | {} / {} |",
                if cold { "dropped before each call" } else { "built" },
                median(old.clone()),
                median(new.clone()),
                runs(&old),
                runs(&new)
            );
        }
    }
}
