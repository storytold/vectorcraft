//! `cargo run --release -p vectorcraft-engine --example layer_select_bench` prints the time of a
//! Shift-click in the Layers panel's selection column (`layer.selectAll {id, add: true}`) on a layer of
//! 2,000, 20,000 and 50,000 rectangles, with all of its art selected (the click removes it) and with
//! every other object selected (the click adds the rest). Each case is timed one id at a time (`old`,
//! the loop the command used before, copied here) and in one pass (`new`), side by side in one
//! process: first the whole command, then the selection step alone. Each case checks that both give
//! the same selection and reply before it is timed.
//! `ITERS` sets the runs per case (default 7); each row is the median wall time in ms, followed by
//! every run of `old` and of `new`.
// Example (dev tool): unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashSet;
use std::hint::black_box;
use std::time::Instant;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, NodeId, Selection};
use vectorcraft_engine::Session;
use vectorcraft_geom::{Anchor, PathData, Point, SubPath};

fn iters() -> usize {
    std::env::var("ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(7).max(1)
}

fn median(ms: &[f64]) -> f64 {
    let mut ms = ms.to_vec();
    ms.sort_by(f64::total_cmp);
    ms[ms.len() / 2]
}

/// A document whose first layer holds `n` 10 pt squares in a row → (document, layer, squares).
fn squares(n: usize) -> (Document, NodeId, Vec<NodeId>) {
    let mut doc = Document::new(1600.0, 1200.0);
    let layer = doc.layers[0].id;
    let ids = (0..n)
        .map(|i| {
            let (x, y) = ((i % 100) as f64 * 12.0, (i / 100) as f64 * 12.0);
            let corners = [(x, y), (x + 10.0, y), (x + 10.0, y + 10.0), (x, y + 10.0)];
            let path = PathData::single(SubPath::new(corners.iter().map(|&(x, y)| Anchor::corner(Point::new(x, y))).collect(), true));
            let id = doc.alloc_id();
            doc.insert(Some(layer), usize::MAX, Node::path(id, path, Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 1.0))).unwrap();
            id
        })
        .collect();
    (doc, layer, ids)
}

/// `layer.selectAll {id: layer, add: true}` before the change: the layer's art checked, removed or
/// added one id at a time with [`Selection::contains`], [`Selection::remove`] and [`Selection::add`].
fn old_command(s: &mut Session, layer: NodeId) -> Value {
    let st = s.doc().unwrap();
    let ids = st.doc.node(layer).unwrap().layer_art(true);
    let all_selected = !ids.is_empty() && ids.iter().all(|i| st.selection.contains(*i));
    let rows = st.layer_rows.clone();
    s.select(|_, sel| old_step(sel, &ids, all_selected)).unwrap();
    let st = s.doc_mut().unwrap();
    st.layer_rows = rows;
    json!({ "selected": st.selection.objects.iter().map(|i| i.0).collect::<Vec<_>>() })
}

fn old_step(sel: &mut Selection, ids: &[NodeId], all_selected: bool) {
    for i in ids {
        if all_selected { sel.remove(*i) } else { sel.add(*i) }
    }
}

fn new_command(s: &mut Session, layer: NodeId) -> Value {
    s.execute("layer.selectAll", &json!({"id": layer.0, "add": true})).unwrap()
}

/// Every run of `old` and of `new`, run in turn, each from `prepare`'s state.
fn time<S>(mut prepare: impl FnMut() -> S, old: impl Fn(&mut S), new: impl Fn(&mut S)) -> (Vec<f64>, Vec<f64>) {
    let run = |s: &mut S, f: &dyn Fn(&mut S)| {
        let t = Instant::now();
        f(s);
        t.elapsed().as_secs_f64() * 1000.0
    };
    let (mut olds, mut news) = (Vec::new(), Vec::new());
    for _ in 0..iters() {
        let mut s = prepare();
        olds.push(run(&mut s, &old));
        black_box(&s);
        let mut s = prepare();
        news.push(run(&mut s, &new));
        black_box(&s);
    }
    (olds, news)
}

fn row(name: &str, (old, new): (Vec<f64>, Vec<f64>)) {
    let (o, n) = (median(&old), median(&new));
    let list = |ms: &[f64]| ms.iter().map(|m| format!("{m:.2}")).collect::<Vec<_>>().join(" ");
    println!("  {name:<40} {o:>12.3} ms {n:>12.3} ms {:>9.1}x", o / n.max(1e-9));
    println!("    old runs: {}", list(&old));
    println!("    new runs: {}", list(&new));
}

fn main() {
    println!("  {:<40} {:>15} {:>15} {:>10}", "case", "old", "new", "speedup");
    for n in [2_000usize, 20_000, 50_000] {
        println!("{n} objects");
        let (doc, layer, ids) = squares(n);
        let mut s = Session::new();
        s.add_document(doc, None);
        let mut all = Selection::default();
        all.set(ids.iter().copied());
        let mut half = Selection::default();
        half.set(ids.iter().copied().step_by(2));
        for (name, start) in [("all selected (removes)", &all), ("half selected (adds)", &half)] {
            let reset = |s: &mut Session| {
                let st = s.doc_mut().unwrap();
                st.selection = start.clone();
                st.layer_rows.clear();
            };
            // Both ways give the same selection and reply.
            reset(&mut s);
            let old_reply = old_command(&mut s, layer);
            let old_sel = s.doc().unwrap().selection.clone();
            reset(&mut s);
            let new_reply = new_command(&mut s, layer);
            assert_eq!(s.doc().unwrap().selection, old_sel, "{name}: the same selection");
            assert_eq!(new_reply, old_reply, "{name}: the same reply");

            let (mut olds, mut news) = (Vec::new(), Vec::new());
            for _ in 0..iters() {
                for (f, ms) in [(old_command as fn(&mut Session, NodeId) -> Value, &mut olds), (new_command, &mut news)] {
                    reset(&mut s);
                    let t = Instant::now();
                    black_box(f(&mut s, layer));
                    ms.push(t.elapsed().as_secs_f64() * 1000.0);
                }
            }
            row(&format!("command, {name}"), (olds, news));
            // The selection step alone, on a copy of the starting selection.
            let art = s.doc().unwrap().doc.node(layer).unwrap().layer_art(true);
            let selected: HashSet<NodeId> = start.objects.iter().copied().collect();
            let all_selected = art.iter().all(|i| selected.contains(i));
            let new_step = |sel: &mut Selection| {
                if all_selected {
                    sel.remove_all(art.iter().copied());
                } else {
                    sel.add_all(art.iter().copied());
                }
            };
            let (mut a, mut b) = (start.clone(), start.clone());
            old_step(&mut a, &art, all_selected);
            new_step(&mut b);
            assert_eq!(a, b, "{name}: the same selection step");
            row(&format!("selection step, {name}"), time(|| start.clone(), |sel| old_step(sel, &art, all_selected), new_step));
        }
    }
}
