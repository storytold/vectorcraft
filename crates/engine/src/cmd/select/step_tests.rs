//! Select > Next Object Above and Below through [`step_target`], which reads only the siblings of
//! the selected object, against a copy of the search that collected every object
//! [`walk_selectable`] visits on each step.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;
use vectorcraft_doc::{Appearance, Document, LayerColor, Node, NodeId, NodeKind};
use vectorcraft_geom::{Rect, shapes};

use super::{selectable_ids, step_target, walk_visits};
use crate::Session;

/// The search as it was before it read only the siblings: the nearest sibling in that direction
/// that `walk_selectable` visits, from the set of every object it visits.
fn step_base(d: &Document, iso: Option<NodeId>, cur: NodeId, dir: i64) -> Option<NodeId> {
    let (par, idx, _) = d.position(cur)?;
    let siblings = d.children(par)?;
    let can = selectable_ids(d, iso);
    let next = if dir > 0 {
        siblings.iter().skip(idx + 1).find(|n| can.contains(&n.id))
    } else {
        siblings.iter().take(idx).rev().find(|n| can.contains(&n.id))
    };
    next.map(|n| n.id)
}

/// A small xorshift generator, so that each seed gives the same document every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    /// True `pct` times in 100.
    fn chance(&mut self, pct: u64) -> bool {
        self.below(100) < pct
    }
}

fn rect_node(d: &mut Document, x: f64, y: f64) -> Node {
    Node::path(d.alloc_id(), shapes::rectangle(Rect::new(x, y, x + 40.0, y + 40.0)), Appearance::default())
}

fn layer_node(d: &mut Document, children: Vec<Node>) -> Node {
    let mut l = Node::layer(d.alloc_id(), "Layer", LayerColor::Preset(0));
    if let Some(ch) = l.children_mut() {
        ch.extend(children.into_iter().map(Arc::new));
    }
    l
}

fn ruler_guide(d: &mut Document, pos: f64) -> Node {
    Node::new(d.alloc_id(), NodeKind::RulerGuide { vertical: pos > 300.0, pos, artboard: None })
}

/// Up to six objects: rectangles on and off the artboard, guides made from paths, ruler guides,
/// groups, and sublayers, some hidden, locked or template layers. Groups and sublayers hold more
/// below `depth`. Sublayers and ruler guides also go inside groups, which the app does not make,
/// so that every shape of tree is compared.
fn objects(r: &mut Rng, d: &mut Document, depth: u32) -> Vec<Node> {
    (0..r.below(7))
        .map(|_| {
            let (x, y) = (r.below(1400) as f64 - 300.0, r.below(1000) as f64 - 200.0);
            let mut n = match r.below(if depth == 0 { 6 } else { 10 }) {
                0..=3 => rect_node(d, x, y),
                4 => {
                    let mut n = rect_node(d, x, y);
                    if let NodeKind::Path { guide, .. } = &mut n.kind {
                        *guide = true;
                    }
                    n
                }
                5 => ruler_guide(d, x),
                6 | 7 => {
                    let kids = objects(r, d, depth - 1);
                    Node::group(d.alloc_id(), kids.into_iter().map(Arc::new).collect())
                }
                _ => {
                    let kids = objects(r, d, depth - 1);
                    layer_node(d, kids)
                }
            };
            n.visible = !r.chance(10);
            n.locked = r.chance(10);
            if let NodeKind::Layer { template, .. } = &mut n.kind {
                *template = r.chance(15);
            }
            n
        })
        .collect()
}

/// A document of one to three layers, some hidden, locked or template layers, holding
/// [`objects`], with its artboard locked in some seeds.
fn random_document(seed: u64) -> Document {
    let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut d = Document::new(800.0, 600.0);
    let layers = (0..1 + r.below(3))
        .map(|_| {
            let kids = objects(&mut r, &mut d, 3);
            let mut l = layer_node(&mut d, kids);
            l.visible = !r.chance(10);
            l.locked = r.chance(10);
            if let NodeKind::Layer { template, .. } = &mut l.kind {
                *template = r.chance(10);
            }
            Arc::new(l)
        })
        .collect();
    d.layers = layers;
    if let Some(a) = d.artboards.first_mut() {
        a.locked = r.chance(40);
    }
    d
}

/// Every node of `d`, layers included, in tree order.
fn all_ids(d: &Document) -> Vec<NodeId> {
    let mut ids = vec![];
    d.walk(|n| ids.push(n.id));
    ids
}

/// The objects that hold `id`, outermost first, and the node itself.
fn holders(d: &Document, id: NodeId) -> (Vec<&Node>, &Node) {
    let a = d.ancestry(id).unwrap();
    let (last, up) = a.split_last().unwrap();
    (up.iter().map(|i| d.node(*i).unwrap()).collect(), d.node(*last).unwrap())
}

