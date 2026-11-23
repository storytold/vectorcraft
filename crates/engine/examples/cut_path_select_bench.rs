//! `cargo run --release -p vectorcraft-engine --example cut_path_select_bench` prints the time
//! Cut Path at Selected Anchor Points takes to select the cut ends, on synthetic documents of 500,
//! 2,000 and 5,000 open paths of 10 anchors each with the 8 anchors between the ends of each
//! direct-selected, and on one path of 2,000 open subpaths selected the same way. For each it times two ways of building the same
//! selection from the cut ends: clearing the selection and adding each path in turn (each add looks
//! through the paths already added), and one `Selection::set` call followed by the anchors. It
//! checks that both give the same selection, every field, before it times them, and then times the
//! whole command once per run on the 2,000-path document.
//! `ITERS` sets the runs per row (default 7); the two ways run in alternating order, each run
//! builds the selection 20 times, and each row is the median time of one build. On Linux the times are the CPU time of the main thread, which runs everything, in
//! ms; elsewhere they are wall times.
// Example (dev tool): unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::hint::black_box;
use std::time::Instant;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, NodeId, Selection};
use vectorcraft_engine::Session;
use vectorcraft_geom::{Anchor, PathData, Point, SubPath};

/// The anchors of each path, or of each subpath of the one compound path.
const ANCHORS: usize = 10;

/// The builds of the selection in each timed run; a row gives the time of one build.
const REPS: usize = 20;

/// What the command selects for one cut end: the path and its anchor (subpath, anchor).
type Pick = (NodeId, (usize, usize));

/// A way of building the selection from the cut ends.
type Rebuild = fn(&mut Selection, &[Pick]);

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

/// The selection built by clearing it and adding each path in turn.
fn one_by_one(sel: &mut Selection, picked: &[Pick]) {
    sel.clear();
    for (id, a) in picked {
        sel.add(*id);
        sel.anchors.entry(*id).or_default().insert(*a);
    }
}

/// The selection built with one `Selection::set` call, as the command builds it.
fn one_call(sel: &mut Selection, picked: &[Pick]) {
    sel.set(picked.iter().map(|(id, _)| *id));
    for (id, a) in picked {
        sel.anchors.entry(*id).or_default().insert(*a);
    }
}

fn exec(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

/// An open subpath of [`ANCHORS`] corner anchors 10 pt apart along row `row`.
fn row(row: usize) -> SubPath {
    let y = row as f64 * 20.0;
    SubPath::new((0..ANCHORS).map(|i| Anchor::corner(Point::new(i as f64 * 10.0, y))).collect(), false)
}

/// A session with the paths in `paths` and every anchor of theirs but the two ends of each
/// subpath direct-selected (a path with all its anchors selected counts as selected whole).
fn fixture(paths: Vec<PathData>) -> Session {
    let mut doc = Document::new(1600.0, 1200.0);
    let l = doc.layers.first().map(|l| l.id);
    let mut items = vec![];
    for pd in paths {
        let id = doc.alloc_id();
        let refs: Vec<[usize; 2]> =
            pd.subpaths.iter().enumerate().flat_map(|(si, sp)| (1..sp.anchors.len().saturating_sub(1)).map(move |ai| [si, ai])).collect();
        doc.insert(l, usize::MAX, Node::path(id, pd, Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0))).unwrap();
        items.push(json!({"id": id.0, "anchors": refs}));
    }
    let mut s = Session::new();
    s.add_document(doc, None);
    exec(&mut s, "select.anchorsMany", json!({"items": items}));
    s
}

/// The cut ends the command selects, in the order it selects them: read back from the selection it
/// leaves, which holds each cut path once in that order.
fn picked_after_cut(s: &Session) -> Vec<Pick> {
    let sel = &s.doc().unwrap().selection;
    sel.objects.iter().flat_map(|id| sel.anchors[id].iter().map(move |a| (*id, *a))).collect()
}

fn bench(name: &str, paths: impl Fn() -> Vec<PathData>) {
    let mut s = fixture(paths());
    exec(&mut s, "path.cutAtAnchors", json!({}));
    let picked = picked_after_cut(&s);
    let mut start = Selection::default();
    one_by_one(&mut start, &[(NodeId(u64::MAX), (0, 3))]);
    let (mut a, mut b) = (start.clone(), start.clone());
    one_by_one(&mut a, &picked);
    one_call(&mut b, &picked);
    assert_eq!(a, b, "both ways give the same selection");
    assert_eq!(b, s.doc().unwrap().selection, "the command gives that selection");
    let (mut old, mut new) = (vec![], vec![]);
    for i in 0..iters() {
        let run = |f: Rebuild, out: &mut Vec<f64>| {
            let ms = measure(|| {
                for _ in 0..REPS {
                    let mut sel = start.clone();
                    f(&mut sel, black_box(&picked));
                    black_box(&sel);
                }
            });
            out.push(ms / REPS as f64);
        };
        if i % 2 == 0 {
            run(one_by_one, &mut old);
            run(one_call, &mut new);
        } else {
            run(one_call, &mut new);
            run(one_by_one, &mut old);
        }
    }
    let (old, new) = (median(old), median(new));
    println!("  {name:<44} {:>7} {:>6} {old:>12.3} {new:>12.3} {:>8.1}x", picked.len(), b.objects.len(), old / new);
}

fn main() {
    let unit = if thread_cpu_ns().is_some() { "CPU ms" } else { "wall ms" };
    println!("  {:<44} {:>7} {:>6} {:>12} {:>12} {:>9}", "document", "picked", "ids", format!("add, {unit}"), format!("set, {unit}"), "speedup");
    for n in [500, 2_000, 5_000] {
        bench(&format!("{n} open paths, {ANCHORS} anchors each"), || (0..n).map(|i| PathData::single(row(i))).collect());
    }
    bench("one path of 2,000 open subpaths", || vec![PathData::new((0..2_000).map(row).collect())]);
    let whole: Vec<f64> = (0..iters())
        .map(|_| {
            let mut s = fixture((0..2_000).map(|i| PathData::single(row(i))).collect());
            measure(|| drop(black_box(exec(&mut s, "path.cutAtAnchors", json!({})))))
        })
        .collect();
    println!("  path.cutAtAnchors, 2,000 open paths, whole command: {:.3} {unit}", median(whole));
}
