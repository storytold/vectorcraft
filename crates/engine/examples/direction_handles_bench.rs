//! `cargo run --release -p vectorcraft-engine --example direction_handles_bench` prints the time
//! Select › Object › Direction Handles takes to select every path of documents of 2,000, 10,000 and
//! 20,000 rectangles, spread over two layers with one group. For each size it times, on the same
//! paths and anchors, the selection built one path at a time (a clear, then an add per path) and
//! the selection built with one `Selection::set` call, checks that the two are equal, and then
//! times the whole `select.object.directionHandles` command.
//! On Linux each row is the thread's CPU time, read from `/proc/thread-self/schedstat`; elsewhere
//! it is wall time, and the first line says which. `ITERS` sets the runs per row (default 7); each
//! row is the median in ms. A run of the two selection rows repeats the selection 1,000,000 / paths
//! times and gives the time of one.
// Example (dev tool): unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::hint::black_box;
use std::time::Instant;

use serde_json::json;
use vectorcraft_color::Color;
use vectorcraft_doc::{Document, NodeId, Selection};
use vectorcraft_engine::Session;
use vectorcraft_geom::Rect;
use vectorcraft_testkit::fixtures::DocBuilder;

type Items = Vec<(NodeId, BTreeSet<(usize, usize)>)>;

fn iters() -> usize {
    std::env::var("ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(7)
}

/// The thread's CPU time in ns, or `None` where `/proc/thread-self/schedstat` cannot be read.
fn cpu_ns() -> Option<u64> {
    std::fs::read_to_string("/proc/thread-self/schedstat").ok()?.split_whitespace().next()?.parse().ok()
}

/// The time `f` takes in ms: CPU time when [`cpu_ns`] reads, wall time otherwise.
fn ms(f: impl FnOnce()) -> f64 {
    let (cpu, wall) = (cpu_ns(), Instant::now());
    f();
    match (cpu, cpu_ns()) {
        (Some(a), Some(b)) => (b - a) as f64 / 1e6,
        _ => wall.elapsed().as_secs_f64() * 1000.0,
    }
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// `n` 10 pt squares: a tenth of them in one group and the rest loose, half on each of two layers.
fn document(n: usize) -> Document {
    let mut b = DocBuilder::new(4000.0, 4000.0);
    let square = |b: &mut DocBuilder, i: usize| {
        let (x, y) = ((i % 200) as f64 * 12.0, (i / 200) as f64 * 12.0);
        b.rect_node(Rect::new(x, y, x + 10.0, y + 10.0), Color::rgb(0.2, 0.4, 0.6))
    };
    let grouped = n / 10;
    let kids = (0..grouped).map(|i| square(&mut b, i)).collect();
    b.group(kids, false);
    for i in grouped..n {
        if i == n / 2 {
            b.layer("Layer 2");
        }
        let node = square(&mut b, i);
        b.add(node);
    }
    b.build()
}

/// Selects `items` one path at a time: a clear, then an add and an anchor insert per path.
fn one_by_one(sel: &mut Selection, items: Items) {
    sel.clear();
    for (id, refs) in items {
        sel.add(id);
        sel.anchors.insert(id, refs);
    }
}

/// Selects `items` with one `Selection::set` call and one extend of the anchors.
fn with_set(sel: &mut Selection, items: Items) {
    sel.set(items.iter().map(|(id, _)| *id));
    sel.anchors.extend(items);
}

fn row(name: &str, ms: f64) {
    println!("  {name:<44} {ms:>10.3} ms");
}

fn bench(n: usize) {
    println!("{n} paths");
    let mut s = Session::new();
    s.add_document(document(n), None);
    s.execute("select.all", &json!({})).unwrap();
    let start = s.doc().unwrap().selection.clone();
    s.execute("select.object.directionHandles", &json!({})).unwrap();
    let command = s.doc().unwrap().selection.clone();
    // The paths and anchors the command selects, in its order.
    let items: Items = command.objects.iter().map(|id| (*id, command.anchors[id].clone())).collect();
    assert_eq!(items.len(), n);

    // Each run selects `reps` times, on copies made before the clock starts, and gives the time of one.
    let reps = (1_000_000 / n).max(1);
    let time = |f: fn(&mut Selection, Items)| -> (f64, Selection) {
        let mut last = Selection::default();
        let runs = (0..iters())
            .map(|_| {
                let mut work: Vec<(Selection, Items)> = (0..reps).map(|_| (start.clone(), items.clone())).collect();
                let t = ms(|| {
                    for (sel, it) in &mut work {
                        f(sel, black_box(std::mem::take(it)));
                    }
                });
                last = work.pop().unwrap().0;
                t / reps as f64
            })
            .collect();
        (median(runs), last)
    };
    let (old, a) = time(one_by_one);
    let (new, b) = time(with_set);
    assert_eq!(a, b, "both ways give the same selection");
    let mut want = start.clone();
    with_set(&mut want, items.clone());
    assert_eq!(want.objects, command.objects);
    assert_eq!(want.anchors, command.anchors);
    row("selection, one add per path", old);
    row("selection, one set call", new);
    println!("  {:<44} {:>10.1}x", "ratio", old / new);

    let runs = (0..iters())
        .map(|_| {
            s.execute("select.all", &json!({})).unwrap();
            ms(|| drop(black_box(s.execute("select.object.directionHandles", &json!({})).unwrap())))
        })
        .collect();
    row("select.object.directionHandles, whole command", median(runs));
}

fn main() {
    println!("{}", if cpu_ns().is_some() { "thread CPU time, median of runs" } else { "wall time, median of runs" });
    for n in [2_000, 10_000, 20_000] {
        bench(n);
    }
}
