//! The Selection tool's on-canvas widgets ask whether an object is editable only once its kind
//! can have the widget: on a locked artboard the question measures the art of the layer object
//! that holds the object.

use std::sync::Arc;

use serde_json::json;
use vectorcraft_doc::objects_on_path::{CENTER, ObjectsOnPath, place};
use vectorcraft_doc::pattern::RepeatSpec;
use vectorcraft_doc::{
    Appearance, Document, ImageObject, LiveShape, Node, NodeId, NodeKind, OrientedBox, Selection, TextKind, TextObject, locked_artboard_measures,
};
use vectorcraft_geom::{Affine, Point, Rect, shapes};

use crate::corners::CornerWidgets;
use crate::mirror::MirrorAxis;
use crate::objects_on_path::Widgets;
use crate::pie::PieWidget;
use crate::repeat_grid::GridHandles;
use crate::repeat_radial::RadialHandles;
use crate::select::hit_center;
use crate::sidewidget::SideWidget;
use crate::testutil::{cx, paint};
use crate::trace_split::TraceDivider;
use crate::typewidget::TypeWidget;

/// The page's left edge of the locked artboard: art placed this far right is on it.
const LOCKED_X: f64 = 2000.0;

/// A widget constructor: its name, whether it needs exactly one selected object, the name of an
/// object (in [`mixed`]) that has the widget, and what it gives for a selection and a pointer (as
/// text).
type Site = (&'static str, bool, &'static str, fn(&Document, &Selection, Point) -> String);

/// The center of the first object of `ids`: where the center widget is looked for.
fn pointer(doc: &Document, ids: &[NodeId]) -> Point {
    ids.first().and_then(|id| doc.node(*id)?.geometric_bounds_with(&doc.symbol_boxes())).map_or(Point::ZERO, |b| b.center())
}

fn center_at(doc: &Document, sel: &Selection, p: Point) -> String {
    let paint = paint();
    format!("{:?}", hit_center(&cx(doc, sel, &paint), p, 1.0))
}

const SITES: [Site; 11] = [
    ("type", true, "point type", |d, s, _| format!("{:?}", TypeWidget::of(d, s, &OrientedBox::aligned(Rect::new(0.0, 0.0, 10.0, 10.0)), 1.0))),
    ("pie", false, "ellipse", |d, s, _| format!("{:?}", PieWidget::of(d, s, 1.0))),
    ("objects on path", true, "objects on path", |d, s, _| format!("{:?}", Widgets::of(d, s, 1.0))),
    ("mirror", true, "mirror", |d, s, _| format!("{:?}", MirrorAxis::of(d, s, 1.0))),
    ("grid", true, "grid", |d, s, _| format!("{:?}", GridHandles::of(d, s, None, 1.0))),
    ("radial", true, "radial", |d, s, _| format!("{:?}", RadialHandles::of(d, s, None, 1.0))),
    ("sides", true, "polygon", |d, s, _| format!("{:?}", SideWidget::of(d, s, &OrientedBox::aligned(Rect::new(0.0, 0.0, 10.0, 10.0)), 1.0))),
    ("trace split", true, "trace", |d, s, _| format!("{:?}", TraceDivider::of(d, s, 1.0))),
    ("corners", false, "live rectangle", |d, s, _| format!("{:?}", CornerWidgets::of(d, s, 1.0, false))),
    ("corners of any path", false, "rectangle", |d, s, _| format!("{:?}", CornerWidgets::of(d, s, 1.0, true))),
    ("center", false, "ellipse", center_at),
];

fn with_live(id: NodeId, shape: LiveShape) -> Node {
    let mut n = Node::path(id, shape.to_path(), Appearance::default_art());
    if let NodeKind::Path { live: slot, .. } = &mut n.kind {
        *slot = Some(Box::new(shape));
    }
    n
}

fn rect(d: &mut Document, r: Rect) -> Node {
    Node::path(d.alloc_id(), shapes::rectangle(r), Appearance::default_art())
}

fn repeat(d: &mut Document, dx: f64, spec: fn(Vec<Arc<Node>>) -> RepeatSpec) -> Node {
    let source = rect(d, Rect::new(dx + 500.0, 500.0, dx + 540.0, 540.0));
    let mut n = rect(d, Rect::new(dx + 500.0, 500.0, dx + 540.0, 540.0));
    n.kind = NodeKind::Repeat(spec(vec![Arc::new(source)]));
    n
}

