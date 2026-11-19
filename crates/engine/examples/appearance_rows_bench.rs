//! `cargo run --release -p vectorcraft-engine --example appearance_rows_bench` prints the time three Appearance row edits
//! take with every path of documents of 2,000, 10,000 and 20,000 squares selected, each square with one fill row and one
//! stroke row. For each size it first times the step that keeps the objects whose shown rows the edit wrote, on the same
//! ids, two ways: each id looked up in the list of written objects (a list of ids, and a list of ids with the kind of row,
//! as the commands held them), and each id looked up in a set built from that list. It checks that both ways keep the
//! same ids in the same order. Then it times the whole `appearance.setItem` (`index` 1, `weight` 2),
//! `appearance.duplicateItem` (`index` 0) and `stroke.set` (`item` 1, `weight` 2) commands, each on a fresh copy of the
//! document made before the clock starts.
//! On Linux each row is the thread's CPU time, read from `/proc/thread-self/schedstat`; elsewhere it is wall time, and
//! the first line says which. `ITERS` sets the runs per row (default 7); each row is the median in ms. The two lookup
//! ways run in alternating order.
// Example (dev tool): unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashSet;
use std::hint::black_box;
use std::time::Instant;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, NodeId};
use vectorcraft_engine::Session;
use vectorcraft_geom::{Anchor, PathData, Point, SubPath};

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

/// `n` closed 10 pt squares, each with a red fill row and a 1 pt black stroke row, and their ids in paint order.
fn document(n: usize) -> (Document, Vec<NodeId>) {
    let mut doc = Document::new(4000.0, 4000.0);
    let layer = doc.layers.first().map(|l| l.id);
    let mut ids = Vec::with_capacity(n);
    for i in 0..n {
        let (x, y) = ((i % 200) as f64 * 12.0, (i / 200) as f64 * 12.0);
        let corners = [(x, y), (x + 10.0, y), (x + 10.0, y + 10.0), (x, y + 10.0)];
        let path = PathData::single(SubPath::new(corners.iter().map(|&(x, y)| Anchor::corner(Point::new(x, y))).collect(), true));
        let id = doc.alloc_id();
        let looks = Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::solid(Color::BLACK), 1.0);
        doc.insert(layer, usize::MAX, Node::path(id, path, looks)).unwrap();
        ids.push(id);
    }
    (doc, ids)
}

/// The ids among `ids` that `wrote` holds, each looked up in the list: the lookup the row edits made.
fn in_list(ids: Vec<NodeId>, wrote: &[NodeId]) -> Vec<NodeId> {
    ids.into_iter().filter(|id| wrote.contains(id)).collect()
}

/// [`in_list`] with each id looked up in a set built from `wrote`.
fn in_set(ids: Vec<NodeId>, wrote: &[NodeId]) -> Vec<NodeId> {
    let wrote: HashSet<NodeId> = wrote.iter().copied().collect();
    ids.into_iter().filter(|id| wrote.contains(id)).collect()
}

/// The ids among `ids` that `edited` holds with either kind of row, each looked up in the list, as `appearance.setItem`
/// did.
fn in_kind_list(ids: Vec<NodeId>, edited: &[(NodeId, bool)]) -> Vec<NodeId> {
    ids.into_iter().filter(|id| edited.iter().any(|(e, _)| e == id)).collect()
}

/// [`in_kind_list`] with each id looked up in a set built from the ids of `edited`.
fn in_kind_set(ids: Vec<NodeId>, edited: &[(NodeId, bool)]) -> Vec<NodeId> {
    let wrote: HashSet<NodeId> = edited.iter().map(|(id, _)| *id).collect();
    ids.into_iter().filter(|id| wrote.contains(id)).collect()
}

fn row(name: &str, ms: f64) {
    println!("  {name:<52} {ms:>10.3} ms");
}

/// The medians of `old` and `new` over [`iters`] runs, alternating which goes first; each run filters `ids` `reps` times
/// on copies made before the clock starts and gives the time of one.
fn time_both<W: ?Sized>(ids: &[NodeId], wrote: &W, old: fn(Vec<NodeId>, &W) -> Vec<NodeId>, new: fn(Vec<NodeId>, &W) -> Vec<NodeId>) -> (f64, f64) {
    let reps = (200_000 / ids.len()).max(1);
    let run = |f: fn(Vec<NodeId>, &W) -> Vec<NodeId>| {
        let mut work: Vec<Vec<NodeId>> = (0..reps).map(|_| ids.to_vec()).collect();
        ms(|| {
            for it in &mut work {
                black_box(f(black_box(std::mem::take(it)), wrote));
            }
        }) / reps as f64
    };
    let (mut a, mut b) = (vec![], vec![]);
    for i in 0..iters() {
        if i % 2 == 0 {
            a.push(run(old));
            b.push(run(new));
        } else {
            b.push(run(new));
            a.push(run(old));
        }
    }
    (median(a), median(b))
}

fn bench(n: usize) {
    println!("{n} squares, all selected");
    let (doc, ids) = document(n);
    // Every object's topmost stroke row was written, as when every selected object takes the edit.
    let edited: Vec<(NodeId, bool)> = ids.iter().map(|id| (*id, false)).collect();
    // A list missing every other id, so lookups that fail are timed too.
    let half: Vec<NodeId> = ids.iter().copied().step_by(2).collect();
    for wrote in [&ids, &half] {
        assert_eq!(in_list(ids.clone(), wrote), in_set(ids.clone(), wrote), "both ways keep the same ids");
    }
    assert_eq!(in_kind_list(ids.clone(), &edited), in_kind_set(ids.clone(), &edited), "both ways keep the same ids");
    let (old, new) = time_both(&ids, ids.as_slice(), in_list, in_set);
    row("keep written ids, every id in the list", old);
    row("keep written ids, every id in a set", new);
    let (old, new) = time_both(&ids, half.as_slice(), in_list, in_set);
    row("keep written ids, half the ids in the list", old);
    row("keep written ids, half the ids in a set", new);
    let (old, new) = time_both(&ids, edited.as_slice(), in_kind_list, in_kind_set);
    row("keep written ids, (id, kind) list", old);
    row("keep written ids, (id, kind) set", new);

    let all: Vec<u64> = ids.iter().map(|id| id.0).collect();
    let commands: [(&str, Value); 3] = [
        ("appearance.setItem", json!({"ids": all, "index": 1, "weight": 2})),
        ("appearance.duplicateItem", json!({"ids": all, "index": 0})),
        ("stroke.set", json!({"ids": all, "item": 1, "weight": 2})),
    ];
    for (cmd, p) in &commands {
        let runs = (0..iters())
            .map(|_| {
                let mut s = Session::new();
                s.add_document(doc.clone(), None);
                let t = ms(|| drop(black_box(s.execute(cmd, p).unwrap())));
                drop(black_box(s));
                t
            })
            .collect();
        row(&format!("{cmd}, whole command"), median(runs));
    }
}

fn main() {
    println!("{}", if cpu_ns().is_some() { "thread CPU time, median of runs" } else { "wall time, median of runs" });
    for n in [2_000, 10_000, 20_000] {
        bench(n);
    }
}
