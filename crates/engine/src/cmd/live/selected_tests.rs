//! The selected blend, envelope and mesh ([`selected_of`] and the checks built on it) are the ones
//! the lookup of every ancestor by id found: the same ids in the same order, the same availability
//! and errors, on selections with keys, nested blends, stale and repeated ids and a damaged tree.

use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;
use vectorcraft_doc::{Node, NodeId, NodeKind, SymbolPart};

use super::Session;
use super::blend::any_without_spine;
use super::common::{any_selected_of, has_blend, has_envelope, has_mesh, is_blend, is_envelope, is_mesh, selected_of};

/// [`selected_of`] as each ancestor was looked up by id: `ancestry`, then `node` for each id from
/// the object up, repeats dropped by a scan of the list.
fn selected_of_by_ids(s: &Session, pred: fn(&Node) -> bool) -> Vec<NodeId> {
    let Some(st) = s.active().filter(|st| st.selection.symbol_part.is_none()) else { return vec![] };
    let mut out = vec![];
    for id in &st.selection.objects {
        for a in st.doc.ancestry(*id).unwrap_or_default().into_iter().rev() {
            if st.doc.node(a).is_some_and(pred) {
                if !out.contains(&a) {
                    out.push(a);
                }
                break;
            }
        }
    }
    out
}

/// The availability [`has_blend`] and the others gave, from [`selected_of_by_ids`].
fn has_by_ids(s: &Session, pred: fn(&Node) -> bool, msg: &str) -> std::result::Result<(), String> {
    super::has_selection(s)?;
    crate::cmd::edit::no_symbol_part(s)?;
    if selected_of_by_ids(s, pred).is_empty() { Err(msg.into()) } else { Ok(()) }
}

/// The no-spine check [`any_without_spine`] made, from [`selected_of_by_ids`].
fn without_spine_by_ids(s: &Session) -> bool {
    let Ok(st) = s.doc() else { return false };
    selected_of_by_ids(s, is_blend).iter().any(|b| matches!(st.doc.node(*b).map(|n| &n.kind), Some(NodeKind::Blend { spec, .. }) if spec.no_spine))
}

/// Every check on the current selection gives what the lookups by id gave.
fn agree(s: &Session, what: &str) {
    type Has = fn(&Session) -> std::result::Result<(), String>;
    for (name, pred, has, msg) in [
        ("blend", is_blend as fn(&Node) -> bool, has_blend as Has, "select a blend"),
        ("envelope", is_envelope, has_envelope, "select an envelope"),
        ("mesh", is_mesh, has_mesh, "select a gradient mesh"),
    ] {
        let want = selected_of_by_ids(s, pred);
        assert_eq!(selected_of(s, pred), want, "{what}: selected {name}s");
        assert_eq!(any_selected_of(s, pred, |_| true), !want.is_empty(), "{what}: any {name}");
        assert_eq!(has(s), has_by_ids(s, pred, msg), "{what}: has {name}");
    }
    assert_eq!(any_without_spine(s), without_spine_by_ids(s), "{what}: a blend without a spine");
}

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn id_of(v: &serde_json::Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn rect(s: &mut Session, x: f64) -> NodeId {
    id_of(&s.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 20, "height": 20})).unwrap())
}

fn select(s: &mut Session, ids: &[NodeId]) {
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
}

fn run(s: &mut Session, cmd: &str, ids: &[NodeId], p: serde_json::Value) -> NodeId {
    select(s, ids);
    id_of(&s.execute(cmd, &p).unwrap())
}

/// Every node id under the layers, parents before children (as Direct Selection can pick them).
fn all_ids(s: &Session) -> Vec<NodeId> {
    fn walk(nodes: &[std::sync::Arc<Node>], out: &mut Vec<NodeId>) {
        for n in nodes {
            out.push(n.id);
            if let Some(ch) = n.children() {
                walk(ch, out);
            }
        }
    }
    let mut out = vec![];
    for l in &s.doc().unwrap().doc.layers {
        walk(l.children().map_or(&[][..], |c| c.as_slice()), &mut out);
    }
    out
}

/// The active document, to change in place.
fn doc_mut(s: &mut Session) -> &mut vectorcraft_doc::Document {
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc)
}

/// Set the selection's objects as given, stale and repeated ids included.
fn set_objects(s: &mut Session, ids: Vec<NodeId>) {
    s.doc_mut().unwrap().selection.objects = ids;
}

/// Two blends, a blend in a key group of a third, an envelope and a mesh; one blend has no spine.
/// Returns the blends `a`, `b` and `c`, the group `g` of `c` that holds `a`, the envelope, and a key
/// of `a`.
fn document() -> (Session, [NodeId; 6]) {
    let mut s = session();
    let r: Vec<NodeId> = (0..10).map(|i| rect(&mut s, 40.0 * i as f64)).collect();
    let a = run(&mut s, "object.blend.make", &[r[0], r[1]], json!({}));
    let b = run(&mut s, "object.blend.make", &[r[2], r[3]], json!({}));
    let g = run(&mut s, "object.group", &[a, r[4]], json!({}));
    let c = run(&mut s, "object.blend.make", &[g, r[5]], json!({}));
    let e = run(&mut s, "object.envelope.makeWithWarp", &[r[6]], json!({"style": "arch", "bend": 50}));
    select(&mut s, &[r[7]]);
    s.execute("object.mesh.create", &json!({"ids": [r[7].0], "rows": 1, "cols": 1})).unwrap();
    if let Some(NodeKind::Blend { spec, .. }) = doc_mut(&mut s).node_mut(b).map(|n| &mut n.kind) {
        spec.no_spine = true;
    }
    (s, [a, b, c, g, e, r[0]])
}

