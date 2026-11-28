//! `select.anchorsMany`'s item loop ([`select_anchor_items`]) against a loop that changes the
//! selection one item at a time through [`Selection::add`], [`Selection::remove`] and
//! [`Selection::toggle`].

use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use vectorcraft_color::Color;
use vectorcraft_doc::{AnchorRef, Document, NodeId, Selection, SymbolPart};
use vectorcraft_geom::Rect;
use vectorcraft_testkit::fixtures::DocBuilder;

use super::{OBJECTS_WALKS, select_anchor_items};

type Items = Vec<(NodeId, BTreeSet<AnchorRef>)>;

const MODES: [&str; 4] = ["set", "add", "toggle", "subtract"];

/// The item loop through `Selection`'s own methods, each of which goes through `objects`.
fn item_by_item(d: &Document, sel: &mut Selection, items: Items, mode: &str) {
    if mode == "set" {
        sel.clear();
    }
    for (id, refs) in items {
        let segment_pick = sel.segments.remove(&id).is_some();
        let all: BTreeSet<AnchorRef> =
            d.node(id).and_then(|n| n.path_data()).map(|p| p.anchors().map(|(s, i, _)| (s, i)).collect()).unwrap_or_default();
        if all.is_empty() {
            match mode {
                "toggle" => sel.toggle(id),
                "subtract" => sel.remove(id),
                _ => sel.add(id),
            }
            continue;
        }
        let now = match sel.partial(id) {
            _ if segment_pick => BTreeSet::new(),
            Some(a) => a.clone(),
            None if sel.contains(id) => all.clone(),
            None => BTreeSet::new(),
        };
        let refs: BTreeSet<AnchorRef> = refs.intersection(&all).copied().collect();
        let next: BTreeSet<AnchorRef> = match mode {
            "toggle" => now.symmetric_difference(&refs).copied().collect(),
            "subtract" => now.difference(&refs).copied().collect(),
            _ => now.union(&refs).copied().collect(),
        };
        if next.is_empty() {
            sel.remove(id);
            continue;
        }
        sel.add(id);
        if next.len() < all.len() {
            sel.anchors.insert(id, next);
        } else {
            sel.anchors.remove(&id);
        }
    }
}

/// A document with `n` four-anchor rectangles and, after them, a group of two more → the
/// rectangles' ids and the group's.
fn doc(n: usize) -> (Document, Vec<NodeId>, NodeId) {
    let mut b = DocBuilder::new(2000.0, 2000.0);
    let rects = (0..n)
        .map(|i| {
            let (x, y) = ((i % 200) as f64 * 10.0, (i / 200) as f64 * 10.0);
            b.rect(Rect::new(x, y, x + 5.0, y + 5.0), Color::rgb(0.2, 0.4, 0.6), |_| {})
        })
        .collect();
    let black = Color::rgb(0.0, 0.0, 0.0);
    let kids = vec![b.rect_node(Rect::new(0.0, 0.0, 1.0, 1.0), black), b.rect_node(Rect::new(2.0, 0.0, 3.0, 1.0), black)];
    let group = b.group(kids, false);
    (b.build(), rects, group)
}

fn anchors(refs: &[(usize, usize)]) -> BTreeSet<AnchorRef> {
    refs.iter().copied().collect()
}

/// A selection with every field set: rectangles 0 to 4 and the group, rectangle 2 listed twice,
/// partial anchors on 1 and 3, a segment pick on 4, rectangle 1 the key, a key anchor on 3, and a
/// target, slices, guides, a key guide, new art, a symbol part and an on-path member.
fn seeded(r: &[NodeId], group: NodeId) -> Selection {
    let mut s = Selection { objects: vec![r[0], r[1], r[2], r[3], group, r[2], r[4]], ..Selection::default() };
    s.anchors.insert(r[1], anchors(&[(0, 0), (0, 1)]));
    s.anchors.insert(r[3], anchors(&[(0, 2)]));
    s.anchors.insert(r[4], anchors(&[(0, 0), (0, 1)]));
    s.segments.insert(r[4], anchors(&[(0, 0)]));
    s.key = Some(r[1]);
    s.key_anchor = Some((r[3], (0, 2)));
    s.target = Some(r[0]);
    s.slices = vec![r[5]];
    s.guides = vec![r[6]];
    s.key_guide = Some(r[6]);
    s.new_art = Arc::new(HashSet::from([r[2]]));
    s.symbol_part = Some(SymbolPart { instance: r[0], path: vec![0] });
    s.on_path_member = Some(r[7]);
    s
}

