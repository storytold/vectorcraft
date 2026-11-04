//! `cargo run --release -p vectorcraft-engine --example anchor_bench` prints the wall time of the
//! engine steps of editing anchors with Direct Selection, on three synthetic documents: one closed path of
//! 50,000 anchors, 2,000 closed paths of 50 anchors each, and one open path of 200,000 anchors.
//! `ITERS` sets the runs per step (default 7); each row is the median in ms.
// Example (dev tool): unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::hint::black_box;
use std::time::Instant;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, NodeId};
use vectorcraft_engine::tools::{Mods, PointerEvent, PointerKind};
use vectorcraft_engine::{Session, ViewInfo};
use vectorcraft_geom::{Affine, Anchor, PathData, Point, SubPath, Vec2};
use vectorcraft_render::{RenderOptions, Renderer};

/// Anchors are 10 pt apart and their handles 3 pt long, along the path.
const STEP: f64 = 10.0;
const HANDLE: f64 = 3.0;

/// A test document: its paths, an anchor to hover, click and drag, a point on a segment, and a
/// marquee from an empty point over about 10,000 anchors.
struct Shape {
    name: &'static str,
    doc: Document,
    ids: Vec<NodeId>,
    anchor: (NodeId, usize),
    segment: Point,
    marquee: (Point, Point),
}

/// A closed path of `n` smooth anchors around `center`.
fn circle(center: Point, n: usize) -> PathData {
    let r = n as f64 * STEP / std::f64::consts::TAU;
    let anchors = (0..n)
        .map(|i| {
            let t = i as f64 / n as f64 * std::f64::consts::TAU;
            let p = center + Vec2::new(r * t.cos(), r * t.sin());
            Anchor::smooth(p, p + Vec2::new(-t.sin(), t.cos()) * HANDLE)
        })
        .collect();
    PathData::single(SubPath::new(anchors, true))
}

fn add(doc: &mut Document, path: PathData) -> NodeId {
    let l = doc.layers.first().map(|l| l.id);
    let id = doc.alloc_id();
    doc.insert(l, usize::MAX, Node::path(id, path, Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0))).unwrap();
    id
}

fn one_outline() -> Shape {
    let n = 50_000;
    let mut doc = Document::new(1600.0, 1200.0);
    let id = add(&mut doc, circle(Point::ZERO, n));
    let r = n as f64 * STEP / std::f64::consts::TAU;
    let a = 72f64.to_radians();
    let mid = (n / 2) as f64 / n as f64 * std::f64::consts::TAU;
    let next = (n / 2 + 1) as f64 / n as f64 * std::f64::consts::TAU;
    let segment = Point::new(r * (mid.cos() + next.cos()) / 2.0, r * (mid.sin() + next.sin()) / 2.0);
    Shape {
        name: "one closed path, 50,000 anchors",
        doc,
        ids: vec![id],
        anchor: (id, n / 2),
        segment,
        marquee: (Point::new(r + 50.0, -5.0), Point::new(r * a.cos() - 1.0, r * a.sin() + 1.0)),
    }
}

fn many_paths() -> Shape {
    let mut doc = Document::new(1600.0, 1200.0);
    let ids: Vec<NodeId> =
        (0..2000).map(|i| add(&mut doc, circle(Point::new((i % 50) as f64 * 200.0 + 100.0, (i / 50) as f64 * 200.0 + 100.0), 50))).collect();
    let r = 50.0 * STEP / std::f64::consts::TAU;
    // Path 1,025 (row 20, column 25), its anchors 0 and 1.
    let c = Point::new(25.0 * 200.0 + 100.0, 20.0 * 200.0 + 100.0);
    let t = std::f64::consts::TAU / 50.0;
    let segment = c + Vec2::new(r * (1.0 + t.cos()) / 2.0, r * t.sin() / 2.0);
    Shape {
        name: "2,000 closed paths, 50 anchors each",
        doc,
        anchor: (ids[1025], 0),
        ids,
        segment,
        marquee: (Point::ZERO, Point::new(10_000.0, 800.0)),
    }
}

fn long_open_path() -> Shape {
    let n = 200_000;
    let mut doc = Document::new(1600.0, 1200.0);
    let anchors = (0..n)
        .map(|i| {
            let p = Point::new(i as f64 * STEP, 0.0);
            Anchor::smooth(p, p + Vec2::new(HANDLE, 0.0))
        })
        .collect();
    let id = add(&mut doc, PathData::single(SubPath::new(anchors, false)));
    Shape {
        name: "one open path, 200,000 anchors",
        doc,
        ids: vec![id],
        anchor: (id, n / 2),
        segment: Point::new((n / 2) as f64 * STEP + STEP / 2.0, 0.0),
        marquee: (Point::new(-5.0, -20.0), Point::new(99_995.0, 30.0)),
    }
}