#[test]
fn selected_blends_envelopes_and_meshes_match_the_lookups_by_id() {
    let (mut s, [a, b, c, g, e, inner_key]) = document();
    s.execute("select.all", &json!({})).unwrap();
    agree(&s, "select all");
    let all = all_ids(&s);
    let keys: Vec<NodeId> = all.iter().copied().filter(|id| ![a, b, c, g, e].contains(id)).collect();
    set_objects(&mut s, keys.clone());
    agree(&s, "keys, as Direct Selection picks them");
    set_objects(&mut s, all.iter().rev().copied().collect());
    agree(&s, "every node, children first");
    set_objects(&mut s, vec![a, keys[0], b, keys[1], keys[0], NodeId(999_999), a]);
    agree(&s, "repeated and stale ids, in first-seen order");
    assert_eq!(selected_of(&s, is_blend), vec![a, b], "the blend with no spine is listed once, in order");
    set_objects(&mut s, vec![NodeId(999_999)]);
    agree(&s, "only a stale id");
    set_objects(&mut s, vec![]);
    agree(&s, "nothing");
    // A blend in another blend's key group: the inner one.
    set_objects(&mut s, vec![inner_key]);
    agree(&s, "a key of a blend nested in another's key group");
    assert_eq!(selected_of(&s, is_blend), vec![a]);
    // An object of a dynamic symbol instance's art: nothing.
    set_objects(&mut s, all.clone());
    s.doc_mut().unwrap().selection.symbol_part = Some(SymbolPart { instance: a, path: vec![0] });
    agree(&s, "a symbol part");
    assert!(selected_of(&s, is_blend).is_empty());
}

/// A damaged tree: an earlier node of another kind with the id of a blend on the selected key's
/// path. `node` gives the earlier node, so that blend does not count, before and after the cache is
/// built again.
#[test]
fn a_repeated_id_counts_as_its_first_node() {
    let (mut s, [a, _, c, _, _, inner_key]) = document();
    let all = all_ids(&s);
    s.doc().unwrap().doc.node(a);
    let doc = doc_mut(&mut s);
    if let Some(ch) = std::sync::Arc::make_mut(&mut doc.layers[0]).children_mut() {
        ch.insert(0, std::sync::Arc::new(Node::group(a, vec![])));
    }
    set_objects(&mut s, all.clone());
    agree(&s, "a repeated id, the cache older than the tree");
    s.doc().unwrap().doc.drop_lookup_cache();
    agree(&s, "a repeated id, no cache");
    s.doc().unwrap().doc.node(a);
    agree(&s, "a repeated id, cached afresh");
    set_objects(&mut s, vec![inner_key]);
    assert_eq!(selected_of(&s, is_blend), vec![c], "the nested blend's id is the group's, so the outer blend counts");
}

/// Large selections: with the lookup cache built, finding the selected blends looks no id up, and
/// the checks stop at the first selected object that has one.
#[test]
fn selected_blends_take_no_lookups_by_id() {
    let mut s = session();
    let mut keys = vec![];
    for i in 0..300 {
        let (x, y) = (rect(&mut s, i as f64), rect(&mut s, i as f64 + 30.0));
        run(&mut s, "object.blend.make", &[x, y], json!({}));
        keys.extend([x, y]);
    }
    for i in 0..1_000 {
        rect(&mut s, i as f64);
    }
    s.execute("select.all", &json!({})).unwrap();
    s.doc().unwrap().doc.drop_lookup_cache();
    s.doc().unwrap().doc.node(keys[0]);
    let before = vectorcraft_doc::path_lookups();
    assert_eq!(selected_of(&s, is_blend).len(), 300);
    assert!(!any_selected_of(&s, is_mesh, |_| true));
    set_objects(&mut s, keys.clone());
    assert_eq!(selected_of(&s, is_blend).len(), 300, "each blend once from its two keys");
    assert_eq!(vectorcraft_doc::path_lookups() - before, 0, "lookups by id");
    // The first key is in a blend: the check tests that key and its blend only.
    static TESTED: AtomicUsize = AtomicUsize::new(0);
    fn counted(n: &Node) -> bool {
        TESTED.fetch_add(1, Ordering::Relaxed);
        is_blend(n)
    }
    assert!(any_selected_of(&s, counted, |_| true));
    assert_eq!(TESTED.load(Ordering::Relaxed), 2, "nodes tested");
}

/// `n` and everything under it with new ids.
fn renumber(d: &mut vectorcraft_doc::Document, n: &Node) -> Node {
    let mut n = n.clone();
    n.id = d.alloc_id();
    if let Some(ch) = n.children_mut() {
        for c in ch.iter_mut() {
            *c = std::sync::Arc::new(renumber(d, c));
        }
    }
    n
}