/// For every node of random documents, outside isolation mode and with each of several nodes
/// isolated, `walk_visits` says what the set of the objects `walk_selectable` visits says, and
/// Next Object Above and Below from it give what the search over that set gave.
#[test]
fn next_object_gives_what_the_search_over_every_selectable_object_gave() {
    let (mut steps, mut moved, mut isolated) = (0, 0, 0);
    for seed in 0..300 {
        let d = random_document(seed);
        let ids = all_ids(&d);
        let mut r = Rng(seed + 7);
        let mut isos = vec![None];
        isos.extend((0..10).map(|_| Some(ids[r.below(ids.len() as u64) as usize])));
        let locks = d.locked_artboards();
        for iso in isos {
            let can = selectable_ids(&d, iso);
            let iso_node = iso.and_then(|i| d.node(i));
            for &id in &ids {
                let (up, n) = holders(&d, id);
                assert_eq!(walk_visits(&up, n, iso_node, &locks), can.contains(&id), "seed {seed}, isolating {iso:?}, node {id:?}");
            }
            for &cur in &ids {
                for dir in [1, -1] {
                    let want = step_base(&d, iso, cur, dir);
                    assert_eq!(step_target(&d, iso, cur, dir), want, "seed {seed}, isolating {iso:?}, from {cur:?}, direction {dir}");
                    steps += 1;
                    moved += usize::from(want.is_some());
                    isolated += usize::from(want.is_some() && iso.is_some());
                }
            }
        }
    }
    // The documents give many steps that select something, inside isolation mode too.
    eprintln!("{moved} of {steps} steps select an object, {isolated} of them in isolation mode");
    assert!(moved > 5_000, "{moved} of {steps} steps select an object");
    assert!(isolated > 500, "{isolated} steps select an object in isolation mode");
}

/// A session holding `d`, with `iso` isolated and `cur` selected.
fn session(d: Document, iso: Option<NodeId>, cur: NodeId) -> Session {
    let mut s = Session::new();
    s.add_document(d, None);
    let st = s.doc_mut().unwrap();
    st.isolation = iso;
    st.selection.set([cur]);
    s
}

fn selected(s: &Session) -> Vec<NodeId> {
    s.doc().unwrap().selection.objects.to_vec()
}

/// In an isolated layer, Next Object Above and Below pass over a sublayer beside the selected object,
/// as the walk looks through it.
#[test]
fn next_object_passes_over_a_sublayer_in_an_isolated_layer() {
    let mut d = Document::new(800.0, 600.0);
    let a = rect_node(&mut d, 0.0, 0.0);
    let inner = rect_node(&mut d, 100.0, 0.0);
    let b = rect_node(&mut d, 200.0, 0.0);
    let sub = layer_node(&mut d, vec![inner]);
    let (a_id, b_id) = (a.id, b.id);
    let l = layer_node(&mut d, vec![a, sub, b]);
    let layer = l.id;
    d.layers = vec![Arc::new(l)];
    assert_eq!(step_base(&d, Some(layer), a_id, 1), Some(b_id));
    let mut s = session(d, Some(layer), a_id);
    s.execute("select.nextAbove", &json!({})).unwrap();
    assert_eq!(selected(&s), vec![b_id]);
    s.execute("select.nextBelow", &json!({})).unwrap();
    assert_eq!(selected(&s), vec![a_id]);
}

/// Next Object Above passes over a ruler guide of the layer, which the walk leaves out, unless the
/// guide is the object isolated.
#[test]
fn next_object_passes_over_a_ruler_guide_of_the_layer() {
    let mut d = Document::new(800.0, 600.0);
    let a = rect_node(&mut d, 0.0, 0.0);
    let b = rect_node(&mut d, 200.0, 0.0);
    let guide = ruler_guide(&mut d, 100.0);
    let (a_id, b_id, guide_id) = (a.id, b.id, guide.id);
    let l = layer_node(&mut d, vec![a, b, guide]);
    d.layers = vec![Arc::new(l)];
    // Nothing above the rectangle but the guide: the selection stays.
    assert_eq!(step_base(&d, None, b_id, 1), None);
    let mut s = session(d.clone(), None, b_id);
    s.execute("select.nextAbove", &json!({})).unwrap();
    assert_eq!(selected(&s), vec![b_id]);
    s.execute("select.nextBelow", &json!({})).unwrap();
    assert_eq!(selected(&s), vec![a_id]);
    // The guide isolated on its own is what there is to select.
    assert_eq!(step_base(&d, Some(guide_id), b_id, 1), Some(guide_id));
    assert_eq!(step_target(&d, Some(guide_id), b_id, 1), Some(guide_id));
}

