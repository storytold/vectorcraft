//! `cargo run --release -p vectorcraft-engine --example link_colors_bench` prints the time
//! `Document::map_solid_paints_in` takes to link the paints of selected objects to swatches, as Add Used
//! Colors and New Color Group do for the selection with global swatches, on synthetic documents: 2,000 and
//! 20,000 rectangles in one layer, and 5,000 rectangles in 1,000 groups of groups, selected with and
//! without the groups that hold them. The fills and strokes use 50 colors, each with a swatch to link to.
//! For each it times two ways of finding the selected objects that no other selected object holds: a
//! search of the id list for each ancestor of each id, and a lookup of each ancestor in a set of the ids
//! (as `map_solid_paints_in` does). Both then link the same paints in the same loop. It checks that both
//! give the same count, visit the same paints in the same order and leave the same document as
//! `map_solid_paints_in` before it times them.
//! `ITERS` sets the runs per row (default 7); the two ways run in alternating order, each run links 10
//! fresh copies of the document, and each row is the median time of one call. On Linux the times are the CPU time of the main thread, which runs
//! everything, in ms; elsewhere they are wall times.
// Example (dev tool): unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{HashMap, HashSet};
use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::swatches::{LinkFn, map_node_solid_paints};
use vectorcraft_doc::{Appearance, Document, Node, NodeId};
use vectorcraft_geom::{Rect, shapes};

/// The colors the rectangles use, each with a swatch to link to.
const COLORS: usize = 50;

/// The calls in each timed run; a row gives the time of one call.
const REPS: usize = 10;

/// A way of finding the ids that no other listed id holds.
type Roots = fn(&Document, &[NodeId]) -> Vec<NodeId>;

/// A way of applying a link function to the paints of the listed ids, returning the count changed.
type Map<'a> = dyn FnMut(&mut Document, &[NodeId], &mut LinkFn) -> usize + 'a;