fn iters() -> usize {
    std::env::var("ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(7).max(1)
}

fn median(mut ms: Vec<f64>) -> f64 {
    ms.sort_by(f64::total_cmp);
    ms[ms.len() / 2]
}

fn time(mut f: impl FnMut()) -> f64 {
    median(
        (0..iters())
            .map(|_| {
                let t = Instant::now();
                f();
                t.elapsed().as_secs_f64() * 1000.0
            })
            .collect(),
    )
}

fn ev(kind: PointerKind, p: Point) -> PointerEvent {
    PointerEvent::new(kind, p.x, p.y)
}

fn exec(s: &mut Session, cmd: &str, p: Value) {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"));
}

fn anchor_at(s: &Session, (id, ai): (NodeId, usize)) -> Anchor {
    s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().subpaths[0].anchors[ai]
}

/// The anchors selected: those direct-selected, and every anchor of a path selected whole.
fn selected_anchors(s: &Session) -> usize {
    let st = s.doc().unwrap();
    let count = |id: &NodeId| st.doc.node(*id).and_then(|n| n.path_data()).map_or(0, |pd| pd.anchors().count());
    st.selection.objects.iter().map(|id| st.selection.partial(*id).map_or_else(|| count(id), |set| set.len())).sum()
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn row(name: &str, ms: f64) {
    println!("  {name:<58} {ms:>10.3} ms");
}

/// Select `count` anchors: the first ones of the first path, or the first anchors of as many
/// paths as needed.
fn select_count(s: &mut Session, shape: &Shape, count: usize) {
    exec(s, "select.none", json!({}));
    let mut left = count;
    for id in &shape.ids {
        if left == 0 {
            break;
        }
        let n = s.doc().unwrap().doc.node(*id).unwrap().path_data().unwrap().subpaths[0].anchors.len();
        let take = left.min(n);
        let refs: Vec<[usize; 2]> = (0..take).map(|i| [0, i]).collect();
        exec(s, "select.anchors", json!({"id": id.0, "anchors": refs, "mode": "add"}));
        left -= take;
    }
}

fn bench(shape: Shape) {
    println!("{}", shape.name);
    let view = ViewInfo { smart_guides: false, ..ViewInfo::default() };
    let mut s = Session::new();
    s.add_document(shape.doc.clone(), None);
    s.select_tool("directSelection", view).unwrap();
    let a = anchor_at(&s, shape.anchor).p;
    let none = Mods::default();
    {
        let doc = s.doc().unwrap().doc.clone();
        let opt = vectorcraft_doc::hit::HitOptions { tol: 3.0, clip_outlines: true, ..Default::default() };
        row("hit test at an anchor", time(|| drop(black_box(vectorcraft_doc::hit::hit_test(&doc, a, opt)))));
        row("hit test at a segment", time(|| drop(black_box(vectorcraft_doc::hit::hit_test(&doc, shape.segment, opt)))));
        row("hit test beside the path", time(|| drop(black_box(vectorcraft_doc::hit::hit_test(&doc, shape.segment + Vec2::new(0.0, 40.0), opt)))));
    }

    // Hover with nothing selected, then with one anchor of the path selected.
    for selected in [false, true] {
        if selected {
            exec(&mut s, "select.anchors", json!({"id": shape.anchor.0.0, "anchors": [[0, shape.anchor.1]], "mode": "set"}));
        }
        let what = if selected { "one anchor selected" } else { "nothing selected" };
        row(&format!("hover over an anchor, {what}"), time(|| drop(s.pointer(&ev(PointerKind::Move, a), view).unwrap())));
        row(&format!("cursor over an anchor, {what}"), time(|| _ = black_box(s.cursor(a, none, view))));
        row(&format!("hover over a segment, {what}"), time(|| drop(s.pointer(&ev(PointerKind::Move, shape.segment), view).unwrap())));
        row(&format!("cursor over a segment, {what}"), time(|| _ = black_box(s.cursor(shape.segment, none, view))));
    }

    // A click that selects one anchor.
    exec(&mut s, "select.none", json!({}));
    row(
        "click that selects one anchor",
        time(|| {
            s.pointer(&ev(PointerKind::Down, a), view).unwrap();
            s.pointer(&ev(PointerKind::Up, a), view).unwrap();
        }),
    );
    assert_eq!(selected_anchors(&s), 1, "the click selects one anchor");

    // One anchor dragged: each pointer move, then the release.
    let undo = undo_len(&s);
    s.pointer(&ev(PointerKind::Down, a), view).unwrap();
    s.pointer(&ev(PointerKind::Drag, a + Vec2::new(0.0, 20.0)), view).unwrap();
    let mut y = 20.0;
    row(
        "anchor drag, one pointer move",
        time(|| {
            y += 1.0;
            s.pointer(&ev(PointerKind::Drag, a + Vec2::new(0.0, y)), view).unwrap();
        }),
    );
    let t = Instant::now();
    s.pointer(&ev(PointerKind::Up, a + Vec2::new(0.0, y)), view).unwrap();
    row("anchor drag, release (commit)", t.elapsed().as_secs_f64() * 1000.0);
    assert_eq!(undo_len(&s), undo + 1, "the anchor drag records one step");
    exec(&mut s, "edit.undo", json!({}));

    // The selected anchor's outgoing handle dragged.
    exec(&mut s, "select.anchors", json!({"id": shape.anchor.0.0, "anchors": [[0, shape.anchor.1]], "mode": "set"}));
    let h = anchor_at(&s, shape.anchor).h_out;
    let undo = undo_len(&s);
    s.pointer(&ev(PointerKind::Down, h), view).unwrap();
    s.pointer(&ev(PointerKind::Drag, h + Vec2::new(0.0, 20.0)), view).unwrap();
    let mut y = 20.0;
    row(
        "handle drag, one pointer move",
        time(|| {
            y += 1.0;
            s.pointer(&ev(PointerKind::Drag, h + Vec2::new(0.0, y)), view).unwrap();
        }),
    );
    let t = Instant::now();
    s.pointer(&ev(PointerKind::Up, h + Vec2::new(0.0, y)), view).unwrap();
    row("handle drag, release (commit)", t.elapsed().as_secs_f64() * 1000.0);
    assert_eq!(undo_len(&s), undo + 1, "the handle drag records one step");
    assert_eq!(anchor_at(&s, shape.anchor).p, a, "the handle drag leaves the anchor in place");
    exec(&mut s, "edit.undo", json!({}));

    // A marquee over about 10,000 anchors.
    let (m0, m1) = shape.marquee;
    let ms = time(|| {
        exec(&mut s, "select.none", json!({}));
        s.pointer(&ev(PointerKind::Down, m0), view).unwrap();
        s.pointer(&ev(PointerKind::Drag, m0.lerp(m1, 0.5)), view).unwrap();
        s.pointer(&ev(PointerKind::Drag, m1), view).unwrap();
        s.pointer(&ev(PointerKind::Up, m1), view).unwrap();
    });
    row(&format!("marquee over {} anchors (press to release)", selected_anchors(&s)), ms);

    // Select All with Direct Selection.
    row("Select All", time(|| exec(&mut s, "select.all", json!({}))));
    row("tool overlays, everything selected", time(|| drop(s.overlays(view))));

    // Nudges of 1, 1,000 and every anchor, and Undo and Redo of each.
    let total: usize = shape.ids.iter().map(|id| s.doc().unwrap().doc.node(*id).unwrap().path_data().unwrap().subpaths[0].anchors.len()).sum();
    for count in [1, 1000, total] {
        select_count(&mut s, &shape, count);
        row(&format!("nudge of {count} anchors"), time(|| exec(&mut s, "path.moveAnchors", json!({"dx": 1, "dy": 0}))));
        row(&format!("undo of a nudge of {count} anchors"), time(|| exec(&mut s, "edit.undo", json!({}))));
        row(&format!("redo of a nudge of {count} anchors"), time(|| exec(&mut s, "edit.redo", json!({}))));
    }

    // The render of the document after one anchor moves (the renderer warm).
    exec(&mut s, "select.anchors", json!({"id": shape.anchor.0.0, "anchors": [[0, shape.anchor.1]], "mode": "set"}));
    let opts = RenderOptions::default();
    let mut r = Renderer::new();
    let at = Affine::translate(Vec2::new(800.0 - a.x, 600.0 - a.y));
    r.render(&s.doc().unwrap().doc, 1600, 1200, at, &opts);
    let ms = median(
        (0..iters())
            .map(|_| {
                exec(&mut s, "path.moveAnchors", json!({"dx": 0, "dy": 1}));
                let doc = s.doc().unwrap().doc.clone();
                let t = Instant::now();
                drop(r.render(&doc, 1600, 1200, at, &opts));
                t.elapsed().as_secs_f64() * 1000.0
            })
            .collect(),
    );
    row("render 1600x1200 around the anchor after it moves", ms);
    let doc = s.doc().unwrap().doc.clone();
    row("render 1600x1200 around the anchor, nothing changed", time(|| drop(r.render(&doc, 1600, 1200, at, &opts))));

    // Delete of one anchor, undone after each run.
    let ms = median(
        (0..iters())
            .map(|_| {
                exec(&mut s, "select.anchors", json!({"id": shape.anchor.0.0, "anchors": [[0, shape.anchor.1]], "mode": "set"}));
                let t = Instant::now();
                exec(&mut s, "edit.clear", json!({}));
                let ms = t.elapsed().as_secs_f64() * 1000.0;
                exec(&mut s, "edit.undo", json!({}));
                ms
            })
            .collect(),
    );
    row("Delete of one anchor", ms);
}

fn main() {
    bench(one_outline());
    bench(many_paths());
    bench(long_open_path());
}
