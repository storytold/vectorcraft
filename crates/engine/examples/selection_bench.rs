//! `cargo run --release -p vectorcraft-engine --example selection_bench` prints the time of the
//! selection steps behind `select.toggle`, `select.add` and `select.inverse` on selections of 2,000,
//! 20,000 and 50,000 objects, done one id at a time (`old`) and in one pass (`new`), side by side in
//! one process. Each step checks that both give the same selection before it is timed.
//! `ITERS` sets the runs per step (default 7); each row is the median wall time in ms.
// Example (dev tool): unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashSet;
use std::hint::black_box;
use std::time::Instant;

use vectorcraft_doc::{NodeId, Selection};

fn iters() -> usize {
    std::env::var("ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(7).max(1)
}

fn median(mut ms: Vec<f64>) -> f64 {
    ms.sort_by(f64::total_cmp);
    ms[ms.len() / 2]
}

/// The median time of `old` and of `new` on clones of `start`, run in turn.
fn time(start: &Selection, old: impl Fn(&mut Selection), new: impl Fn(&mut Selection)) -> (f64, f64) {
    let (mut a, mut b) = (start.clone(), start.clone());
    old(&mut a);
    new(&mut b);
    assert_eq!(a, b, "both ways give the same selection");
    let run = |f: &dyn Fn(&mut Selection)| {
        let mut s = start.clone();
        let t = Instant::now();
        f(&mut s);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        black_box(s);
        ms
    };
    let (mut olds, mut news) = (Vec::new(), Vec::new());
    for _ in 0..iters() {
        olds.push(run(&old));
        news.push(run(&new));
    }
    (median(olds), median(news))
}

/// `select.add` before the change: [`Selection::add`] for each id.
fn old_add(s: &mut Selection, ids: &[NodeId]) {
    for id in ids {
        s.add(*id);
    }
}

/// `select.toggle` before the change: [`Selection::toggle`] for each id.
fn old_toggle(s: &mut Selection, ids: &[NodeId]) {
    for id in ids {
        s.toggle(*id);
    }
}

/// `select.inverse` before the change: the selectable ids not found by [`Selection::contains`].
fn old_inverse(s: &mut Selection, selectable: &[NodeId]) {
    let ids: Vec<NodeId> = selectable.iter().copied().filter(|id| !s.contains(*id)).collect();
    s.set(ids);
}

/// `select.inverse` after the change: the selectable ids not in a set of the selected ones.
fn new_inverse(s: &mut Selection, selectable: &[NodeId]) {
    let selected: HashSet<NodeId> = s.objects.iter().copied().collect();
    let ids: Vec<NodeId> = selectable.iter().copied().filter(|id| !selected.contains(id)).collect();
    s.set(ids);
}

fn row(name: &str, (old, new): (f64, f64)) {
    println!("  {name:<44} {old:>12.3} ms {new:>12.3} ms {:>9.1}x", old / new.max(1e-9));
}

fn main() {
    println!("  {:<44} {:>15} {:>15} {:>10}", "step", "old", "new", "speedup");
    for n in [2_000u64, 20_000, 50_000] {
        println!("{n} objects");
        let all: Vec<NodeId> = (1..=n).map(NodeId).collect();
        let none = Selection::default();
        let mut every = Selection::default();
        every.set(all.iter().copied());
        let mut half = Selection::default();
        half.set(all.iter().copied().step_by(2));
        let interleaved: Vec<NodeId> = all.iter().copied().skip(n as usize / 4).chain(all.iter().copied().take(n as usize / 4)).collect();
        let half_new: Vec<NodeId> = all.iter().copied().skip(n as usize / 2).collect();
        row("toggle every object off", time(&every, |s| old_toggle(s, &all), |s| s.toggle_all(all.iter().copied())));
        row("toggle every object on", time(&none, |s| old_toggle(s, &all), |s| s.toggle_all(all.iter().copied())));
        row("toggle every object, half selected", time(&half, |s| old_toggle(s, &interleaved), |s| s.toggle_all(interleaved.iter().copied())));
        row("add half the objects, half selected", time(&half, |s| old_add(s, &half_new), |s| s.add_all(half_new.iter().copied())));
        row("inverse, half selected", time(&half, |s| old_inverse(s, &all), |s| new_inverse(s, &all)));
    }
}