/// The objects every widget is asked about, by name, placed `dx` right of the page's origin. `big`
/// is a group of that many rectangles.
fn objects(d: &mut Document, dx: f64, big: usize) -> Vec<(&'static str, Node)> {
    let mut out = vec![("rectangle", rect(d, Rect::new(dx + 50.0, 50.0, dx + 150.0, 150.0)))];
    let mut centered = rect(d, Rect::new(dx + 50.0, 200.0, dx + 150.0, 260.0));
    centered.edit_attrs(|a| a.show_center = Some(true));
    out.push(("rectangle with its center shown", centered));
    let ellipse = |d: &mut Document, y: f64| {
        with_live(d.alloc_id(), LiveShape::Ellipse { w: 120.0, h: 60.0, pie: (0.0, 90.0), xf: Affine::translate((dx + 200.0, y)) })
    };
    out.push(("ellipse", ellipse(d, 50.0)));
    let mut hidden_center = ellipse(d, 150.0);
    hidden_center.edit_attrs(|a| a.show_center = Some(false));
    out.push(("ellipse with its center hidden", hidden_center));
    let polygon = LiveShape::Polygon {
        radius: 50.0,
        sides: 6,
        xf: Affine::translate((dx + 400.0, 100.0)),
        radii: vec![],
        kinds: vec![],
        radii_relative: false,
    };
    out.push(("polygon", with_live(d.alloc_id(), polygon)));
    let live_rect = LiveShape::Rectangle {
        w: 100.0,
        h: 80.0,
        radii: [0.0; 4],
        kinds: Default::default(),
        radii_relative: false,
        xf: Affine::translate((dx + 50.0, 300.0)),
    };
    out.push(("live rectangle", with_live(d.alloc_id(), live_rect)));
    let point = TextObject::point(Point::new(dx + 300.0, 300.0), "Point", Default::default());
    out.push(("point type", Node::new(d.alloc_id(), NodeKind::Text(Box::new(point)))));
    let mut area = TextObject::point(Point::new(dx + 300.0, 400.0), "Area", Default::default());
    area.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, 120.0, 40.0)) };
    out.push(("area type", Node::new(d.alloc_id(), NodeKind::Text(Box::new(area)))));
    let line = Node::path(d.alloc_id(), shapes::line(Point::new(dx, 700.0), Point::new(dx + 400.0, 700.0)), Appearance::default_art());
    let (a, b) = (rect(d, Rect::new(dx + 50.0, 600.0, dx + 70.0, 620.0)), rect(d, Rect::new(dx + 100.0, 600.0, dx + 120.0, 620.0)));
    let rec = ObjectsOnPath::new(line.id, &[&a, &b], CENTER, 0.0);
    let mut on_path = Node::new(d.alloc_id(), NodeKind::Group { children: vec![Arc::new(line), Arc::new(a), Arc::new(b)], clip: false });
    on_path.extras_mut().objects_on_path = Some(Box::new(rec));
    place(&mut on_path, &Document::new(1.0, 1.0).symbol_boxes());
    out.push(("objects on path", on_path));
    out.push(("mirror", repeat(d, dx, |s| RepeatSpec::mirror(s, 90.0, 10.0))));
    out.push(("grid", repeat(d, dx, |s| RepeatSpec::grid(s, 20.0, 20.0))));
    out.push(("radial", repeat(d, dx, |s| RepeatSpec::radial(s, 8, Some(100.0)))));
    let im = ImageObject {
        key: "k".into(),
        width: 100,
        height: 50,
        xf: Affine::translate((dx + 600.0, 600.0)),
        link: None,
        placement: Default::default(),
        pixels: None,
    };
    let source = Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)));
    let mut traced = Node::group(d.alloc_id(), vec![Arc::new(source)]);
    traced.extras_mut().trace = Some(Box::new(json!({"view": "split", "split": 0.5})));
    out.push(("trace", traced));
    out.push(("big group", grid_of_rects(d, Point::new(dx + 700.0, 50.0), big, 50)));
    out
}

/// A group of `count` 3 × 3 rectangles, `per_row` to a row, 5 apart, from `at`.
fn grid_of_rects(d: &mut Document, at: Point, count: usize, per_row: usize) -> Node {
    let members = (0..count)
        .map(|i| {
            let (x, y) = (at.x + (i % per_row) as f64 * 5.0, at.y + (i / per_row) as f64 * 5.0);
            Arc::new(rect(d, Rect::new(x, y, x + 3.0, y + 3.0)))
        })
        .collect();
    Node::group(d.alloc_id(), members)
}

fn is_empty(widget: &str) -> bool {
    matches!(widget, "None" | "[]") || widget == format!("{:?}", Widgets::default())
}

