//! `cargo run --release -p vectorcraft-engine --example stroke_targets_bench` prints the time `stroke.set`
//! takes to list its targets on documents of 2,000, 10,000 and 20,000 rectangles, all in one group. For
//! each size it times, on the same leaves, the merge that checks each leaf against the growing list and
//! the merge that checks it against a set, checks that the two lists are equal, and then times the whole
//! `stroke.set {weight: 3}` command twice: once with the group named and once with every rectangle named.
//! The whole-command rows include the document copy the command makes for undo; each run is undone
//! before the next, outside the timed part.
//! On Linux each row is the thread's CPU time, read from `/proc/thread-self/schedstat`; elsewhere
//! it is wall time, and the first line says which. `ITERS` sets the runs per row (default 7); each
//! row is the median in ms. A run of the two merge rows repeats the merge 1,000,000 / paths times
//! and gives the time of one.
// Example (dev tool): unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashSet;
use std::hint::black_box;
use std::time::Instant;

use serde_json::{Value, json};
use vectorcraft_color::Color;
use vectorcraft_doc::{Document, NodeId};
use vectorcraft_engine::Session;
use vectorcraft_geom::Rect;
use vectorcraft_testkit::fixtures::DocBuilder;

fn iters() -> usize {
    std::env::var("ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(7).max(1)
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

/// `n` 10 pt squares in one group, and the squares' ids in stacking order.
fn document(n: usize) -> (Document, NodeId, Vec<NodeId>) {
    let mut b = DocBuilder::new(4000.0, 4000.0);
    let kids: Vec<_> = (0..n)
        .map(|i| {
            let (x, y) = ((i % 200) as f64 * 12.0, (i / 200) as f64 * 12.0);
            b.rect_node(Rect::new(x, y, x + 10.0, y + 10.0), Color::rgb(0.2, 0.4, 0.6))
        })
        .collect();
    let leaves = kids.iter().map(|k| k.id).collect();
    let group = b.group(kids, false);
    (b.build(), group, leaves)
}

/// The merge that checks each leaf against the growing list.
fn listed(mut out: Vec<NodeId>, leaves: Vec<NodeId>) -> Vec<NodeId> {
    for id in leaves {
        if !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

/// The merge that checks each leaf against a set seeded with `out`.
fn with_set(mut out: Vec<NodeId>, leaves: Vec<NodeId>) -> Vec<NodeId> {
    let mut seen: HashSet<NodeId> = out.iter().copied().collect();
    for id in leaves {
        if seen.insert(id) {
            out.push(id);
        }
    }
    out
}

fn row(name: &str, ms: f64) {
    println!("  {name:<52} {ms:>10.3} ms");
}

/// The median time of `stroke.set {weight: 3}` on `ids`, each run undone outside the timed part.
fn whole_command(s: &mut Session, ids: &[NodeId], n: usize) -> f64 {
    let p: Value = json!({"ids": ids.iter().map(|id| id.0).collect::<Vec<_>>(), "weight": 3});
    let runs = (0..iters())
        .map(|_| {
            let mut r = Value::Null;
            let t = ms(|| r = black_box(s.execute("stroke.set", &p).unwrap()));
            assert_eq!(r["objects"], n, "{r}");
            s.execute("edit.undo", &json!({})).unwrap();
            t
        })
        .collect();
    median(runs)
}

fn bench(n: usize) {
    println!("{n} paths in one group");
    let (doc, group, leaves) = document(n);

    // Each run merges `reps` times, on copies made before the clock starts, and gives the time of one.
    let reps = (1_000_000 / n).max(1);
    let time = |f: fn(Vec<NodeId>, Vec<NodeId>) -> Vec<NodeId>| -> (f64, Vec<NodeId>) {
        let mut last = Vec::new();
        let runs = (0..iters())
            .map(|_| {
                let mut work: Vec<Vec<NodeId>> = (0..reps).map(|_| leaves.clone()).collect();
                let mut done = Vec::with_capacity(reps);
                let t = ms(|| {
                    for input in &mut work {
                        done.push(f(Vec::new(), black_box(std::mem::take(input))));
                    }
                });
                last = done.pop().unwrap();
                t / reps as f64
            })
            .collect();
        (median(runs), last)
    };
    let (old, a) = time(listed);
    let (new, b) = time(with_set);
    assert_eq!(a, b, "both merges give the same list");
    assert_eq!(a, leaves);
    row("target merge, list search", old);
    row("target merge, set", new);
    println!("  {:<52} {:>10.1}x", "ratio", old / new);

    let mut s = Session::new();
    s.add_document(doc, None);
    row("stroke.set, whole command, the group named", whole_command(&mut s, &[group], n));
    row("stroke.set, whole command, every path named", whole_command(&mut s, &leaves, n));
}

fn main() {
    println!("{}", if cpu_ns().is_some() { "thread CPU time, median of runs" } else { "wall time, median of runs" });
    for n in [2_000, 10_000, 20_000] {
        bench(n);
    }
}