/// Item lists with repeated ids, removals followed by additions, the group, anchors outside a
/// path, items that empty a path, and the empty list, then pseudo-random lists.
fn item_streams(r: &[NodeId], group: NodeId) -> Vec<Items> {
    let all = [(0, 0), (0, 1), (0, 2), (0, 3)];
    let mut streams: Vec<Items> = vec![
        vec![],
        vec![(r[2], anchors(&[(0, 0)]))],
        vec![(r[2], anchors(&all)), (r[2], anchors(&all))],
        vec![(r[2], anchors(&all)), (r[5], anchors(&[(0, 1)])), (r[2], anchors(&[(0, 3)]))],
        vec![(group, anchors(&[])), (group, anchors(&[(0, 0)])), (group, anchors(&[]))],
        vec![(r[6], anchors(&[(0, 0)])), (r[6], anchors(&[(0, 0)])), (r[7], anchors(&[(0, 9), (3, 0)]))],
        vec![(r[1], anchors(&[(0, 0), (0, 1)])), (r[3], anchors(&[(0, 2)])), (r[1], anchors(&all)), (r[3], anchors(&all))],
        vec![(r[4], anchors(&[])), (r[4], anchors(&[(0, 2)]))],
        vec![(r[1], anchors(&all)), (r[3], anchors(&all)), (r[1], anchors(&[(0, 0)])), (r[3], anchors(&[(0, 2)]))],
        vec![(r[8], anchors(&[(0, 0)])), (r[0], anchors(&all)), (r[8], anchors(&all)), (r[0], anchors(&[(0, 1)]))],
    ];
    let pool: Vec<NodeId> = r.iter().copied().chain([group]).collect();
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for len in [3, 9, 25, 80, 200] {
        let items = (0..len)
            .map(|_| {
                let id = pool[(next() % pool.len() as u64) as usize];
                // Bits 0 to 4 of a draw pick anchors 0 to 3 and one past the end.
                let bits = next();
                let refs = (0..5).filter(|i| (bits >> i) & 1 == 1).map(|i| (0, i)).collect();
                (id, refs)
            })
            .collect();
        streams.push(items);
    }
    streams
}

/// Starting selections: the seeded one, none, every rectangle whole, and every rectangle with
/// one anchor.
fn starts(r: &[NodeId], group: NodeId) -> Vec<Selection> {
    let mut whole = Selection::default();
    whole.set(r.iter().copied().chain([group]));
    let mut partial = whole.clone();
    for id in r {
        partial.anchors.insert(*id, anchors(&[(0, 1)]));
    }
    vec![seeded(r, group), Selection::default(), whole, partial]
}

#[test]
fn anchor_items_do_what_the_item_by_item_loop_does() {
    let (d, r, group) = doc(12);
    for start in starts(&r, group) {
        for items in item_streams(&r, group) {
            for mode in MODES {
                let mut expected = start.clone();
                item_by_item(&d, &mut expected, items.clone(), mode);
                let mut got = start.clone();
                select_anchor_items(&d, &mut got, items.clone(), mode);
                assert_eq!(got, expected, "mode {mode}, items {items:?}, from {start:?}");
                assert_eq!(got.new_art, expected.new_art);
            }
        }
    }
}

#[test]
fn a_path_deselected_and_selected_again_goes_last_without_its_key() {
    let (d, r, group) = doc(12);
    let all = anchors(&[(0, 0), (0, 1), (0, 2), (0, 3)]);
    let mut s = seeded(&r, group);
    // Toggling the selected anchors of 1 and 3 deselects them; toggling again selects them again.
    let items = vec![(r[1], anchors(&[(0, 0), (0, 1)])), (r[3], anchors(&[(0, 2)])), (r[1], anchors(&[(0, 0)])), (r[3], all)];
    select_anchor_items(&d, &mut s, items, "toggle");
    assert_eq!(s.objects, [r[0], r[2], group, r[2], r[4], r[1], r[3]]);
    assert_eq!(s.anchors.get(&r[1]), Some(&anchors(&[(0, 0)])));
    assert_eq!(s.anchors.get(&r[3]), None);
    assert_eq!((s.key, s.key_anchor), (None, None));
}