/// A document with one artboard that is not locked and, at [`LOCKED_X`], one that is, holding
/// [`objects`] on each, plus an ellipse on a locked layer and a polygon on a hidden one. Each
/// object's name, whether it is on the locked artboard, and its id.
fn mixed(big: usize) -> (Document, Vec<(&'static str, bool, NodeId)>) {
    let mut d = Document::new(1000.0, 1000.0);
    let mut board = d.artboards[0].clone();
    board.rect = Rect::new(LOCKED_X, 0.0, LOCKED_X + 1000.0, 1000.0);
    board.locked = true;
    d.artboards.push(board);
    let l = d.layers[0].id;
    let mut ids = vec![];
    for (dx, on) in [(0.0, false), (LOCKED_X, true)] {
        for (name, n) in objects(&mut d, dx, big) {
            ids.push((name, on, n.id));
            d.insert(Some(l), 0, n).unwrap();
        }
    }
    for (name, hide) in [("ellipse", false), ("polygon", true)] {
        let layer = d.add_layer(None);
        let n = objects(&mut d, 0.0, 0).into_iter().find(|(o, _)| *o == name).map(|(_, n)| n).unwrap();
        ids.push((name, false, n.id));
        d.insert(Some(layer), 0, n).unwrap();
        let layer = d.node_mut(layer).unwrap();
        if hide {
            layer.visible = false;
        } else {
            layer.locked = true;
        }
    }
    (d, ids)
}

fn selection(ids: &[NodeId]) -> Selection {
    let mut s = Selection::default();
    s.set(ids.iter().copied());
    s
}

/// `doc` with every artboard and layer unlocked and every layer shown: everything in it is
/// editable.
fn all_editable(doc: &Document) -> Document {
    let mut free = doc.clone();
    for a in &mut free.artboards {
        a.locked = false;
    }
    for l in doc.layers.iter().map(|l| l.id) {
        let n = free.node_mut(l).unwrap();
        (n.visible, n.locked) = (true, false);
    }
    free
}

/// Every widget gives what it gave when it asked about editability first: for an editable object,
/// what it gives with everything editable, and for one that is not, what it gives without it.
#[test]
fn the_widgets_are_the_same_as_when_editability_came_first() {
    let (doc, ids) = mixed(20);
    let free = all_editable(&doc);
    let all: Vec<NodeId> = ids.iter().map(|o| o.2).collect();
    let mut selections: Vec<Vec<NodeId>> = all.iter().map(|id| vec![*id]).collect();
    selections.extend(all.windows(2).map(<[NodeId]>::to_vec));
    selections.extend([all.clone(), all.iter().rev().copied().collect(), vec![], vec![NodeId(u64::MAX)]]);
    let editable = doc.editable();
    for ids in &selections {
        let sel = selection(ids);
        let kept = selection(&ids.iter().copied().filter(|id| editable(*id)).collect::<Vec<_>>());
        let p = pointer(&doc, ids);
        for (name, single, _, f) in SITES {
            // A widget of one object only shows for a selection of one, editable or not.
            let old = f(&free, if single && ids.len() != 1 { &sel } else { &kept }, p);
            assert_eq!(f(&doc, &sel, p), old, "{name} of {ids:?}");
        }
    }
    // Each widget shows for an object of its kind off the locked artboard and not on it, so the
    // comparison is not of empty results alone.
    for (name, _, has, f) in SITES {
        for on in [false, true] {
            let (_, _, id) = ids.iter().find(|o| o.0 == has && o.1 == on).unwrap();
            let w = f(&doc, &selection(&[*id]), pointer(&doc, &[*id]));
            assert_eq!(is_empty(&w), on, "{name} of the {has} (on the locked artboard: {on}): {w}");
        }
    }
}

/// An object of another kind is not measured for the locked artboard, alone or with others; an
/// object of the widget's kind still is, and its widget is left out.
#[test]
fn widgets_measure_no_art_for_objects_of_other_kinds() {
    let (doc, ids) = mixed(500);
    let id = |name: &str, on: bool| ids.iter().find(|o| o.0 == name && o.1 == on).map(|o| o.2).unwrap();
    let big = id("big group", true);
    let others = [big, id("rectangle", true), id("area type", true), id("trace", true)];
    let mut measured = vec![];
    for (name, single, has, f) in SITES {
        let several: Vec<NodeId> = others.iter().copied().filter(|o| *o != id(has, true)).collect();
        for ids in [vec![big], several] {
            if single && ids.len() != 1 {
                continue;
            }
            let before = locked_artboard_measures();
            std::hint::black_box(f(&doc, &selection(&ids), pointer(&doc, &ids)));
            if locked_artboard_measures() != before {
                measured.push(format!("{name} of {ids:?}"));
            }
        }
        let before = locked_artboard_measures();
        std::hint::black_box(f(&doc, &selection(&[id(has, true)]), pointer(&doc, &[id(has, true)])));
        assert!(locked_artboard_measures() > before, "{name} of a {has} on the locked artboard");
    }
    assert_eq!(measured, Vec::<String>::new());
}

fn thread_cpu() -> Option<std::time::Duration> {
    let stat = std::fs::read_to_string("/proc/thread-self/schedstat").ok()?;
    stat.split_whitespace().next()?.parse().ok().map(std::time::Duration::from_nanos)
}

/// The time `f` takes: this thread's CPU time where Linux reports it, otherwise the wall clock.
fn cpu_time(f: impl FnOnce()) -> std::time::Duration {
    match thread_cpu() {
        Some(before) => {
            f();
            thread_cpu().map_or(std::time::Duration::ZERO, |after| after.saturating_sub(before))
        }
        None => {
            let start = std::time::Instant::now();
            f();
            start.elapsed()
        }
    }
}

/// Timings of each widget for a group of 5,000 rectangles and for 128 rectangles, selected on a
/// locked artboard and with no artboard locked, 100 calls a run: with the editability check first
/// (the selection's editable objects are kept, then the widget is asked about them in the document
/// with everything editable, as in [`the_widgets_are_the_same_as_when_editability_came_first`]),
/// and as the widgets are now. 7 runs each, in turn. Ignored by default; it prints the median per
/// call and sets no bound on it:
/// `cargo test --release -p vectorcraft-tools --lib widget_order_perf -- --ignored --nocapture`.
#[test]
#[ignore = "a benchmark: run with --ignored --nocapture"]
fn widget_order_perf() {
    const RUNS: usize = 7;
    const CALLS: usize = 100;
    eprintln!("widget_order_perf: {}", if thread_cpu().is_some() { "thread CPU time" } else { "wall clock" });
    let (mut locked, _) = mixed(0);
    let l = locked.layers[0].id;
    let group = grid_of_rects(&mut locked, Point::new(LOCKED_X, 0.0), 5000, 100);
    let group_id = group.id;
    locked.insert(Some(l), 0, group).unwrap();
    let mut rects = vec![];
    for i in 0..128 {
        let (x, y) = (LOCKED_X + (i % 16) as f64 * 60.0, 600.0 + (i / 16) as f64 * 40.0);
        let n = rect(&mut locked, Rect::new(x, y, x + 30.0, y + 30.0));
        rects.push(n.id);
        locked.insert(Some(l), 0, n).unwrap();
    }
    let unlocked = all_editable(&locked);
    for (doc, board) in [(&locked, "locked artboard"), (&unlocked, "no artboard locked")] {
        for (what, ids) in [("a group of 5,000", vec![group_id]), ("128 rectangles", rects.clone())] {
            timings(doc, &selection(&ids), &format!("{what}, {board}"));
        }
    }

    fn timings(doc: &Document, sel: &Selection, case: &str) {
        // The widgets' own editability checks find nothing locked here, so each object's
        // editability is measured once, as when it came first.
        let free = all_editable(doc);
        for (name, single, _, f) in SITES {
            if single && sel.objects.len() != 1 {
                continue;
            }
            let p = pointer(doc, &sel.objects);
            let before = || {
                for _ in 0..CALLS {
                    let editable = doc.editable();
                    let kept = selection(&sel.objects.iter().copied().filter(|id| editable(*id)).collect::<Vec<_>>());
                    std::hint::black_box(f(std::hint::black_box(&free), &kept, p));
                }
            };
            let after = || {
                for _ in 0..CALLS {
                    std::hint::black_box(f(std::hint::black_box(doc), sel, p));
                }
            };
            let (mut old, mut new) = (vec![], vec![]);
            for run in 0..RUNS {
                // The two take turns going first.
                if run % 2 == 0 {
                    old.push(cpu_time(before));
                    new.push(cpu_time(after));
                } else {
                    new.push(cpu_time(after));
                    old.push(cpu_time(before));
                }
            }
            old.sort();
            new.sort();
            let us = |d: std::time::Duration| d.as_secs_f64() * 1e6 / CALLS as f64;
            eprintln!("{name}, {case}: {:.2} µs a call with the check first, {:.2} µs without", us(old[RUNS / 2]), us(new[RUNS / 2]));
        }
    }
}