/// The first selected object anchors the search even when it can no longer be selected itself.
#[test]
fn next_object_steps_from_a_hidden_first_selected_object() {
    let mut d = Document::new(800.0, 600.0);
    let mut a = rect_node(&mut d, 0.0, 0.0);
    a.visible = false;
    let b = rect_node(&mut d, 200.0, 0.0);
    let (a_id, b_id) = (a.id, b.id);
    let l = layer_node(&mut d, vec![a, b]);
    d.layers = vec![Arc::new(l)];
    assert_eq!(step_base(&d, None, a_id, 1), Some(b_id));
    let mut s = session(d, None, a_id);
    s.execute("select.nextAbove", &json!({})).unwrap();
    assert_eq!(selected(&s), vec![b_id]);
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

/// A document of `groups` groups of `per` rectangles on one layer, or of `per` rectangles when
/// `groups` is 0 → the document and the object in the middle of the layer.
fn wide(groups: usize, per: usize) -> (Document, NodeId) {
    let mut d = Document::new(800.0, 600.0);
    let mut top = vec![];
    if groups == 0 {
        top.extend((0..per).map(|i| rect_node(&mut d, (i % 200) as f64 * 4.0, (i / 200 % 150) as f64 * 4.0)));
    } else {
        for g in 0..groups {
            let kids: Vec<Arc<Node>> = (0..per).map(|i| Arc::new(rect_node(&mut d, (i % 20) as f64 * 4.0, (g % 150) as f64 * 4.0))).collect();
            top.push(Node::group(d.alloc_id(), kids));
        }
    }
    let mid = top[top.len() / 2].id;
    let l = layer_node(&mut d, top);
    d.layers = vec![Arc::new(l)];
    (d, mid)
}

/// A search for the object Next Object Above or Below selects: [`step_base`] or [`step_target`].
type Step = fn(&Document, Option<NodeId>, NodeId, i64) -> Option<NodeId>;

/// Timings of Next Object Above and Below, stepping up and down from the object in the middle of
/// the layer, by the search over every selectable object (200 steps a run) and by the siblings
/// alone (200,000 steps a run, as one step takes too little time to measure), on 100,000
/// rectangles and on 1,000 groups of 100, run in turn on the same document, 7 runs each. Ignored by
/// default; it prints the median, fastest and slowest time per step and sets no bound on them:
/// `cargo test --release -p vectorcraft-engine --lib next_object_perf -- --ignored --nocapture`.
#[test]
#[ignore = "a benchmark: run with --ignored --nocapture"]
fn next_object_perf() {
    const RUNS: usize = 7;
    const OLD_STEPS: usize = 200;
    const NEW_STEPS: usize = 200_000;
    let per_step = |d: Duration, steps: usize| d.as_secs_f64() * 1e6 / steps as f64;
    eprintln!("next_object_perf: {}", if thread_cpu().is_some() { "thread CPU time" } else { "wall clock" });
    eprintln!("next_object_perf: {:<26} {:>32} {:>32}", "document", "every object µs/step (min-max)", "siblings µs/step (min-max)");
    for (name, groups, per) in [("100,000 rectangles", 0, 100_000), ("1,000 groups of 100", 1_000, 100)] {
        let (d, mid) = wide(groups, per);
        let walk = |f: Step, steps: usize| {
            let mut cur = mid;
            let mut seen = Vec::with_capacity(steps);
            for i in 0..steps {
                cur = f(&d, None, cur, if i % 2 == 0 { 1 } else { -1 }).unwrap_or(cur);
                seen.push(cur);
            }
            std::hint::black_box(seen)
        };
        let (mut old, mut new) = (Vec::new(), Vec::new());
        let (mut a, mut b) = (vec![], vec![]);
        for run in 0..RUNS {
            // The two take turns going first.
            if run % 2 == 0 {
                old.push(time(|| a = walk(step_base, OLD_STEPS)));
                new.push(time(|| b = walk(step_target, NEW_STEPS)));
            } else {
                new.push(time(|| b = walk(step_target, NEW_STEPS)));
                old.push(time(|| a = walk(step_base, OLD_STEPS)));
            }
            assert_eq!(a[..], b[..OLD_STEPS], "{name}");
            assert_ne!(a[0], mid, "{name}: the first step selects another object");
        }
        old.sort();
        new.sort();
        eprintln!(
            "next_object_perf: {name:<26} {:>12.3} ({:>8.3}-{:>9.3}) {:>12.3} ({:>8.3}-{:>9.3})",
            per_step(old[RUNS / 2], OLD_STEPS),
            per_step(old[0], OLD_STEPS),
            per_step(old[RUNS - 1], OLD_STEPS),
            per_step(new[RUNS / 2], NEW_STEPS),
            per_step(new[0], NEW_STEPS),
            per_step(new[RUNS - 1], NEW_STEPS),
        );
    }
}