/// Every rectangle with one anchor, as a Direct Selection marquee sends them.
fn marquee(r: &[NodeId]) -> Items {
    r.iter().map(|id| (*id, anchors(&[(0, 0)]))).collect()
}

/// Every rectangle with all four anchors, which deselects each in subtract mode.
fn every_anchor(r: &[NodeId]) -> Items {
    r.iter().map(|id| (*id, anchors(&[(0, 0), (0, 1), (0, 2), (0, 3)]))).collect()
}

#[test]
fn anchor_items_walk_the_selected_objects_at_most_twice() {
    let (d, r, group) = doc(2000);
    let mut all = Selection::default();
    all.set(r.iter().copied().chain([group]));
    for (start, items, mode) in [
        (Selection::default(), marquee(&r), "set"),
        (all.clone(), marquee(&r), "toggle"),
        (all.clone(), every_anchor(&r), "subtract"),
        (all.clone(), every_anchor(&r).into_iter().chain(marquee(&r)).collect(), "toggle"),
    ] {
        let mut s = start;
        OBJECTS_WALKS.set(0);
        select_anchor_items(&d, &mut s, items, mode);
        let walks = OBJECTS_WALKS.get();
        assert!(walks <= 2, "{mode}: {walks} walks over the selected objects");
    }
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

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Timings of the item loop and the item-by-item loop on 2,000, 20,000 and 50,000 rectangles, run
/// in turn on clones of the same selection, 7 runs each. Ignored by default; it prints the median,
/// fastest and slowest run and sets no bound on them:
/// `cargo test --release -p vectorcraft-engine --lib anchor_items_perf -- --ignored --nocapture`.
#[test]
#[ignore = "a benchmark: run with --ignored --nocapture"]
fn anchor_items_perf() {
    const RUNS: usize = 7;
    eprintln!("anchor_items_perf: {}", if thread_cpu().is_some() { "thread CPU time" } else { "wall clock" });
    eprintln!("anchor_items_perf: {:>6} {:<26} {:>28} {:>28}", "paths", "case", "item by item ms (min-max)", "item loop ms (min-max)");
    for n in [2_000, 20_000, 50_000] {
        let (d, r, group) = doc(n);
        let mut all = Selection::default();
        all.set(r.iter().copied().chain([group]));
        for (case, start, items, mode) in [
            ("set, marquee", Selection::default(), marquee(&r), "set"),
            ("add, marquee", Selection::default(), marquee(&r), "add"),
            ("toggle, Select All", all.clone(), marquee(&r), "toggle"),
            ("subtract every anchor", all.clone(), every_anchor(&r), "subtract"),
        ] {
            let (mut old, mut new) = (Vec::new(), Vec::new());
            for run in 0..RUNS {
                let (mut a, mut b) = (start.clone(), start.clone());
                let (ia, ib) = (items.clone(), items.clone());
                // The two loops take turns going first.
                if run % 2 == 0 {
                    old.push(time(|| item_by_item(&d, &mut a, ia, mode)));
                    new.push(time(|| select_anchor_items(&d, &mut b, ib, mode)));
                } else {
                    new.push(time(|| select_anchor_items(&d, &mut b, ib, mode)));
                    old.push(time(|| item_by_item(&d, &mut a, ia, mode)));
                }
                assert_eq!(a, b, "{n} paths, {case}");
            }
            old.sort();
            new.sort();
            eprintln!(
                "anchor_items_perf: {n:>6} {case:<26} {:>10.3} ({:>7.3}-{:>8.3}) {:>10.3} ({:>7.3}-{:>8.3})",
                ms(old[RUNS / 2]),
                ms(old[0]),
                ms(old[RUNS - 1]),
                ms(new[RUNS / 2]),
                ms(new[0]),
                ms(new[RUNS - 1]),
            );
        }
    }
}