fn iters() -> usize {
    std::env::var("ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(7).max(1)
}

fn median(mut ms: Vec<f64>) -> f64 {
    ms.sort_by(f64::total_cmp);
    ms[ms.len() / 2]
}

/// The CPU time of the calling thread in ns, from `/proc/thread-self/schedstat` (Linux only).
fn thread_cpu_ns() -> Option<u64> {
    std::fs::read_to_string("/proc/thread-self/schedstat").ok()?.split_whitespace().next()?.parse().ok()
}

/// The time `f` takes, in ms: CPU time where the system reports it, wall time otherwise.
fn measure(f: impl FnOnce()) -> f64 {
    match thread_cpu_ns() {
        Some(start) => {
            f();
            thread_cpu_ns().map_or(f64::NAN, |end| end.saturating_sub(start) as f64 / 1e6)
        }
        None => {
            let t = Instant::now();
            f();
            t.elapsed().as_secs_f64() * 1000.0
        }
    }
}

/// Color `i` of [`COLORS`]: each has its own red value.
fn color(i: usize) -> Color {
    Color::rgb(i as f32 / COLORS as f32, 0.5, 0.25)
}

/// The bits of a color's red value, which tell the colors of [`color`] apart; `u32::MAX` for other color models.
fn red_bits(c: &Color) -> u32 {
    match c {
        Color::Rgb { r, .. } => r.to_bits(),
        _ => u32::MAX,
    }
}

/// The ids in the order given, less those with an ancestor in the list: a search of the list for each ancestor.
fn roots_by_search(d: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    ids.iter().copied().filter(|id| !d.ancestry(*id).is_some_and(|a| a[..a.len() - 1].iter().any(|p| ids.contains(p)))).collect()
}

/// [`roots_by_search`] with each ancestor looked up in a set of the ids.
fn roots_by_set(d: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    let listed: HashSet<NodeId> = ids.iter().copied().collect();
    ids.iter().copied().filter(|id| !d.ancestry(*id).is_some_and(|a| a[..a.len() - 1].iter().any(|p| listed.contains(p)))).collect()
}

/// `f` applied to the paints of the subtrees of the ids `roots` finds, each node holding a changed
/// paint put in place: the work of `map_solid_paints_in`.
fn map_in(d: &mut Document, ids: &[NodeId], roots: Roots, f: &mut LinkFn) -> usize {
    let mut n = 0;
    for id in roots(d, ids) {
        let Some(mut node) = d.node(id).cloned() else { continue };
        let count = map_node_solid_paints(&mut node, f);
        if count > 0
            && let Some(slot) = d.node_mut(id)
        {
            *slot = node;
            n += count;
        }
    }
    n
}

/// Link every paint whose color has a swatch to that swatch, as Add Used Colors does for global
/// swatches, recording each paint visited. Returns the count and the visits.
fn link(d: &mut Document, ids: &[NodeId], map: &mut Map) -> (usize, Vec<(Color, Option<String>)>) {
    let names: HashMap<u32, String> = (0..COLORS).map(|i| (red_bits(&color(i)), format!("Swatch {i}"))).collect();
    let mut visits = vec![];
    let n = map(d, ids, &mut |c, s, _| {
        visits.push((*c, s.clone()));
        match names.get(&red_bits(c)) {
            Some(name) if s.is_none() => {
                *s = Some(name.clone());
                true
            }
            _ => false,
        }
    });
    (n, visits)
}

fn rect(d: &mut Document, i: usize) -> Arc<Node> {
    let id = d.alloc_id();
    let (x, y) = ((i % 100) as f64 * 12.0, (i / 100) as f64 * 12.0);
    let r = shapes::rectangle(Rect::new(x, y, x + 10.0, y + 10.0));
    let a = Appearance::basic(Paint::solid(color(i % COLORS)), Paint::solid(color((i + 7) % COLORS)), 1.0);
    Arc::new(Node::path(id, r, a))
}

/// `n` rectangles in the first layer, all selected.
fn flat(n: usize) -> (Document, Vec<NodeId>) {
    let mut d = Document::new(1600.0, 1200.0);
    let l = d.layers[0].id;
    let mut ids = vec![];
    for i in 0..n {
        let node = rect(&mut d, i);
        ids.push(node.id);
        d.insert(Some(l), usize::MAX, Arc::unwrap_or_clone(node)).unwrap();
    }
    (d, ids)
}

/// 1,000 groups in the first layer, each holding a group of 5 rectangles: the rectangles, and with
/// `groups` the two groups above each of them too, listed first.
fn nested(groups: bool) -> (Document, Vec<NodeId>) {
    let mut d = Document::new(1600.0, 1200.0);
    let l = d.layers[0].id;
    let (mut outer_ids, mut leaves) = (vec![], vec![]);
    for g in 0..1_000 {
        let rects: Vec<Arc<Node>> = (0..5).map(|i| rect(&mut d, g * 5 + i)).collect();
        leaves.extend(rects.iter().map(|r| r.id));
        let inner = Node::group(d.alloc_id(), rects);
        let outer = Node::group(d.alloc_id(), vec![Arc::new(inner.clone())]);
        if groups {
            outer_ids.extend([outer.id, inner.id]);
        }
        d.insert(Some(l), usize::MAX, outer).unwrap();
    }
    outer_ids.extend(leaves);
    (d, outer_ids)
}

fn bench(name: &str, (doc, ids): (Document, Vec<NodeId>)) {
    let mut shipped = doc.clone();
    let want = link(&mut shipped, &ids, &mut |d, ids, f| d.map_solid_paints_in(ids, f));
    for roots in [roots_by_search as Roots, roots_by_set] {
        let mut d = doc.clone();
        let got = link(&mut d, &ids, &mut |d, ids, f| map_in(d, ids, roots, f));
        assert_eq!(got, want, "{name}: the same count and visits");
        assert_eq!(d.layers, shipped.layers, "{name}: the same document");
    }
    let (mut old, mut new, mut real) = (vec![], vec![], vec![]);
    let timed = |map: &mut Map| {
        let mut docs = vec![doc.clone(); REPS];
        measure(|| {
            for d in &mut docs {
                black_box(link(d, black_box(&ids), &mut *map));
            }
        }) / REPS as f64
    };
    for i in 0..iters() {
        let run = |roots: Roots, out: &mut Vec<f64>| out.push(timed(&mut |d, ids, f| map_in(d, ids, roots, f)));
        if i % 2 == 0 {
            run(roots_by_search, &mut old);
            run(roots_by_set, &mut new);
        } else {
            run(roots_by_set, &mut new);
            run(roots_by_search, &mut old);
        }
        real.push(timed(&mut |d, ids, f| d.map_solid_paints_in(ids, f)));
    }
    let (old, new, real) = (median(old), median(new), median(real));
    println!("  {name:<46} {:>7} {:>7} {old:>12.3} {new:>12.3} {real:>12.3} {:>8.1}x", ids.len(), want.0, old / new);
}

fn main() {
    let unit = if thread_cpu_ns().is_some() { "CPU ms" } else { "wall ms" };
    println!(
        "  {:<46} {:>7} {:>7} {:>12} {:>12} {:>12} {:>9}",
        "document",
        "ids",
        "linked",
        format!("search, {unit}"),
        format!("set, {unit}"),
        format!("shipped, {unit}"),
        "speedup"
    );
    bench("2,000 rectangles in one layer", flat(2_000));
    bench("20,000 rectangles in one layer", flat(20_000));
    bench("5,000 rectangles in 1,000 groups of groups", nested(false));
    bench("the same with the 2,000 groups listed first", nested(true));
}