/// This thread's CPU time in nanoseconds, where the system reports it (Linux).
fn cpu_ns() -> Option<u64> {
    std::fs::read_to_string("/proc/thread-self/schedstat").ok()?.split_whitespace().next()?.parse().ok()
}

/// Milliseconds one call of `f` takes over `calls` calls, in CPU time where the system reports it,
/// otherwise in wall-clock time.
fn ms_per_call(calls: u32, f: &dyn Fn()) -> f64 {
    let (cpu, wall) = (cpu_ns(), std::time::Instant::now());
    for _ in 0..calls {
        f();
    }
    let ms = match (cpu, cpu_ns()) {
        (Some(a), Some(b)) => b.saturating_sub(a) as f64 / 1e6,
        _ => wall.elapsed().as_secs_f64() * 1e3,
    };
    ms / f64::from(calls)
}

/// The medians of 7 runs of `a` and of `b`, the runs taking turns.
fn medians(calls: u32, a: &dyn Fn(), b: &dyn Fn()) -> (f64, f64) {
    let (mut ta, mut tb) = (vec![], vec![]);
    for _ in 0..7 {
        ta.push(ms_per_call(calls, a));
        tb.push(ms_per_call(calls, b));
    }
    ta.sort_by(f64::total_cmp);
    tb.sort_by(f64::total_cmp);
    (ta[3], tb[3])
}

/// Timings of the lookups by id and of the one-path lookup on 100,000 rectangles in 10 layers,
/// 2,000 two-key blends (every fourth one in a group) and 200 envelopes, all selected. Run with
/// `cargo test --release -p vectorcraft-engine --lib selected_tests::timing -- --ignored --nocapture`.
#[test]
#[ignore = "timing; run by hand"]
fn timing_of_the_selected_blend_checks() {
    let mut s = session();
    let (a, b, c) = (rect(&mut s, 0.0), rect(&mut s, 40.0), rect(&mut s, 80.0));
    let square = rect(&mut s, 120.0);
    let blend = run(&mut s, "object.blend.make", &[a, b], json!({}));
    let envelope = run(&mut s, "object.envelope.makeWithWarp", &[c], json!({"style": "arch", "bend": 50}));
    let doc = doc_mut(&mut s);
    let templates: Vec<Node> = [square, blend, envelope].iter().map(|id| doc.node(*id).cloned().unwrap()).collect();
    let mut layers = vec![];
    for l in 0..10 {
        let mut layer = Node::layer(doc.alloc_id(), &format!("Layer {l}"), vectorcraft_doc::LayerColor::Preset(0));
        let mut kids = vec![];
        for _ in 0..10_000 {
            kids.push(std::sync::Arc::new(renumber(doc, &templates[0])));
        }
        for i in 0..200 {
            let b = renumber(doc, &templates[1]);
            let b = if i % 4 == 0 { Node::group(doc.alloc_id(), vec![std::sync::Arc::new(b)]) } else { b };
            kids.push(std::sync::Arc::new(b));
        }
        for _ in 0..20 {
            kids.push(std::sync::Arc::new(renumber(doc, &templates[2])));
        }
        if let Some(ch) = layer.children_mut() {
            *ch = kids;
        }
        layers.push(std::sync::Arc::new(layer));
    }
    doc.layers = layers;
    s.execute("select.all", &json!({})).unwrap();
    let first = s.doc().unwrap().selection.objects[0];
    s.doc().unwrap().doc.drop_lookup_cache();
    s.doc().unwrap().doc.node(first);
    println!("{} objects selected", s.doc().unwrap().selection.objects.len());
    type Check = fn(&Session) -> bool;
    let checks: [(&str, Check, Check); 5] = [
        ("has_blend", |s| has_by_ids(s, is_blend, "").is_ok(), |s| has_blend(s).is_ok()),
        ("has_envelope", |s| has_by_ids(s, is_envelope, "").is_ok(), |s| has_envelope(s).is_ok()),
        ("has_mesh", |s| has_by_ids(s, is_mesh, "").is_ok(), |s| has_mesh(s).is_ok()),
        (
            "selected blends (object.blend.info and the Blend commands)",
            |s| !selected_of_by_ids(s, is_blend).is_empty(),
            |s| !selected_of(s, is_blend).is_empty(),
        ),
        ("a blend without a spine (Reverse Spine, the Blend panel)", without_spine_by_ids, any_without_spine),
    ];
    assert_eq!(selected_of(&s, is_blend), selected_of_by_ids(&s, is_blend));
    println!("| Check | Lookups by id (ms) | One path (ms) |");
    println!("|---|---:|---:|");
    for (name, by_ids, one_path) in checks {
        assert_eq!(by_ids(&s), one_path(&s), "{name}");
        let (old, new) = medians(
            15,
            &|| {
                std::hint::black_box(by_ids(&s));
            },
            &|| {
                std::hint::black_box(one_path(&s));
            },
        );
        println!("| {name} | {old:.3} | {new:.3} |");
    }
}
